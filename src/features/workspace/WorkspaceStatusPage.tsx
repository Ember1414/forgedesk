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

import { useMemo, useState } from 'react';

import { useMutation, useQuery } from '@tanstack/react-query';
import { Copy, FolderOpen, Minus, Plus, RefreshCw, RotateCcw, Rows3, History } from 'lucide-react';
import type { TFunction } from 'i18next';
import { useTranslation } from 'react-i18next';
import { useNavigate, useParams } from 'react-router-dom';

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
import { DiffView } from '@/features/diff/DiffView';
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
import {
  Sheet,
  SheetBody,
  SheetContent,
  SheetDescription,
  SheetTitle,
} from '@/ui/components/sheet';
import { ToggleGroup } from '@/ui/components/toggle-group';
import { VirtualList } from '@/ui/components/virtual-list';
import {
  workspaceDiscard,
  workspaceReveal,
  workspaceStage,
  workspaceStatus,
  workspaceUnstage,
} from '@/lib/ipc/workspace';
import type { DiscardScope, PatchViewSpec, StageScope } from '@/lib/ipc/workspace';
import { normalizeError, useAppError } from '@/lib/errors';
import { cn } from '@/lib/utils';
import { useRepoChangeInvalidation } from '@/lib/repoChanged';
import { useSettingsStore, WATCH_AUTO_REFRESH_KEY } from '@/stores/settingsStore';

/** TanStack Query 的 key 约定（定义在 `@/lib/queryKeys`，各页面从那里引用）。 */
import { STATUS_QUERY_KEY } from '@/lib/queryKeys';

/**
 * 轮询兜底：只在**关掉自动刷新**时使用。
 *
 * 为什么保留：文件监听在两类环境下不可靠——网络盘 / 超大仓库，以及用户
 * 主动关掉开关（见设置页的"自动刷新"）。没有轮询，那些情况下界面永远不会更新。
 * 开着监听时不轮询：那是 T1.10 的性能验收（大仓库上 CPU 空闲占用接近 0）。
 */
const STATUS_FALLBACK_REFRESH_MS = 15_000;
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
    action: 'stage' | 'unstage' | 'discard' | 'reveal' | 'copy' | 'history',
    entry: WorkspaceFileChange,
  ) => void;
  /** 点击文件名打开 diff（T1.5）。 */
  readonly onOpenDiff: (entry: WorkspaceFileChange) => void;
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
    onOpenDiff,
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
                  onOpenDiff={onOpenDiff}
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
    action: 'stage' | 'unstage' | 'discard' | 'reveal' | 'copy' | 'history',
    entry: WorkspaceFileChange,
  ) => void;
  /** 点击文件名打开 diff（T1.5）。 */
  readonly onOpenDiff: (entry: WorkspaceFileChange) => void;
}) {
  const { entry, selected, onToggleSelect, onAction, onOpenDiff } = props;
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
        <button
          type="button"
          className="fd-transition rounded-sm text-left hover:text-brand hover:underline focus-visible:outline focus-visible:outline-2 focus-visible:outline-brand"
          onClick={() => {
            onOpenDiff(entry);
          }}
        >
          {name}
        </button>
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
        <IconButton
          label={t('workspace.actions.fileHistory')}
          size="sm"
          onClick={() => onAction('history', entry)}
          data-testid="workspace-file-history"
        >
          <History aria-hidden="true" className="size-3.5" />
        </IconButton>
      </div>
    </div>
  );
}
/** 状态面板主组件。 */
export function WorkspaceStatusPage() {
  const { t } = useTranslation('shell');
  const params = useParams();
  const navigate = useNavigate();
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
  /**
   * 待确认的"部分丢弃"（按块 / 按行）。
   *
   * 与 `pendingDiscard` 分开：文件级丢弃列的是路径清单，部分丢弃给的是选择摘要，
   * 而且两者走的后端通道不同（`git restore` / 反向补丁）。
   */
  const [pendingPartialDiscard, setPendingPartialDiscard] = useState<{
    scope: StageScope;
    view: PatchViewSpec;
  } | null>(null);
  /**
   * 写操作成功计数：作为 DiffView 的 key 的一部分。
   *
   * 暂存之后 hunks 已经变了，旧的"选中第 3 行"可能指向完全不同的内容。
   * 用 key 让组件重建（React 官方的"重置 state"手法），比在组件里
   * 用 effect 监听数据变化再把 state 改回去（会触发 set-state-in-effect 告警）干净。
   */
  const [selectionEpoch, setSelectionEpoch] = useState(0);
  // 点文件名打开 diff（T1.5）。放在页面级状态而不是 store：
  // 只有这个页面用它，且"关掉即忘"符合临时查看的语义。
  const [diffTarget, setDiffTarget] = useState<{
    path: string;
    target: 'staged' | 'unstaged';
  } | null>(null);
  /**
   * "刚才发生的是一次大量变更"。
   *
   * 监听在变化量超过阈值时只发一个 `large` 事件（没有路径可列）。那种情况下
   * 列表会整体刷新一次，界面上要说明原因——否则用户只会看到内容"自己跳了一下"。
   */
  const [largeChange, setLargeChange] = useState(false);
  /** 自动刷新开关：关掉时退回轮询（见 `STATUS_FALLBACK_REFRESH_MS`）。 */
  const autoRefresh = useSettingsStore((state) =>
    state.getJson<boolean>(WATCH_AUTO_REFRESH_KEY, true),
  );

  const query = useQuery({
    queryKey: [STATUS_QUERY_KEY, repoId, includeIgnored],
    queryFn: () => workspaceStatus(repoId, includeIgnored),
    enabled: Number.isFinite(repoId),
    // 监听开着时由事件驱动刷新（T1.10）；关掉开关才退回定时轮询
    ...(autoRefresh ? {} : { refetchInterval: STATUS_FALLBACK_REFRESH_MS }),
  });
  useRepoChangeInvalidation(repoId, {
    onLargeChange: () => {
      setLargeChange(true);
    },
  });

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

  /** 写操作成功后：清空文件选择、让 diff 查看器重建（它的选择已经过期）。 */
  function afterWrite(): void {
    setSelected(new Set());
    setSelectionEpoch((epoch) => epoch + 1);
  }

  const stageMutation = useMutation({
    mutationFn: (request: { scope: StageScope; view?: PatchViewSpec }) =>
      workspaceStage(repoId, request.scope, request.view),
    onSuccess: afterWrite,
    onError: (error) => show(normalizeError(error)),
  });
  const unstageMutation = useMutation({
    mutationFn: (request: { scope: StageScope; view?: PatchViewSpec }) =>
      workspaceUnstage(repoId, request.scope, request.view),
    onSuccess: afterWrite,
    onError: (error) => show(normalizeError(error)),
  });
  const discardMutation = useMutation({
    mutationFn: (request: { scope: DiscardScope; view?: PatchViewSpec }) =>
      workspaceDiscard(repoId, request.scope, request.view),
    onSuccess: () => {
      afterWrite();
      setPendingDiscard(null);
      setPendingPartialDiscard(null);
    },
    onError: (error) => show(normalizeError(error)),
  });

  function handleRowAction(
    action: 'stage' | 'unstage' | 'discard' | 'reveal' | 'copy' | 'history',
    entry: WorkspaceFileChange,
  ): void {
    switch (action) {
      case 'stage':
        stageMutation.mutate({ scope: { kind: 'files', paths: [entry.path] } });
        break;
      case 'unstage':
        unstageMutation.mutate({ scope: { kind: 'files', paths: [entry.path] } });
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
      case 'history':
        // 文件历史（T2.3）：路径过滤 + 跟随重命名经 URL 进入历史页
        // （筛选状态以 URL 为真相源，刷新 / 分享都能复现）
        navigate(`/repo/${repoId}/history?path=${encodeURIComponent(entry.path)}&follow=1`);
        break;
    }
  }

  function stageSelected(): void {
    const paths = [...selected].filter(
      (path) => !groups?.staged.some((entry) => entry.path === path),
    );
    stageMutation.mutate({ scope: { kind: 'files', paths } });
  }

  function unstageSelected(): void {
    const paths = [...selected].filter((path) =>
      groups?.staged.some((entry) => entry.path === path),
    );
    unstageMutation.mutate({ scope: { kind: 'files', paths } });
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
    diffTarget: 'staged' | 'unstaged',
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
        onOpenDiff={(entry) => {
          setDiffTarget({ path: entry.path, target: diffTarget });
        }}
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
          {largeChange ? (
            <div
              className="flex flex-wrap items-center gap-2 rounded-lg border border-line bg-surface-sunken p-3 text-13"
              data-testid="workspace-large-change"
            >
              {t('workspace.largeChange')}
              <button
                type="button"
                className="text-brand underline"
                onClick={() => {
                  setLargeChange(false);
                  void query.refetch();
                }}
              >
                {t('workspace.refresh')}
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
                    'unstaged',
                  ),
                  renderGroup(
                    t('workspace.groups.staged'),
                    counts.staged,
                    'text-success',
                    groups.staged,
                    'staged',
                  ),
                  renderGroup(
                    t('workspace.groups.unstaged'),
                    counts.unstaged,
                    'text-info',
                    groups.unstaged,
                    'unstaged',
                  ),
                  renderGroup(
                    t('workspace.groups.untracked'),
                    counts.untracked,
                    'text-fg-muted',
                    groups.untracked,
                    'unstaged',
                  ),
                ]}
          </div>
        </>
      )}

      {/* diff 侧滑（T1.5）：点文件名打开；右侧滑出保持列表可见，便于对照操作 */}
      <Sheet
        open={diffTarget !== null}
        onOpenChange={(open) => {
          if (!open) {
            setDiffTarget(null);
          }
        }}
      >
        <SheetContent
          side="right"
          closeLabel={t('diff.close')}
          className="flex w-[min(960px,85vw)] flex-col p-0"
        >
          <div className="border-b border-border/60 px-3 py-2">
            <SheetTitle className="truncate font-mono text-13">{diffTarget?.path ?? ''}</SheetTitle>
            <SheetDescription className="sr-only">{t('diff.sheetDescription')}</SheetDescription>
          </div>
          <SheetBody className="min-h-0 flex-1 overflow-hidden">
            {diffTarget !== null && (
              <DiffView
                // 每次写操作成功后重建（清空行选择与折叠态）：暂存之后 hunk 已经变了，
                // 旧的"选中第 3 行"可能指向完全不同的内容。
                key={`${diffTarget.path}:${diffTarget.target}:${selectionEpoch}`}
                repoId={repoId}
                path={diffTarget.path}
                target={diffTarget.target}
                className="h-full"
                onStage={(scope, view) => stageMutation.mutate({ scope, view })}
                onUnstage={(scope, view) => unstageMutation.mutate({ scope, view })}
                onDiscard={(scope, view) => setPendingPartialDiscard({ scope, view })}
                busy={
                  stageMutation.isPending || unstageMutation.isPending || discardMutation.isPending
                }
              />
            )}
          </SheetBody>
        </SheetContent>
      </Sheet>

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
                  discardMutation.mutate({
                    scope: {
                      kind: 'files',
                      tracked: pendingDiscard.tracked,
                      untracked: pendingDiscard.untracked,
                    },
                  });
                }
              }}
            >
              {t('workspace.discard.confirm')}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* 部分丢弃的确认框（红线 R7 在 UI 层的闸门）：必须说清"影响"才允许执行 */}
      <AlertDialog
        open={pendingPartialDiscard !== null}
        onOpenChange={(next) => {
          if (!next) {
            setPendingPartialDiscard(null);
          }
        }}
      >
        <AlertDialogContent
          impactLabel={t('workspace.discard.impactLabel')}
          impact={t('workspace.discard.linesDescription')}
        >
          <AlertDialogHeader>
            <AlertDialogTitle>{t('workspace.discard.title')}</AlertDialogTitle>
            <AlertDialogDescription>
              {pendingPartialDiscard === null
                ? ''
                : partialDiscardSummary(t, pendingPartialDiscard.scope)}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t('common:actions.cancel')}</AlertDialogCancel>
            <AlertDialogAction
              disabled={discardMutation.isPending}
              onClick={() => {
                if (pendingPartialDiscard !== null) {
                  discardMutation.mutate({
                    scope: toDiscardScope(pendingPartialDiscard.scope),
                    view: pendingPartialDiscard.view,
                  });
                }
              }}
            >
              {t('workspace.discard.confirm')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </section>
  );
}

/** 把"行 / 块"选择转换成丢弃请求（文件粒度的形状不同，这里不该出现）。 */
function toDiscardScope(scope: StageScope): DiscardScope {
  if (scope.kind === 'hunks') {
    return { kind: 'hunks', path: scope.path, hunkIndices: scope.hunkIndices };
  }
  if (scope.kind === 'lines') {
    return { kind: 'lines', path: scope.path, selections: scope.selections };
  }
  // 兜底：块级 / 行级之外的调用点走文件级确认框，这里保守地按已跟踪路径处理
  return { kind: 'files', tracked: scope.paths, untracked: [] };
}

/** 确认框里的选择摘要（"哪个文件、多少行 / 块"）。 */
function partialDiscardSummary(t: TFunction, scope: StageScope): string {
  if (scope.kind === 'hunks') {
    return t('workspace.discard.summaryHunks', {
      path: scope.path,
      count: scope.hunkIndices.length,
    });
  }
  if (scope.kind === 'lines') {
    const count = scope.selections.reduce((total, item) => total + item.lines.length, 0);
    return t('workspace.discard.summaryLines', { path: scope.path, count });
  }
  return t('workspace.discard.summaryFiles', { count: scope.paths.length });
}

function allSelected(status: WorkspaceStatus): readonly string[] {
  return [...status.conflicted, ...status.staged, ...status.unstaged, ...status.untracked].map(
    (entry) => entry.path,
  );
}
