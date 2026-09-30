/**
 * 回滚报告面板（T3.9 建立，T3.10 起被操作历史页复用）。
 *
 * 三件事必须同屏，缺一件用户就无法回答"我现在到底安不安全"：
 *
 * 1. **结局**——完成 / 已退回动手之前 / 需要人工介入；
 * 2. **阶段清单**——哪一步成了、哪一步没成、为什么、各花了多久；
 * 3. **未跟踪内容的实情**——恢复了几个、找不回几个、没删几个。
 *
 * 紧急模式下再挂上可复制的恢复指引：那是"绝不让用户无从下手"的最后一道。
 */
import { Check, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import type { RestoreReport } from '@/lib/ipc/snapshots';

import { outcomeClass, summarizePaths } from './reportFormat';

export interface RestoreReportPanelProps {
  readonly report: RestoreReport;
  /** 关闭入口；不传则不渲染关闭按钮（嵌在页面里时不需要）。 */
  readonly onDismiss?: (() => void) | null;
}

export function RestoreReportPanel({ report, onDismiss = null }: RestoreReportPanelProps) {
  const { t } = useTranslation('shell');

  const summarize = (paths: readonly string[]): string =>
    summarizePaths(paths, (count) => t('snapshots.report.more', { count }));

  return (
    <section
      className="flex flex-col gap-2 rounded-md border border-line p-3 text-12"
      data-testid="snapshot-report"
    >
      <header className="flex items-center justify-between gap-2">
        <h3 className="font-medium">{t('snapshots.report.title')}</h3>
        {onDismiss === null ? null : (
          <button
            type="button"
            className="fd-transition rounded-sm text-fg-subtle hover:text-fg"
            onClick={onDismiss}
          >
            {t('snapshots.dismiss')}
          </button>
        )}
      </header>

      {/* 结局：一句话回答"我现在能不能安心"（图标 + 文字，不只靠颜色） */}
      <p
        className={`flex items-center gap-1.5 font-medium ${outcomeClass(report.outcome)}`}
        data-testid="report-outcome"
      >
        {report.outcome === 'completed' ? (
          <Check aria-hidden="true" className="size-3.5 shrink-0" />
        ) : (
          <X aria-hidden="true" className="size-3.5 shrink-0" />
        )}
        {t(`snapshots.outcome.${report.outcome}`)}
      </p>

      {/* 阶段清单（T3.9）：哪一步成了、哪一步没成、各花了多久 */}
      <ol className="flex flex-col gap-1" data-testid="report-stages">
        {report.stages.map((stage) => (
          <li key={stage.stage} className="flex flex-wrap items-baseline gap-x-2">
            {stage.ok ? (
              <Check aria-hidden="true" className="size-3 shrink-0 text-success" />
            ) : (
              <X aria-hidden="true" className="size-3 shrink-0 text-danger" />
            )}
            <span className="text-fg">{t(`snapshots.stage.${stage.stage}`)}</span>
            <span className="text-fg-subtle">
              {t('snapshots.report.duration', { ms: stage.durationMs })}
            </span>
            {stage.detail === null ? null : <span className="text-danger">{stage.detail}</span>}
          </li>
        ))}
      </ol>

      <ul className="flex flex-col gap-1 text-fg-muted">
        <li>{t('snapshots.report.head', { oid: report.headOid.slice(0, 7) })}</li>
        <li>{t('snapshots.report.untracked', { count: report.untrackedRestored })}</li>
        <li>
          {report.verified ? t('snapshots.report.verified') : t('snapshots.report.unverified')}
        </li>
        {report.untrackedFailed.length === 0 ? null : (
          <li className="text-danger" data-testid="snapshot-report-failed">
            {t('snapshots.report.failed', {
              count: report.untrackedFailed.length,
              paths: summarize(report.untrackedFailed),
            })}
          </li>
        )}
        {report.untrackedExtra.length === 0 ? null : (
          <li data-testid="snapshot-report-extra">
            {t('snapshots.report.extra', {
              count: report.untrackedExtra.length,
              paths: summarize(report.untrackedExtra),
            })}
          </li>
        )}
      </ul>

      {/* 紧急模式：给用户一份能照着做的恢复指引（命令一条一条可复制） */}
      {report.emergency === null ? null : (
        <div
          className="flex flex-col gap-2 rounded-md border border-danger bg-danger/10 p-2"
          data-testid="report-emergency"
        >
          <p className="font-medium">{t('snapshots.emergency.title')}</p>
          <ul className="flex list-disc flex-col gap-1 pl-4 text-fg-muted">
            {report.emergency.notes.map((note) => (
              <li key={note}>{note}</li>
            ))}
          </ul>
          <p className="font-medium">{t('snapshots.emergency.commands')}</p>
          <ul className="flex flex-col gap-1">
            {report.emergency.commands.map((command) => (
              <li key={command} className="flex items-center gap-2">
                <code className="min-w-0 flex-1 truncate rounded-sm bg-surface-sunken px-1 font-mono">
                  {command}
                </code>
                <button
                  type="button"
                  className="fd-transition shrink-0 rounded-sm border border-line px-1.5 hover:bg-surface-sunken"
                  onClick={() => {
                    void navigator.clipboard.writeText(command).catch(() => undefined);
                  }}
                >
                  {t('snapshots.emergency.copy')}
                </button>
              </li>
            ))}
          </ul>
          {report.emergency.backupDir === null ? null : (
            <p className="text-fg-muted">
              {t('snapshots.emergency.backupDir', { dir: report.emergency.backupDir })}
            </p>
          )}
        </div>
      )}
    </section>
  );
}
