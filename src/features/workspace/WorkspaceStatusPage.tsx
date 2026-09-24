//! 工作区状态面板（M1 / T1.4）。
//!
//! 结构：工具条（刷新/视图/全选/批量操作）→ 分组列表（冲突/已暂存/未暂存/未跟踪）
//! → 行（状态图标+文字、文件名、灰显路径、行内操作）。
//!
//! # 三个刻意的取舍
//!
//! 1. **行内操作直接可用**（暂存/取消暂存/放弃/复制路径/在文件管理器显示）；
//!    "在编辑器打开"随 M5 编辑器落地，先以禁用态占位——如实表达"还不能用"。
//! 2. **放弃走确认对话框**并列出将丢失的修改（危险操作；快照安全网 M3 接入）。
//! 3. **大数据量**：列表统一走 VirtualList（树视图展开为"目录行+文件行"序列），
//!    基准数据见 docs/benchmarks/status-panel.md。

import { useEffect, useMemo, useState } from 'react';

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Copy, FolderOpen, Minus, Plus, RefreshCw, RotateCcw, Rows3 } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useParams } from 'react-router-dom';

import {
  buildRows,
  countsOf,
  groupsOf,
  statusLabelKey,
  statusToneClass,
} from '@/features/workspace/statusModel';
import type {
  WorkspaceFileChange,
  WorkspaceStatus,
  WorkspaceView,
} from '@/features/workspace/statusModel';
import { PlaceholderPage } from '@/ui/PlaceholderPage';
import { Button } from '@/ui/components/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/ui/components/dialog';
import { EmptyState } from '@/ui/components/empty-state';
import { ErrorState } from '@/ui/components/error-state';
import { IconButton } from '@/ui/components/icon-button';
import { Skeleton } from '@/ui/components/skeleton';
import { ToggleGroup } from '@/ui/components/toggle-group';
import { VirtualList } from '@/ui/components/virtual-list';
import {
  onRepoChanged,
  workspaceDiscard,
  workspaceReveal,
  workspaceStage,
  workspaceStatus,
  workspaceUnstage,
} from '@/lib/ipc/workspace';
import { normalizeError, useAppError } from '@/lib/errors';
import { cn } from '@/lib/utils';
import { isTauriRuntime } from '@/lib/ipc/client';

/** TanStack Query 的 key 约定（T1.4 第 7 条：["status", repoId]）。 */
export const STATUS_QUERY_KEY = 'status';

/** 刷新间隔：无文件监听前的兜底（T1.10 之后可移除）。 */
const STATUS_REFRESH_MS = 15_000;

/** 订阅 repo:changed，失效对应仓库的状态查询。 */
function useInvalidateOnRepoChanged(repoId: number): void {
  const queryClient = useQueryClient();
  useEffect(() => {
    // 测试环境（jsdom）与浏览器预览没有 Tauri IPC：事件订阅只应在桌面运行时存在
    if (!isTauriRuntime()) return;
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void onRepoChanged((payload) => {
      if (payload.repoId === repoId) {
        void queryClient.invalidateQueries({ queryKey: [STATUS_QUERY_KEY, repoId] });
      }
    }).then((fn) => {
      if (cancelled) {
        fn();
      } else {
        unlisten = fn;
      }
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [queryClient, repoId]);
}
/** 单个分组的可折叠列表。 */
function GroupSection(props: {
  readonly title: string;
  readonly count: number;
  readonly tone: string;
  readonly entries: readonly WorkspaceFileChange[];
  readonly selected: ReadonlySet<string>;
  readonly collapsed: ReadonlySet<string>;
  readonly view: WorkspaceView;
  readonly onToggleDir: (dir: string) => void;
  readonly onToggleSelect: (path: string, additive: boolean) => void;
  readonly onRowAction: (
    action: 'stage' | 'unstage' | 'discard' | 'reveal' | 'copy',
    entry: WorkspaceFileChange,
  ) => void;
}) {
  const { t } = useTranslation('shell');
  const {
    title,
    count,
    tone,
    entries,
    selected,
    collapsed,
    view,
    onToggleDir,
    onToggleSelect,
    onRowAction,
  } = props;
  const [open, setOpen] = useState(true);

  const rows = useMemo(() => buildRows(entries, view, collapsed), [entries, view, collapsed]);

  return (
    <section className="rounded-lg border border-line bg-surface">
      <button
        type="button"
        aria-expanded={open}
        onClick={() => {
          setOpen((value) => !value);
        }}
        className="flex w-full items-center justify-between rounded-lg px-3 py-2 text-left hover:bg-surface-sunken"
      >
        <span className="flex items-center gap-2 text-13 font-medium">
          {open ? (
            <Minus aria-hidden="true" className="size-3.5" />
          ) : (
            <Plus aria-hidden="true" className="size-3.5" />
          )}
          {title}
        </span>
        <span className={cn('rounded-full bg-surface-sunken px-2 py-0.5 text-12', tone)}>
          {count}
        </span>
      </button>
      {open ? (
        rows.length > 0 ? (
          <VirtualList
            items={rows}
            itemHeight={32}
            height={Math.min(rows.length * 32, 320)}
            getKey={(row) => (row.type === 'dir' ? `dir:${row.dir}` : row.entry.path)}
            label={t('workspace.listLabel')}
            renderItem={(row) =>
              row.type === 'dir' ? (
                <button
                  type="button"
                  onClick={() => {
                    onToggleDir(row.dir);
                  }}
                  className="flex h-8 w-full items-center gap-2 px-3 text-left text-12 text-fg-subtle hover:bg-surface-sunken"
                >
                  {collapsed.has(row.dir) ? (
                    <Plus aria-hidden="true" className="size-3" />
                  ) : (
                    <Minus aria-hidden="true" className="size-3" />
                  )}
                  <FolderOpen aria-hidden="true" className="size-3.5" />
                  {row.dir || '/'}
                  <span className="ml-auto">{row.count}</span>
                </button>
              ) : (
                <FileRow
                  entry={row.entry}
                  selected={selected.has(row.entry.path)}
                  onToggleSelect={onToggleSelect}
                  onAction={onRowAction}
                />
              )
            }
          />
        ) : null
      ) : null}
    </section>
  );
}

/** 文件行：状态图标+文字、文件名、灰显路径、行内操作。 */
function FileRow(props: {
  readonly entry: WorkspaceFileChange;
  readonly selected: boolean;
  readonly onToggleSelect: (path: string, additive: boolean) => void;
  readonly onAction: (
    action: 'stage' | 'unstage' | 'discard' | 'reveal' | 'copy',
    entry: WorkspaceFileChange,
  ) => void;
}) {
  const { entry, selected, onToggleSelect, onAction } = props;
  const { t } = useTranslation('shell');
  const dir = entry.path.includes('/') ? entry.path.slice(0, entry.path.lastIndexOf('/')) : '';
  const name = entry.path.slice(dir === '' ? 0 : dir.length + 1);

  return (
    <div
      role="row"
      aria-selected={selected}
      className={cn(
        'fd-transition flex h-8 items-center gap-2 px-3 text-13',
        selected ? 'bg-brand-subtle' : 'hover:bg-surface-sunken',
      )}
    >
      <input
        type="checkbox"
        checked={selected}
        onChange={(event) => {
          onToggleSelect(entry.path, (event.nativeEvent as MouseEvent).shiftKey);
        }}
        aria-label={entry.path}
        className="size-3.5"
      />
      <span
        className={cn('w-16 shrink-0 text-12 font-medium', statusToneClass(entry))}
        title={t(statusLabelKey(entry))}
      >
        {t(statusLabelKey(entry))}
      </span>
      <span className="min-w-0 flex-1 truncate" title={entry.path}>
        {name}
        {dir === '' ? null : <span className="text-fg-subtle"> · {dir}</span>}
      </span>
      {entry.isLfs ? <span className="rounded-sm bg-surface-sunken px-1 text-10">LFS</span> : null}
      <div className="flex shrink-0 items-center gap-0.5">
        {entry.indexStatus === '.' ? (
          <IconButton
            label={t('workspace.actions.stage')}
            size="sm"
            onClick={() => onAction('stage', entry)}
          >
            <Plus aria-hidden="true" className="size-3.5" />
          </IconButton>
        ) : (
          <IconButton
            label={t('workspace.actions.unstage')}
            size="sm"
            onClick={() => onAction('unstage', entry)}
          >
            <Minus aria-hidden="true" className="size-3.5" />
          </IconButton>
        )}
        <IconButton
          label={t('workspace.actions.discard')}
          size="sm"
          onClick={() => onAction('discard', entry)}
        >
          <RotateCcw aria-hidden="true" className="size-3.5" />
        </IconButton>
        <IconButton
          label={t('workspace.actions.openEditor')}
          size="sm"
          disabled
          onClick={() => onAction('copy', entry)}
        >
          <Rows3 aria-hidden="true" className="size-3.5" />
        </IconButton>
        <IconButton
          label={t('workspace.actions.reveal')}
          size="sm"
          onClick={() => onAction('reveal', entry)}
        >
          <FolderOpen aria-hidden="true" className="size-3.5" />
        </IconButton>
        <IconButton
          label={t('workspace.actions.copyPath')}
          size="sm"
          onClick={() => onAction('copy', entry)}
        >
          <Copy aria-hidden="true" className="size-3.5" />
        </IconButton>
      </div>
    </div>
  );
}
/** 状态面板主组件。 */
export function WorkspaceStatusPage() {
  const { t } = useTranslation('shell');
  const params = useParams();
  const repoId = Number(params.repoId);
  const { show } = useAppError();
  const [includeIgnored, setIncludeIgnored] = useState(false);
  const [view, setView] = useState<WorkspaceView>('tree');
  const [collapsedDirs, setCollapsedDirs] = useState<ReadonlySet<string>>(new Set());
  const [selected, setSelected] = useState<ReadonlySet<string>>(new Set());
  const [pendingDiscard, setPendingDiscard] = useState<{
    tracked: string[];
    untracked: string[];
  } | null>(null);

  const query = useQuery({
    queryKey: [STATUS_QUERY_KEY, repoId, includeIgnored],
    queryFn: () => workspaceStatus(repoId, includeIgnored),
    enabled: Number.isFinite(repoId),
    refetchInterval: STATUS_REFRESH_MS,
  });
  useInvalidateOnRepoChanged(repoId);

  const groups = query.data ? groupsOf(query.data) : undefined;
  const counts = groups ? countsOf(groups) : undefined;
  const totalChanges = counts
    ? counts.staged + counts.unstaged + counts.untracked + counts.conflicted
    : 0;
  const isClean = query.data !== undefined && totalChanges === 0;

  function toggleSelect(path: string, additive: boolean): void {
    setSelected((previous) => {
      const next = new Set(previous);
      if (next.has(path) && !additive) {
        next.delete(path);
      } else {
        next.add(path);
      }
      return next;
    });
  }

  function toggleDir(dir: string): void {
    setCollapsedDirs((previous) => {
      const next = new Set(previous);
      if (next.has(dir)) {
        next.delete(dir);
      } else {
        next.add(dir);
      }
      return next;
    });
  }

  const stageMutation = useMutation({
    mutationFn: (paths: readonly string[]) => workspaceStage(repoId, paths),
    onSuccess: () => setSelected(new Set()),
    onError: (error) => show(normalizeError(error)),
  });
  const unstageMutation = useMutation({
    mutationFn: (paths: readonly string[]) => workspaceUnstage(repoId, paths),
    onSuccess: () => setSelected(new Set()),
    onError: (error) => show(normalizeError(error)),
  });
  const discardMutation = useMutation({
    mutationFn: (spec: { tracked: string[]; untracked: string[] }) =>
      workspaceDiscard(repoId, spec.tracked, spec.untracked),
    onSuccess: () => {
      setSelected(new Set());
      setPendingDiscard(null);
    },
    onError: (error) => show(normalizeError(error)),
  });

  function handleRowAction(
    action: 'stage' | 'unstage' | 'discard' | 'reveal' | 'copy',
    entry: WorkspaceFileChange,
  ): void {
    switch (action) {
      case 'stage':
        stageMutation.mutate([entry.path]);
        break;
      case 'unstage':
        unstageMutation.mutate([entry.path]);
        break;
      case 'discard':
        setPendingDiscard({
          tracked: entry.worktreeStatus !== '.' ? [entry.path] : [],
          untracked: entry.kind === 'untracked' ? [entry.path] : [],
        });
        break;
      case 'reveal':
        workspaceReveal(repoId, entry.path).catch(show);
        break;
      case 'copy':
        void navigator.clipboard.writeText(entry.path).catch(show);
        break;
    }
  }

  function stageSelected(): void {
    const paths = [...selected].filter(
      (path) => !groups?.staged.some((entry) => entry.path === path),
    );
    stageMutation.mutate(paths);
  }

  function unstageSelected(): void {
    const paths = [...selected].filter((path) =>
      groups?.staged.some((entry) => entry.path === path),
    );
    unstageMutation.mutate(paths);
  }

  function requestDiscardSelected(): void {
    const tracked = [...selected].filter((path) =>
      groups?.unstaged.some((entry) => entry.path === path),
    );
    const untracked = [...selected].filter((path) =>
      groups?.untracked.some((entry) => entry.path === path),
    );
    setPendingDiscard({ tracked, untracked });
  }

  if (!Number.isFinite(repoId)) {
    return (
      <PlaceholderPage
        plannedTask="T1.4"
        titleKey="pages.repoStatus.title"
        descriptionKey="pages.repoStatus.description"
      />
    );
  }
  const renderGroup = (
    title: string,
    count: number,
    tone: string,
    entries: readonly WorkspaceFileChange[],
  ) =>
    count > 0 ? (
      <GroupSection
        title={title}
        count={count}
        tone={tone}
        entries={entries}
        selected={selected}
        collapsed={collapsedDirs}
        view={view}
        onToggleDir={toggleDir}
        onToggleSelect={toggleSelect}
        onRowAction={handleRowAction}
      />
    ) : null;

  return (
    <section className="flex h-full min-h-0 flex-col gap-3">
      <header className="flex flex-wrap items-center justify-between gap-2">
        <h1 className="text-20 font-semibold tracking-tight">{t('pages.repoStatus.title')}</h1>
        <div className="flex items-center gap-2">
          <ToggleGroup
            label={t('workspace.view.label')}
            value={view}
            options={[
              { value: 'tree', label: t('workspace.view.tree') },
              { value: 'flat', label: t('workspace.view.flat') },
            ]}
            onValueChange={(next) => setView(next as WorkspaceView)}
          />
          <IconButton label={t('workspace.refresh')} size="sm" onClick={() => void query.refetch()}>
            <RefreshCw aria-hidden="true" className="size-4" />
          </IconButton>
        </div>
      </header>
      {query.data === undefined ? (
        query.isError ? (
          (() => {
            const normalized = normalizeError(query.error);
            return (
              <ErrorState
                title={t('workspace.error.title')}
                hint={t('workspace.error.hint')}
                {...(normalized.detail === undefined ? {} : { details: normalized.detail })}
                retryLabel={t('common:actions.retry')}
                onRetry={() => void query.refetch()}
              />
            );
          })()
        ) : (
          <Skeleton className="h-40" />
        )
      ) : isClean ? (
        <EmptyState title={t('workspace.clean.title')} description={t('workspace.clean.hint')} />
      ) : (
        <>
          {query.data.operation !== 'none' ? (
            <div className="rounded-lg border border-warning bg-surface p-3 text-13">
              {t('workspace.operationBanner', { operation: query.data.operation })}
              {' · '}
              <button
                type="button"
                className="text-brand underline"
                onClick={() => setView('tree')}
              >
                {t('workspace.goToConflicts')}
              </button>
            </div>
          ) : null}
          {counts === undefined ? null : (
            <div className="flex flex-wrap items-center gap-2" data-testid="workspace-toolbar">
              <Button
                size="sm"
                variant="secondary"
                onClick={() => setSelected(new Set(allSelected(query.data)))}
              >
                {t('workspace.toolbar.selectAll')}
              </Button>
              <Button size="sm" variant="secondary" onClick={() => setSelected(new Set())}>
                {t('workspace.toolbar.clear')}
              </Button>
              <Button size="sm" loading={stageMutation.isPending} onClick={stageSelected}>
                {t('workspace.toolbar.stage')} ({selected.size})
              </Button>
              <Button
                size="sm"
                variant="secondary"
                loading={unstageMutation.isPending}
                onClick={unstageSelected}
              >
                {t('workspace.toolbar.unstage')}
              </Button>
              <Button size="sm" variant="danger" onClick={requestDiscardSelected}>
                {t('workspace.toolbar.discard')}
              </Button>
              <Button
                size="sm"
                variant="secondary"
                onClick={() => {
                  setIncludeIgnored((value) => !value);
                }}
              >
                {includeIgnored
                  ? t('workspace.toolbar.includeIgnoredOn')
                  : t('workspace.toolbar.includeIgnored')}
              </Button>
            </div>
          )}
          <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-auto">
            {groups === undefined || counts === undefined
              ? null
              : [
                  renderGroup(
                    t('workspace.groups.conflicted'),
                    counts.conflicted,
                    'text-warning',
                    groups.conflicted,
                  ),
                  renderGroup(
                    t('workspace.groups.staged'),
                    counts.staged,
                    'text-success',
                    groups.staged,
                  ),
                  renderGroup(
                    t('workspace.groups.unstaged'),
                    counts.unstaged,
                    'text-info',
                    groups.unstaged,
                  ),
                  renderGroup(
                    t('workspace.groups.untracked'),
                    counts.untracked,
                    'text-fg-muted',
                    groups.untracked,
                  ),
                ]}
          </div>
        </>
      )}

      <Dialog
        open={pendingDiscard !== null}
        onOpenChange={(next) => {
          if (!next) {
            setPendingDiscard(null);
          }
        }}
      >
        <DialogContent closeLabel={t('common:actions.close')}>
          <DialogHeader>
            <DialogTitle>{t('workspace.discard.title')}</DialogTitle>
            <DialogDescription>{t('workspace.discard.description')}</DialogDescription>
          </DialogHeader>
          <ul className="max-h-48 overflow-auto rounded-md border border-line bg-surface-sunken p-2 font-mono text-12">
            {(pendingDiscard?.tracked ?? []).map((path) => (
              <li key={path}>{path}</li>
            ))}
            {(pendingDiscard?.untracked ?? []).map((path) => (
              <li key={path}>{path}</li>
            ))}
          </ul>
          <DialogFooter>
            <Button variant="secondary" onClick={() => setPendingDiscard(null)}>
              {t('common:actions.cancel')}
            </Button>
            <Button
              variant="danger"
              loading={discardMutation.isPending}
              onClick={() => {
                if (pendingDiscard !== null) {
                  discardMutation.mutate(pendingDiscard);
                }
              }}
            >
              {t('workspace.discard.confirm')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </section>
  );
}

function allSelected(status: WorkspaceStatus): readonly string[] {
  return [...status.conflicted, ...status.staged, ...status.unstaged, ...status.untracked].map(
    (entry) => entry.path,
  );
}
