/**
 * 合并对话框（T3.4）：源分支 + 策略 → 预览（计划）→ 执行。
 *
 * 两段式：先 gitMergePrepare 拿计划（快进裁决 / 独有提交 / 冲突预检 /
 * 等价命令），用户看完预览再执行；执行返回 conflicted 时导航到冲突页。
 * `previewAvailable === false`（git 太旧）时明示"无法预检"，不假装预检过。
 */
import { useMemo, useState } from 'react';

import { useMutation, useQueryClient } from '@tanstack/react-query';
import { AlertTriangle, CheckCircle2, CircleAlert } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';

import { gitMergeExecute, gitMergePrepare } from '@/lib/ipc';
import type { MergePlan, MergeStrategy } from '@/lib/ipc';
import {
  BRANCHES_QUERY_KEY,
  CONFLICT_QUERY_KEY,
  LOG_QUERY_KEY,
  STATUS_QUERY_KEY,
} from '@/lib/queryKeys';
import { useAppError } from '@/lib/errors';
import { pushToast } from '@/stores/toastStore';

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
import { ToggleGroup } from '@/ui/components/toggle-group';

/** 策略选项（顺序即界面顺序；最常用在前）。 */
const STRATEGIES: readonly { readonly value: MergeStrategy; readonly labelKey: string }[] = [
  { value: 'merge', labelKey: 'branches.merge.strategy.merge' },
  { value: 'noFf', labelKey: 'branches.merge.strategy.noFf' },
  { value: 'squash', labelKey: 'branches.merge.strategy.squash' },
  { value: 'fastForwardOnly', labelKey: 'branches.merge.strategy.fastForwardOnly' },
  { value: 'ours', labelKey: 'branches.merge.strategy.ours' },
  { value: 'theirs', labelKey: 'branches.merge.strategy.theirs' },
];

/** 合并对话框。 */
export function MergeDialog({
  repoId,
  currentBranch,
  locals,
  open,
  onOpenChange,
}: {
  readonly repoId: number;
  readonly currentBranch: string | null;
  /** 本地分支名（源分支候选；排除当前分支）。 */
  readonly locals: readonly string[];
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
}) {
  const { t } = useTranslation('shell');
  const appError = useAppError();
  const navigate = useNavigate();
  const queryClient = useQueryClient();

  const [source, setSource] = useState('');
  const [strategy, setStrategy] = useState<MergeStrategy>('merge');
  const [message, setMessage] = useState('');
  const [plan, setPlan] = useState<MergePlan | null>(null);

  const candidates = useMemo(
    () => locals.filter((name) => name !== currentBranch),
    [locals, currentBranch],
  );

  const invalidate = () => {
    for (const key of [BRANCHES_QUERY_KEY, LOG_QUERY_KEY, STATUS_QUERY_KEY, CONFLICT_QUERY_KEY]) {
      void queryClient.invalidateQueries({ queryKey: [key, repoId] });
    }
  };

  const prepareMutation = useMutation({
    mutationFn: () => gitMergePrepare(repoId, { source, strategy }),
    onSuccess: (result) => {
      setPlan(result);
      setMessage(result.defaultMessage);
    },
    onError: appError.show,
  });

  const executeMutation = useMutation({
    mutationFn: () =>
      gitMergeExecute(repoId, {
        planId: plan?.planId ?? '',
        ...(message === '' ? {} : { message }),
      }),
    onSuccess: (outcome) => {
      invalidate();
      onOpenChange(false);
      setPlan(null);
      if (outcome.kind === 'conflicted') {
        // 冲突是结果不是错误：把用户送到冲突页逐个解决
        void navigate(`/repo/${repoId}/conflict`);
        return;
      }
      if (outcome.kind !== 'alreadyUpToDate') {
        pushToast({ tone: 'success', title: t('branches.merge.done') });
      }
    },
    onError: appError.show,
  });

  const close = (next: boolean) => {
    if (!next) {
      setPlan(null);
      setSource('');
      setStrategy('merge');
      setMessage('');
    }
    onOpenChange(next);
  };

  return (
    <Dialog open={open} onOpenChange={close}>
      <DialogContent closeLabel={t('common:actions.close')} className="max-w-xl">
        <DialogHeader>
          <DialogTitle>{t('branches.merge.title')}</DialogTitle>
          <DialogDescription>{t('branches.merge.hint')}</DialogDescription>
        </DialogHeader>

        {plan === null ? (
          <div className="flex flex-col gap-3">
            <label className="flex flex-col gap-1">
              <span className="text-12 font-medium text-fg-muted">
                {t('branches.merge.source')}
              </span>
              <select
                className="h-8 rounded-md border border-line bg-surface px-2 text-13"
                value={source}
                onChange={(event) => setSource(event.target.value)}
                data-testid="merge-source"
              >
                <option value="">{t('branches.merge.sourcePlaceholder')}</option>
                {candidates.map((name) => (
                  <option key={name} value={name}>
                    {name}
                  </option>
                ))}
              </select>
            </label>
            <ToggleGroup
              label={t('branches.merge.strategyLabel')}
              value={strategy}
              onValueChange={(value) => setStrategy(value as MergeStrategy)}
              options={STRATEGIES.map((item) => ({
                value: item.value,
                label: t(item.labelKey),
              }))}
            />
          </div>
        ) : prepareMutation.isPending ? (
          <Skeleton className="h-32 w-full" />
        ) : (
          <div className="flex flex-col gap-3" data-testid="merge-plan">
            {/* 裁决 + 预检结果 */}
            {plan.verdict === 'upToDate' ? (
              <p className="flex items-center gap-2 text-13" data-testid="merge-verdict">
                <CheckCircle2 aria-hidden className="size-4 text-success" />
                {t('branches.merge.upToDate')}
              </p>
            ) : plan.conflicted.length > 0 ? (
              <div
                className="flex items-start gap-2 rounded-md border border-warning bg-warning/10 p-2 text-13"
                data-testid="merge-conflicts"
              >
                <AlertTriangle aria-hidden className="mt-0.5 size-4 shrink-0 text-warning" />
                <div className="min-w-0">
                  <p>{t('branches.merge.conflictPreview')}</p>
                  <p className="truncate font-mono text-12 text-fg-muted">
                    {plan.conflicted.join(', ')}
                  </p>
                </div>
              </div>
            ) : (
              <p className="flex items-center gap-2 text-13" data-testid="merge-verdict">
                <CheckCircle2 aria-hidden className="size-4 text-success" />
                {plan.verdict === 'fastForward'
                  ? t('branches.merge.willFastForward', { count: plan.sourceCommitCount })
                  : t('branches.merge.willMergeCommit', { count: plan.sourceCommitCount })}
              </p>
            )}

            {plan.previewAvailable ? null : (
              <p
                className="flex items-center gap-2 text-13 text-warning"
                data-testid="merge-preview-unavailable"
              >
                <CircleAlert aria-hidden className="size-4" />
                {t('branches.merge.previewUnavailable')}
              </p>
            )}

            {/* 独有提交清单 */}
            {plan.sourceOnlyCommits.length > 0 ? (
              <div className="max-h-32 overflow-y-auto rounded-md border border-line">
                {plan.sourceOnlyCommits.map((commit) => (
                  <p
                    key={commit.oid}
                    className="truncate border-b border-line px-2 py-1 font-mono text-12 last:border-b-0"
                  >
                    <span className="text-fg-subtle">{commit.oid.slice(0, 7)} </span>
                    {commit.subject}
                  </p>
                ))}
              </div>
            ) : null}

            <Input
              value={message}
              onChange={(event) => setMessage(event.target.value)}
              label={t('branches.merge.messageLabel')}
              data-testid="merge-message"
            />
            <p
              className="rounded-md bg-surface-sunken px-2 py-1 font-mono text-12 text-fg-muted"
              data-testid="merge-equivalent"
            >
              {t('branches.merge.equivalent')}：{plan.equivalentCommand}
            </p>
          </div>
        )}

        <DialogFooter>
          <DialogClose asChild>
            <Button variant="secondary">{t('common:actions.cancel')}</Button>
          </DialogClose>
          {plan === null ? (
            <Button
              disabled={source === '' || prepareMutation.isPending}
              onClick={() => prepareMutation.mutate()}
              data-testid="merge-preview"
            >
              {t('branches.merge.preview')}
            </Button>
          ) : (
            <Button
              disabled={executeMutation.isPending || plan.verdict === 'upToDate'}
              onClick={() => executeMutation.mutate()}
              data-testid="merge-execute"
            >
              {t('branches.merge.execute')}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
