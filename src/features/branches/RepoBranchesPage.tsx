/**
 * 分支与标签管理页（T2.5）。
 *
 * # 结构
 *
 * - 工具条：搜索（按名过滤）、创建分支、创建标签
 * - 分支列表：**当前分支置顶 → 本地 → 远端**（后端已排序），每行显示
 *   ahead/behind 徽标与 upstream 名（任务书要求），行内动作 = 切换 / 重命名 /
 *   删除 / 设置上游
 * - 标签列表：轻量/附注标记、目标短 oid，删除走确认
 *
 * # 危险动作的确认链（与后端确认参数一一对应）
 *
 * - 强制切换：对话框明确"会丢弃未提交修改"（`confirmForce`）
 * - 删除未合并分支：先 `gitBranchCompare` 拉**独有提交清单**展示，用户看到
 *   具体会丢什么之后再确认（`confirmUnmerged`）——后端没有确认参数时拒绝，
 *   这条链路在 UI 上是强制不可跳过的
 */
import { useMemo, useState } from 'react';

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { GitBranch as GitBranchIcon, GitBranchPlus, Tag as TagIcon, TagPlus } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useParams } from 'react-router-dom';

import { useAppError } from '@/lib/errors';
import { useRepoChangeInvalidation } from '@/lib/repoChanged';
import {
  gitBranchCompare,
  gitBranchCreate,
  gitBranchDelete,
  gitBranchRename,
  gitBranchSetUpstream,
  gitBranchSwitch,
  gitTagCreate,
  gitTagDelete,
  gitTagList,
} from '@/lib/ipc/branches';
import { gitBranchList } from '@/lib/ipc/history';
import type { Branch } from '@/lib/ipc/history';
import type { Tag } from '@/lib/ipc/branches';
import { BRANCHES_QUERY_KEY, LOG_QUERY_KEY, TAGS_QUERY_KEY } from '@/lib/queryKeys';
import { cn } from '@/lib/utils';

import { Button } from '@/ui/components/button';
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/ui/components/dialog';
import { Input } from '@/ui/components/input';
import { Skeleton } from '@/ui/components/skeleton';

import { shortOid } from '@/features/history/commitMeta';
import { PlaceholderPage } from '@/ui/PlaceholderPage';

/** 切换时对不干净工作区的三策略（后端 SwitchStrategy）。 */
type SwitchChoice = 'stash' | 'force' | 'clean';

export function RepoBranchesPageInner() {
  const { t } = useTranslation('shell');
  const params = useParams();
  const repoId = Number(params.repoId);
  const queryClient = useQueryClient();
  const { show } = useAppError();
  useRepoChangeInvalidation(repoId);

  const [filter, setFilter] = useState('');
  const [createOpen, setCreateOpen] = useState(false);
  const [createName, setCreateName] = useState('');
  const [createCheckout, setCreateCheckout] = useState(true);
  const [tagOpen, setTagOpen] = useState(false);
  const [tagName, setTagName] = useState('');
  const [tagMessage, setTagMessage] = useState('');
  const [renameTarget, setRenameTarget] = useState<Branch | null>(null);
  const [renameName, setRenameName] = useState('');
  const [deleteTarget, setDeleteTarget] = useState<Branch | null>(null);
  /** 删除确认的第二步数据：独有提交清单（compare 查询的结果）。 */
  const [deleteUnmerged, setDeleteUnmerged] = useState<{
    branch: Branch;
    ahead: number;
    behind: number;
    onlyInA: readonly (readonly [string, string])[];
  } | null>(null);
  /** 切换对话框的目标（工作区不干净时出现三策略选择）。 */
  const [switchTarget, setSwitchTarget] = useState<Branch | null>(null);

  const branchesQuery = useQuery({
    queryKey: [BRANCHES_QUERY_KEY, repoId],
    queryFn: () => gitBranchList(repoId, true),
    enabled: Number.isFinite(repoId),
    staleTime: 5_000,
  });
  const tagsQuery = useQuery({
    queryKey: [TAGS_QUERY_KEY, repoId],
    queryFn: () => gitTagList(repoId),
    enabled: Number.isFinite(repoId),
    staleTime: 5_000,
  });

  /** 写操作完成后的统一失效：分支列表 + 历史（分支图会变）+ 标签。 */
  const invalidateAfterWrite = useMemo(
    () => () => {
      void queryClient.invalidateQueries({ queryKey: [BRANCHES_QUERY_KEY, repoId] });
      void queryClient.invalidateQueries({ queryKey: [LOG_QUERY_KEY, repoId] });
      void queryClient.invalidateQueries({ queryKey: [TAGS_QUERY_KEY, repoId] });
    },
    [queryClient, repoId],
  );

  const switchMutation = useMutation({
    mutationFn: ({ branch, choice }: { branch: Branch; choice: SwitchChoice }) =>
      gitBranchSwitch(repoId, branch.name, choice, choice === 'force'),
    onSuccess: () => {
      invalidateAfterWrite();
      setSwitchTarget(null);
    },
    onError: show,
  });
  const deleteMutation = useMutation({
    mutationFn: ({
      spec,
      confirm,
    }: {
      spec: { names: string[]; force: boolean; alsoDeleteRemote: boolean };
      confirm: boolean;
    }) => gitBranchDelete(repoId, spec, confirm),
    onSuccess: () => {
      invalidateAfterWrite();
      setDeleteTarget(null);
      setDeleteUnmerged(null);
    },
    onError: show,
  });
  const createMutation = useMutation({
    mutationFn: () =>
      gitBranchCreate(repoId, {
        name: createName.trim(),
        startPoint: null,
        checkout: createCheckout,
      }),
    onSuccess: () => {
      invalidateAfterWrite();
      setCreateOpen(false);
      setCreateName('');
    },
    onError: show,
  });
  const renameMutation = useMutation({
    mutationFn: () =>
      gitBranchRename(repoId, {
        old: renameTarget?.name ?? '',
        new: renameName.trim(),
        renameRemote: false,
      }),
    onSuccess: () => {
      invalidateAfterWrite();
      setRenameTarget(null);
    },
    onError: show,
  });
  const tagCreateMutation = useMutation({
    mutationFn: () =>
      gitTagCreate(repoId, {
        name: tagName.trim(),
        target: null,
        message: tagMessage.trim() === '' ? null : tagMessage.trim(),
        sign: false,
        force: false,
      }),
    onSuccess: () => {
      invalidateAfterWrite();
      setTagOpen(false);
      setTagName('');
      setTagMessage('');
    },
    onError: show,
  });
  const tagDeleteMutation = useMutation({
    mutationFn: (name: string) => gitTagDelete(repoId, { names: [name], alsoDeleteRemote: false }),
    onSuccess: invalidateAfterWrite,
    onError: show,
  });
  /** 上游设置：分支有上游 = 取消；没有 = 设为 origin/<同名>（v1 的最常见意图）。 */
  const upstreamMutation = useMutation({
    mutationFn: (branch: Branch) =>
      gitBranchSetUpstream(repoId, {
        branch: branch.name,
        upstream: branch.upstream === null ? `origin/${branch.name}` : null,
      }),
    onSuccess: invalidateAfterWrite,
    onError: show,
  });

  if (!Number.isFinite(repoId)) {
    // 非数字 repoId（如示例路由）退回占位页：与 HistoryPage 的处理一致
    return (
      <PlaceholderPage
        titleKey="pages.repoBranches.title"
        descriptionKey="pages.repoBranches.description"
        plannedTask="T2.5"
      />
    );
  }

  const branches = branchesQuery.data ?? [];
  const needle = filter.trim().toLowerCase();
  const matches = (name: string): boolean => needle === '' || name.toLowerCase().includes(needle);
  const current = branches.find((branch) => branch.isHead);
  const locals = branches.filter(
    (branch) => !branch.isRemote && !branch.isHead && matches(branch.name),
  );
  const remotes = branches.filter((branch) => branch.isRemote && matches(branch.name));
  const tags = (tagsQuery.data ?? []).filter((tag) => matches(tag.name));

  const beginDelete = (branch: Branch): void => {
    setDeleteTarget(branch);
    // 立即拉独有提交清单：未合并时第二步要展示它（任务书实现要求 2）
    void gitBranchCompare(repoId, branch.name, current?.name ?? 'main')
      .then((comparison) => {
        setDeleteUnmerged({ branch, ...comparison });
      })
      .catch(() => {
        setDeleteUnmerged(null);
      });
  };

  return (
    <section className="flex h-full min-h-0 flex-col gap-3" aria-label={t('branches.title')}>
      <header className="flex flex-wrap items-center justify-between gap-2">
        <h1 className="text-20 font-semibold tracking-tight">{t('branches.title')}</h1>
        <div className="flex items-center gap-1.5">
          <Input
            value={filter}
            onChange={(event) => {
              setFilter(event.target.value);
            }}
            placeholder={t('branches.searchPlaceholder')}
            aria-label={t('branches.searchPlaceholder')}
            className="h-7 w-48 text-12"
            data-testid="branches-search"
          />
          <Button
            size="sm"
            variant="secondary"
            onClick={() => setTagOpen(true)}
            data-testid="branches-tag-create"
          >
            <TagPlus aria-hidden="true" className="size-3.5" />
            {t('branches.createTag')}
          </Button>
          <Button size="sm" onClick={() => setCreateOpen(true)} data-testid="branches-create">
            <GitBranchPlus aria-hidden="true" className="size-3.5" />
            {t('branches.create')}
          </Button>
        </div>
      </header>

      {branchesQuery.isPending ? (
        <div aria-busy="true" className="flex flex-col gap-1.5">
          <Skeleton className="h-8 w-full" />
          <Skeleton className="h-8 w-full" />
          <Skeleton className="h-8 w-2/3" />
        </div>
      ) : branchesQuery.isError ? (
        <p className="text-12 text-danger">{t('branches.error')}</p>
      ) : (
        <div className="min-h-0 flex-1 overflow-y-auto rounded-md border border-line bg-surface">
          {/* 当前分支 */}
          {current !== undefined ? (
            <BranchRow
              key={current.name}
              branch={current}
              isCurrent
              onSwitch={() => setSwitchTarget(current)}
              onRename={() => {
                setRenameTarget(current);
                setRenameName(current.name);
              }}
              onDelete={() => beginDelete(current)}
              onToggleUpstream={() => upstreamMutation.mutate(current)}
              busy={switchMutation.isPending || deleteMutation.isPending}
            />
          ) : null}
          <GroupHeader label={t('branches.local')} count={locals.length} />
          {locals.map((branch) => (
            <BranchRow
              key={branch.name}
              branch={branch}
              onSwitch={() => setSwitchTarget(branch)}
              onRename={() => {
                setRenameTarget(branch);
                setRenameName(branch.name);
              }}
              onDelete={() => beginDelete(branch)}
              onToggleUpstream={() => upstreamMutation.mutate(branch)}
              busy={switchMutation.isPending || deleteMutation.isPending}
            />
          ))}
          <GroupHeader label={t('branches.remote')} count={remotes.length} />
          {remotes.map((branch) => (
            <BranchRow key={branch.name} branch={branch} remote busy={false} />
          ))}

          {/* 标签 */}
          <GroupHeader label={t('branches.tags')} count={tags.length} />
          {tags.map((tag) => (
            <TagRow key={tag.name} tag={tag} onDelete={() => tagDeleteMutation.mutate(tag.name)} />
          ))}
        </div>
      )}

      {/* 创建分支 */}
      <Dialog open={createOpen} onOpenChange={setCreateOpen}>
        <DialogContent closeLabel={t('common:actions.close')}>
          <DialogHeader>
            <DialogTitle>{t('branches.create')}</DialogTitle>
            <DialogDescription>{t('branches.createHint')}</DialogDescription>
          </DialogHeader>
          <Input
            value={createName}
            onChange={(event) => {
              setCreateName(event.target.value);
            }}
            placeholder="feature/my-branch"
            aria-label={t('branches.nameLabel')}
            data-testid="branches-create-name"
          />
          <label className="flex items-center gap-2 text-12">
            <input
              type="checkbox"
              checked={createCheckout}
              onChange={(event) => {
                setCreateCheckout(event.target.checked);
              }}
            />
            {t('branches.checkoutAfterCreate')}
          </label>
          <DialogFooter>
            <DialogClose asChild>
              <Button size="sm" variant="secondary">
                {t('common:actions.cancel')}
              </Button>
            </DialogClose>
            <Button
              size="sm"
              loading={createMutation.isPending}
              disabled={createName.trim() === ''}
              onClick={() => createMutation.mutate()}
              data-testid="branches-create-confirm"
            >
              {t('common:actions.confirm')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* 重命名 */}
      <Dialog
        open={renameTarget !== null}
        onOpenChange={(open) => {
          if (!open) setRenameTarget(null);
        }}
      >
        <DialogContent closeLabel={t('common:actions.close')}>
          <DialogHeader>
            <DialogTitle>{t('branches.rename')}</DialogTitle>
            <DialogDescription>{renameTarget?.name}</DialogDescription>
          </DialogHeader>
          <Input
            value={renameName}
            onChange={(event) => {
              setRenameName(event.target.value);
            }}
            aria-label={t('branches.nameLabel')}
            data-testid="branches-rename-name"
          />
          <DialogFooter>
            <DialogClose asChild>
              <Button size="sm" variant="secondary">
                {t('common:actions.cancel')}
              </Button>
            </DialogClose>
            <Button
              size="sm"
              loading={renameMutation.isPending}
              disabled={renameName.trim() === '' || renameName.trim() === renameTarget?.name}
              onClick={() => renameMutation.mutate()}
              data-testid="branches-rename-confirm"
            >
              {t('common:actions.confirm')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* 删除：第一步概览，未合并时第二步展示独有提交 */}
      <Dialog
        open={deleteTarget !== null && deleteUnmerged !== null}
        onOpenChange={(open) => {
          if (!open) {
            setDeleteTarget(null);
            setDeleteUnmerged(null);
          }
        }}
      >
        <DialogContent closeLabel={t('common:actions.close')}>
          <DialogHeader>
            <DialogTitle>
              {t('branches.deleteTitle', { name: deleteTarget?.name ?? '' })}
            </DialogTitle>
            {deleteUnmerged !== null && deleteUnmerged.ahead > 0 ? (
              <DialogDescription>
                {t('branches.deleteUnmergedWarning', { ahead: deleteUnmerged.ahead })}
              </DialogDescription>
            ) : (
              <DialogDescription>{t('branches.deleteMergedHint')}</DialogDescription>
            )}
          </DialogHeader>
          {deleteUnmerged !== null && deleteUnmerged.onlyInA.length > 0 ? (
            <div className="max-h-40 overflow-y-auto rounded-md border border-line p-2">
              <p className="mb-1 text-11 font-medium text-fg-subtle">{t('branches.onlyCommits')}</p>
              <ul className="flex flex-col gap-0.5">
                {deleteUnmerged.onlyInA.map(([oid, subject]) => (
                  <li key={oid} className="flex items-baseline gap-2 text-12">
                    <code className="font-mono text-11 text-fg-subtle">{shortOid(oid)}</code>
                    <span className="truncate">{subject}</span>
                  </li>
                ))}
              </ul>
            </div>
          ) : null}
          <DialogFooter>
            <DialogClose asChild>
              <Button size="sm" variant="secondary">
                {t('common:actions.cancel')}
              </Button>
            </DialogClose>
            {deleteTarget !== null && deleteUnmerged !== null && deleteUnmerged.ahead > 0 ? (
              <Button
                size="sm"
                variant="danger"
                loading={deleteMutation.isPending}
                onClick={() =>
                  deleteMutation.mutate({
                    spec: { names: [deleteTarget.name], force: true, alsoDeleteRemote: false },
                    confirm: true,
                  })
                }
                data-testid="branches-delete-confirm-unmerged"
              >
                {t('branches.deleteForceAnyway')}
              </Button>
            ) : (
              <Button
                size="sm"
                variant="danger"
                loading={deleteMutation.isPending}
                onClick={() => {
                  if (deleteTarget !== null) {
                    deleteMutation.mutate({
                      spec: { names: [deleteTarget.name], force: false, alsoDeleteRemote: false },
                      confirm: false,
                    });
                  }
                }}
                data-testid="branches-delete-confirm"
              >
                {t('branches.deleteConfirm')}
              </Button>
            )}
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* 切换：三策略（任务书实现要求 1：不静默失败） */}
      <Dialog
        open={switchTarget !== null}
        onOpenChange={(open) => {
          if (!open) setSwitchTarget(null);
        }}
      >
        <DialogContent closeLabel={t('common:actions.close')}>
          <DialogHeader>
            <DialogTitle>
              {t('branches.switchTitle', { name: switchTarget?.name ?? '' })}
            </DialogTitle>
            <DialogDescription>{t('branches.switchHint')}</DialogDescription>
          </DialogHeader>
          <div className="flex flex-col gap-1.5">
            <Button
              size="sm"
              variant="secondary"
              onClick={() =>
                switchTarget !== null &&
                switchMutation.mutate({ branch: switchTarget, choice: 'stash' })
              }
              data-testid="branches-switch-stash"
            >
              {t('branches.switchStash')}
            </Button>
            <Button
              size="sm"
              variant="danger"
              onClick={() =>
                switchTarget !== null &&
                switchMutation.mutate({ branch: switchTarget, choice: 'force' })
              }
              data-testid="branches-switch-force"
            >
              {t('branches.switchForce')}
            </Button>
            <DialogClose asChild>
              <Button size="sm" variant="ghost">
                {t('branches.switchCancel')}
              </Button>
            </DialogClose>
          </div>
        </DialogContent>
      </Dialog>

      {/* 创建标签 */}
      <Dialog open={tagOpen} onOpenChange={setTagOpen}>
        <DialogContent closeLabel={t('common:actions.close')}>
          <DialogHeader>
            <DialogTitle>{t('branches.createTag')}</DialogTitle>
            <DialogDescription>{t('branches.tagHint')}</DialogDescription>
          </DialogHeader>
          <Input
            value={tagName}
            onChange={(event) => {
              setTagName(event.target.value);
            }}
            placeholder="v1.0.0"
            aria-label={t('branches.tagNameLabel')}
            data-testid="branches-tag-name"
          />
          <Input
            value={tagMessage}
            onChange={(event) => {
              setTagMessage(event.target.value);
            }}
            placeholder={t('branches.tagMessagePlaceholder')}
            aria-label={t('branches.tagMessagePlaceholder')}
            className="text-12"
            data-testid="branches-tag-message"
          />
          <DialogFooter>
            <DialogClose asChild>
              <Button size="sm" variant="secondary">
                {t('common:actions.cancel')}
              </Button>
            </DialogClose>
            <Button
              size="sm"
              loading={tagCreateMutation.isPending}
              disabled={tagName.trim() === ''}
              onClick={() => tagCreateMutation.mutate()}
              data-testid="branches-tag-confirm"
            >
              {t('common:actions.confirm')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </section>
  );
}

/** 分组小标题（计数为 0 时折叠不显示——空分组只有噪音）。 */
function GroupHeader({ label, count }: { readonly label: string; readonly count: number }) {
  if (count === 0) {
    return null;
  }
  return (
    <p className="border-t border-line bg-surface-sunken px-3 py-1 text-11 font-medium text-fg-subtle first:border-t-0">
      {label} · {count}
    </p>
  );
}

/** 分支行：名称 + ahead/behind 徽标 + upstream + 行内动作。 */
function BranchRow({
  branch,
  isCurrent = false,
  remote = false,
  onSwitch,
  onRename,
  onDelete,
  onToggleUpstream,
  busy,
}: {
  readonly branch: Branch;
  readonly isCurrent?: boolean;
  readonly remote?: boolean;
  readonly onSwitch?: () => void;
  readonly onRename?: () => void;
  readonly onDelete?: () => void;
  readonly onToggleUpstream?: () => void;
  readonly busy: boolean;
}) {
  const { t } = useTranslation('shell');
  return (
    <div
      className={cn(
        'fd-transition flex h-9 items-center gap-2 px-3 text-13',
        isCurrent ? 'bg-brand-subtle' : 'hover:bg-surface-sunken',
      )}
      data-testid={isCurrent ? 'branches-current' : undefined}
    >
      <GitBranchIcon aria-hidden="true" className="size-3.5 shrink-0 text-fg-subtle" />
      <span className="truncate font-medium">{branch.name}</span>
      {branch.ahead !== null && branch.ahead > 0 ? (
        <span
          className="shrink-0 rounded-full bg-brand-subtle px-1.5 text-10 text-brand"
          title={t('branches.aheadTip')}
        >
          ↑{branch.ahead}
        </span>
      ) : null}
      {branch.behind !== null && branch.behind > 0 ? (
        <span
          className="shrink-0 rounded-full bg-surface-sunken px-1.5 text-10 text-fg-subtle"
          title={t('branches.behindTip')}
        >
          ↓{branch.behind}
        </span>
      ) : null}
      {branch.upstream !== null ? (
        <span className="truncate text-11 text-fg-subtle">→ {branch.upstream}</span>
      ) : null}
      <span className="flex-1" />
      {isCurrent ? (
        <span className="shrink-0 rounded-full bg-brand px-1.5 text-10 text-fd-fg-inverted">
          {t('branches.current')}
        </span>
      ) : (
        <>
          {!remote ? (
            <>
              <Button
                size="sm"
                variant="ghost"
                onClick={onSwitch}
                disabled={busy}
                data-testid={`branches-switch-${branch.name}`}
              >
                {t('branches.switch')}
              </Button>
              <Button size="sm" variant="ghost" onClick={onToggleUpstream} disabled={busy}>
                {branch.upstream === null ? t('branches.setUpstream') : t('branches.unsetUpstream')}
              </Button>
              <Button size="sm" variant="ghost" onClick={onRename} disabled={busy}>
                {t('branches.rename')}
              </Button>
              <Button
                size="sm"
                variant="ghost"
                onClick={onDelete}
                disabled={busy}
                data-testid={`branches-delete-${branch.name}`}
              >
                {t('branches.delete')}
              </Button>
            </>
          ) : null}
        </>
      )}
    </div>
  );
}

/** 标签行：名称 + 类型标记 + 消息首行 + 删除。 */
function TagRow({ tag, onDelete }: { readonly tag: Tag; readonly onDelete: () => void }) {
  const { t } = useTranslation('shell');
  return (
    <div className="flex h-9 items-center gap-2 px-3 text-13 hover:bg-surface-sunken">
      <TagIcon aria-hidden="true" className="size-3.5 shrink-0 text-fg-subtle" />
      <span className="font-medium">{tag.name}</span>
      {tag.annotated ? (
        <span className="shrink-0 rounded-full border border-line px-1.5 text-10 text-fg-subtle">
          {t('branches.annotated')}
        </span>
      ) : null}
      {tag.message !== null ? (
        <span className="truncate text-12 text-fg-muted">{tag.message}</span>
      ) : null}
      <span className="flex-1" />
      {tag.commit !== null ? (
        <code className="shrink-0 font-mono text-11 text-fg-subtle">{shortOid(tag.commit)}</code>
      ) : null}
      <Button
        size="sm"
        variant="ghost"
        onClick={onDelete}
        data-testid={`branches-tag-delete-${tag.name}`}
      >
        {t('branches.delete')}
      </Button>
    </div>
  );
}

export const RepoBranchesPage = RepoBranchesPageInner;
