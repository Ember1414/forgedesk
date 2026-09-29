/**
 * 储藏面板（T2.8）。
 *
 * # 它挂在状态页而不是单独一页
 *
 * 储藏是"对工作区做的事"：用户在状态页看到一堆不想提交又不想丢的改动时才会想起它。
 * 放进仓库外壳的另一个页签等于要求用户先离开现场。
 *
 * # 三个结论必须在界面上说清楚
 *
 * 1. **`stashed=false` 不是失败**：干净工作区点"储藏"就是"没有可储藏的内容"。
 * 2. **冲突是结果**：应用储藏冲突时仓库已进入冲突状态——横幅列出冲突文件，
 *    并把用户引去状态页的冲突区（那里有解决冲突的既有入口）。
 * 3. **丢弃不可逆**：确认框里写清将丢掉哪条（信息 + oid），
 *    以及"gc 回收之前还能按 oid 找回"这个唯一的安全网。
 */
import { useState } from 'react';

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Archive, ArchiveRestore, ChevronDown, ChevronRight, Trash2 } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useNavigate, useParams } from 'react-router-dom';

import { normalizeError, useAppError } from '@/lib/errors';
import {
  gitStashApply,
  gitStashBranch,
  gitStashClear,
  gitStashDrop,
  gitStashList,
  gitStashPop,
  gitStashSave,
  gitStashShow,
} from '@/lib/ipc';
import type { StashEntry, StashShowOutcome } from '@/lib/ipc';
import { statusKey, stashKey } from '@/lib/queryKeys';

import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/ui/components/alert-dialog';
import { Button } from '@/ui/components/button';
import { EmptyState } from '@/ui/components/empty-state';
import { Input } from '@/ui/components/input';
import { Skeleton } from '@/ui/components/skeleton';

/** 展开后的内容：两份文件清单（相对 base 的改动 + 未跟踪文件）。 */
function StashFiles({ outcome }: { readonly outcome: StashShowOutcome }) {
  const { t } = useTranslation('shell');

  const tracked = outcome.diff as { readonly files: readonly { readonly path: string }[] };
  // 后端 `Option<DiffReportDto>` 经 serde 序列化为 **null**（不是 undefined）：
  // 无未跟踪文件的 stash 是常态，用 `!== undefined` 判断会在真实数据下崩掉
  // 整个状态页（e2e/stash-reset.spec.ts 抓到）。
  const untracked = (outcome.untracked ?? null) as {
    readonly files: readonly { readonly path: string }[];
  } | null;

  return (
    <div className="flex flex-col gap-2 pl-2" data-testid="stash-files">
      <ul className="flex flex-col gap-0.5 font-mono text-12">
        {tracked.files.map((file) => (
          <li key={file.path}>{file.path}</li>
        ))}
      </ul>
      {untracked !== null && untracked.files.length > 0 ? (
        <div className="flex flex-col gap-1">
          {/* 未跟踪文件在 stash 的第三个父提交里：单独列出，否则用户以为清单是完整的 */}
          <p className="text-12 text-fg-muted">{t('stash.untrackedFiles')}</p>
          <ul className="flex flex-col gap-0.5 font-mono text-12 text-fg-muted">
            {untracked.files.map((file) => (
              <li key={file.path}>{file.path}</li>
            ))}
          </ul>
        </div>
      ) : null}
    </div>
  );
}

export function StashPanel() {
  const params = useParams();
  const repoId = Number(params.repoId);
  const { t } = useTranslation('shell');
  const { show: showError } = useAppError();
  const queryClient = useQueryClient();
  const navigate = useNavigate();

  const [expanded, setExpanded] = useState<number | null>(null);
  const [pendingDrop, setPendingDrop] = useState<StashEntry | null>(null);
  const [confirmClear, setConfirmClear] = useState(false);
  const [branchFor, setBranchFor] = useState<StashEntry | null>(null);
  const [branchName, setBranchName] = useState('');
  /** 最近一次"应用"的结果：冲突时展示横幅（冲突是结果，不是弹窗错误）。 */
  const [conflicts, setConflicts] = useState<readonly string[] | null>(null);

  const list = useQuery({
    queryKey: stashKey(repoId),
    queryFn: () => gitStashList(repoId),
    enabled: Number.isFinite(repoId) && repoId > 0,
  });

  const detail = useQuery({
    queryKey: [stashKey(repoId), 'show', expanded],
    queryFn: () => gitStashShow(repoId, expanded ?? 0),
    enabled: expanded !== null,
  });

  const invalidate = () => {
    void queryClient.invalidateQueries({ queryKey: stashKey(repoId) });
    void queryClient.invalidateQueries({ queryKey: statusKey(repoId) });
  };

  const save = useMutation({
    mutationFn: () =>
      gitStashSave(repoId, { includeUntracked: true, message: t('stash.defaultMessage') }),
    onSuccess: (result) => {
      invalidate();
      if (!result.stashed) {
        // "没有可储藏的内容"是正常结果：用提示而不是错误表达
        showError(new Error(t('stash.nothingToStash')));
      }
    },
    onError: showError,
  });

  const applyOrPop = useMutation({
    mutationFn: ({ index, pop }: { readonly index: number; readonly pop: boolean }) =>
      pop ? gitStashPop(repoId, index) : gitStashApply(repoId, index),
    onSuccess: (result) => {
      setConflicts(result.conflicts.length > 0 ? result.conflicts : null);
      invalidate();
    },
    onError: showError,
  });

  const drop = useMutation({
    mutationFn: (index: number) => gitStashDrop(repoId, index),
    onSuccess: () => {
      setPendingDrop(null);
      invalidate();
    },
    onError: showError,
  });

  const clear = useMutation({
    mutationFn: () => gitStashClear(repoId),
    onSuccess: () => {
      setConfirmClear(false);
      invalidate();
    },
    onError: showError,
  });

  const branch = useMutation({
    mutationFn: ({ index, name }: { readonly index: number; readonly name: string }) =>
      gitStashBranch(repoId, index, name),
    onSuccess: () => {
      setBranchFor(null);
      setBranchName('');
      invalidate();
    },
    onError: showError,
  });

  const busy =
    save.isPending || applyOrPop.isPending || drop.isPending || clear.isPending || branch.isPending;
  const entries = list.data ?? [];

  return (
    <section className="flex flex-col gap-2" data-testid="stash-panel">
      <header className="flex flex-wrap items-center justify-between gap-2">
        <h2 className="flex items-center gap-2 text-16 font-semibold tracking-tight">
          <Archive aria-hidden="true" className="size-4" />
          {t('stash.title')}
          {entries.length > 0 ? (
            <span className="text-13 font-normal text-fg-subtle">({entries.length})</span>
          ) : null}
        </h2>
        <div className="flex items-center gap-2">
          <Button
            variant="ghost"
            size="sm"
            disabled={busy}
            onClick={() => save.mutate()}
            data-testid="stash-save"
          >
            <ArchiveRestore aria-hidden="true" className="size-4" />
            {t('stash.save')}
          </Button>
          {entries.length > 0 ? (
            <Button
              variant="ghost"
              size="sm"
              disabled={busy}
              onClick={() => setConfirmClear(true)}
              data-testid="stash-clear"
            >
              {t('stash.clear')}
            </Button>
          ) : null}
        </div>
      </header>

      {conflicts !== null ? (
        <div
          className="flex flex-wrap items-center gap-2 rounded-md border border-warning bg-surface p-3 text-13"
          data-testid="stash-conflict-banner"
        >
          <span>{t('stash.conflict', { count: conflicts.length })}</span>
          <span className="font-mono text-12 text-fg-subtle">{conflicts.join(', ')}</span>
          <button
            type="button"
            className="text-brand underline"
            onClick={() => navigate(`/repo/${repoId}/status`)}
          >
            {t('stash.conflictGuide')}
          </button>
        </div>
      ) : null}

      {list.isError ? (
        (() => {
          const normalized = normalizeError(list.error);
          return (
            <p className="text-13 text-fg-subtle" data-testid="stash-unavailable">
              {t('stash.unavailable')}
              {normalized.detail === undefined ? '' : ` · ${normalized.detail}`}
            </p>
          );
        })()
      ) : list.data === undefined ? (
        <Skeleton className="h-16" />
      ) : entries.length === 0 ? (
        <EmptyState title={t('stash.empty')} description={t('stash.emptyHint')} />
      ) : (
        <ul className="flex flex-col gap-1" data-testid="stash-list">
          {entries.map((entry) => (
            <li
              key={entry.oid}
              className="flex flex-col gap-1 rounded-md border border-line bg-surface px-3 py-2"
            >
              <div className="flex flex-wrap items-center gap-2">
                <button
                  type="button"
                  className="flex items-center gap-1 text-13"
                  onClick={() => setExpanded(expanded === entry.index ? null : entry.index)}
                  data-testid={`stash-toggle-${entry.index}`}
                >
                  {expanded === entry.index ? (
                    <ChevronDown aria-hidden="true" className="size-4" />
                  ) : (
                    <ChevronRight aria-hidden="true" className="size-4" />
                  )}
                  <span>{entry.message}</span>
                </button>
                {entry.includesUntracked ? (
                  <span className="rounded-sm border border-line px-1.5 text-12 text-fg-muted">
                    {t('stash.includesUntracked')}
                  </span>
                ) : null}
                <span className="ml-auto font-mono text-12 text-fg-subtle">
                  {entry.oid.slice(0, 7)}
                </span>
              </div>

              <div className="flex flex-wrap items-center gap-1">
                <Button
                  variant="ghost"
                  size="sm"
                  disabled={busy}
                  onClick={() => applyOrPop.mutate({ index: entry.index, pop: false })}
                  data-testid={`stash-apply-${entry.index}`}
                >
                  {t('stash.apply')}
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  disabled={busy}
                  onClick={() => applyOrPop.mutate({ index: entry.index, pop: true })}
                  data-testid={`stash-pop-${entry.index}`}
                >
                  {t('stash.pop')}
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  disabled={busy}
                  onClick={() => {
                    setBranchFor(entry);
                    setBranchName('');
                  }}
                  data-testid={`stash-branch-${entry.index}`}
                >
                  {t('stash.branch')}
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  disabled={busy}
                  onClick={() => setPendingDrop(entry)}
                  data-testid={`stash-drop-${entry.index}`}
                >
                  <Trash2 aria-hidden="true" className="size-4" />
                  {t('stash.drop')}
                </Button>
              </div>

              {expanded === entry.index ? (
                detail.data !== undefined && detail.data.entry.oid === entry.oid ? (
                  <StashFiles outcome={detail.data} />
                ) : (
                  <Skeleton className="h-10" />
                )
              ) : null}
            </li>
          ))}
        </ul>
      )}

      {/* 丢弃确认：写清将丢掉哪条，以及"还能按 oid 找回"这个唯一的安全网 */}
      <AlertDialog
        open={pendingDrop !== null}
        onOpenChange={(next) => {
          if (!next) {
            setPendingDrop(null);
          }
        }}
      >
        <AlertDialogContent
          impact={t('stash.dropImpact')}
          impactLabel={t('stash.dropImpactLabel')}
          data-testid="stash-drop-dialog"
        >
          <AlertDialogHeader>
            <AlertDialogTitle>{t('stash.dropTitle')}</AlertDialogTitle>
            <AlertDialogDescription>
              {pendingDrop?.message}
              {pendingDrop !== null ? ` · ${pendingDrop.oid.slice(0, 7)}` : ''}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t('common:actions.cancel')}</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => pendingDrop !== null && drop.mutate(pendingDrop.index)}
              data-testid="stash-drop-confirm"
            >
              {t('stash.dropConfirm')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <AlertDialog
        open={confirmClear}
        onOpenChange={(next) => {
          if (!next) {
            setConfirmClear(false);
          }
        }}
      >
        <AlertDialogContent
          impact={t('stash.clearImpact', { count: entries.length })}
          impactLabel={t('stash.dropImpactLabel')}
          data-testid="stash-clear-dialog"
        >
          <AlertDialogHeader>
            <AlertDialogTitle>{t('stash.clearTitle')}</AlertDialogTitle>
            <AlertDialogDescription>{t('stash.dropRecoverHint')}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t('common:actions.cancel')}</AlertDialogCancel>
            <AlertDialogAction onClick={() => clear.mutate()} data-testid="stash-clear-confirm">
              {t('stash.clearConfirm')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <AlertDialog
        open={branchFor !== null}
        onOpenChange={(next) => {
          if (!next) {
            setBranchFor(null);
          }
        }}
      >
        <AlertDialogContent
          impact={t('stash.branchImpact')}
          impactLabel={t('stash.dropImpactLabel')}
          data-testid="stash-branch-dialog"
        >
          <AlertDialogHeader>
            <AlertDialogTitle>{t('stash.branchTitle')}</AlertDialogTitle>
            <AlertDialogDescription>{t('stash.branchHint')}</AlertDialogDescription>
          </AlertDialogHeader>
          <Input
            value={branchName}
            placeholder={t('stash.branchPlaceholder')}
            onChange={(event) => {
              setBranchName(event.target.value);
            }}
            data-testid="stash-branch-name"
          />
          <AlertDialogFooter>
            <AlertDialogCancel>{t('common:actions.cancel')}</AlertDialogCancel>
            <AlertDialogAction
              disabled={branchName.trim() === ''}
              onClick={() =>
                branchFor !== null &&
                branch.mutate({ index: branchFor.index, name: branchName.trim() })
              }
              data-testid="stash-branch-confirm"
            >
              {t('stash.branchConfirm')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </section>
  );
}
