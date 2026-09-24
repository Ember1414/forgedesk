//! 快照与回滚页面（M1 / T1.9）。
//!
//! # 交互纪律（红线 R7 在 UI 层的最后一环）
//!
//! 回滚是界面上破坏性最强的按钮：它把工作区、索引与 HEAD 一起搬回过去。
//! 因此点"回滚"后的顺序是固定的：**先取差异摘要 → 在确认框里展示 → 用户确认 →
//! 执行 → 展示结果与保护点**。跳过差异摘要直接弹确认框，
//! 等于让用户对一个他看不懂的东西说"是"。
//!
//! # 列表里要不要显示 label
//!
//! 不显示：v1 里 label 恒等于 kind 的短名（两列一样的内容），显示 kind 的
//! i18n 文案就够了。等出现"同场景多次打点需要区分"的需求（T2.8 的 stash 等）
//! 再把 label 变成自由文本。

import { useState } from 'react';

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Trash2 } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useParams } from 'react-router-dom';

import { STATUS_QUERY_KEY } from '@/features/workspace/WorkspaceStatusPage';
import { normalizeError, useAppError } from '@/lib/errors';
import { snapshotDiff, snapshotList, snapshotPrune, snapshotRestore } from '@/lib/ipc/snapshots';
import type { SnapshotDiff, SnapshotMeta } from '@/lib/ipc/snapshots';
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

const SNAPSHOTS_QUERY_KEY = 'snapshots';

/** kind 短名 → i18n key（只增不改的清单，见 SnapshotKind::key）。 */
const KIND_LABEL_KEYS: Readonly<Record<string, string>> = {
  manual: 'snapshots.kind.manual',
  'pre-commit': 'snapshots.kind.preCommit',
  'pre-restore': 'snapshots.kind.preRestore',
  'pre-sync': 'snapshots.kind.preSync',
  'pre-head-move': 'snapshots.kind.preHeadMove',
};

/** 时间列的显示格式（本地时区；表格里不需要秒以下的精度）。 */
function formatTime(ms: number): string {
  return new Date(ms).toLocaleString();
}

export function SnapshotsPage() {
  const { t } = useTranslation('shell');
  const params = useParams();
  const repoId = Number(params.repoId);
  const queryClient = useQueryClient();
  const { show } = useAppError();

  const [target, setTarget] = useState<SnapshotMeta | null>(null);
  const [diff, setDiff] = useState<SnapshotDiff | null>(null);

  const listQuery = useQuery({
    queryKey: [SNAPSHOTS_QUERY_KEY, repoId],
    queryFn: () => snapshotList(repoId, 50),
    enabled: Number.isFinite(repoId),
  });

  const invalidate = () => {
    void queryClient.invalidateQueries({ queryKey: [SNAPSHOTS_QUERY_KEY, repoId] });
    void queryClient.invalidateQueries({ queryKey: [STATUS_QUERY_KEY, repoId] });
  };

  const restoreMutation = useMutation({
    mutationFn: (snapshotId: number) => snapshotRestore(repoId, snapshotId),
    onSuccess: (report) => {
      setTarget(null);
      setDiff(null);
      pushToast({
        tone: 'success',
        title: t('snapshots.restored', { oid: report.headOid.slice(0, 7) }),
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

  return (
    <section className="flex h-full flex-col gap-4" data-testid="snapshots-page">
      <header className="flex flex-wrap items-end justify-between gap-3">
        <div className="flex flex-col gap-1">
          <h2 className="text-16 font-semibold tracking-tight">{t('snapshots.title')}</h2>
          <p className="text-12 text-fg-subtle">{t('snapshots.description')}</p>
        </div>
        <Button
          variant="secondary"
          size="sm"
          loading={pruneMutation.isPending}
          onClick={() => {
            pruneMutation.mutate();
          }}
        >
          <Trash2 aria-hidden="true" className="size-3.5" />
          {t('snapshots.prune')}
        </Button>
      </header>

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
