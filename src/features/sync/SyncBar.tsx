/**
 * 仓库外壳上的同步条（T2.6）：Fetch / Pull（策略下拉）/ Push + ahead-behind + 进度。
 *
 * # 为什么要"一条"而不是三个分散的入口
 *
 * 远端同步是一个**有顺序的活动**："先看落后几个提交 → 拉取 → 推送"。把三件事
 * 放在同一条上、并让 ahead/behind 与进度出现在同一处，用户不用在页面之间跳。
 *
 * # 进度与结果从哪来
 *
 * 三个操作都是长任务：`git_*` 立即返回 `jobId`，进度与结果经 `job:*` 事件
 * （编排见 `useSyncJobs`）。进度条就在这条上（也能在状态栏看到任务数），
 * 可展开的详细日志用于"进度卡住"时看 git 的原始输出。
 *
 * # 冲突与被拒是两个对话框，不是两条 toast
 *
 * - 拉取冲突：git 的退出码非零，但那是**要用户解决的现场**——弹对话框给出冲突
 *   文件清单并引导到冲突页（M3 的向导入口）；
 * - 推送被拒：`PUSH_REJECTED` 自带三条修复路径（先拉取 / force-with-lease / 取消），
 *   三条都必须真的可执行，所以由本组件按 `action.id` 接线（toast 只放得下一条）。
 */
import { useState } from 'react';

import { useQuery } from '@tanstack/react-query';
import { ArrowDownToLine, ArrowUpFromLine, Check, ChevronDown, RefreshCw } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useNavigate, useParams } from 'react-router-dom';

import { errorHintKey, errorTitleKey } from '@/lib/errors';
import type { NormalizedError } from '@/lib/errors';
import { settingsGet, settingsSet } from '@/lib/ipc';
import type { PullStrategy, PushSpec } from '@/lib/ipc/sync';
import {
  PULL_STRATEGIES,
  PULL_STRATEGY_SETTING_KEY,
  parsePullStrategy,
  serializePullStrategy,
} from '@/features/sync/pullStrategy';
import { useSyncStatus } from '@/features/sync/syncStatus';
import type { SyncStatus } from '@/features/sync/syncStatus';
import { useSyncJobs } from '@/features/sync/useSyncJobs';
import type { LeaseStage, SyncProgress } from '@/features/sync/useSyncJobs';

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
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from '@/ui/components/dropdown-menu';
import { Progress } from '@/ui/components/progress';

export function SyncBar() {
  const { t } = useTranslation('shell');
  const params = useParams();
  const repoId = Number(params.repoId);
  const jobs = useSyncJobs(repoId);
  const statusQuery = useSyncStatus(repoId);
  // 用户这次选的策略（未选过时为 null，用设置里的值）
  const [chosen, setChosen] = useState<PullStrategy | null>(null);
  const [detailOpen, setDetailOpen] = useState(false);

  // 与历史筛选同一机制：仓库级设置，值是一个 JSON 字符串
  const settingQuery = useQuery({
    queryKey: ['settings', 'repo', repoId, PULL_STRATEGY_SETTING_KEY],
    queryFn: () => settingsGet('repo', PULL_STRATEGY_SETTING_KEY, repoId),
    enabled: Number.isFinite(repoId),
    staleTime: Number.POSITIVE_INFINITY,
  });

  if (!Number.isFinite(repoId)) {
    return null;
  }

  const status = statusQuery.data;
  const strategy = chosen ?? parsePullStrategy(settingQuery.data ?? null);
  const busy = jobs.busy;

  const chooseStrategy = (next: PullStrategy): void => {
    setChosen(next);
    // 写失败不阻塞界面：策略只是"下次的默认值"
    settingsSet('repo', PULL_STRATEGY_SETTING_KEY, serializePullStrategy(next), repoId).catch(
      () => undefined,
    );
  };

  // 还没有上游时用 `--set-upstream`（推到同名分支并建立追踪）；
  // 已有上游时什么都不用指定——git 会推到上游，这也正是用户期待的语义。
  const pushSpec: PushSpec = status?.upstream == null ? { setUpstream: true } : {};

  return (
    <div
      className="flex w-full flex-col gap-1.5"
      aria-label={t('sync.ariaLabel')}
      data-testid="sync-bar"
    >
      <div className="flex flex-wrap items-center gap-2">
        <Button
          size="sm"
          variant="secondary"
          disabled={busy}
          title={t('sync.fetchHint')}
          onClick={() => {
            jobs.runFetch({});
          }}
          data-testid="sync-fetch"
        >
          <RefreshCw aria-hidden="true" className="size-3.5" />
          {t('sync.fetch')}
        </Button>

        <div className="flex items-center">
          <Button
            size="sm"
            variant="secondary"
            disabled={busy}
            title={t('sync.pullHint')}
            className="rounded-r-none"
            onClick={() => {
              jobs.runPull(strategy);
            }}
            data-testid="sync-pull"
          >
            <ArrowDownToLine aria-hidden="true" className="size-3.5" />
            {t('sync.pull')}
          </Button>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button
                size="sm"
                variant="secondary"
                disabled={busy}
                aria-label={t('sync.strategy.label')}
                className="rounded-l-none border-l-0 px-1.5"
                data-testid="sync-strategy"
              >
                <ChevronDown aria-hidden="true" className="size-3.5" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              <DropdownMenuLabel>{t('sync.strategy.label')}</DropdownMenuLabel>
              {PULL_STRATEGIES.map((option) => (
                <DropdownMenuItem
                  key={option}
                  onSelect={() => {
                    chooseStrategy(option);
                  }}
                  data-testid={`sync-strategy-${option}`}
                >
                  {strategy === option ? (
                    <Check aria-hidden="true" className="size-3.5 text-brand" />
                  ) : (
                    <span aria-hidden="true" className="size-3.5" />
                  )}
                  <span className="text-12">{t(`sync.strategy.${option}`)}</span>
                </DropdownMenuItem>
              ))}
            </DropdownMenuContent>
          </DropdownMenu>
        </div>

        <Button
          size="sm"
          variant="secondary"
          disabled={busy}
          title={t('sync.pushHint')}
          onClick={() => {
            jobs.runPush(pushSpec);
          }}
          data-testid="sync-push"
        >
          <ArrowUpFromLine aria-hidden="true" className="size-3.5" />
          {t('sync.push')}
        </Button>

        <SyncBadges status={status} />
      </div>

      {jobs.progress !== null ? (
        <SyncProgressStrip
          progress={jobs.progress}
          open={detailOpen}
          onToggle={() => {
            setDetailOpen((current) => !current);
          }}
          onCancel={jobs.cancel}
        />
      ) : null}

      <ConflictDialog
        files={jobs.conflict?.files ?? null}
        repoId={repoId}
        onDismiss={jobs.dismissConflict}
      />

      <RejectionDialog
        rejection={jobs.rejection}
        onFetchFirst={jobs.fetchThenRetry}
        onForceWithLease={jobs.prepareForceWithLease}
        onDismiss={jobs.dismissRejection}
      />

      {/* 被拒后的第二条路的第二步：先拉取（已完成）→ 用户看着新鲜对比确认覆盖 */}
      <LeaseConfirmDialog
        stage={jobs.leaseStage}
        status={status}
        onConfirm={jobs.confirmForceWithLease}
        onCancel={jobs.cancelForceWithLease}
      />
    </div>
  );
}

/** 上游与 ahead/behind。加载中什么都不显示——半个事实比没有更糟。 */
function SyncBadges({ status }: { readonly status: SyncStatus | undefined }) {
  const { t } = useTranslation('shell');

  if (status === undefined) {
    return null;
  }
  if (status.upstream === null || status.remote === null) {
    return (
      <span className="text-12 text-fg-subtle" data-testid="sync-no-upstream">
        {t('sync.noUpstream')}
      </span>
    );
  }

  const synchronized = status.ahead === 0 && status.behind === 0;
  return (
    <span className="flex items-center gap-1.5 text-12" data-testid="sync-status">
      <span className="truncate font-mono text-fg-muted" title={t('sync.upstreamTip')}>
        {status.upstream}
      </span>
      {synchronized ? (
        <span className="text-fg-subtle">{t('sync.upToDate')}</span>
      ) : (
        <>
          <span
            className="rounded bg-surface-sunken px-1 font-mono"
            title={t('sync.aheadTip')}
            data-testid="sync-ahead"
          >
            {`↑${String(status.ahead)}`}
          </span>
          <span
            className="rounded bg-surface-sunken px-1 font-mono"
            title={t('sync.behindTip')}
            data-testid="sync-behind"
          >
            {`↓${String(status.behind)}`}
          </span>
        </>
      )}
    </span>
  );
}

/** 进度条 + 取消 + 可展开的详细日志。 */
function SyncProgressStrip({
  progress,
  open,
  onToggle,
  onCancel,
}: {
  readonly progress: SyncProgress;
  readonly open: boolean;
  readonly onToggle: () => void;
  readonly onCancel: () => void;
}) {
  const { t } = useTranslation('shell');
  // 后端可能新增阶段：认不出时退回原始短名（那是数据，不是文案）
  const label = t(`sync.phase.${progress.phase}`, { defaultValue: progress.phase });
  const hasCounts = progress.current !== null && progress.total !== null;

  return (
    <div
      className="flex flex-col gap-1 rounded-md border border-line bg-surface px-2 py-1.5"
      data-testid="sync-progress"
    >
      <div className="flex items-center gap-2">
        <div className="min-w-0 flex-1">
          <Progress
            value={progress.percent}
            label={label}
            {...(hasCounts
              ? { hint: t('sync.steps', { current: progress.current, total: progress.total }) }
              : {})}
          />
        </div>
        <Button
          size="sm"
          variant="ghost"
          onClick={onCancel}
          data-testid="sync-cancel"
          aria-label={t('sync.cancel')}
        >
          {t('sync.cancel')}
        </Button>
        <Button
          size="sm"
          variant="ghost"
          onClick={onToggle}
          aria-expanded={open}
          data-testid="sync-detail-toggle"
        >
          {open ? t('sync.hideDetail') : t('sync.showDetail')}
        </Button>
      </div>

      {open ? (
        <ol
          className="max-h-32 list-none overflow-y-auto font-mono text-11 leading-relaxed text-fg-subtle"
          data-testid="sync-detail"
        >
          {progress.messages.map((line, index) => (
            <li key={`${String(index)}:${line}`}>{line}</li>
          ))}
        </ol>
      ) : null}
    </div>
  );
}

/** 冲突引导：列出冲突文件，并把用户送到冲突页（M3 的向导入口）。 */
function ConflictDialog({
  files,
  repoId,
  onDismiss,
}: {
  readonly files: readonly string[] | null;
  readonly repoId: number;
  readonly onDismiss: () => void;
}) {
  const { t } = useTranslation('shell');
  const navigate = useNavigate();

  return (
    <AlertDialog
      open={files !== null}
      onOpenChange={(next) => {
        if (!next) {
          onDismiss();
        }
      }}
    >
      <AlertDialogContent
        impact={t('sync.conflict.impact')}
        impactLabel={t('sync.conflict.impactLabel')}
        tone="warning"
        data-testid="sync-conflict-dialog"
      >
        <AlertDialogHeader>
          <AlertDialogTitle>{t('sync.conflict.title')}</AlertDialogTitle>
          <AlertDialogDescription>
            {t('sync.conflict.files', { count: files?.length ?? 0 })}
          </AlertDialogDescription>
        </AlertDialogHeader>

        <ul
          className="mt-3 max-h-40 overflow-y-auto font-mono text-12 text-fg"
          data-testid="sync-conflict-files"
        >
          {(files ?? []).map((file) => (
            <li key={file}>{file}</li>
          ))}
        </ul>

        <AlertDialogFooter>
          <AlertDialogCancel onClick={onDismiss}>{t('sync.conflict.later')}</AlertDialogCancel>
          <AlertDialogAction
            destructive={false}
            onClick={() => {
              onDismiss();
              navigate(`/repo/${String(repoId)}/conflict`);
            }}
            data-testid="sync-conflict-guide"
          >
            {t('sync.conflict.guide')}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

/**
 * 推送被拒：三条修复路径。
 *
 * 按钮文案取后端 `FixAction.labelKey`（`errors:actions.*`）——这样"给用户哪些
 * 选项"这件事只有后端一个真相源；取不到才回退到本地的同义 key。
 */
function RejectionDialog({
  rejection,
  onFetchFirst,
  onForceWithLease,
  onDismiss,
}: {
  readonly rejection: NormalizedError | null;
  readonly onFetchFirst: () => void;
  readonly onForceWithLease: () => void;
  readonly onDismiss: () => void;
}) {
  const { t } = useTranslation('shell');
  const { t: tErrors } = useTranslation('errors');

  // 没有错误就没有对话框内容：早退比在 JSX 里到处写 `?.` 更清楚
  if (rejection === null) {
    return null;
  }

  const label = (id: string, fallbackKey: string): string => {
    const action = rejection.actions.find((candidate) => candidate.id === id);
    return tErrors(action?.labelKey ?? fallbackKey);
  };

  return (
    <AlertDialog
      open
      onOpenChange={(next) => {
        if (!next) {
          onDismiss();
        }
      }}
    >
      <AlertDialogContent
        impact={t('sync.rejected.impact')}
        impactLabel={t('sync.rejected.impactLabel')}
        data-testid="sync-rejected-dialog"
      >
        <AlertDialogHeader>
          <AlertDialogTitle>{tErrors(errorTitleKey(rejection.code))}</AlertDialogTitle>
          <AlertDialogDescription>
            {rejection.hint ?? tErrors(errorHintKey(rejection.code))}
          </AlertDialogDescription>
        </AlertDialogHeader>

        {rejection.detail !== undefined && rejection.detail !== '' ? (
          <p className="mt-3 font-mono text-12 text-fg-subtle" data-testid="sync-rejected-reason">
            {t('sync.rejected.reason', { reason: rejection.detail })}
          </p>
        ) : null}

        <AlertDialogFooter>
          <AlertDialogCancel onClick={onDismiss}>
            {label('cancel', 'actions.cancel')}
          </AlertDialogCancel>
          <AlertDialogAction
            destructive={false}
            onClick={onFetchFirst}
            data-testid="sync-rejected-fetch-first"
          >
            {label('fetch-first', 'actions.pushFetchFirst')}
          </AlertDialogAction>
          <AlertDialogAction onClick={onForceWithLease} data-testid="sync-rejected-force">
            {label('force-with-lease', 'actions.pushForceWithLease')}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

/**
 * `--force-with-lease` 的第二步确认（红线 R7）。
 *
 * 到了这里，"远端当前状态"才是**刚拉取回来的**：下面给出的领先/落后就是
 * 用户即将用本地历史覆盖掉的东西。少了这一步，`--force-with-lease`
 * 就从"远端变了就拒绝"退化成"无条件覆盖"——那正是 R7 要消灭的东西。
 */
function LeaseConfirmDialog({
  stage,
  status,
  onConfirm,
  onCancel,
}: {
  readonly stage: LeaseStage | null;
  readonly status: SyncStatus | undefined;
  readonly onConfirm: () => void;
  readonly onCancel: () => void;
}) {
  const { t } = useTranslation('shell');

  if (stage !== 'ready') {
    return null;
  }

  return (
    <AlertDialog
      open
      onOpenChange={(next) => {
        if (!next) {
          onCancel();
        }
      }}
    >
      <AlertDialogContent
        impact={t('sync.lease.impact', {
          ahead: status?.ahead ?? 0,
          behind: status?.behind ?? 0,
        })}
        impactLabel={t('sync.lease.impactLabel')}
        data-testid="sync-lease-dialog"
      >
        <AlertDialogHeader>
          <AlertDialogTitle>{t('sync.lease.title')}</AlertDialogTitle>
          <AlertDialogDescription>
            {t('sync.lease.description', { upstream: status?.upstream ?? '' })}
          </AlertDialogDescription>
        </AlertDialogHeader>

        <AlertDialogFooter>
          <AlertDialogCancel onClick={onCancel}>{t('sync.lease.cancel')}</AlertDialogCancel>
          <AlertDialogAction onClick={onConfirm} data-testid="sync-lease-confirm">
            {t('sync.lease.confirm')}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
