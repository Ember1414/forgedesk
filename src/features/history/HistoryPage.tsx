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
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { ReactNode } from 'react';

import { useQuery } from '@tanstack/react-query';
import {
  Activity,
  ChevronDown,
  ChevronUp,
  List,
  Map,
  Maximize2,
  Network,
  ZoomIn,
  ZoomOut,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useSearchParams, useParams } from 'react-router-dom';

import { normalizeError } from '@/lib/errors';
import { settingsGet, settingsSet } from '@/lib/ipc';
import { gitBranchList, gitLogAuthors } from '@/lib/ipc';
import { PERFORMANCE_PAGE_SIZE, usePerformanceMode } from '@/lib/performanceMode';
import { useRepoChangeInvalidation } from '@/lib/repoChanged';
import { AUTHORS_QUERY_KEY, BRANCHES_QUERY_KEY } from '@/lib/queryKeys';
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
import {
  EMPTY_FILTERS_STATE,
  filtersFromSearchParams,
  filtersFromSettingValue,
  filtersToQuery,
  filtersToSearchParams,
  filtersToSettingValue,
  hasActiveFilters,
  HISTORY_FILTERS_SETTING_KEY,
  searchParamsHaveFilters,
} from '@/features/history/historyFilters';
import type { HistoryFiltersState } from '@/features/history/historyFilters';
import { HistoryFilterBar } from '@/features/history/HistoryFilterBar';
import { HistoryOpsPanel } from '@/features/history/HistoryOpsPanel';
import { GraphListMode } from '@/features/history/GraphListMode';
import { GraphOverlay } from '@/features/history/GraphOverlay';
import { useGraphPerfStore } from '@/features/history/graphPerfStore';
import {
  NO_MODIFIERS,
  useGraphSelectionStore,
  ZOOM_STEP,
} from '@/features/history/graphSelectionStore';
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

  // 筛选状态的真相源是 URL（可分享 / 刷新保持）；repo 级 settings 是它的
  // 持久化副本——URL 没带参数时用设置回填，变化时防抖写回。
  const [searchParams, setSearchParams] = useSearchParams();
  const filterState = useMemo(() => filtersFromSearchParams(searchParams), [searchParams]);
  const filters = useMemo(() => filtersToQuery(filterState), [filterState]);

  // 性能模式（T2.9）：大仓库把每页行数减半，首屏更快、滚动更跟手。
  // pageSize 变化会改变查询签名并重取第一页——模式判定基于状态条目数，
  // 打开仓库后基本稳定，不会出现来回抖动。
  const perfMode = usePerformanceMode(repoId);
  const graph = useGraphQuery(repoId, filters, perfMode ? { pageSize: PERFORMANCE_PAGE_SIZE } : {});
  // 引用 / 大量变更时按类别失效历史页（`refs` 与 `large` 已覆盖 `[LOG_QUERY_KEY, repoId]` 前缀）
  useRepoChangeInvalidation(repoId);

  // 作者与分支列表（筛选下拉的数据源；低频数据，60s 新鲜期足够）
  const authorsQuery = useQuery({
    queryKey: [AUTHORS_QUERY_KEY, repoId],
    queryFn: () => gitLogAuthors(repoId),
    enabled: Number.isFinite(repoId),
    staleTime: 60_000,
  });
  const branchesQuery = useQuery({
    queryKey: [BRANCHES_QUERY_KEY, repoId],
    queryFn: () => gitBranchList(repoId),
    enabled: Number.isFinite(repoId),
    staleTime: 60_000,
  });

  const applyFilters = useCallback(
    (next: HistoryFiltersState) => {
      setSearchParams(filtersToSearchParams(next), { replace: true });
    },
    [setSearchParams],
  );

  // 初始化：每个仓库只做一次；URL 没带筛选参数时用 repo 设置回填
  const settingsQuery = useQuery({
    queryKey: ['settings', 'repo', repoId, HISTORY_FILTERS_SETTING_KEY],
    queryFn: () => settingsGet('repo', HISTORY_FILTERS_SETTING_KEY, repoId),
    enabled: Number.isFinite(repoId),
    staleTime: Number.POSITIVE_INFINITY,
  });
  const initializedRepoRef = useRef<number | null>(null);
  useEffect(() => {
    if (!Number.isFinite(repoId) || settingsQuery.data === undefined) {
      return;
    }
    if (initializedRepoRef.current === repoId) {
      return;
    }
    initializedRepoRef.current = repoId;
    if (!searchParamsHaveFilters(searchParams)) {
      const stored = filtersFromSettingValue(settingsQuery.data);
      if (hasActiveFilters(stored)) {
        setSearchParams(filtersToSearchParams(stored), { replace: true });
      }
    }
  }, [repoId, settingsQuery.data, searchParams, setSearchParams]);

  // 写回：筛选变化后防抖持久化（settings 写失败不阻塞界面——它只是便利副本）
  useEffect(() => {
    if (!Number.isFinite(repoId) || initializedRepoRef.current !== repoId) {
      return;
    }
    const value = filtersToSettingValue(filterState);
    const timer = setTimeout(() => {
      settingsSet('repo', HISTORY_FILTERS_SETTING_KEY, value, repoId).catch(() => undefined);
    }, 500);
    return () => {
      clearTimeout(timer);
    };
  }, [filterState, repoId]);

  // 搜索命中（T2.3）：在**已加载**的行里找 subject 命中（列表查询不带正文，
  // 正文命中只能靠后端 grep 的结果集反映——高亮与跳转的范围是已加载部分）。
  const keyword = filterState.keyword.trim();
  const matchOids = useMemo(() => {
    const set = new Set<string>();
    if (keyword === '') {
      return set;
    }
    const needle = filterState.caseInsensitive ? keyword.toLowerCase() : keyword;
    for (const commit of graph.model.commits) {
      const haystack = filterState.caseInsensitive ? commit.subject.toLowerCase() : commit.subject;
      if (haystack.includes(needle)) {
        set.add(commit.oid);
      }
    }
    return set;
  }, [graph.model.commits, keyword, filterState.caseInsensitive]);

  // 行序的命中列表（跳转的"上一处 / 下一处"沿它循环）；当前命中记 oid 而不是
  // 下标——关键词变化后下标会错位，oid 天然稳定（不在列表里就从头开始）。
  const matchOrder = useMemo(() => {
    const rows = [...graph.model.rows].sort((left, right) => left.row - right.row);
    return rows.filter((row) => matchOids.has(row.oid)).map((row) => row.oid);
  }, [graph.model.rows, matchOids]);
  const [jumpOid, setJumpOid] = useState<string | null>(null);
  const [scrollRequest, setScrollRequest] = useState<{ row: number; token: number } | null>(null);
  const scrollTokenRef = useRef(0);

  const select = useGraphSelectionStore((state) => state.select);
  const order = useMemo(
    () => [...graph.model.rows].sort((left, right) => left.row - right.row).map((row) => row.oid),
    [graph.model.rows],
  );

  const jumpToMatch = useCallback(
    (delta: 1 | -1) => {
      if (matchOrder.length === 0) {
        return;
      }
      const current = jumpOid === null ? -1 : matchOrder.indexOf(jumpOid);
      const nextIndex = (current + delta + matchOrder.length) % matchOrder.length;
      const oid = matchOrder[nextIndex];
      if (oid === undefined) {
        return;
      }
      setJumpOid(oid);
      // 跳转 = 选中并打开详情（跟随模式），画布滚动到该行
      select(oid, NO_MODIFIERS, order);
      const row = graph.model.index.get(oid);
      if (row !== undefined) {
        scrollTokenRef.current += 1;
        setScrollRequest({ row: row.row, token: scrollTokenRef.current });
      }
    },
    [graph.model.index, jumpOid, matchOrder, order, select],
  );

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
    body = hasActiveFilters(filterState) ? (
      <EmptyState
        title={t('history.emptyFiltered.title')}
        description={t('history.emptyFiltered.description')}
        action={
          <Button
            variant="secondary"
            size="sm"
            onClick={() => {
              applyFilters(EMPTY_FILTERS_STATE);
            }}
            data-testid="history-clear-filters"
          >
            {t('history.emptyFiltered.clear')}
          </Button>
        }
      />
    ) : (
      <EmptyState title={t('history.empty.title')} description={t('history.empty.hint')} />
    );
  } else {
    body = (
      <div className="flex min-h-0 flex-1 flex-col gap-2">
        <div className="min-h-0 flex-1">
          {viewMode === 'graph' ? (
            <GraphOverlay
              model={graph.model}
              texts={texts}
              matchOids={matchOids}
              scrollToRow={scrollRequest}
              onNeedMore={handleNeedMore}
              className="h-full"
            />
          ) : (
            <GraphListMode
              texts={texts}
              matchOids={matchOids}
              focusOid={jumpOid}
              className="h-full"
            />
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

            {/* 搜索导航：有关键词时提供"上一处 / 下一处"跳转（T2.3） */}
            {keyword !== '' ? (
              <div
                role="group"
                aria-label={t('history.filter.searchNav')}
                className="flex items-center gap-1"
                data-testid="history-search-nav"
              >
                <span
                  className="min-w-14 text-center font-mono text-11 text-fg-subtle"
                  data-testid="history-search-count"
                >
                  {t('history.filter.matchCount', {
                    current:
                      matchOrder.indexOf(jumpOid ?? '') + 1 > 0
                        ? matchOrder.indexOf(jumpOid ?? '') + 1
                        : 0,
                    total: matchOrder.length,
                  })}
                </span>
                <IconButton
                  label={t('history.filter.prevMatch')}
                  size="sm"
                  disabled={matchOrder.length === 0}
                  onClick={() => {
                    jumpToMatch(-1);
                  }}
                  data-testid="history-search-prev"
                >
                  <ChevronUp aria-hidden="true" className="size-4" />
                </IconButton>
                <IconButton
                  label={t('history.filter.nextMatch')}
                  size="sm"
                  disabled={matchOrder.length === 0}
                  onClick={() => {
                    jumpToMatch(1);
                  }}
                  data-testid="history-search-next"
                >
                  <ChevronDown aria-hidden="true" className="size-4" />
                </IconButton>
              </div>
            ) : null}

            {/* 筛选栏（T2.3）：关键词搜索 + 分支多选 / 作者 / 时间 / 开关 */}
            <HistoryFilterBar
              state={filterState}
              onChange={applyFilters}
              authors={authorsQuery.data ?? []}
              branches={branchesQuery.data ?? []}
            />

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

      {/*
        历史操作（T2.8）：拣选 / 反转 / 重置到选中提交，以及 reflog 恢复。
        动作作用于"当前选中的提交"（图上点选的那一个），因此放在历史页同屏。
      */}
      <HistoryOpsPanel />

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
