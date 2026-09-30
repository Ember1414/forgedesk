//! 冲突页（T3.1 最小状态机视图）。
//!
//! 结构：操作横幅（操作类型 + rebase 进度 + 目标分支）→ 冲突文件列表
//! （类别标签 + 三方版本可用性 + 标记已解决）→ 底部操作条（继续 / 跳过 / 中止）。
//!
//! # 刻意的取舍
//!
//! 1. **本轮只做状态机视图**：逐块解决、三方内容对照与字符级差异属于
//!    T3.2 的三栏编辑器（交互方案待用户确认）。本页给 T3.2 提供
//!    "从状态采集到 continue/abort/skip 的完整骨架"。
//! 2. **数据源是 `git_conflict_state`**（index stage）：即使工作区文件的
//!    `<<<<<<<` 标记被手动删掉，文件仍然出现在列表里——后端保证。
//! 3. **"又停在冲突上"不是错误**：continue 的结果带新的冲突清单时，
//!    本页只刷新状态（query 失效），不弹错误提示。
//! 4. 中止是破坏性动作（撤销操作的全部改动）：走 `AlertDialog`，
//!    `impact` 必填——红线 R7 在 UI 层的闸门。

import { useState } from 'react';

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { CheckCircle2, GitBranch, RefreshCw } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useParams } from 'react-router-dom';

import { ConflictEditor, EditorPlaceholder } from '@/features/conflict/ConflictEditor';

import { EmptyState } from '@/ui/components/empty-state';
import { ErrorState } from '@/ui/components/error-state';
import { IconButton } from '@/ui/components/icon-button';
import { Skeleton } from '@/ui/components/skeleton';
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
import { useAppError } from '@/lib/errors';
import {
  gitConflictAbort,
  gitConflictContinue,
  gitConflictSkip,
  gitConflictState,
} from '@/lib/ipc';
import { conflictKey } from '@/lib/queryKeys';

/** 冲突页面：T3.1 状态机视图（三栏编辑器归 T3.2）。 */
export function RepoConflictPage() {
  const params = useParams();
  const repoId = Number(params.repoId);
  const { t } = useTranslation('shell');
  const appError = useAppError();
  const queryClient = useQueryClient();
  const [abortOpen, setAbortOpen] = useState(false);

  const stateQuery = useQuery({
    queryKey: conflictKey(repoId),
    queryFn: () => gitConflictState(repoId),
    enabled: Number.isFinite(repoId) && repoId > 0,
  });

  const invalidate = () => {
    void queryClient.invalidateQueries({ queryKey: conflictKey(repoId) });
  };

  const continueOp = useMutation({
    mutationFn: () => gitConflictContinue(repoId),
    // "又停在新的冲突上"也是成功：状态刷新后列表更新，不弹错误
    onSuccess: invalidate,
    onError: appError.show,
  });
  const skipOp = useMutation({
    mutationFn: () => gitConflictSkip(repoId),
    onSuccess: invalidate,
    onError: appError.show,
  });
  const abortOp = useMutation({
    mutationFn: () => gitConflictAbort(repoId),
    onSuccess: () => {
      setAbortOpen(false);
      invalidate();
    },
    onError: appError.show,
  });

  // 载荷来自 IPC：真实后端不会给 null，但防御通用 mock / 异常路径，
  // 崩溃的空白页比"没有冲突"的空态更让用户困惑
  const state = stateQuery.data;
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  // 本会话已解决的文件（state 只列未解决；已解决项在侧栏里可识别但不可编辑，
  // 内容复查走工作区 diff 页——T3.2 不重复做）
  const [resolvedPaths, setResolvedPaths] = useState<readonly string[]>([]);
  const fileResolved = (path: string) => {
    setResolvedPaths((previous) => (previous.includes(path) ? previous : [...previous, path]));
    setSelectedPath(null);
  };
  const progress =
    state?.currentStep != null && state.totalSteps !== null
      ? t('pages.repoConflict.progress', {
          current: state.currentStep,
          total: state.totalSteps,
        })
      : null;

  return (
    <section className="flex h-full min-h-0 flex-col" data-testid="conflict-page">
      <header className="flex flex-wrap items-center justify-between gap-2 border-b border-line px-4 py-3">
        <h1 className="text-20 font-semibold tracking-tight">{t('pages.repoConflict.title')}</h1>
        <div className="flex flex-wrap items-center gap-2">
          {state?.opKind ? (
            <>
              <GitBranch aria-hidden className="size-4 text-fg-muted" />
              <span className="font-medium" data-testid="conflict-op-kind">
                {t(`pages.repoConflict.op.${state.opKind}`)}
              </span>
              {progress === null ? null : (
                <span className="text-sm text-fg-muted" data-testid="conflict-progress">
                  {progress}
                </span>
              )}
              {state.intoBranch === null ? null : (
                <span className="text-sm text-fg-muted" data-testid="conflict-into-branch">
                  {t('pages.repoConflict.intoBranch', { branch: state.intoBranch })}
                </span>
              )}
              {state.headName === null ? null : (
                <span className="text-sm text-fg-muted" data-testid="conflict-head-name">
                  {t('pages.repoConflict.headName', { name: state.headName })}
                </span>
              )}
              {/* 文件级解决进度（T3.3 任务书第 4 条）：已解决数来自本会话记录，
                  总数 = 已解决 + 当前未解决（会话外解决的不计入，如实显示） */}
              <span className="text-sm text-fg-muted" data-testid="conflict-file-progress">
                {t('pages.repoConflict.fileProgress', {
                  resolved: resolvedPaths.length,
                  total: resolvedPaths.length + state.files.length,
                })}
              </span>
            </>
          ) : null}
          <IconButton
            label={t('pages.repoConflict.refresh')}
            onClick={() => void stateQuery.refetch()}
          >
            <RefreshCw aria-hidden className="size-4" />
          </IconButton>
        </div>
      </header>

      {stateQuery.isPending ? (
        <div className="flex flex-col gap-3 p-4" data-testid="conflict-page-loading">
          <Skeleton className="h-12 w-full" />
          <Skeleton className="h-16 w-full" />
          <Skeleton className="h-16 w-full" />
        </div>
      ) : stateQuery.isError ? (
        <div className="p-4">
          <ErrorState
            title={t('pages.repoConflict.title')}
            hint={t('pages.repoConflict.emptyHint')}
            retryLabel={t('common:actions.retry')}
            onRetry={() => void stateQuery.refetch()}
          />
        </div>
      ) : !state?.opKind ? (
        <div className="p-4" data-testid="conflict-page-empty">
          <EmptyState
            title={t('pages.repoConflict.empty')}
            description={t('pages.repoConflict.emptyHint')}
          />
        </div>
      ) : (
        <div className="flex min-h-0 flex-1">
          <aside
            className="w-64 shrink-0 overflow-y-auto border-r border-line"
            data-testid="conflict-file-list"
          >
            {state.files.length === 0 ? (
              <div
                className="flex items-center gap-2 px-4 py-6 text-sm text-success"
                data-testid="conflict-all-resolved"
              >
                <CheckCircle2 aria-hidden className="size-4" />
                <span>{t('pages.repoConflict.allResolved')}</span>
              </div>
            ) : (
              <ul>
                {state.files.map((file) => {
                  const selected = file.path === selectedPath;
                  return (
                    <li key={file.path}>
                      <button
                        type="button"
                        className="w-full truncate px-4 py-2 text-left font-mono text-13 hover:bg-surface-sunken"
                        aria-current={selected ? 'true' : undefined}
                        data-selected={selected ? 'true' : undefined}
                        data-testid={`conflict-list-item-${file.path}`}
                        onClick={() => setSelectedPath(file.path)}
                      >
                        {file.path}
                      </button>
                    </li>
                  );
                })}
              </ul>
            )}
            {resolvedPaths.length === 0 ? null : (
              <ul className="border-t border-line">
                {resolvedPaths.map((path) => (
                  <li
                    key={path}
                    className="px-4 py-2 font-mono text-13 text-fg-muted"
                    data-testid={`conflict-resolved-item-${path}`}
                  >
                    <CheckCircle2 aria-hidden className="mr-1 inline size-3 text-success" />
                    {path}
                  </li>
                ))}
              </ul>
            )}
          </aside>
          <main className="flex min-h-0 min-w-0 flex-1 flex-col">
            {selectedPath === null || resolvedPaths.includes(selectedPath) ? (
              <EditorPlaceholder />
            ) : (
              <ConflictEditor
                key={`${selectedPath}-${state.files.length}`}
                repoId={repoId}
                path={selectedPath}
                onResolved={fileResolved}
              />
            )}
          </main>
        </div>
      )}

      {state?.opKind ? (
        <div className="flex flex-wrap items-center gap-2 border-t border-line px-4 py-3">
          <span className="text-sm text-fg-muted" data-testid="conflict-unresolved-count">
            {state.files.length === 0
              ? t('pages.repoConflict.allResolved')
              : t('pages.repoConflict.unresolved', { count: state.files.length })}
          </span>
          <div className="ml-auto flex items-center gap-2">
            {state.canSkip ? (
              <Button
                variant="secondary"
                disabled={skipOp.isPending}
                onClick={() => skipOp.mutate()}
                data-testid="conflict-skip"
              >
                {t('pages.repoConflict.skip')}
              </Button>
            ) : null}
            <Button
              variant="secondary"
              disabled={!state.canAbort || abortOp.isPending}
              onClick={() => setAbortOpen(true)}
              data-testid="conflict-abort"
            >
              {t('pages.repoConflict.abort')}
            </Button>
            <Button
              disabled={!state.canContinue || continueOp.isPending}
              onClick={() => continueOp.mutate()}
              data-testid="conflict-continue"
            >
              {t('pages.repoConflict.continue')}
            </Button>
          </div>
        </div>
      ) : null}

      {/* 中止确认（破坏性动作：AlertDialog + impact，红线 R7） */}
      <AlertDialog open={abortOpen} onOpenChange={setAbortOpen}>
        <AlertDialogContent impact={t('pages.repoConflict.abortImpact')} tone="danger">
          <AlertDialogHeader>
            <AlertDialogTitle>{t('pages.repoConflict.abortTitle')}</AlertDialogTitle>
            <AlertDialogDescription>
              {t('pages.repoConflict.abortDescription')}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t('pages.repoConflict.cancel')}</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => abortOp.mutate()}
              data-testid="conflict-abort-confirm"
            >
              {t('pages.repoConflict.abortConfirm')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </section>
  );
}
