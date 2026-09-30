/**
 * 预览区：执行后的历史（列表形式）。
 *
 * 用列表而不是复刻 DAG：面板的目标是"看清这次重写会发生什么"
 * （哪些被压、哪些被丢、哪些信息被改、有没有碰已推送的提交），
 * 列表的信息密度更高且不需要另做一套图布局；执行后回到历史页看真实 DAG。
 *
 * 行序与语义见 `buildPreviewRows`：存活行按新顺序在前，被丢弃的提交
 * 排在最后并划掉（它们在新历史里不存在，插在中间反而误导）。
 */
import { AlertTriangle } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import type { RebasePreview } from '@/lib/ipc';
import { cn } from '@/lib/utils';

import { buildPreviewRows, type PlanEntry } from './planState';

export interface RebasePreviewPaneProps {
  readonly entries: readonly PlanEntry[];
  /** 当前计划的预览（null = 尚未生成或计划非法）。 */
  readonly preview: RebasePreview | null;
  /** 预览正在生成。 */
  readonly pending: boolean;
}

/** 执行后历史的预览列表。 */
export function RebasePreviewPane({ entries, preview, pending }: RebasePreviewPaneProps) {
  const { t } = useTranslation('shell');

  if (preview === null) {
    return (
      <p className="text-12 text-fg-muted" data-testid="rebase-preview-empty">
        {pending ? t('history.rebase.preview.pending') : t('history.rebase.preview.blocked')}
      </p>
    );
  }

  const rows = buildPreviewRows(entries, preview);
  return (
    <div className="flex flex-col gap-2" data-testid="rebase-preview">
      <p className="text-12 text-fg-muted">
        {t('history.rebase.preview.affected', { count: preview.affectedCount })}
      </p>
      {preview.touchesPushed ? (
        <p
          className="flex items-start gap-2 rounded-md border border-warning bg-warning/10 p-2 text-12"
          data-testid="rebase-pushed-warning"
        >
          <AlertTriangle aria-hidden className="mt-0.5 size-3.5 shrink-0 text-warning" />
          {t('history.rebase.preview.pushed')}
        </p>
      ) : null}
      <ul className="flex flex-col">
        {rows.map((row) => {
          const dropped = row.kind === 'dropped';
          return (
            <li
              key={`${row.kind}-${row.oid}`}
              className="flex items-center gap-2 border-b border-line px-1 py-1.5 last:border-b-0"
            >
              <span
                className={cn(
                  'w-16 shrink-0 font-mono text-12',
                  dropped ? 'text-danger line-through' : 'text-fg-subtle',
                )}
              >
                {row.oid.slice(0, 7)}
              </span>
              <span
                className={cn(
                  'min-w-0 flex-1 truncate text-13',
                  dropped && 'text-fg-muted line-through',
                )}
                title={row.subject}
              >
                {row.subject}
              </span>
              {row.mergedOids.length > 0 ? (
                <span
                  className="shrink-0 rounded-sm bg-surface-sunken px-1 text-11 text-fg-muted"
                  data-testid={`rebase-merged-${row.oid}`}
                >
                  {t('history.rebase.preview.merged', { count: row.mergedOids.length })}
                </span>
              ) : null}
              {row.reworded ? (
                <span className="shrink-0 text-11 text-brand">
                  {t('history.rebase.preview.reworded')}
                </span>
              ) : null}
              {dropped ? (
                <span className="shrink-0 text-11 text-danger">
                  {t('history.rebase.preview.dropped')}
                </span>
              ) : null}
            </li>
          );
        })}
      </ul>
    </div>
  );
}
