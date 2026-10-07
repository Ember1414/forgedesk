import { useState } from 'react';

import { useQuery } from '@tanstack/react-query';
import type { TFunction } from 'i18next';
import { ClipboardCopy, Download, Trash2 } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { normalizeError, useAppError } from '@/lib/errors';
import { auditExport, auditList, auditPrune, pickSavePath, repoRecentList } from '@/lib/ipc';
import type { AuditEntry, AuditExportFormat, AuditPage } from '@/lib/ipc';
import {
  AUDIT_RETENTION_DAYS_KEY,
  AUDIT_RETENTION_MAX_KEY,
  AUDIT_RETENTION_DEFAULT_DAYS,
  AUDIT_RETENTION_DEFAULT_ROWS,
  useSettingsStore,
} from '@/stores/settingsStore';
import { pushToast } from '@/stores/toastStore';
import { Button } from '@/ui/components/button';
import { ErrorState } from '@/ui/components/error-state';
import { Input } from '@/ui/components/input';
import { SelectField } from '@/ui/components/select';
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
  TableEmptyRow,
  TableSkeletonRows,
} from '@/ui/components/table';

/** 一页多少条（与后端 `OperationQuery::DEFAULT_LIMIT` 同一个量级）。 */
const PAGE_SIZE = 50;

/** 界面认识的操作类型（稳定短名，与后端 `services::audit::op_type` 一一对应）。 */
/**
 * 操作类型短名清单（T3.10 起导出给操作历史页共用）。
 *
 * 导出而不是各写一份：两个页面展示同一份数据，多一份清单就会在某次新增操作类型时
 * 漏掉一处——而漏掉的那处会把英文短名直接显示给用户。
 */
export const OP_TYPES = [
  'commit',
  'stage',
  'unstage',
  'discard',
  'clone',
  'init',
  'forget',
  'close',
  'stash_save',
  'stash_apply',
  'stash_drop',
  'stash_branch',
  'reset',
  'cherry_pick',
  'revert',
  'reflog_branch',
  'conflict_resolve',
  'conflict_continue',
  'conflict_abort',
  'conflict_skip',
  'snapshot_restore',
  'snapshot_prune',
  'snapshot_create',
  'snapshot_cleanup',
  'snapshot_restore_abandon',
  'audit_export',
  'audit_prune',
] as const;

/**
 * 建议的导出文件名（保存对话框里的默认名）。
 *
 * 用**本地时间**而不是 UTC：用户是拿它跟"我刚才导出的是哪一次"对照的，
 * 本地时间才是他/她看到的时间。扩展名直接取格式名，因此不可能出现
 * "内容按 CSV 生成、扩展名却是 .json"这种后端会拒绝的组合。
 */
function suggestedExportName(format: AuditExportFormat): string {
  const now = new Date();
  const pad = (value: number): string => String(value).padStart(2, '0');
  const stamp =
    `${now.getFullYear()}${pad(now.getMonth() + 1)}${pad(now.getDate())}` +
    `-${pad(now.getHours())}${pad(now.getMinutes())}${pad(now.getSeconds())}`;
  return `forgedesk-audit-${stamp}.${format}`;
}

/**
 * 操作类型短名 → i18n key 后缀。
 *
 * 认不出的类型（例如更新版本写下的记录）在界面上显示原始短名：
 * 显示一个空白的"未知操作"比显示 `future_op` 更没用——排查时原文才有价值。
 */
function opLabelKey(opType: string): string | null {
  const camel = opType.replace(/_([a-z])/g, (_, letter: string) => letter.toUpperCase());
  return (OP_TYPES as readonly string[]).includes(opType) ? `settings.audit.op.${camel}` : null;
}

/** 结果短名 → i18n key。 */
function resultKey(result: string): string {
  return `settings.audit.result.${result}`;
}

/** 毫秒 → 本地时间（`yyyy-MM-dd HH:mm:ss`）。 */
function formatTime(ms: number | null): string {
  if (ms === null) {
    return '—';
  }
  const date = new Date(ms);
  const pad = (value: number): string => String(value).padStart(2, '0');
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(
    date.getHours(),
  )}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`;
}

/** 毫秒 → 人类可读的耗时（秒以下按毫秒显示：写操作大多是毫秒级）。 */
function formatDuration(t: TFunction, ms: number | null): string {
  if (ms === null) {
    return '—';
  }
  if (ms < 1_000) {
    return t('settings.audit.durationMs', { ms });
  }
  return t('settings.audit.durationSeconds', { seconds: (ms / 1_000).toFixed(1) });
}

/**
 * 操作历史（T1.11）。
 *
 * 三条界面纪律：
 *
 * 1. **`running` 要能一眼看出来**：它意味着"上一次写操作没有收尾"，
 *    也就是应用崩在了写操作中间。混在成功记录里等于没有这个信号；
 * 2. **失败的行要能读到 git 的原话**：`stderrSummary` 已脱敏，展开即见，
 *    不必去翻日志；
 * 3. **导出的路径要显示出来**：本任务只写临时目录，用户得知道自己拿到了什么，
 *    以及"这还不是最终位置"。
 */
export function AuditHistoryPanel() {
  const { t } = useTranslation('shell');
  const { show } = useAppError();

  const [repoId, setRepoId] = useState<string>('all');
  const [opType, setOpType] = useState<string>('all');
  const [offset, setOffset] = useState(0);
  const [exportedPath, setExportedPath] = useState<string | null>(null);
  const [busy, setBusy] = useState<'export' | 'prune' | null>(null);

  // 逐项经 selector 订阅（selector 内调用 getJson）：值变才重渲染，
  // 否则改保留策略时输入框会停在旧值（订阅函数引用等于没有订阅）
  const setJson = useSettingsStore((state) => state.setJson);
  const retentionDays = useSettingsStore((state) =>
    state.getJson<number>(AUDIT_RETENTION_DAYS_KEY, AUDIT_RETENTION_DEFAULT_DAYS),
  );
  const retentionRows = useSettingsStore((state) =>
    state.getJson<number>(AUDIT_RETENTION_MAX_KEY, AUDIT_RETENTION_DEFAULT_ROWS),
  );

  const filter = {
    repoId: repoId === 'all' ? null : Number(repoId),
    opType: opType === 'all' ? null : opType,
  };

  const recentQuery = useQuery({
    queryKey: ['recent-repositories', 50],
    queryFn: () => repoRecentList(50),
  });

  const listQuery = useQuery<AuditPage>({
    queryKey: ['audit', filter.repoId, filter.opType, offset],
    queryFn: () => auditList(filter, PAGE_SIZE, offset),
  });

  const repoOptions = [
    { value: 'all', label: t('settings.audit.filters.allRepos') },
    ...(recentQuery.data ?? []).map((repo) => ({
      value: String(repo.id),
      label: repo.name,
    })),
  ];

  const opOptions = [
    { value: 'all', label: t('settings.audit.filters.allOps') },
    ...OP_TYPES.map((value) => ({
      value,
      label: opLabelKey(value) === null ? value : t(opLabelKey(value) as string),
    })),
  ];

  function resetPage(): void {
    setOffset(0);
  }

  async function runExport(format: AuditExportFormat): Promise<void> {
    setBusy('export');
    try {
      // 先让用户选位置（T7.6）。取消就整个中止——**不要**在取消后偷偷写到临时目录：
      // 用户会以为文件在自己选的地方，而真正的副本在别处，找起来比不给还糟。
      const target = await pickSavePath(
        t('settings.audit.export.chooseTitle'),
        suggestedExportName(format),
      );
      if (target === null) {
        return;
      }

      const result = await auditExport(filter, format, target);
      setExportedPath(result.path);
      pushToast({
        title: t('settings.audit.export.done', { rows: result.rows }),
        description: result.path,
        tone: 'success',
      });
    } catch (error) {
      show(normalizeError(error));
    } finally {
      setBusy(null);
    }
  }

  async function runPrune(): Promise<void> {
    setBusy('prune');
    try {
      const result = await auditPrune();
      pushToast({
        title: t('settings.audit.prune.done', { count: result.removed }),
        description: t('settings.audit.prune.policy', {
          days: result.retentionDays,
          rows: result.retentionRows,
        }),
        tone: 'success',
      });
      void listQuery.refetch();
    } catch (error) {
      show(normalizeError(error));
    } finally {
      setBusy(null);
    }
  }

  const total = listQuery.data?.total ?? 0;
  const entries: readonly AuditEntry[] = listQuery.data?.entries ?? [];
  const pageStart = total === 0 ? 0 : offset + 1;
  const pageEnd = offset + entries.length;

  return (
    <div className="flex min-h-0 flex-col gap-3 rounded-lg border border-line bg-surface p-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex max-w-xl flex-col gap-0.5">
          <span className="text-14 font-medium">{t('settings.audit.title')}</span>
          <span className="text-12 text-fg-subtle">{t('settings.audit.hint')}</span>
        </div>
        <div className="flex items-center gap-2">
          <Button
            size="sm"
            variant="secondary"
            loading={busy === 'export'}
            onClick={() => void runExport('csv')}
          >
            <Download aria-hidden="true" className="size-3.5" />
            {t('settings.audit.export.csv')}
          </Button>
          <Button
            size="sm"
            variant="secondary"
            loading={busy === 'export'}
            onClick={() => void runExport('json')}
          >
            <Download aria-hidden="true" className="size-3.5" />
            {t('settings.audit.export.json')}
          </Button>
        </div>
      </div>

      {/* 导出只写临时目录：把路径显示出来，用户才知道"这还不是最终位置" */}
      {exportedPath === null ? null : (
        <div className="flex flex-wrap items-center gap-2 rounded-sm bg-surface-sunken px-2 py-1.5 text-12">
          <span className="text-fg-subtle">{t('settings.audit.export.path')}</span>
          <code className="min-w-0 break-all text-fg">{exportedPath}</code>
          <button
            type="button"
            className="inline-flex items-center gap-1 text-brand hover:underline"
            onClick={() => {
              void navigator.clipboard.writeText(exportedPath).catch(show);
            }}
          >
            <ClipboardCopy aria-hidden="true" className="size-3" />
            {t('settings.audit.export.copy')}
          </button>
        </div>
      )}

      <div className="flex flex-wrap items-end gap-3">
        <div className="w-52">
          <SelectField
            label={t('settings.audit.filters.repo')}
            value={repoId}
            options={repoOptions}
            onValueChange={(value) => {
              setRepoId(value);
              resetPage();
            }}
          />
        </div>
        <div className="w-52">
          <SelectField
            label={t('settings.audit.filters.opType')}
            value={opType}
            options={opOptions}
            onValueChange={(value) => {
              setOpType(value);
              resetPage();
            }}
          />
        </div>
        <span className="text-12 text-fg-subtle">
          {t('settings.audit.page.range', { from: pageStart, to: pageEnd, total })}
        </span>
        <div className="ml-auto flex items-center gap-2">
          <Button
            size="sm"
            variant="secondary"
            disabled={offset === 0 || listQuery.isFetching}
            onClick={() => {
              setOffset((current) => Math.max(0, current - PAGE_SIZE));
            }}
          >
            {t('settings.audit.page.prev')}
          </Button>
          <Button
            size="sm"
            variant="secondary"
            disabled={offset + PAGE_SIZE >= total || listQuery.isFetching}
            onClick={() => {
              setOffset((current) => current + PAGE_SIZE);
            }}
          >
            {t('settings.audit.page.next')}
          </Button>
        </div>
      </div>

      {listQuery.isError ? (
        <ErrorState
          title={t('settings.audit.loadFailed')}
          hint={t('settings.audit.loadFailedHint')}
          retryLabel={t('settings.audit.retry')}
          onRetry={() => void listQuery.refetch()}
        />
      ) : (
        <div className="min-h-0 overflow-auto">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead widthClassName="w-44">{t('settings.audit.column.time')}</TableHead>
                <TableHead widthClassName="w-32">{t('settings.audit.column.type')}</TableHead>
                <TableHead widthClassName="w-20">{t('settings.audit.column.result')}</TableHead>
                <TableHead widthClassName="w-20">{t('settings.audit.column.duration')}</TableHead>
                <TableHead widthClassName="w-20">{t('settings.audit.column.snapshot')}</TableHead>
                <TableHead>{t('settings.audit.column.summary')}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {listQuery.isPending ? (
                <TableSkeletonRows rows={5} columns={6} />
              ) : entries.length === 0 ? (
                <TableEmptyRow colSpan={6}>{t('settings.audit.empty')}</TableEmptyRow>
              ) : (
                entries.map((entry) => {
                  const labelKey = opLabelKey(entry.opType);
                  return (
                    <TableRow key={entry.id}>
                      <TableCell className="text-fg-muted">
                        {formatTime(entry.startedAtMs)}
                      </TableCell>
                      <TableCell>{labelKey === null ? entry.opType : t(labelKey)}</TableCell>
                      <TableCell>
                        <span
                          className={
                            entry.result === 'ok'
                              ? 'text-success'
                              : entry.result === 'running'
                                ? 'text-warning'
                                : 'text-danger'
                          }
                        >
                          {t(resultKey(entry.result))}
                        </span>
                      </TableCell>
                      <TableCell className="text-fg-muted">
                        {formatDuration(t, entry.durationMs)}
                      </TableCell>
                      <TableCell className="text-fg-muted">
                        {entry.snapshotId === null ? '—' : `#${entry.snapshotId}`}
                      </TableCell>
                      {/* 摘要与失败原话放同一格：排查时它们总是一起被读 */}
                      <TableCell className="max-w-0">
                        <span className="block truncate" title={entry.argsJson ?? ''}>
                          {entry.argsJson ?? '—'}
                        </span>
                        {entry.stderrSummary === null ? null : (
                          <span
                            className="mt-0.5 block truncate text-danger"
                            title={entry.stderrSummary}
                          >
                            {entry.stderrSummary}
                          </span>
                        )}
                      </TableCell>
                    </TableRow>
                  );
                })
              )}
            </TableBody>
          </Table>
        </div>
      )}

      {/* 保留策略：默认 90 天 / 10000 条，可改；清理动作本身也会被记录 */}
      <div className="flex flex-wrap items-end gap-3 border-t border-line pt-3">
        <div className="w-40">
          <Input
            label={t('settings.audit.retention.days')}
            type="number"
            min={1}
            max={3650}
            value={String(retentionDays)}
            onChange={(event) => {
              const next = Number(event.target.value);
              if (Number.isFinite(next) && next > 0) {
                void setJson(AUDIT_RETENTION_DAYS_KEY, next).catch(show);
              }
            }}
          />
        </div>
        <div className="w-40">
          <Input
            label={t('settings.audit.retention.rows')}
            type="number"
            min={100}
            max={1_000_000}
            value={String(retentionRows)}
            onChange={(event) => {
              const next = Number(event.target.value);
              if (Number.isFinite(next) && next > 0) {
                void setJson(AUDIT_RETENTION_MAX_KEY, next).catch(show);
              }
            }}
          />
        </div>
        <Button
          size="sm"
          variant="secondary"
          loading={busy === 'prune'}
          onClick={() => void runPrune()}
        >
          <Trash2 aria-hidden="true" className="size-3.5" />
          {t('settings.audit.prune.button')}
        </Button>
        <p className="max-w-md text-11 text-fg-subtle">{t('settings.audit.retention.hint')}</p>
      </div>
    </div>
  );
}
