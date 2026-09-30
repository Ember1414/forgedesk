//! 快照与回滚页面（M1 / T1.9；T3.8 补内容备份的可见性）。
//!
//! # 交互纪律（红线 R7 在 UI 层的最后一环）
//!
//! 回滚是界面上破坏性最强的按钮：它把工作区、索引与 HEAD 一起搬回过去。
//! 因此点"回滚"后的顺序是固定的：**先取差异摘要 → 在确认框里展示 → 用户确认 →
//! 执行 → 展示结果与保护点**。跳过差异摘要直接弹确认框，
//! 等于让用户对一个他看不懂的东西说"是"。
//!
//! # T3.8 之后这个页面多了三件事
//!
//! 1. **占用与配额**：快照的内容备份落在应用缓存目录，用户必须能看到"用了多少、
//!    上限多少、有没有孤儿目录"，否则磁盘被吃满时无从判断是谁干的；
//! 2. **手动打点**：`snapshot_create` 返回的是完整结果（体积、跳过的文件、告警），
//!    所以"立即打快照"之后要把"这次打点包含什么、不包含什么"说清楚——
//!    超限被整体跳过时**绝不静默**；
//! 3. **回滚报告**：报告里现在有"未跟踪文件恢复了几个、有没有多余的没删、
//!    校验过没过"，用可折叠清单展示，而不是一句"回滚成功"。
//!
//! # 列表里要不要显示 label
//!
//! 不显示：自动快照的 label 恒等于 kind 的短名（两列一样的内容），显示 kind 的
//! i18n 文案就够了。手动快照的 label 也固定是 `manual`——它现在的价值是审计与
//! 排查时的场景标记，不是用户自定义的备注。

import { useState } from 'react';

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Camera, Trash2 } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useParams } from 'react-router-dom';

import { normalizeError, useAppError } from '@/lib/errors';
import { SNAPSHOT_USAGE_QUERY_KEY, SNAPSHOTS_QUERY_KEY, STATUS_QUERY_KEY } from '@/lib/queryKeys';
import { useRepoChangeInvalidation } from '@/lib/repoChanged';
import {
  snapshotCleanup,
  snapshotCreate,
  snapshotDiff,
  snapshotList,
  snapshotPrune,
  snapshotRestore,
  snapshotUsage,
} from '@/lib/ipc/snapshots';
import type {
  RestoreReport,
  SnapshotDiff,
  SnapshotMeta,
  SnapshotOutcome,
  SnapshotWarning,
} from '@/lib/ipc/snapshots';
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
import { EmptyState } from '@/ui/components/empty-state';
import { ErrorState } from '@/ui/components/error-state';
import { Skeleton } from '@/ui/components/skeleton';

/** kind 短名 → i18n key（只增不改的清单，见 SnapshotKind::key）。 */
const KIND_LABEL_KEYS: Readonly<Record<string, string>> = {
  manual: 'snapshots.kind.manual',
  'pre-commit': 'snapshots.kind.preCommit',
  'pre-restore': 'snapshots.kind.preRestore',
  'pre-sync': 'snapshots.kind.preSync',
  'pre-head-move': 'snapshots.kind.preHeadMove',
  'pre-worktree-change': 'snapshots.kind.preWorktreeChange',
};

/** 时间列的显示格式（本地时区；表格里不需要秒以下的精度）。 */
function formatTime(ms: number): string {
  return new Date(ms).toLocaleString();
}

/** 字节数的可读格式（配额与体积用；一位小数足够看出量级）。 */
function formatBytes(bytes: number): string {
  if (bytes < 1024) {
    return `${String(bytes)} B`;
  }
  const units = ['KB', 'MB', 'GB', 'TB'] as const;
  let value = bytes / 1024;
  let index = 0;
  while (value >= 1024 && index < units.length - 1) {
    value /= 1024;
    index += 1;
  }
  return `${value.toFixed(1)} ${units[index] ?? 'B'}`;
}

export function SnapshotsPage() {
  const { t } = useTranslation('shell');
  const params = useParams();
  const repoId = Number(params.repoId);
  const queryClient = useQueryClient();
  const { show } = useAppError();

  const [target, setTarget] = useState<SnapshotMeta | null>(null);
  const [diff, setDiff] = useState<SnapshotDiff | null>(null);
  /** 最近一次手动打点的结果（有告警时要在页面上说清楚，不能只弹一个 toast）。 */
  const [created, setCreated] = useState<SnapshotOutcome | null>(null);
  /** 最近一次回滚的报告（折叠展示）。 */
  const [report, setReport] = useState<RestoreReport | null>(null);

  const listQuery = useQuery({
    queryKey: [SNAPSHOTS_QUERY_KEY, repoId],
    queryFn: () => snapshotList(repoId, 50),
    enabled: Number.isFinite(repoId),
  });

  const usageQuery = useQuery({
    queryKey: [SNAPSHOT_USAGE_QUERY_KEY, repoId],
    queryFn: () => snapshotUsage(repoId),
    enabled: Number.isFinite(repoId),
  });

  // 外部提交（终端里的 git commit）同样会产生"提交前"快照：列表必须跟着更新，
  // 否则用户会以为自己刚打的点丢了。`refs` 类别已经包含快照键，无需额外参数。
  useRepoChangeInvalidation(repoId);

  const invalidate = () => {
    void queryClient.invalidateQueries({ queryKey: [SNAPSHOTS_QUERY_KEY, repoId] });
    void queryClient.invalidateQueries({ queryKey: [SNAPSHOT_USAGE_QUERY_KEY, repoId] });
    void queryClient.invalidateQueries({ queryKey: [STATUS_QUERY_KEY, repoId] });
  };

  const createMutation = useMutation({
    mutationFn: () => snapshotCreate(repoId),
    onSuccess: (outcome) => {
      setCreated(outcome);
      pushToast({
        // 有告警时用 warning：toast 只是通知，"不包含什么"在页面上的那条里
        tone: outcome.warnings.length > 0 ? 'warning' : 'success',
        title: t('snapshots.created'),
      });
      invalidate();
    },
    onError: (error) => {
      show(normalizeError(error));
    },
  });

  const cleanupMutation = useMutation({
    mutationFn: () => snapshotCleanup(repoId),
    onSuccess: (outcome) => {
      pushToast({
        tone: 'info',
        title: t('snapshots.cleaned', {
          count: outcome.reclaimed.length + outcome.orphansRemoved,
          size: formatBytes(outcome.freedBytes),
        }),
      });
      invalidate();
    },
    onError: (error) => {
      show(normalizeError(error));
    },
  });

  const restoreMutation = useMutation({
    mutationFn: (snapshotId: number) => snapshotRestore(repoId, snapshotId),
    onSuccess: (restored) => {
      setTarget(null);
      setDiff(null);
      setReport(restored);
      pushToast({
        tone: 'success',
        title: t('snapshots.restored', { oid: restored.headOid.slice(0, 7) }),
      });
      invalidate();
    },
    onError: (error) => {
      show(normalizeError(error));
    },
  });

  const pruneMutation = useMutation({
    mutationFn: () => snapshotPrune(repoId),
    onSuccess: (pruned) => {
      pushToast({ tone: 'info', title: t('snapshots.pruned', { count: pruned.length }) });
      invalidate();
    },
    onError: (error) => {
      show(normalizeError(error));
    },
  });

  /**
   * 点"回滚"：先取差异摘要再弹确认框。
   *
   * 刻意做成"用户动作 → 一次 await → 一次展示"，确认框里永远有刚取到的摘要，
   * 而不是缓存里可能过期的那份。
   */
  const requestRestore = async (meta: SnapshotMeta): Promise<void> => {
    try {
      setDiff(await snapshotDiff(repoId, meta.id));
      setTarget(meta);
    } catch (error) {
      show(normalizeError(error));
    }
  };

  /** 前几条路径 + "等 N 项"：报告里不该把上百个路径铺开，但也不能假装只有几条。 */
  const summarizePaths = (paths: readonly string[], limit = 5): string => {
    const head = paths.slice(0, limit).join(', ');
    return paths.length > limit
      ? head + t('snapshots.report.more', { count: paths.length - limit })
      : head;
  };

  /** 一条告警的人话说明（类型由后端给，文案在这里）。 */
  const warningText = (warning: SnapshotWarning): string => {
    switch (warning.kind) {
      case 'untrackedBackupSkipped':
        return t('snapshots.warning.untrackedBackupSkipped', {
          count: warning.count ?? 0,
          size: formatBytes(warning.bytes ?? 0),
          limit: formatBytes(warning.limit ?? 0),
        });
      case 'untrackedBackupPartial':
        return t('snapshots.warning.untrackedBackupPartial', {
          count: warning.paths.length,
          detail: warning.detail ?? '',
        });
      case 'backupDirUnavailable':
        return t('snapshots.warning.backupDirUnavailable', { detail: warning.detail ?? '' });
      case 'spaceReclaimed':
        return t('snapshots.warning.spaceReclaimed', {
          count: warning.removed.length,
          size: formatBytes(warning.freedBytes ?? 0),
        });
      case 'orphansRemoved':
        return t('snapshots.warning.orphansRemoved', { count: warning.count ?? 0 });
      default:
        return warning.kind;
    }
  };

  // 后端不该返回 `null`，但"缺数据"必须被防御：旧版本、mock 或异常路径都可能给 null，
  // 而 `null` 会让下面每一处 `usage.xxx` 直接把页面炸掉（T3.8 的 E2E 抓到过）
  const usage = usageQuery.data ?? null;

  return (
    <section className="flex h-full flex-col gap-4" data-testid="snapshots-page">
      <header className="flex flex-wrap items-end justify-between gap-3">
        <div className="flex flex-col gap-1">
          <h2 className="text-16 font-semibold tracking-tight">{t('snapshots.title')}</h2>
          <p className="text-12 text-fg-subtle">{t('snapshots.description')}</p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          {/* 手动打点：用户主动说"就现在这样，记住它" */}
          <Button
            variant="primary"
            size="sm"
            loading={createMutation.isPending}
            onClick={() => {
              createMutation.mutate();
            }}
            data-testid="snapshot-create"
          >
            <Camera aria-hidden="true" className="size-3.5" />
            {t('snapshots.create')}
          </Button>
          <Button
            variant="secondary"
            size="sm"
            loading={cleanupMutation.isPending}
            onClick={() => {
              cleanupMutation.mutate();
            }}
            data-testid="snapshot-cleanup"
          >
            <Trash2 aria-hidden="true" className="size-3.5" />
            {t('snapshots.cleanup')}
          </Button>
          <Button
            variant="ghost"
            size="sm"
            loading={pruneMutation.isPending}
            onClick={() => {
              pruneMutation.mutate();
            }}
          >
            {t('snapshots.prune')}
          </Button>
        </div>
      </header>

      {/* 占用与配额：内容备份用掉多少、上限多少、有没有孤儿目录 */}
      {usage === null ? null : (
        <div
          className="flex flex-wrap items-center gap-x-3 gap-y-1 text-12 text-fg-muted"
          data-testid="snapshot-usage"
        >
          <span>{t('snapshots.usage.snapshots', { count: usage.snapshotCount })}</span>
          <span>{t('snapshots.usage.bytes', { size: formatBytes(usage.backupBytes) })}</span>
          <span>
            {usage.maxRepoBytes === 0
              ? t('snapshots.usage.unlimited')
              : t('snapshots.usage.limit', { size: formatBytes(usage.maxRepoBytes) })}
          </span>
          {usage.orphanDirs.length > 0 ? (
            <span className="text-warning" data-testid="snapshot-orphans">
              {t('snapshots.usage.orphans', { count: usage.orphanDirs.length })}
            </span>
          ) : null}
        </div>
      )}

      {/* 手动打点的告警：一眼能看出来"这次没备上什么" */}
      {created === null || created.warnings.length === 0 ? null : (
        <div
          className="flex flex-col gap-1 rounded-md border border-warning bg-warning/10 p-3 text-12"
          data-testid="snapshot-warning"
        >
          <div className="flex items-center justify-between gap-2 font-medium">
            <span>{t('snapshots.createdWithWarning')}</span>
            <button
              type="button"
              className="fd-transition rounded-sm px-1 text-fg-subtle hover:text-fg"
              onClick={() => {
                setCreated(null);
              }}
            >
              {t('snapshots.dismiss')}
            </button>
          </div>
          <ul className="flex list-disc flex-col gap-1 pl-4 text-fg-muted">
            {created.warnings.map((warning, index) => (
              <li key={`${warning.kind}-${String(index)}`}>{warningText(warning)}</li>
            ))}
          </ul>
        </div>
      )}

      {/* 回滚报告：可折叠清单（用户能看到"哪一步成了、哪一步没成"） */}
      {report === null ? null : (
        <details
          className="rounded-md border border-line p-3 text-12"
          data-testid="snapshot-report"
          open
        >
          <summary className="cursor-pointer font-medium">{t('snapshots.report.title')}</summary>
          <ul className="mt-2 flex flex-col gap-1 text-fg-muted">
            <li>{t('snapshots.report.head', { oid: report.headOid.slice(0, 7) })}</li>
            <li>{t('snapshots.report.index')}</li>
            <li>{t('snapshots.report.untracked', { count: report.untrackedRestored })}</li>
            <li>
              {report.verified ? t('snapshots.report.verified') : t('snapshots.report.unverified')}
            </li>
            {report.untrackedFailed.length === 0 ? null : (
              <li className="text-danger" data-testid="snapshot-report-failed">
                {t('snapshots.report.failed', {
                  count: report.untrackedFailed.length,
                  paths: summarizePaths(report.untrackedFailed),
                })}
              </li>
            )}
            {report.untrackedExtra.length === 0 ? null : (
              <li data-testid="snapshot-report-extra">
                {t('snapshots.report.extra', {
                  count: report.untrackedExtra.length,
                  paths: summarizePaths(report.untrackedExtra),
                })}
              </li>
            )}
          </ul>
          <button
            type="button"
            className="fd-transition mt-2 rounded-sm text-fg-subtle hover:text-fg"
            onClick={() => {
              setReport(null);
            }}
          >
            {t('snapshots.dismiss')}
          </button>
        </details>
      )}

      {listQuery.isPending ? (
        <div className="flex flex-col gap-2" role="status">
          <Skeleton className="h-6 w-full" />
          <Skeleton className="h-6 w-5/6" />
          <span className="sr-only">{t('snapshots.loading')}</span>
        </div>
      ) : listQuery.isError ? (
        <ErrorState
          title={t('snapshots.error')}
          retryLabel={t('snapshots.retry')}
          onRetry={() => {
            void listQuery.refetch();
          }}
        />
      ) : listQuery.data.length === 0 ? (
        <EmptyState title={t('snapshots.empty')} description={t('snapshots.emptyHint')} />
      ) : (
        <div className="overflow-hidden rounded-lg border border-line">
          <table className="w-full text-13">
            <thead>
              <tr className="border-b border-line bg-surface-sunken text-left text-11 text-fg-subtle">
                <th className="px-3 py-1.5 font-medium">{t('snapshots.column.kind')}</th>
                <th className="px-3 py-1.5 font-medium">{t('snapshots.column.time')}</th>
                <th className="px-3 py-1.5 font-medium">{t('snapshots.column.head')}</th>
                <th className="px-3 py-1.5 font-medium">{t('snapshots.column.branch')}</th>
                <th className="px-3 py-1.5 font-medium">{t('snapshots.column.actions')}</th>
              </tr>
            </thead>
            <tbody>
              {listQuery.data.map((meta) => (
                <tr key={meta.id} className="border-b border-line/60 last:border-b-0">
                  <td className="px-3 py-1.5">
                    {t(KIND_LABEL_KEYS[meta.kind] ?? 'snapshots.kind.other')}
                  </td>
                  <td className="px-3 py-1.5 text-fg-muted">{formatTime(meta.createdAtMs)}</td>
                  <td className="px-3 py-1.5 font-mono text-fg-muted">
                    {meta.headOid.slice(0, 7)}
                  </td>
                  <td className="px-3 py-1.5 text-fg-muted">
                    {meta.detached ? t('snapshots.detached') : (meta.branch ?? '')}
                  </td>
                  <td className="px-3 py-1.5">
                    <Button
                      variant="secondary"
                      size="sm"
                      onClick={() => {
                        void requestRestore(meta);
                      }}
                    >
                      {t('snapshots.restore')}
                    </Button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {/* 确认框：差异摘要是必答题（红线 R7）。锚点丢失时直接拒绝并说明 */}
      <AlertDialog
        open={target !== null}
        onOpenChange={(open) => {
          if (!open) {
            setTarget(null);
            setDiff(null);
          }
        }}
      >
        <AlertDialogContent
          impactLabel={t('snapshots.impactLabel')}
          impact={
            diff === null ? (
              t('snapshots.impactUnknown')
            ) : diff.refMissing ? (
              t('snapshots.impactRefMissing')
            ) : !diff.headChanged && !diff.indexChanged ? (
              t('snapshots.impactUnchanged')
            ) : (
              <>
                {t('snapshots.impactHead', {
                  oid: (target?.headOid ?? '').slice(0, 7),
                  current: (diff.currentHeadOid ?? '').slice(0, 7),
                })}
                {diff.indexChanged ? <> {t('snapshots.impactIndex')}</> : null}
                <br />
                {t('snapshots.impactUntracked')}
                {/* 未跟踪内容的三分类（T3.8）：会恢复什么、找不回什么、不会删什么 */}
                {diff.untrackedRestorable.length === 0 ? null : (
                  <>
                    <br />
                    {t('snapshots.impactRestorable', { count: diff.untrackedRestorable.length })}
                  </>
                )}
                {diff.untrackedMissing.length === 0 ? null : (
                  <>
                    <br />
                    <span className="text-warning">
                      {t('snapshots.impactMissing', { count: diff.untrackedMissing.length })}
                    </span>
                  </>
                )}
                {diff.untrackedExtra.length === 0 ? null : (
                  <>
                    <br />
                    {t('snapshots.impactExtra', { count: diff.untrackedExtra.length })}
                  </>
                )}
              </>
            )
          }
        >
          <AlertDialogHeader>
            <AlertDialogTitle>{t('snapshots.restoreConfirmTitle')}</AlertDialogTitle>
            <AlertDialogDescription>
              {target === null
                ? ''
                : t('snapshots.restoreConfirmDescription', {
                    oid: target.headOid.slice(0, 7),
                  })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t('snapshots.cancel')}</AlertDialogCancel>
            {/* 锚点丢失或无需回滚时禁用确认：可预见的无效操作不受理 */}
            <AlertDialogAction
              disabled={
                restoreMutation.isPending ||
                diff === null ||
                diff.refMissing ||
                (!diff.headChanged && !diff.indexChanged)
              }
              onClick={() => {
                if (target !== null) {
                  restoreMutation.mutate(target.id);
                }
              }}
            >
              {t('snapshots.restoreConfirm')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </section>
  );
}
