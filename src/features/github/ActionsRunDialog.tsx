/**
 * workflow run 详情对话框（T4.9 UI）：job 列表 + 取消/重跑 + 日志入口。
 *
 * # 可取消与可重跑的判定
 *
 * `status`（queued/in_progress/completed）先于 `conclusion`：只有
 * `completed` 的 run 才显示"重跑"，只有未完成的才显示"取消"。取消一个
 * 排队中的 run GitHub 会 409（映射 `GIT_CONFLICT`），错误提示按 code
 * 走 i18n，这里不做本地预判——远端状态随时在变，预判只会过时。
 *
 * # 动作后本地刷新
 *
 * 取消/重跑成功后刷新 jobs 并通知父级刷新 run 列表（重跑后 run 的
 * status 会从 completed 回到 queued）。
 */
import { useEffect, useState } from 'react';

import { useTranslation } from 'react-i18next';

import { useAppError } from '@/lib/errors';
import { repoActionsRunCancel, repoActionsRunJobs, repoActionsRunRerun } from '@/lib/ipc';
import type { RunJob, WorkflowRunSummary } from '@/lib/ipc';
import { pushToast } from '@/stores/toastStore';

import { Button } from '@/ui/components/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/ui/components/dialog';
import { ErrorState } from '@/ui/components/error-state';

import { ActionsLogDialog } from './ActionsLogDialog';

/** 与其他 GitHub 页一致的站点常量。 */
const HOST = 'github.com';

export interface ActionsRunDialogProps {
  /** 目标 run；`null` 表示关闭。 */
  readonly target: {
    readonly owner: string;
    readonly repo: string;
    readonly run: WorkflowRunSummary;
  } | null;
  readonly onOpenChange: (open: boolean) => void;
  /** 取消/重跑成功后回调（父级刷新 run 列表）。 */
  readonly onRunChanged: () => void;
}

export function ActionsRunDialog({ target, onOpenChange, onRunChanged }: ActionsRunDialogProps) {
  const { t } = useTranslation('shell');
  const { show } = useAppError();
  const [jobs, setJobs] = useState<readonly RunJob[]>([]);
  const [failed, setFailed] = useState(false);
  const [loading, setLoading] = useState(false);
  const [acting, setActing] = useState(false);
  const [logTarget, setLogTarget] = useState<{
    owner: string;
    repo: string;
    jobId: number;
    jobName: string;
  } | null>(null);

  const run = target?.run ?? null;

  useEffect(() => {
    if (target === null) {
      void Promise.resolve().then(() => {
        setJobs([]);
        setFailed(false);
        setLogTarget(null);
      });
      return;
    }
    const cancelled = { value: false };
    void Promise.resolve()
      .then(() => {
        setLoading(true);
        setFailed(false);
        return repoActionsRunJobs(HOST, target.owner, target.repo, target.run.id);
      })
      .then((list) => {
        if (!cancelled.value) {
          setJobs(list);
        }
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

  const refreshJobs = async (): Promise<void> => {
    if (target === null) {
      return;
    }
    const list = await repoActionsRunJobs(HOST, target.owner, target.repo, target.run.id).catch(
      () => [] as RunJob[],
    );
    setJobs(list);
  };

  const act = async (kind: 'cancel' | 'rerun'): Promise<void> => {
    if (run === null || target === null) {
      return;
    }
    setActing(true);
    try {
      if (kind === 'cancel') {
        await repoActionsRunCancel(HOST, target.owner, target.repo, run.id);
        pushToast({ tone: 'success', title: t('github.actions.cancelToast') });
      } else {
        await repoActionsRunRerun(HOST, target.owner, target.repo, run.id);
        pushToast({ tone: 'success', title: t('github.actions.rerunToast') });
      }
      await refreshJobs();
      onRunChanged();
    } catch (raw) {
      show(raw);
    } finally {
      setActing(false);
    }
  };

  return (
    <Dialog open={target !== null} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-2xl" closeLabel={t('common:actions.close')}>
        <DialogHeader>
          <DialogTitle>{run?.name ?? ''}</DialogTitle>
          <DialogDescription>
            {run
              ? `${target?.owner}/${target?.repo} · ${t('github.actions.runNumber', { number: run.runNumber })}`
              : ''}
          </DialogDescription>
        </DialogHeader>

        {run !== null ? (
          <div className="flex flex-wrap items-center gap-2 text-12 text-fg-subtle">
            <span
              className="rounded-sm border border-line px-1.5 py-0.5"
              data-testid="actions-run-status"
            >
              {run.status === 'completed' && run.conclusion !== null
                ? t(`github.actions.conclusion.${run.conclusion}`, { defaultValue: run.conclusion })
                : t(`github.actions.status.${run.status}`, { defaultValue: run.status })}
            </span>
            {run.headBranch !== null && run.headBranch !== undefined ? (
              <span className="font-mono">{run.headBranch}</span>
            ) : null}
            <span>{t('github.prs.author', { author: run.actor })}</span>
            {run.status !== 'completed' ? (
              <Button
                type="button"
                variant="secondary"
                disabled={acting}
                onClick={() => void act('cancel')}
                data-testid="actions-run-cancel"
              >
                {t('github.actions.cancelRun')}
              </Button>
            ) : (
              <Button
                type="button"
                variant="secondary"
                disabled={acting}
                onClick={() => void act('rerun')}
                data-testid="actions-run-rerun"
              >
                {t('github.actions.rerunRun')}
              </Button>
            )}
          </div>
        ) : null}

        {loading ? (
          <p className="text-13 text-fg-subtle" data-testid="actions-jobs-loading">
            {t('github.repos.loading')}
          </p>
        ) : null}

        {failed && !loading ? <ErrorState title={t('github.repos.listErrorHint')} /> : null}

        {!loading && !failed ? (
          <ul className="flex max-h-[50vh] flex-col gap-1 overflow-auto" data-testid="actions-jobs">
            {jobs.length === 0 ? (
              <li className="text-13 text-fg-subtle">{t('github.actions.noJobs')}</li>
            ) : (
              jobs.map((job) => (
                <li
                  key={job.id}
                  className="flex items-center gap-2 rounded-md border border-line bg-surface px-2.5 py-1.5 text-13"
                >
                  <span className="font-medium">{job.name}</span>
                  <span className="text-12 text-fg-muted">
                    {job.status === 'completed' && job.conclusion !== null
                      ? t(`github.actions.conclusion.${job.conclusion}`, {
                          defaultValue: job.conclusion,
                        })
                      : t(`github.actions.status.${job.status}`, { defaultValue: job.status })}
                  </span>
                  <Button
                    type="button"
                    variant="secondary"
                    className="ml-auto"
                    onClick={() =>
                      setLogTarget({
                        owner: target?.owner ?? '',
                        repo: target?.repo ?? '',
                        jobId: job.id,
                        jobName: job.name,
                      })
                    }
                    data-testid={`actions-log-open-${job.id}`}
                  >
                    {t('github.actions.viewLogs')}
                  </Button>
                </li>
              ))
            )}
          </ul>
        ) : null}

        <ActionsLogDialog
          target={logTarget}
          onOpenChange={(open) => setLogTarget(open ? logTarget : null)}
        />
      </DialogContent>
    </Dialog>
  );
}
