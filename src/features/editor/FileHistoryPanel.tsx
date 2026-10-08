/**
 * 文件历史面板（T5.8）：时间线列表（变更类型 A/M/D/R）+ 与历史版本对比。
 *
 * 点击条目 → 用 git_file_at(rev) 取历史内容，与当前编辑器内容并排对比
 * （复用编辑器的对比区）；条目上的提交哈希点击 → 打开提交详情（Dialog）。
 */
import { useMemo, useState } from 'react';

import { useQuery } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';

import { normalizeError } from '@/lib/errors';

import { gitFileHistory } from '@/lib/ipc/blame';
import { gitCommitDetail } from '@/lib/ipc';
import { Button } from '@/ui/components/button';
import { Dialog, DialogContent, DialogTitle } from '@/ui/components/dialog';
import { cn } from '@/lib/utils';
import { formatDateTime } from '@/lib/i18n/intl';

export interface FileHistoryPanelProps {
  readonly repoId: number;
  readonly path: string;
  /** 与选中的历史版本对比（父组件把历史内容填进编辑器对比区）。 */
  readonly onCompare: (historicalContent: string) => void;
  readonly onClose: () => void;
}

/** 变更类型徽标的颜色语义（A 增 / M 改 / D 删 / R 改名）。 */
function changeKindClass(kind: string): string {
  switch (kind) {
    case 'A':
      return 'text-success';
    case 'D':
      return 'text-danger';
    case 'R':
      return 'text-info';
    default:
      return 'text-warning';
  }
}

export function CommitDetailDialog({
  repoId,
  oid,
  onClose,
}: {
  readonly repoId: number;
  readonly oid: string;
  readonly onClose: () => void;
}) {
  const { t } = useTranslation('shell');
  const detail = useQuery({
    queryKey: ['commit-detail', repoId, oid],
    queryFn: () => gitCommitDetail(repoId, oid),
  });

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent closeLabel={t('editor.history.close')} className="max-w-xl">
        <DialogTitle>{t('editor.history.commitDetail')}</DialogTitle>
        {detail.data ? (
          <div className="flex flex-col gap-2 text-13">
            <p className="font-medium">{detail.data.meta?.subject ?? ''}</p>
            <p className="text-fg-muted font-mono text-12">
              {detail.data.meta?.author?.name ?? ''} ·{' '}
              {formatDateTime((detail.data.meta?.author?.time ?? 0) * 1000)}
            </p>
            <p className="text-fg-subtle text-12">
              {t('editor.history.filesChanged', {
                count: detail.data.files?.length ?? 0,
              })}
            </p>
          </div>
        ) : detail.isError ? (
          <p className="text-danger text-13">{t('editor.history.detailError')}</p>
        ) : (
          <p className="text-fg-muted text-13">…</p>
        )}
      </DialogContent>
    </Dialog>
  );
}

export function FileHistoryPanel({ repoId, path, onCompare, onClose }: FileHistoryPanelProps) {
  const { t } = useTranslation('shell');
  const [page, setPage] = useState(0);
  const [detailOid, setDetailOid] = useState<string | null>(null);
  const [comparing, setComparing] = useState<string | null>(null);

  const history = useQuery({
    queryKey: ['file-history', repoId, path, page],
    queryFn: () => gitFileHistory(repoId, path, { follow: true, limit: 50, cursor: page * 50 }),
  });

  // 失败时把后端的原始 detail（git 的 stderr 等）亮出来——"加载失败"无法定位问题
  const normalized = useMemo(() => normalizeError(history.error), [history.error]);

  const compareWith = async (oid: string) => {
    setComparing(oid);
    try {
      const result = await import('@/lib/ipc/blame').then((m) => m.gitFileAt(repoId, path, oid));
      const text = atob(result.contentBase64);
      onCompare(text);
    } finally {
      setComparing(null);
    }
  };

  return (
    <div className="border-line flex h-full min-h-0 w-72 shrink-0 flex-col border-s">
      <div className="border-line flex items-center gap-2 border-b px-2 py-1.5">
        <span className="text-fg-muted min-w-0 flex-1 truncate font-mono text-12">{path}</span>
        <Button size="sm" variant="ghost" onClick={onClose}>
          {t('editor.history.close')}
        </Button>
      </div>

      <div className="min-h-0 flex-1 overflow-auto p-1">
        {history.isLoading ? (
          <p className="text-fg-subtle p-2 text-12">…</p>
        ) : history.isError ? (
          // 列表失败此前复用"提交详情加载失败"的文案（张冠李戴），而且没有重试入口：
          // 用户看到一句对不上的错误，也不知道能做什么。原始 detail 一并展示——
          // "加载失败"四个字无法定位问题，git 的 stderr 才是答案。
          <div className="flex flex-col items-start gap-2 p-2">
            <p className="text-danger text-12">{t('editor.history.listError')}</p>
            {normalized.detail !== undefined && normalized.detail !== '' ? (
              <p className="text-fg-subtle max-h-24 overflow-auto break-all font-mono text-11">
                {normalized.detail}
              </p>
            ) : null}
            <Button size="sm" variant="secondary" onClick={() => void history.refetch()}>
              {t('editor.history.retry')}
            </Button>
          </div>
        ) : (
          <ul className="flex flex-col gap-0.5">
            {(history.data?.items ?? []).map((entry) => (
              <li
                key={`${entry.oid}-${entry.changeKind}`}
                className="hover:bg-surface-sunken flex items-start gap-1.5 rounded-md px-1.5 py-1"
              >
                <span
                  className={cn(
                    'mt-0.5 w-4 shrink-0 text-center font-mono text-11 font-bold',
                    changeKindClass(entry.changeKind),
                  )}
                  title={t('editor.history.changeKind', { kind: entry.changeKind })}
                >
                  {entry.changeKind}
                </span>
                <div className="min-w-0 flex-1">
                  <button
                    type="button"
                    className="block w-full truncate text-start text-12 hover:underline"
                    title={entry.subject}
                    onClick={() => setDetailOid(entry.oid)}
                  >
                    {entry.subject}
                  </button>
                  <span className="text-fg-subtle text-11">
                    {entry.author} · {new Date(entry.authorTime * 1000).toLocaleDateString()}
                  </span>
                </div>
                <Button
                  size="sm"
                  variant="ghost"
                  disabled={comparing === entry.oid}
                  onClick={() => void compareWith(entry.oid)}
                  aria-label={t('editor.history.compareAria', { oid: entry.oid.slice(0, 8) })}
                >
                  {t('editor.history.compare')}
                </Button>
              </li>
            ))}
          </ul>
        )}
        {history.data?.nextCursor !== null && history.data?.nextCursor !== undefined ? (
          <div className="p-1">
            <Button size="sm" variant="secondary" onClick={() => setPage((p) => p + 1)}>
              {t('editor.history.loadMore')}
            </Button>
          </div>
        ) : null}
      </div>

      {detailOid !== null ? (
        <CommitDetailDialog repoId={repoId} oid={detailOid} onClose={() => setDetailOid(null)} />
      ) : null}
    </div>
  );
}
