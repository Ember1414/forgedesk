/**
 * 合并进行中横幅（T3.4）：`workspace_status.operation === 'merge'` 时在仓库
 * 布局顶部常驻，给出冲突文件数与继续 / 中止动作。
 *
 * 为什么放布局层而不是状态页：合并可能发生在任意入口（分支页合并对话框、
 * 同步条拉取、终端里的 git merge），用户回到哪个页面都该看到"仓库正处于
 * 合并中途"——这是持久状态（刷新后仍在），不是某个页面的局部信息。
 *
 * 数据源：`workspaceStatus`（operation 字段，T1.4 起存在）+
 * `gitConflictState`（T3.1，文件数与继续/中止可用性）。
 */
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { GitMerge } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useParams } from 'react-router-dom';

import { gitConflictAbort, gitConflictState, gitMergeContinue } from '@/lib/ipc';
import { workspaceStatus } from '@/lib/ipc/workspace';
import { CONFLICT_QUERY_KEY, STATUS_QUERY_KEY } from '@/lib/queryKeys';
import { useAppError } from '@/lib/errors';

import { Button } from '@/ui/components/button';

/** 合并进行中横幅。仓库未打开时不渲染。 */
export function MergeBanner() {
  const { t } = useTranslation('shell');
  const params = useParams();
  const repoId = Number(params.repoId);
  const appError = useAppError();
  const queryClient = useQueryClient();

  const statusQuery = useQuery({
    queryKey: [STATUS_QUERY_KEY, repoId],
    queryFn: () => workspaceStatus(repoId),
    enabled: Number.isFinite(repoId) && repoId > 0,
    staleTime: 5_000,
  });

  const conflictQuery = useQuery({
    queryKey: [CONFLICT_QUERY_KEY, repoId],
    queryFn: () => gitConflictState(repoId),
    enabled: Number.isFinite(repoId) && repoId > 0 && statusQuery.data?.operation === 'merge',
  });

  const invalidate = () => {
    void queryClient.invalidateQueries({ queryKey: [STATUS_QUERY_KEY, repoId] });
    void queryClient.invalidateQueries({ queryKey: [CONFLICT_QUERY_KEY, repoId] });
  };

  const continueMutation = useMutation({
    mutationFn: () => gitMergeContinue(repoId),
    onSuccess: invalidate,
    onError: appError.show,
  });
  const abortMutation = useMutation({
    mutationFn: () => gitConflictAbort(repoId),
    onSuccess: invalidate,
    onError: appError.show,
  });

  if (statusQuery.data?.operation !== 'merge' || conflictQuery.data === undefined) {
    return null;
  }
  const state = conflictQuery.data;
  if (state.opKind === null) {
    return null;
  }

  return (
    <div
      className="flex flex-wrap items-center gap-2 rounded-md border border-brand bg-brand/10 px-3 py-2"
      data-testid="merge-banner"
    >
      <GitMerge aria-hidden className="size-4 text-brand" />
      <span className="text-13 font-medium">
        {state.files.length === 0
          ? t('mergeBanner.resolved')
          : t('mergeBanner.conflicted', { count: state.files.length })}
      </span>
      <span className="ml-auto flex items-center gap-2">
        {state.canSkip ? (
          <Button
            size="sm"
            variant="ghost"
            onClick={() => continueMutation.mutate()}
            disabled={continueMutation.isPending}
            data-testid="merge-banner-skip"
          >
            {t('mergeBanner.skip')}
          </Button>
        ) : null}
        <Button
          size="sm"
          variant="ghost"
          disabled={!state.canAbort || abortMutation.isPending}
          onClick={() => abortMutation.mutate()}
          data-testid="merge-banner-abort"
        >
          {t('mergeBanner.abort')}
        </Button>
        <Button
          size="sm"
          disabled={!state.canContinue || continueMutation.isPending}
          onClick={() => continueMutation.mutate()}
          data-testid="merge-banner-continue"
        >
          {t('mergeBanner.continue')}
        </Button>
      </span>
    </div>
  );
}
