/**
 * PR 详情对话框（T4.7 UI）：条件展示 + 三策略合并。
 *
 * # 合并前的"是否满足条件"
 *
 * docs/PLAN.md M4.2 要求合并前展示合并条件；本对话框把
 * `mergeable` / `mergeableState` / 审查状态 / 变更统计全部铺出来，
 * 再让用户选策略并确认。合并请求带 `expectedHeadSha`（打开详情时的
 * head）：期间远端有新提交会被 422 拒绝，错误 hint 是 `head-changed`，
 * 界面提示"远端有新提交，请刷新后重试"——远端版的 PLAN_STALE。
 *
 * # 描述 HTML 已消毒
 *
 * `bodyHtml` 来自后端白名单渲染（与 README 同一规则），这里不再对它
 * 做任何二次解析；链接点击委托为复制（无 opener 插件，与全应用一致）。
 */
import { useEffect, useState } from 'react';

import { useTranslation } from 'react-i18next';

import { useAppError } from '@/lib/errors';
import { repoPullGet, repoPullMerge, repoPullReviews } from '@/lib/ipc';
import type { PullDetail, PullMergeOutcome, PullReview } from '@/lib/ipc';
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
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/ui/components/dialog';
import { ErrorState } from '@/ui/components/error-state';

/** 与其他 GitHub 页一致的站点常量。 */
const HOST = 'github.com';

/** 合并策略。 */
type MergeStrategy = 'merge' | 'squash' | 'rebase';

const STRATEGIES: readonly MergeStrategy[] = ['merge', 'squash', 'rebase'];

/** 详情对话框的目标（仓库 + PR 号）。 */
export interface PullTarget {
  readonly owner: string;
  readonly repo: string;
  readonly number: number;
}

interface PullDetailDialogProps {
  /** 目标；`null` 表示关闭。 */
  readonly target: PullTarget | null;
  readonly onOpenChange: (open: boolean) => void;
  /** 合并成功后回调（列表刷新用）。 */
  readonly onMerged: (outcome: PullMergeOutcome) => void;
}

export function PullDetailDialog({ target, onOpenChange, onMerged }: PullDetailDialogProps) {
  const { t } = useTranslation('shell');
  const { show } = useAppError();
  const [detail, setDetail] = useState<PullDetail | null>(null);
  const [reviews, setReviews] = useState<readonly PullReview[]>([]);
  const [failed, setFailed] = useState(false);
  const [loading, setLoading] = useState(false);
  const [strategy, setStrategy] = useState<MergeStrategy>('squash');
  const [deleteBranch, setDeleteBranch] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [merging, setMerging] = useState(false);

  useEffect(() => {
    if (target === null) {
      void Promise.resolve().then(() => {
        setDetail(null);
        setReviews([]);
        setFailed(false);
        setConfirming(false);
      });
      return;
    }
    const cancelled = { value: false };
    void Promise.resolve()
      .then(() => {
        setLoading(true);
        setFailed(false);
        return Promise.all([
          repoPullGet(HOST, target.owner, target.repo, target.number),
          repoPullReviews(HOST, target.owner, target.repo, target.number).catch(
            () => [] as PullReview[],
          ),
        ]);
      })
      .then(([pull, reviewList]) => {
        if (cancelled.value) {
          return;
        }
        setDetail(pull);
        setReviews(reviewList);
      })
      .catch((raw: unknown) => {
        if (!cancelled.value) {
          setFailed(true);
          show(raw);
        }
      })
      .finally(() => {
        if (!cancelled.value) {
          setLoading(false);
        }
      });
    return () => {
      cancelled.value = true;
    };
  }, [target, show]);

  const headBranch = detail?.headLabel.split(':').pop() ?? '';

  const merge = async () => {
    if (detail === null || target === null) {
      return;
    }
    setMerging(true);
    try {
      const outcome = await repoPullMerge({
        host: HOST,
        owner: target.owner,
        repo: target.repo,
        number: detail.number,
        strategy,
        expectedHeadSha: detail.headSha,
        deleteBranch,
        ...(deleteBranch && headBranch !== '' ? { headBranch } : {}),
      });
      pushToast({
        tone: 'success',
        title: t('github.prs.mergedToast', { number: detail.number }),
        ...(outcome.message === undefined ? {} : { description: outcome.message }),
      });
      setConfirming(false);
      onOpenChange(false);
      onMerged(outcome);
    } catch (raw) {
      setConfirming(false);
      show(raw);
    } finally {
      setMerging(false);
    }
  };

  const mergeConditionKey = (pull: PullDetail): string => {
    if (pull.merged) {
      return 'github.prs.alreadyMerged';
    }
    switch (pull.mergeableState) {
      case 'clean':
        return 'github.prs.conditionClean';
      case 'dirty':
        return 'github.prs.conditionDirty';
      case 'blocked':
        return 'github.prs.conditionBlocked';
      case 'unstable':
        return 'github.prs.conditionUnstable';
      default:
        return pull.mergeable === true
          ? 'github.prs.conditionClean'
          : 'github.prs.conditionUnknown';
    }
  };

  return (
    <Dialog open={target !== null} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-2xl" closeLabel={t('common:actions.close')}>
        <DialogHeader>
          <DialogTitle>
            {t('github.prs.detailTitle', {
              number: target?.number ?? 0,
              title: detail?.title ?? '',
            })}
          </DialogTitle>
          <DialogDescription>{target ? `${target.owner}/${target.repo}` : ''}</DialogDescription>
        </DialogHeader>

        {loading ? (
          <p className="text-13 text-fg-subtle" data-testid="pull-loading">
            {t('github.repos.loading')}
          </p>
        ) : null}

        {failed && !loading ? <ErrorState title={t('github.repos.listErrorHint')} /> : null}

        {detail !== null && !loading ? (
          <div className="flex max-h-[60vh] flex-col gap-3 overflow-auto" data-testid="pull-detail">
            <div className="flex flex-wrap gap-2 text-12 text-fg-subtle">
              <span>{t('github.prs.author', { author: detail.author })}</span>
              <span className="font-mono">
                {detail.headLabel} → {detail.baseLabel}
              </span>
              <span data-testid="pull-stats">
                +{detail.additions} −{detail.deletions} ·{' '}
                {t('github.prs.changedFiles', { count: detail.changedFiles })}
              </span>
            </div>

            <div className="rounded-md border border-line p-3" data-testid="pull-merge-conditions">
              <p className="text-13 font-medium">{t('github.prs.conditionsTitle')}</p>
              <p className="text-12 text-fg-muted" data-testid="pull-mergeable">
                {t(mergeConditionKey(detail))}
              </p>
              <ul className="mt-1 flex flex-col gap-0.5 text-12" data-testid="pull-reviews">
                {reviews.length === 0 ? (
                  <li className="text-fg-subtle">{t('github.prs.noReviews')}</li>
                ) : (
                  reviews.map((review) => (
                    <li key={review.id}>
                      {t('github.prs.reviewItem', {
                        author: review.author,
                        state: t(`github.prs.reviewState.${review.state}`),
                      })}
                    </li>
                  ))
                )}
              </ul>
            </div>

            {detail.bodyHtml !== undefined ? (
              <div
                className="max-h-48 overflow-auto rounded-md border border-line bg-surface p-3 text-13 leading-relaxed"
                onClick={(event) => {
                  const anchor = (event.target as HTMLElement).closest('a');
                  if (anchor !== null) {
                    event.preventDefault();
                    const href = anchor.getAttribute('href') ?? '';
                    if (href !== '') {
                      void navigator.clipboard?.writeText(href).catch(() => undefined);
                      pushToast({ tone: 'info', title: t('github.repos.linkCopiedToast') });
                    }
                  }
                }}
                data-testid="pull-body"
                dangerouslySetInnerHTML={{ __html: detail.bodyHtml }}
              />
            ) : null}

            {!detail.merged ? (
              <div className="flex flex-col gap-2" data-testid="pull-merge-controls">
                <p className="text-13 font-medium">{t('github.prs.strategyTitle')}</p>
                <div
                  role="radiogroup"
                  aria-label={t('github.prs.strategyTitle')}
                  className="flex gap-2"
                >
                  {STRATEGIES.map((item) => (
                    <Button
                      key={item}
                      type="button"
                      variant={strategy === item ? 'primary' : 'secondary'}
                      aria-pressed={strategy === item}
                      onClick={() => setStrategy(item)}
                      data-testid={`pull-strategy-${item}`}
                    >
                      {t(`github.prs.strategy.${item}`)}
                    </Button>
                  ))}
                </div>
                <label className="flex items-center gap-2 text-12">
                  <input
                    type="checkbox"
                    checked={deleteBranch}
                    onChange={(event) => setDeleteBranch(event.target.checked)}
                    data-testid="pull-delete-branch"
                  />
                  {t('github.prs.deleteBranch', { branch: headBranch })}
                </label>
              </div>
            ) : null}
          </div>
        ) : null}

        <div className="flex justify-end gap-2">
          <Button type="button" variant="secondary" onClick={() => onOpenChange(false)}>
            {t('common:actions.close')}
          </Button>
          {detail !== null && !loading && !detail.merged ? (
            <Button
              type="button"
              disabled={merging}
              onClick={() => setConfirming(true)}
              data-testid="pull-merge-open"
            >
              {t('github.prs.mergeOpen')}
            </Button>
          ) : null}
        </div>

        <AlertDialog open={confirming} onOpenChange={setConfirming}>
          <AlertDialogContent impact={t('github.prs.mergeImpact')}>
            <AlertDialogHeader>
              <AlertDialogTitle>
                {t('github.prs.mergeConfirmTitle', { number: detail?.number ?? 0 })}
              </AlertDialogTitle>
              <AlertDialogDescription>
                {t(`github.prs.strategy.${strategy}`)}
                {deleteBranch && headBranch !== ''
                  ? ` · ${t('github.prs.deleteBranch', { branch: headBranch })}`
                  : ''}
              </AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel data-testid="pull-merge-cancel">
                {t('common:actions.cancel')}
              </AlertDialogCancel>
              <AlertDialogAction
                data-testid="pull-merge-confirm"
                onClick={(event) => {
                  event.preventDefault();
                  void merge();
                }}
              >
                {t('github.prs.mergeConfirm')}
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      </DialogContent>
    </Dialog>
  );
}
