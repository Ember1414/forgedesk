/**
 * 拖拽式 Rebase 面板（T3.6）。
 *
 * 数据流：
 *   open → gitRebaseRange（TanStack Query；区间清单来自仓库事实，不是界面的分页数据）
 *        → entries（全 pick 的初始计划；用户编辑存在本地覆盖状态里，不污染 query 缓存）
 *        → debounce 250ms → gitRebasePreviewOnly（后端预测 + 兜底校验）
 *        → 确认对话框（将重写 N 个提交 + force-with-lease 警示）
 *        → gitRebaseExecute（执行前打 pre-head-move 快照，id 随结果回传）
 *        → 三结局：completed / pausedConflict（去冲突页）/ pausedEdit（改完继续）
 *
 * 状态模型说明（为什么不是 useEffect 装载 + setState）：
 * 区间清单是服务端状态 → TanStack Query；用户对 steps 的编辑是本地覆盖状态，
 * 用 `{ key, entries }` 绑定到当前区间——区间变了覆盖自然失效，不需要在
 * effect 里做"卸载旧状态"的同步 setState（那会触发级联渲染，且被 lint 明确
 * 拦下）。预览同理：结果与请求时的计划签名绑定，签名不匹配视为未就绪。
 *
 * 进度：执行期间每 400ms 轮询 `gitConflictState` 的 currentStep/totalSteps
 * （sequencer 进度，T3.1 状态机已有；只读、零后端改动）。本地 rebase 通常
 * 毫秒级完成，进度显示是长尾场景的兜底——不为此引入 rebase:step 事件：
 * T3.7 的"一次调用到暂停/完成"语义与幂等设计已被测试钉住，改成逐步驱动
 * 的收益不抵风险（评估结论记在回报里，供人类复核）。
 *
 * 中止：走 `gitConflictAbort`（git rebase --abort + 它自己的快照与校验）；
 * 失败时把 execute 回传的 pre-head-move 快照 id 展示出来当兜底——用户
 * 永远能看到"还能从哪里回去"。
 */
import { useEffect, useMemo, useRef, useState } from 'react';

import { AlertTriangle, CheckCircle2 } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import { useQuery, useQueryClient } from '@tanstack/react-query';

import {
  gitConflictAbort,
  gitConflictState,
  gitRebaseContinueEdit,
  gitRebaseExecute,
  gitRebasePreviewOnly,
  gitRebaseRange,
} from '@/lib/ipc';
import type { RebaseOutcome, RebasePreview, ReorderAction } from '@/lib/ipc';
import { useAppError } from '@/lib/errors';
import {
  BRANCHES_QUERY_KEY,
  CONFLICT_QUERY_KEY,
  LOG_QUERY_KEY,
  STATUS_QUERY_KEY,
} from '@/lib/queryKeys';
import { pushToast } from '@/stores/toastStore';

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
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/ui/components/dialog';
import { Skeleton } from '@/ui/components/skeleton';

import { RebasePreviewPane } from './RebasePreviewPane';
import { RebaseStepList } from './RebaseStepList';
import {
  computeIssues,
  initialEntries,
  isExecutable,
  setAction,
  toRequestEntries,
  type PlanEntry,
} from './planState';

export interface RebasePanelProps {
  readonly repoId: number;
  /** 区间左端（最旧选中提交的父）。 */
  readonly base: string;
  /** 区间右端（最新选中提交）。 */
  readonly head: string;
  /** 入口预设：打开时把指定提交的初始动作设为该值（右键 reword/edit/drop 用）。 */
  readonly preset?: { readonly oid: string; readonly action: ReorderAction } | null;
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
}

/** 计划签名：预览结果只有对同一份计划才有效。 */
function planSignature(entries: readonly PlanEntry[]): string {
  return entries.map((entry) => `${entry.oid}:${entry.action}:${entry.newMessage ?? ''}`).join('|');
}

/** 拖拽式 rebase 面板。 */
export function RebasePanel({ repoId, base, head, preset, open, onOpenChange }: RebasePanelProps) {
  const { t } = useTranslation('shell');
  const appError = useAppError();
  const navigate = useNavigate();
  const queryClient = useQueryClient();

  const [edited, setEdited] = useState<{
    readonly key: string;
    readonly entries: readonly PlanEntry[];
  } | null>(null);
  const [previewState, setPreviewState] = useState<{
    readonly key: string;
    readonly data: RebasePreview;
  } | null>(null);
  const [previewFailedFor, setPreviewFailedFor] = useState<string | null>(null);
  const [executing, setExecuting] = useState(false);
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState<{ current: number; total: number } | null>(null);
  const [outcome, setOutcome] = useState<RebaseOutcome | null>(null);
  const [confirmOpen, setConfirmOpen] = useState(false);
  const [todoOpen, setTodoOpen] = useState(false);
  const previewSeq = useRef(0);

  const presetOid = preset?.oid ?? null;
  const presetAction = preset?.action ?? null;
  const conflictPath = `/repo/${String(repoId)}/conflict`;
  const rangeKey = `${String(repoId)}:${base}:${head}`;

  // 区间清单：服务端状态走 TanStack Query（打开时才请求；每次打开取新值）
  const rangeQuery = useQuery({
    queryKey: ['rebase-range', repoId, base, head],
    queryFn: () => gitRebaseRange(repoId, base, head),
    enabled: open,
    staleTime: 0,
    gcTime: 0,
  });

  // 初始计划（从区间清单派生；preset 在派生期应用）
  const derivedEntries = useMemo(() => {
    if (rangeQuery.data === undefined) {
      return null;
    }
    let next: readonly PlanEntry[] = initialEntries(rangeQuery.data);
    if (presetOid !== null && presetAction !== null) {
      const index = next.findIndex((entry) => entry.oid === presetOid);
      if (index >= 0) {
        next = setAction(next, index, presetAction);
      }
    }
    return next;
  }, [rangeQuery.data, presetOid, presetAction]);

  // 本地编辑覆盖：与区间签名绑定，换区间（key 变）自动失效
  const entries: readonly PlanEntry[] | null =
    edited !== null && edited.key === rangeKey ? edited.entries : derivedEntries;
  const updateEntries = (next: readonly PlanEntry[]) => {
    setEdited({ key: rangeKey, entries: next });
  };

  const issues = entries === null ? [] : computeIssues(entries);
  const hasIssues = issues.length > 0;
  const signature = entries === null || hasIssues ? null : planSignature(entries);

  // 预览：debounce 250ms；结果与计划签名绑定（签名不匹配 = 未就绪）
  useEffect(() => {
    if (entries === null || signature === null) {
      return;
    }
    const seq = previewSeq.current + 1;
    previewSeq.current = seq;
    const timer = window.setTimeout(() => {
      gitRebasePreviewOnly(repoId, { base, head, steps: toRequestEntries(entries) })
        .then((data) => {
          if (previewSeq.current === seq) {
            setPreviewState({ key: signature, data });
            setPreviewFailedFor(null);
          }
        })
        .catch(() => {
          if (previewSeq.current === seq) {
            setPreviewState(null);
            setPreviewFailedFor(signature);
          }
        });
    }, 250);
    return () => {
      window.clearTimeout(timer);
    };
  }, [entries, signature, repoId, base, head]);

  const preview =
    previewState !== null && previewState.key === signature ? previewState.data : null;
  const previewFailed = signature !== null && previewFailedFor === signature;
  const previewPending = signature !== null && preview === null && !previewFailed;

  const invalidate = () => {
    for (const key of [BRANCHES_QUERY_KEY, LOG_QUERY_KEY, STATUS_QUERY_KEY, CONFLICT_QUERY_KEY]) {
      void queryClient.invalidateQueries({ queryKey: [key, repoId] });
    }
  };

  const execute = async () => {
    if (entries === null) {
      return;
    }
    setConfirmOpen(false);
    setExecuting(true);
    setProgress(null);
    // 执行期间轮询 sequencer 进度（只读；失败静默——进度是增强信息）
    const poll = window.setInterval(() => {
      void gitConflictState(repoId)
        .then((state) => {
          if (state.currentStep !== null && state.totalSteps !== null) {
            setProgress({ current: state.currentStep, total: state.totalSteps });
          }
        })
        .catch(() => undefined);
    }, 400);
    try {
      const result = await gitRebaseExecute(repoId, {
        base,
        head,
        steps: toRequestEntries(entries),
      });
      setOutcome(result);
      invalidate();
      if (result.kind === 'completed') {
        pushToast({ tone: 'success', title: t('history.rebase.completed') });
      }
    } catch (error: unknown) {
      appError.show(error);
    } finally {
      window.clearInterval(poll);
      setExecuting(false);
      setProgress(null);
    }
  };

  const abort = async () => {
    setBusy(true);
    try {
      await gitConflictAbort(repoId);
      invalidate();
      pushToast({ tone: 'success', title: t('history.rebase.abortDone') });
      onOpenChange(false);
    } catch (error: unknown) {
      appError.show(error);
    } finally {
      setBusy(false);
    }
  };

  const continueEdit = async () => {
    setBusy(true);
    try {
      const result = await gitRebaseContinueEdit(repoId);
      setOutcome(result);
      invalidate();
      if (result.kind === 'completed') {
        pushToast({ tone: 'success', title: t('history.rebase.completed') });
      }
    } catch (error: unknown) {
      appError.show(error);
    } finally {
      setBusy(false);
    }
  };

  const close = (next: boolean) => {
    if (!next) {
      setEdited(null);
      setPreviewState(null);
      setPreviewFailedFor(null);
      setOutcome(null);
      setConfirmOpen(false);
      setTodoOpen(false);
    }
    onOpenChange(next);
  };

  const canExecute = entries !== null && isExecutable(entries) && preview !== null && !executing;

  const loadFailed = rangeQuery.isError;

  return (
    <>
      <Dialog open={open} onOpenChange={close}>
        <DialogContent closeLabel={t('common:actions.close')} className="max-w-5xl">
          <DialogHeader>
            <DialogTitle>{t('history.rebase.title')}</DialogTitle>
            <DialogDescription>
              {t('history.rebase.description', { base: base.slice(0, 7), head: head.slice(0, 7) })}
            </DialogDescription>
          </DialogHeader>

          {outcome !== null ? (
            <div className="flex flex-col gap-3" data-testid="rebase-outcome">
              {outcome.kind === 'completed' ? (
                <>
                  <p className="flex items-center gap-2 text-13">
                    <CheckCircle2 aria-hidden className="size-4 text-success" />
                    {t('history.rebase.completed')}
                  </p>
                  <p className="font-mono text-12 text-fg-muted">{outcome.oid.slice(0, 7)}</p>
                </>
              ) : outcome.kind === 'pausedConflict' ? (
                <>
                  <p className="flex items-center gap-2 text-13">
                    <AlertTriangle aria-hidden className="size-4 text-warning" />
                    {t('history.rebase.pausedConflict')}
                  </p>
                  <p className="truncate font-mono text-12 text-fg-muted">
                    {outcome.conflicts.join(', ')}
                  </p>
                </>
              ) : (
                <>
                  <p className="flex items-center gap-2 text-13">
                    <AlertTriangle aria-hidden className="size-4 text-warning" />
                    {t('history.rebase.pausedEdit')}
                  </p>
                  <p className="font-mono text-12 text-fg-muted">{outcome.oid.slice(0, 7)}</p>
                </>
              )}
              <p className="text-12 text-fg-muted" data-testid="rebase-snapshot-hint">
                {outcome.snapshotId === null
                  ? t('history.rebase.noSnapshot')
                  : t('history.rebase.snapshotHint', { id: outcome.snapshotId })}
              </p>
            </div>
          ) : loadFailed ? (
            <p
              className="flex items-center gap-2 text-13 text-danger"
              data-testid="rebase-load-failed"
            >
              <AlertTriangle aria-hidden className="size-4" />
              {t('history.rebase.loadFailed')}
            </p>
          ) : entries === null ? (
            <Skeleton className="h-48 w-full" />
          ) : (
            <div className="grid grid-cols-[minmax(0,3fr)_minmax(0,2fr)] gap-4">
              <div className="max-h-[50vh] overflow-y-auto rounded-md border border-line">
                <div className="border-b border-line bg-surface-sunken px-2 py-1 text-12 text-fg-muted">
                  {t('history.rebase.stepListHint')}
                </div>
                <RebaseStepList
                  entries={entries}
                  issues={issues}
                  disabled={executing}
                  onChange={updateEntries}
                />
              </div>
              <div className="max-h-[50vh] overflow-y-auto">
                <RebasePreviewPane entries={entries} preview={preview} pending={previewPending} />
              </div>
            </div>
          )}

          {outcome === null && entries !== null && preview !== null && todoOpen ? (
            <pre
              className="mt-3 max-h-40 overflow-auto rounded-md bg-surface-sunken p-2 font-mono text-12 text-fg-muted"
              data-testid="rebase-todo"
            >
              {preview.todoText.trimEnd()}
            </pre>
          ) : null}

          <DialogFooter>
            {outcome === null ? (
              <>
                <span className="mr-auto text-12 text-fg-muted" data-testid="rebase-progress">
                  {executing
                    ? progress === null
                      ? t('history.rebase.runningIndeterminate')
                      : t('history.rebase.running', {
                          current: progress.current,
                          total: progress.total,
                        })
                    : null}
                </span>
                <Button
                  variant="ghost"
                  size="sm"
                  disabled={preview === null}
                  onClick={() => setTodoOpen((current) => !current)}
                  data-testid="rebase-todo-toggle"
                >
                  {t('history.rebase.todoToggle')}
                </Button>
                <DialogClose asChild>
                  <Button variant="secondary">{t('common:actions.cancel')}</Button>
                </DialogClose>
                <Button
                  disabled={!canExecute}
                  onClick={() => setConfirmOpen(true)}
                  data-testid="rebase-execute"
                >
                  {t('history.rebase.execute')}
                </Button>
              </>
            ) : outcome.kind === 'completed' ? (
              <DialogClose asChild>
                <Button data-testid="rebase-close">{t('common:actions.close')}</Button>
              </DialogClose>
            ) : (
              <>
                {outcome.kind === 'pausedConflict' ? (
                  <Button
                    variant="secondary"
                    onClick={() => {
                      close(false);
                      void navigate(conflictPath);
                    }}
                    data-testid="rebase-goto-conflict"
                  >
                    {t('history.rebase.gotoConflict')}
                  </Button>
                ) : (
                  <Button
                    disabled={busy}
                    onClick={() => void continueEdit()}
                    data-testid="rebase-continue-edit"
                  >
                    {t('history.rebase.continueEdit')}
                  </Button>
                )}
                <Button
                  variant="danger"
                  disabled={busy}
                  onClick={() => void abort()}
                  data-testid="rebase-abort"
                >
                  {t('history.rebase.abort')}
                </Button>
              </>
            )}
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <AlertDialog open={confirmOpen} onOpenChange={setConfirmOpen}>
        <AlertDialogContent
          impactLabel={t('history.rebase.confirm.impact')}
          tone={preview?.touchesPushed === true ? 'warning' : 'danger'}
          impact={
            <span className="flex flex-col gap-1">
              <span>
                {t('history.rebase.confirm.rewrite', { count: preview?.affectedCount ?? 0 })}
              </span>
              {preview?.touchesPushed === true ? (
                <span className="text-warning">{t('history.rebase.confirm.pushed')}</span>
              ) : null}
              <span>{t('history.rebase.confirm.snapshot')}</span>
            </span>
          }
        >
          <AlertDialogHeader>
            <AlertDialogTitle>{t('history.rebase.confirm.title')}</AlertDialogTitle>
            <AlertDialogDescription>
              {t('history.rebase.confirm.description')}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t('common:actions.cancel')}</AlertDialogCancel>
            <AlertDialogAction onClick={() => void execute()} data-testid="rebase-confirm-execute">
              {t('history.rebase.confirm.action')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}
