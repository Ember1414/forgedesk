/**
 * 操作历史页（T3.10）。
 *
 * 这一页回答两个问题，缺一个它就没有存在价值：
 *
 * 1. **"刚才那一步做了什么？"**——时间线按天分组，每行是时间、操作（人话名称 +
 *    原始参数摘要）、结果（图标 + 文案）、有没有留下快照；
 * 2. **"还能不能回去？"**——每行在有快照**且锚点此刻仍在**时给出回滚入口。
 *    锚点会消失（外部 clone、gc、手工删 ref），所以"能不能回滚"由后端现算，
 *    不由界面按记录字段猜。
 *
 * # 为什么与设置页的审计面板并存
 *
 * 审计面板面向排查（跨仓库、导出、保留策略），这一页面向"我刚做了什么、
 * 还能不能撤"——同一份数据的两种读法，目标读者不同，所以两处都要有。
 *
 * # 回滚在这里的交互
 *
 * 与快照页同一条纪律（红线 R7）：**先取差异摘要 → 二次确认 → 执行 → 展示报告**。
 * 确认框里额外要求勾选一次：从操作历史滚回去比从快照页滚回去更容易"点错行"，
 * 而它丢掉的可能是几十分钟的未提交工作。
 */
import { useState } from 'react';

import { useQuery, useQueryClient } from '@tanstack/react-query';
import { AlertTriangle, CheckCircle2, Loader2, RotateCcw } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useParams } from 'react-router-dom';

import { OP_TYPES } from '@/features/settings/AuditHistoryPanel';
import { RestoreReportPanel } from '@/features/snapshots/RestoreReportPanel';
import { normalizeError, useAppError } from '@/lib/errors';
import { operationHistory } from '@/lib/ipc/audit';
import type { OperationHistoryEntry } from '@/lib/ipc/audit';
import { snapshotDiff, snapshotRestore } from '@/lib/ipc/snapshots';
import type { RestoreReport, SnapshotDiff } from '@/lib/ipc/snapshots';
import {
  OPERATION_HISTORY_QUERY_KEY,
  SNAPSHOT_RESTORE_PENDING_QUERY_KEY,
  SNAPSHOTS_QUERY_KEY,
  STATUS_QUERY_KEY,
} from '@/lib/queryKeys';
import { useRepoChangeInvalidation } from '@/lib/repoChanged';
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
import { Input } from '@/ui/components/input';
import { Skeleton } from '@/ui/components/skeleton';

/** 每页条数（与审计面板一致：一屏能看完的量）。 */
const PAGE_SIZE = 50;

/** 操作类型短名 → i18n key 后缀（`snapshot_restore` → `snapshotRestore`）。 */
function opLabelKey(opType: string): string {
  const camel = opType.replace(/_([a-z])/g, (_, letter: string) => letter.toUpperCase());
  return (OP_TYPES as readonly string[]).includes(opType)
    ? `settings.audit.op.${camel}`
    : // 认不出的类型显示原始短名：排查时原文比"未知操作"有用得多
      opType;
}

/** 结果短名 → 展示元素（图标 + 文案，不靠颜色单独表意）。 */
function ResultBadge({ result }: { readonly result: string }) {
  const { t } = useTranslation('shell');
  if (result === 'running') {
    return (
      <span className="flex items-center gap-1 text-warning">
        <Loader2 aria-hidden="true" className="size-3 animate-spin" />
        {t('settings.audit.result.running')}
      </span>
    );
  }
  if (result === 'failed') {
    return (
      <span className="flex items-center gap-1 text-danger">
        <AlertTriangle aria-hidden="true" className="size-3" />
        {t('settings.audit.result.failed')}
      </span>
    );
  }
  return (
    <span className="flex items-center gap-1 text-success">
      <CheckCircle2 aria-hidden="true" className="size-3" />
      {t('settings.audit.result.ok')}
    </span>
  );
}

/** 毫秒 → 本地 `HH:mm:ss`（同一天内只需要时刻）。 */
function formatTime(ms: number | null): string {
  if (ms === null) {
    return '—';
  }
  const date = new Date(ms);
  const pad = (value: number): string => String(value).padStart(2, '0');
  return `${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`;
}

/** 毫秒 → 本地日期分组标题（`2026-09-30`）。 */
function formatDay(ms: number | null): string {
  if (ms === null) {
    return '—';
  }
  const date = new Date(ms);
  const pad = (value: number): string => String(value).padStart(2, '0');
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

/** 按天分组（保持"新的在前"；同一天的记录归到同一组）。 */
function groupByDay(
  entries: readonly OperationHistoryEntry[],
): readonly { readonly day: string; readonly entries: readonly OperationHistoryEntry[] }[] {
  const groups: { day: string; entries: OperationHistoryEntry[] }[] = [];
  for (const entry of entries) {
    const day = formatDay(entry.startedAtMs);
    const last = groups[groups.length - 1];
    if (last !== undefined && last.day === day) {
      last.entries.push(entry);
    } else {
      groups.push({ day, entries: [entry] });
    }
  }
  return groups;
}

export function OperationsPage() {
  const { t } = useTranslation('shell');
  const params = useParams();
  const repoId = Number(params.repoId);
  const queryClient = useQueryClient();
  const { show } = useAppError();

  const [keyword, setKeyword] = useState('');
  const [opType, setOpType] = useState('');
  const [onlyDangerous, setOnlyDangerous] = useState(false);
  const [onlyReversible, setOnlyReversible] = useState(false);
  const [page, setPage] = useState(0);
  const [expanded, setExpanded] = useState<number | null>(null);
  const [diff, setDiff] = useState<SnapshotDiff | null>(null);
  const [target, setTarget] = useState<OperationHistoryEntry | null>(null);
  const [confirmed, setConfirmed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [report, setReport] = useState<RestoreReport | null>(null);

  const filter = {
    keyword,
    opType,
    onlyDangerous,
    onlyReversible,
  } as const;

  const historyQuery = useQuery({
    queryKey: [
      OPERATION_HISTORY_QUERY_KEY,
      repoId,
      keyword,
      opType,
      onlyDangerous,
      onlyReversible,
      page,
    ],
    queryFn: () => operationHistory(repoId, filter, PAGE_SIZE, page * PAGE_SIZE),
    enabled: Number.isFinite(repoId),
  });

  // 外部提交、终端里的操作同样会写审计：列表要跟着更新
  useRepoChangeInvalidation(repoId);

  const requestRollback = async (entry: OperationHistoryEntry): Promise<void> => {
    if (entry.snapshotId === null) {
      return;
    }
    try {
      // 先取差异摘要：确认框里必须有"将会发生什么"，否则用户是在对一个
      // 自己看不懂的东西点确认
      setDiff(await snapshotDiff(repoId, entry.snapshotId));
      setConfirmed(false);
      setTarget(entry);
    } catch (error) {
      show(normalizeError(error));
    }
  };

  const runRollback = async (): Promise<void> => {
    if (target?.snapshotId === null || target === null) {
      return;
    }
    setBusy(true);
    try {
      const restored = await snapshotRestore(repoId, target.snapshotId);
      setTarget(null);
      setDiff(null);
      setReport(restored);
      pushToast({
        tone: restored.outcome === 'completed' ? 'success' : 'warning',
        title:
          restored.outcome === 'completed'
            ? t('snapshots.restored', { oid: restored.headOid.slice(0, 7) })
            : t(`snapshots.outcome.${restored.outcome}`),
      });
      // 回滚本身也是一条操作记录：列表、快照列表与状态都得重取
      await queryClient.invalidateQueries({
        queryKey: [OPERATION_HISTORY_QUERY_KEY, repoId],
      });
      void queryClient.invalidateQueries({ queryKey: [SNAPSHOTS_QUERY_KEY, repoId] });
      void queryClient.invalidateQueries({
        queryKey: [SNAPSHOT_RESTORE_PENDING_QUERY_KEY, repoId],
      });
      void queryClient.invalidateQueries({ queryKey: [STATUS_QUERY_KEY, repoId] });
    } catch (error) {
      show(normalizeError(error));
    } finally {
      setBusy(false);
    }
  };

  const entries = historyQuery.data?.entries ?? [];
  const total = historyQuery.data?.total ?? 0;
  const groups = groupByDay(entries);
  const lastPage = Math.max(0, Math.ceil(total / PAGE_SIZE) - 1);

  return (
    <section className="flex h-full min-h-0 flex-col gap-3" data-testid="operations-page">
      <header className="flex flex-col gap-1">
        <h2 className="text-16 font-semibold tracking-tight">{t('operations.title')}</h2>
        <p className="text-12 text-fg-subtle">{t('operations.description')}</p>
      </header>

      {/* 筛选：搜索 + 类型 + 两个开关（"只看危险"与"只看可回滚"是最常用的两问） */}
      <div className="flex flex-wrap items-center gap-2" data-testid="operations-filters">
        <Input
          label={t('operations.filter.keyword')}
          value={keyword}
          onChange={(event) => {
            setKeyword(event.target.value);
            setPage(0);
          }}
          placeholder={t('operations.filter.keywordPlaceholder')}
          data-testid="operations-keyword"
        />
        <label className="flex items-center gap-1 text-12" htmlFor="operations-op-type">
          <span className="text-fg-subtle">{t('operations.filter.opType')}</span>
          <select
            id="operations-op-type"
            className="rounded-md border border-line bg-surface px-2 py-1 text-12"
            value={opType}
            onChange={(event) => {
              setOpType(event.target.value);
              setPage(0);
            }}
            data-testid="operations-op-type"
          >
            <option value="">{t('operations.filter.allTypes')}</option>
            {OP_TYPES.map((value) => (
              <option key={value} value={value}>
                {t(opLabelKey(value))}
              </option>
            ))}
          </select>
        </label>
        <label className="flex items-center gap-1 text-12">
          <input
            type="checkbox"
            checked={onlyDangerous}
            onChange={(event) => {
              setOnlyDangerous(event.target.checked);
              setPage(0);
            }}
            data-testid="operations-only-dangerous"
          />
          {t('operations.filter.onlyDangerous')}
        </label>
        <label className="flex items-center gap-1 text-12">
          <input
            type="checkbox"
            checked={onlyReversible}
            onChange={(event) => {
              setOnlyReversible(event.target.checked);
              setPage(0);
            }}
            data-testid="operations-only-reversible"
          />
          {t('operations.filter.onlyReversible')}
        </label>
      </div>

      {report === null ? null : (
        <RestoreReportPanel
          report={report}
          onDismiss={() => {
            setReport(null);
          }}
        />
      )}

      {historyQuery.isPending ? (
        <div className="flex flex-col gap-2" role="status">
          <Skeleton className="h-6 w-full" />
          <Skeleton className="h-6 w-5/6" />
          <span className="sr-only">{t('operations.loading')}</span>
        </div>
      ) : historyQuery.isError ? (
        <ErrorState
          title={t('operations.error')}
          retryLabel={t('snapshots.retry')}
          onRetry={() => {
            void historyQuery.refetch();
          }}
        />
      ) : entries.length === 0 ? (
        <EmptyState title={t('operations.empty')} description={t('operations.emptyHint')} />
      ) : (
        <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto">
          {groups.map((group) => (
            <section key={group.day} className="flex flex-col gap-1">
              <h3 className="text-11 text-fg-subtle" data-testid={`operations-day-${group.day}`}>
                {group.day}
              </h3>
              <ul className="flex flex-col rounded-lg border border-line">
                {group.entries.map((entry) => (
                  <li
                    key={entry.id}
                    className="border-b border-line/60 last:border-b-0"
                    data-testid={`operation-row-${entry.id}`}
                  >
                    <div className="flex flex-wrap items-center gap-2 px-3 py-1.5 text-13">
                      <span className="w-16 shrink-0 font-mono text-12 text-fg-muted">
                        {formatTime(entry.startedAtMs)}
                      </span>
                      <span className="min-w-0 flex-1 truncate" title={entry.argsJson ?? ''}>
                        {t(opLabelKey(entry.opType))}
                      </span>
                      <ResultBadge result={entry.result} />
                      <span className="shrink-0 text-12 text-fg-subtle">
                        {entry.snapshotId === null
                          ? t('operations.snapshot.none')
                          : entry.canRollback
                            ? t('operations.snapshot.restorable')
                            : // 记录里有快照、锚点却没了：如实说"这个点已经作废"
                              t('operations.snapshot.gone')}
                      </span>
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => {
                          setExpanded((current) => (current === entry.id ? null : entry.id));
                        }}
                        data-testid={`operation-expand-${entry.id}`}
                      >
                        {expanded === entry.id
                          ? t('operations.detail.collapse')
                          : t('operations.detail.expand')}
                      </Button>
                      {entry.canRollback && entry.snapshotId !== null ? (
                        <Button
                          variant="secondary"
                          size="sm"
                          onClick={() => {
                            void requestRollback(entry);
                          }}
                          data-testid={`operation-rollback-${entry.id}`}
                        >
                          <RotateCcw aria-hidden="true" className="size-3.5" />
                          {t('operations.rollback.action')}
                        </Button>
                      ) : null}
                    </div>
                    {expanded === entry.id ? (
                      <dl
                        className="flex flex-col gap-1 bg-surface-sunken px-3 py-2 text-12"
                        data-testid={`operation-detail-${entry.id}`}
                      >
                        <dt className="text-fg-subtle">{t('operations.detail.args')}</dt>
                        <dd className="break-all font-mono text-fg-muted">
                          {entry.argsJson ?? '—'}
                        </dd>
                        {entry.stderrSummary === null ? null : (
                          <>
                            <dt className="text-fg-subtle">{t('operations.detail.stderr')}</dt>
                            <dd className="break-all font-mono text-danger">
                              {entry.stderrSummary}
                            </dd>
                          </>
                        )}
                        <dt className="text-fg-subtle">{t('operations.detail.timing')}</dt>
                        <dd className="text-fg-muted">
                          {t('operations.detail.duration', { ms: entry.durationMs ?? 0 })}
                          {entry.exitCode === null
                            ? null
                            : ` · ${t('operations.detail.exitCode', { code: entry.exitCode })}`}
                        </dd>
                      </dl>
                    ) : null}
                  </li>
                ))}
              </ul>
            </section>
          ))}

          <div className="flex items-center justify-between gap-2 text-12 text-fg-subtle">
            <span data-testid="operations-total">{t('operations.total', { count: total })}</span>
            <div className="flex items-center gap-2">
              <Button
                variant="ghost"
                size="sm"
                disabled={page === 0}
                onClick={() => {
                  setPage((current) => Math.max(0, current - 1));
                }}
              >
                {t('operations.prev')}
              </Button>
              <Button
                variant="ghost"
                size="sm"
                disabled={page >= lastPage}
                onClick={() => {
                  setPage((current) => current + 1);
                }}
              >
                {t('operations.next')}
              </Button>
            </div>
          </div>
        </div>
      )}

      {/* 回滚确认：差异摘要 + 一次显式勾选（红线 R7 的闸门） */}
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
          tone={diff?.refMissing === true ? 'danger' : 'warning'}
          impact={
            diff === null ? (
              t('snapshots.impactUnknown')
            ) : diff.refMissing ? (
              t('snapshots.impactRefMissing')
            ) : (
              <>
                {t('operations.rollback.impactHead', {
                  oid: (diff.currentHeadOid ?? '').slice(0, 7),
                  snapshot: target?.snapshotId ?? 0,
                })}
                <br />
                {t('snapshots.impactUntracked')}
                {diff.untrackedRestorable.length === 0 ? null : (
                  <>
                    <br />
                    {t('snapshots.impactRestorable', { count: diff.untrackedRestorable.length })}
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
            <AlertDialogTitle>{t('operations.rollback.title')}</AlertDialogTitle>
            <AlertDialogDescription>{t('operations.rollback.description')}</AlertDialogDescription>
          </AlertDialogHeader>
          <label className="mt-3 flex items-start gap-2 text-12">
            <input
              type="checkbox"
              checked={confirmed}
              onChange={(event) => {
                setConfirmed(event.target.checked);
              }}
              data-testid="operations-rollback-confirm-check"
            />
            {t('operations.rollback.confirmHint')}
          </label>
          <AlertDialogFooter>
            <AlertDialogCancel>{t('snapshots.cancel')}</AlertDialogCancel>
            <AlertDialogAction
              disabled={!confirmed || busy || diff === null || diff.refMissing}
              onClick={() => {
                void runRollback();
              }}
              data-testid="operations-rollback-confirm"
            >
              {t('operations.rollback.confirm')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </section>
  );
}
