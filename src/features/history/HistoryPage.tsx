//! 提交历史页面（T2.2）。
//!
//! # 职责边界
//!
//! 本组件是**容器**：它只负责把数据通路（`useGraphQuery`）、UI 状态
//! （`graphSelectionStore` / `graphPerfStore`）与表现层（`GraphOverlay` 图模式 /
//! `GraphListMode` 列表模式）接起来，再加一条工具条与空/错/加载三态。
//! 它**不做 Git 语义**（AGENTS.md §6）：泳道布局、折叠、可达性全在后端，
//! 前端只消费 `HistoryPage` DTO。
//!
//! # 详情面板为什么不在这里渲染
//!
//! 提交详情挂在 `RepoLayout` 既有的详情位（右侧 / 底部 / 隐藏），那是所有仓库子页
//! 共享的外壳。本页面通过 `graphSelectionStore.detailOid` 驱动它（选中即打开详情），
//! 面板自己从 Query 缓存里按 oid 取提交（见 `CommitDetailPanel.tsx`）。
//! 离开本页时清掉 `detailOid`，避免陈旧提交泄漏到别的仓库页的详情位。
import { useCallback, useEffect, useMemo } from 'react';
import type { ReactNode } from 'react';

import {
  Activity,
  ChevronDown,
  Filter,
  List,
  Map,
  Maximize2,
  Network,
  ZoomIn,
  ZoomOut,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useParams } from 'react-router-dom';

import { normalizeError } from '@/lib/errors';
import { useRepoChangeInvalidation } from '@/lib/repoChanged';
import { cn } from '@/lib/utils';

import { PlaceholderPage } from '@/ui/PlaceholderPage';
import { GraphPerfPanel } from '@/ui/__dev__/GraphPerfPanel';
import { Button } from '@/ui/components/button';
import { EmptyState } from '@/ui/components/empty-state';
import { ErrorState } from '@/ui/components/error-state';
import { IconButton } from '@/ui/components/icon-button';
import { Skeleton } from '@/ui/components/skeleton';

import { buildRowTexts } from '@/features/history/commitMeta';
import type { RowTextFormat } from '@/features/history/commitMeta';
import { GraphListMode } from '@/features/history/GraphListMode';
import { GraphOverlay } from '@/features/history/GraphOverlay';
import { useGraphPerfStore } from '@/features/history/graphPerfStore';
import { useGraphSelectionStore, ZOOM_STEP } from '@/features/history/graphSelectionStore';
import { useGraphQuery } from '@/features/history/useGraphQuery';

/**
 * 模块加载时的时间基准（秒）。
 *
 * react-hooks/purity 禁止在渲染期调用 Date.now()，而相对时间只需要分钟级精度。
 * 模块加载到组件挂载的延迟可忽略（同一 tick）；长时间挂着不操作的场景里
 * 相对时间会偏大几分钟，但用户刷新 / 切页就会重新加载模块。
 */
const HISTORY_TIME_BASE_SECONDS = Math.floor(Date.now() / 1000);

export function HistoryPage() {
  const { t } = useTranslation('shell');
  const params = useParams();
  const repoId = Number(params.repoId);

  const graph = useGraphQuery(repoId);
  // 引用 / 大量变更时按类别失效历史页（`refs` 与 `large` 已覆盖 `[LOG_QUERY_KEY, repoId]` 前缀）
  useRepoChangeInvalidation(repoId);

  const viewMode = useGraphSelectionStore((state) => state.viewMode);
  const setViewMode = useGraphSelectionStore((state) => state.setViewMode);
  const scale = useGraphSelectionStore((state) => state.scale);
  const zoomBy = useGraphSelectionStore((state) => state.zoomBy);
  const resetView = useGraphSelectionStore((state) => state.resetView);
  const minimapOpen = useGraphSelectionStore((state) => state.minimapOpen);
  const toggleMinimap = useGraphSelectionStore((state) => state.toggleMinimap);
  const setDetailOid = useGraphSelectionStore((state) => state.setDetailOid);

  const perfEnabled = useGraphPerfStore((state) => state.enabled);
  const togglePerf = useGraphPerfStore((state) => state.toggleEnabled);
  const setPerfEnabled = useGraphPerfStore((state) => state.setEnabled);

  // 离开历史页时清掉详情，避免陈旧提交留在共享详情位（切仓库后尤其明显）
  useEffect(
    () => () => {
      setDetailOid(null);
    },
    [setDetailOid],
  );

  // react-hooks/purity 禁止在渲染期调用 Date.now()。
  // 模块加载时的时间基准对分钟级粒度的相对时间显示足够准确：
  // - 组件挂载到数据到达之间的时间差可忽略（秒级）
  // - 长时间运行后 repo:changed 刷新数据时组件重渲染，但时间基准不更新
  //   → 可接受：用户不会在同一页面停留数小时不操作
  const nowSeconds = HISTORY_TIME_BASE_SECONDS;

  const format = useMemo<RowTextFormat>(
    () => ({
      relativeTime: (value) => t(`history.time.${value.unit}`, { count: value.count }),
      collapsed: (count) => t('history.collapsed', { count }),
      pending: t('history.pending'),
    }),
    [t],
  );

  const texts = useMemo(
    () => buildRowTexts(graph.model.rows, graph.model.commitByOid, nowSeconds, format),
    [graph.model.rows, graph.model.commitByOid, nowSeconds, format],
  );

  const loadMore = graph.loadMore;
  const handleNeedMore = useCallback(() => {
    loadMore();
  }, [loadMore]);

  if (!Number.isFinite(repoId)) {
    return (
      <PlaceholderPage
        plannedTask="T2.2"
        titleKey="history.title"
        descriptionKey="history.description"
      />
    );
  }

  const showToolbar = !graph.isPending && !graph.isError;

  let body: ReactNode;
  if (graph.isError) {
    const normalized = normalizeError(graph.error);
    body = (
      <ErrorState
        title={t('history.error.title')}
        hint={t('history.error.hint')}
        {...(normalized.detail === undefined ? {} : { details: normalized.detail })}
        retryLabel={t('common:actions.retry')}
        retryLoading={graph.isFetching}
        onRetry={graph.refresh}
      />
    );
  } else if (graph.isPending) {
    body = (
      <div aria-busy="true" className="flex h-full min-h-40 flex-col gap-2">
        <span className="sr-only">{t('history.loading')}</span>
        <Skeleton className="h-8 w-full" />
        <Skeleton className="min-h-0 flex-1" />
      </div>
    );
  } else if (graph.model.rowCount === 0) {
    body = <EmptyState title={t('history.empty.title')} description={t('history.empty.hint')} />;
  } else {
    body = (
      <div className="flex min-h-0 flex-1 flex-col gap-2">
        <div className="min-h-0 flex-1">
          {viewMode === 'graph' ? (
            <GraphOverlay
              model={graph.model}
              texts={texts}
              onNeedMore={handleNeedMore}
              className="h-full"
            />
          ) : (
            <GraphListMode texts={texts} className="h-full" />
          )}
        </div>
        <div className="flex shrink-0 items-center justify-between gap-2 text-12 text-fg-subtle">
          <span data-testid="history-loaded-count">
            {t('history.loadedCount', { count: graph.model.rowCount })}
          </span>
          {graph.hasNextPage ? (
            <Button
              size="sm"
              variant="secondary"
              loading={graph.isFetchingNextPage}
              onClick={handleNeedMore}
              data-testid="history-load-more"
            >
              <ChevronDown aria-hidden="true" className="size-3.5" />
              {graph.isFetchingNextPage ? t('history.loadingMore') : t('history.loadMore')}
            </Button>
          ) : null}
        </div>
      </div>
    );
  }

  return (
    <section
      className="flex h-full min-h-0 flex-col gap-3"
      data-testid="history-page"
      aria-label={t('history.title')}
    >
      <header className="flex flex-wrap items-center justify-between gap-2">
        <h1 className="text-20 font-semibold tracking-tight">{t('history.title')}</h1>

        {showToolbar ? (
          <div className="flex flex-wrap items-center gap-2" data-testid="history-toolbar">
            {/* 视图模式：图 / 列表（无障碍等价路径，AGENTS.md 要求）。用 aria-pressed
                而不是 radio 语义：两个按钮各自表达"当前是否处于该模式"。 */}
            <div
              role="group"
              aria-label={t('history.mode.label')}
              className="flex items-center gap-1"
            >
              <Button
                size="sm"
                variant="secondary"
                aria-pressed={viewMode === 'graph'}
                onClick={() => {
                  setViewMode('graph');
                }}
                data-testid="history-mode-graph"
              >
                <Network aria-hidden="true" className="size-3.5" />
                {t('history.mode.graph')}
              </Button>
              <Button
                size="sm"
                variant="secondary"
                aria-pressed={viewMode === 'list'}
                onClick={() => {
                  setViewMode('list');
                }}
                data-testid="history-mode-list"
              >
                <List aria-hidden="true" className="size-3.5" />
                {t('history.mode.list')}
              </Button>
            </div>

            {/* 缩放（仅图模式有意义；列表模式是定高表格，缩放对它无影响） */}
            <div
              role="group"
              aria-label={t('history.zoom.label')}
              className={cn(
                'flex items-center gap-1',
                viewMode === 'list' ? 'opacity-50' : undefined,
              )}
            >
              <IconButton
                label={t('history.zoom.out')}
                size="sm"
                disabled={viewMode === 'list'}
                onClick={() => {
                  zoomBy(1 / ZOOM_STEP);
                }}
                data-testid="history-zoom-out"
              >
                <ZoomOut aria-hidden="true" className="size-4" />
              </IconButton>
              <span
                className="min-w-12 text-center font-mono text-12 text-fg-subtle"
                data-testid="history-zoom-level"
              >
                {t('history.zoom.level', { percent: Math.round(scale * 100) })}
              </span>
              <IconButton
                label={t('history.zoom.in')}
                size="sm"
                disabled={viewMode === 'list'}
                onClick={() => {
                  zoomBy(ZOOM_STEP);
                }}
                data-testid="history-zoom-in"
              >
                <ZoomIn aria-hidden="true" className="size-4" />
              </IconButton>
              <IconButton
                label={t('history.zoom.reset')}
                size="sm"
                onClick={resetView}
                data-testid="history-zoom-reset"
              >
                <Maximize2 aria-hidden="true" className="size-4" />
              </IconButton>
            </div>

            {/* 迷你地图开关（仅图模式渲染迷你地图） */}
            <IconButton
              label={minimapOpen ? t('history.minimap.close') : t('history.minimap.open')}
              size="sm"
              aria-pressed={minimapOpen}
              disabled={viewMode === 'list'}
              onClick={toggleMinimap}
              data-testid="history-minimap-toggle"
            >
              <Map aria-hidden="true" className="size-4" />
            </IconButton>

            {/* 筛选入口：T2.3 才接通（分支/作者/路径/时间）。如实标注为禁用而不是藏起来，
                否则用户会以为"没有筛选功能"。 */}
            <Button
              size="sm"
              variant="secondary"
              disabled
              title={t('history.filter.placeholder')}
              data-testid="history-filter"
            >
              <Filter aria-hidden="true" className="size-3.5" />
              {t('history.filter.label')}
            </Button>

            {/* dev 性能面板开关（生产构建里这个分支被静态替换掉） */}
            {import.meta.env.DEV ? (
              <IconButton
                label={t('history.perf.toggle')}
                size="sm"
                aria-pressed={perfEnabled}
                onClick={togglePerf}
                data-testid="history-perf-toggle"
              >
                <Activity aria-hidden="true" className="size-4" />
              </IconButton>
            ) : null}
          </div>
        ) : null}
      </header>

      {body}

      {import.meta.env.DEV && perfEnabled ? (
        <GraphPerfPanel
          onClose={() => {
            setPerfEnabled(false);
          }}
        />
      ) : null}
    </section>
  );
}
