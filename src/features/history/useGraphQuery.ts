/**
 * 提交历史的数据通路（T2.2）。
 *
 * # 这个 hook 负责什么
 *
 * 把"分页的 `git_log_page`"变成"一张可以整块渲染的图"：
 *   1. 每页一条独立的 Query 缓存记录，键是 `logKey(repoId, cursor, signature)`；
 *   2. 已加载哪些页的**游标列表**存在 Zustand（`graphPagingStore`）；
 *   3. 用 `useQueries` + `combine` 把各页合并成一个 `GraphModel`（视图模型）。
 *
 * # 为什么合并放在 `combine` 里，而不是 `useMemo`
 *
 * `useQueries` 返回的结果数组**每次渲染都是新引用**，用它做 `useMemo` 的依赖
 * 等于没有 memo；用"数据时间戳拼成的字符串"当依赖能绕过这一点，但会被
 * `react-hooks/preserve-manual-memoization` 判为依赖不匹配（本仓库开了这条 error 规则）。
 * TanStack 的 `combine` 是**结构化共享**的：只有当某个查询结果真的变了，
 * 它才重新执行并返回新对象。于是合并的开销（十万行的排序与建索引）
 * 只在数据变化时付一次，且写法上没有任何取巧。
 * 前提是 `combine` 必须是稳定引用——所以 `combinePages` 定义在模块顶层。
 *
 * # 首屏预期
 *
 * 首屏只请求一页（`DEFAULT_PAGE_SIZE` = 200 行），后端也只对这一页做布局。
 * Rust 侧全量布局的基准数字（5000 节点 ~1ms、50000 ~8.4s、100000 ~34.2s，
 * 泳道数很多时超线性）**与首屏无关**，不要拿它当首屏预算；面板上的
 * "布局耗时"量的是这一页的 IPC 往返（见 `graphPerfStore.ts` 的说明）。
 */
import { useCallback } from 'react';

import { keepPreviousData, useQueries, useQueryClient } from '@tanstack/react-query';
import type { QueryObserverResult } from '@tanstack/react-query';

import { gitLogPage } from '@/lib/ipc/history';
import type { Commit, GraphEdge, GraphRow, HistoryPage, HistoryQuery } from '@/lib/ipc/history';
import { logKey, logKeyPrefix } from '@/lib/queryKeys';

import { nodeLabelFor } from '@/features/history/commitMeta';
import type { NodeLabel } from '@/features/history/commitMeta';
import {
  FIRST_CURSOR,
  FIRST_PAGE_CURSORS,
  useGraphPagingStore,
} from '@/features/history/graphPagingStore';
import { isGraphPerfEnabled, recordGraphPerf } from '@/features/history/graphPerfStore';

/** 默认每页行数。 */
export const DEFAULT_PAGE_SIZE = 200;

/**
 * 每页行数上限（与后端 `MAX_HISTORY_PAGE_SIZE` 一致）。
 *
 * 前端自己也要夹一次：后端会静默钳制，若前端以为"我请求了 5000 条"，
 * 累积的行号与游标就会和后端给的对不上（表现为翻页时跳过一段提交）。
 */
export const MAX_PAGE_SIZE = 500;

/**
 * 一页历史的缓存新鲜期（任务约定 5 秒）。
 *
 * 为什么需要它：滚动、缩放、切换视图模式都会触发重渲染，若没有 staleTime，
 * 每次窗口聚焦都可能重取当前挂载的页。历史数据只在 `repo:changed`
 * （refs 变化）时才真的需要更新，而那条路径由 `repoChanged.ts` 主动失效。
 */
export const HISTORY_STALE_TIME_MS = 5_000;

/** 筛选条件（`HistoryQuery` 去掉分页两兄弟：它们由本模块管理）。 */
export type HistoryFilters = Omit<HistoryQuery, 'pageSize' | 'cursor'>;

/** 空筛选（模块级常量，保证默认参数的引用稳定）。 */
export const EMPTY_FILTERS: HistoryFilters = {};

/** 参与签名的标量字段（`paths` 单独处理，见 `filtersSignature`）。 */
const SIGNATURE_FIELDS = [
  'revision',
  'allBranches',
  'author',
  'since',
  'until',
  'messageContains',
  'firstParentOnly',
  'followRenames',
  'collapseMergedBranches',
] as const satisfies readonly (keyof HistoryFilters)[];

/** 把每页行数夹到 `[1, MAX_PAGE_SIZE]`。 */
export function clampPageSize(pageSize: number): number {
  if (!Number.isFinite(pageSize)) {
    return DEFAULT_PAGE_SIZE;
  }
  return Math.min(MAX_PAGE_SIZE, Math.max(1, Math.floor(pageSize)));
}

/**
 * 算出筛选条件的**稳定签名**。
 *
 * # 为什么要签名而不是直接把 filters 对象放进键
 *
 * Query 的键按结构化相等比较，但调用方通常每次渲染都新建一个 filters 字面量；
 * 对象里字段顺序不同、或 `paths` 数组顺序不同，都会被当成两个不同的键，
 * 结果是同一份数据被请求两次、失效时又只命中其中一份。
 * 压成一个字符串后，键的形状是 `[LOG_QUERY_KEY, repoId, cursor, string]`，
 * 完全由值决定，与调用方怎么构造对象无关。
 *
 * `paths` 排序后再拼：`git log -- a b` 与 `-- b a` 是同一个查询。
 * 分隔符用 `\u0000`（路径里不可能出现，因此不会产生歧义拼接）。
 */
export function filtersSignature(filters: HistoryFilters, pageSize: number): string {
  const parts: string[] = [`pageSize=${pageSize}`];
  for (const field of SIGNATURE_FIELDS) {
    const value = filters[field];
    if (value !== undefined) {
      parts.push(`${field}=${JSON.stringify(value)}`);
    }
  }
  const paths = filters.paths ?? [];
  parts.push(`paths=${[...paths].sort().join('\u0000')}`);
  return parts.join('&');
}

// ---------------------------------------------------------------- 视图模型

/**
 * 累积后的视图模型：Canvas、文本列、命中检测、详情面板都吃这一个对象。
 *
 * 行号是**全局**的（后端已把页内行号平移过），因此跨页拼接不需要任何偏移，
 * 滚动位置、迷你地图与 Shift 区间选都能直接按行号工作。
 */
export interface GraphModel {
  /** 提交（按行序）。 */
  readonly commits: readonly Commit[];
  /** 布局行（按全局行号升序）。 */
  readonly rows: readonly GraphRow[];
  /** 边（已去重）。 */
  readonly edges: readonly GraphEdge[];
  /** oid → 行。 */
  readonly index: ReadonlyMap<string, GraphRow>;
  /** oid → 提交。 */
  readonly commitByOid: ReadonlyMap<string, Commit>;
  /** oid → 节点文案（首字母 / ref / 折叠数）。 */
  readonly labels: ReadonlyMap<string, NodeLabel>;
  /** 各页 `laneCount` 的最大值（决定图列宽度；每页独立布局，因此必须取最大）。 */
  readonly laneCount: number;
  /** 内容总行数 = `lastRow + 1`（行号全局连续，不删除隐藏行）。 */
  readonly rowCount: number;
  readonly lastRow: number;
  /** 最后一个已加载页给出的下一页游标；`null` 表示已到末页。 */
  readonly nextCursor: number | null;
  /** 已加载的页数。 */
  readonly loadedPages: number;
}

/** 空模型（没有数据时的返回值；模块级常量，避免每次渲染新建）。 */
export const EMPTY_GRAPH_MODEL: GraphModel = {
  commits: [],
  rows: [],
  edges: [],
  index: new Map(),
  commitByOid: new Map(),
  labels: new Map(),
  laneCount: 0,
  rowCount: 0,
  lastRow: -1,
  nextCursor: null,
  loadedPages: 0,
};

/**
 * 把若干页合并成一个视图模型。
 *
 * 去重规则：同一个 oid 以**先到的那一页**为准。分页基于 `skip`，
 * 两次请求之间若有新提交落入，窗口会整体后移，于是相邻页可能出现重叠的 oid；
 * 不去重的表现是同一行被画两次（颜色更深）且行号冲突。
 *
 * 边同样按 `fromOid → toOid` 去重：重叠的边会让一条线被描两遍，
 * 在低缩放下看起来像"这条分支特别粗"，那是纯粹的渲染假象。
 */
export function buildGraphModel(pages: readonly (HistoryPage | null | undefined)[]): GraphModel {
  const commits: Commit[] = [];
  const rows: GraphRow[] = [];
  const edges: GraphEdge[] = [];
  const index = new Map<string, GraphRow>();
  const commitByOid = new Map<string, Commit>();
  const labels = new Map<string, NodeLabel>();
  const edgeKeys = new Set<string>();
  let laneCount = 0;
  let lastRow = -1;
  let nextCursor: number | null = null;
  let loadedPages = 0;

  for (const page of pages) {
    if (page === null || page === undefined) {
      continue;
    }
    loadedPages += 1;
    for (const commit of page.commits) {
      if (!commitByOid.has(commit.oid)) {
        commitByOid.set(commit.oid, commit);
        commits.push(commit);
      }
    }
    for (const row of page.layout.rows) {
      if (index.has(row.oid)) {
        continue;
      }
      index.set(row.oid, row);
      rows.push(row);
      labels.set(row.oid, nodeLabelFor(commitByOid.get(row.oid), row));
      if (row.row > lastRow) {
        lastRow = row.row;
      }
    }
    for (const edge of page.layout.edges) {
      const key = `${edge.fromOid}\u0000${edge.toOid}`;
      if (edgeKeys.has(key)) {
        continue;
      }
      edgeKeys.add(key);
      edges.push(edge);
    }
    laneCount = Math.max(laneCount, page.layout.laneCount);
    // 只有最后一页的 nextCursor 有意义（前面各页的游标都已经被请求过了）
    nextCursor = page.nextCursor;
  }

  if (loadedPages === 0) {
    return EMPTY_GRAPH_MODEL;
  }
  // 正常情况下各页的行号已经升序，这次排序是空转；但重叠页去重之后
  // 顺序可能被打破，而渲染层与命中网格都假设"rows 按行号升序"。
  rows.sort((left, right) => left.row - right.row);
  return {
    commits,
    rows,
    edges,
    index,
    commitByOid,
    labels,
    laneCount,
    rowCount: lastRow + 1,
    lastRow,
    nextCursor,
    loadedPages,
  };
}

// ---------------------------------------------------------------- IPC 与采样

/** 取一页；开着性能面板时顺带量一次 IPC 往返（这就是面板上的"布局耗时"）。 */
async function fetchHistoryPage(
  repoId: number,
  filters: HistoryFilters,
  pageSize: number,
  cursor: number,
): Promise<HistoryPage> {
  const query: HistoryQuery = { ...filters, pageSize, cursor };
  if (!isGraphPerfEnabled()) {
    return gitLogPage(repoId, query);
  }
  const started = performance.now();
  const page = await gitLogPage(repoId, query);
  recordGraphPerf({ layoutMs: performance.now() - started });
  return page;
}

/** `useQueries` 的合并结果。 */
export interface CombinedHistory {
  readonly model: GraphModel;
  /** 首页还没到（此时才该显示骨架屏）。 */
  readonly isPending: boolean;
  readonly isFetching: boolean;
  /** 正在加载**续页**（首页已在屏幕上，只该显示一个底部指示器）。 */
  readonly isFetchingNextPage: boolean;
  readonly isError: boolean;
  readonly error: unknown;
}

/**
 * 合并各页结果。
 *
 * 必须是模块级的稳定引用：内联箭头函数每次渲染都换引用，
 * `useQueries` 就会每次渲染都重跑合并（十万行的排序 + 建索引）。
 *
 * `isPending` 只看首页：续页加载时视图里已经有内容，若也报 pending，
 * 界面会闪回骨架屏，用户会以为刚才那些提交丢了。
 */
function combinePages(
  results: readonly QueryObserverResult<HistoryPage, Error>[],
): CombinedHistory {
  const model = buildGraphModel(results.map((result) => result.data));
  const first = results[0];
  const last = results[results.length - 1];
  const failed = results.find((result) => result.isError);
  return {
    model,
    isPending: first?.isPending ?? false,
    isFetching: results.some((result) => result.isFetching),
    isFetchingNextPage: results.length > 1 && (last?.isFetching ?? false),
    isError: failed !== undefined,
    error: failed?.error ?? null,
  };
}

// ---------------------------------------------------------------- hook

/** `useGraphQuery` 的选项。 */
export interface UseGraphQueryOptions {
  readonly pageSize?: number;
  /** 额外的开关（与 `Number.isFinite(repoId)` 取与）。 */
  readonly enabled?: boolean;
}

/** `useGraphQuery` 的返回值。 */
export interface GraphQueryResult extends CombinedHistory {
  /** 当前筛选的签名（也是 `graphPagingStore` 的分桶键）。 */
  readonly signature: string;
  readonly pageSize: number;
  /** 已请求的游标列表（升序）。 */
  readonly cursors: readonly number[];
  /** 还有下一页吗（以最后一个已加载页的 `nextCursor` 为准）。 */
  readonly hasNextPage: boolean;
  /**
   * 请求下一页。
   *
   * @returns 是否真的排上了新页（已经在加载中、或已到末页时返回 `false`）。
   *   滚动哨兵据此决定要不要继续重试，避免一次抖动排出十几个请求。
   */
  loadMore(): boolean;
  /** 主动失效本仓库的全部历史页（工具栏的刷新按钮）。 */
  refresh(): void;
}

/**
 * 拉取并累积提交历史。
 *
 * @param repoId 仓库记录 id；非有限数时查询被禁用（页面应退回占位态）
 */
export function useGraphQuery(
  repoId: number,
  filters: HistoryFilters = EMPTY_FILTERS,
  options: UseGraphQueryOptions = {},
): GraphQueryResult {
  const pageSize = clampPageSize(options.pageSize ?? DEFAULT_PAGE_SIZE);
  const signature = filtersSignature(filters, pageSize);
  const enabled = (options.enabled ?? true) && Number.isFinite(repoId);

  // 选择器返回的必须是稳定引用：这里要么是 store 里那个数组，要么是模块级常量。
  // 若写成 `?? [FIRST_CURSOR]`，每次渲染都会新建数组 → Zustand 认为状态变了 → 无限重渲染。
  const cursors = useGraphPagingStore(
    (state) => state.cursorsBySignature[signature] ?? FIRST_PAGE_CURSORS,
  );

  const combined = useQueries({
    queries: cursors.map((cursor) => ({
      // 键的形状由 `@/lib/queryKeys` 统一提供：`repoChanged.ts` 按
      // `[LOG_QUERY_KEY, repoId]` 前缀失效所有页，键在这里写错就会静默不刷新。
      queryKey: logKey(repoId, cursor, signature),
      queryFn: () => fetchHistoryPage(repoId, filters, pageSize, cursor),
      enabled,
      staleTime: HISTORY_STALE_TIME_MS,
      // 续页的键是全新的，keepPreviousData 在这里保住的是"仓库切换 / 筛选变更"
      // 那一瞬间的旧数据：新页到达前继续显示上一页，而不是一片空白。
      placeholderData: keepPreviousData,
    })),
    combine: combinePages,
  });

  const queryClient = useQueryClient();

  const loadMore = useCallback((): boolean => {
    const store = useGraphPagingStore.getState();
    const loaded = store.cursorsFor(signature);
    const lastCursor = loaded[loaded.length - 1] ?? FIRST_CURSOR;
    // 直接问缓存要 nextCursor，而不是在渲染期把它抄进 ref：
    // 后者会被 `react-hooks/refs` 判为渲染期读写 ref（本仓库开了这条 error 规则）。
    const page = queryClient.getQueryData<HistoryPage>(logKey(repoId, lastCursor, signature));
    const next = page?.nextCursor ?? null;
    if (next === null) {
      return false;
    }
    return store.appendCursor(signature, next);
  }, [queryClient, repoId, signature]);

  const refresh = useCallback((): void => {
    void queryClient.invalidateQueries({ queryKey: logKeyPrefix(repoId) });
  }, [queryClient, repoId]);

  return {
    model: combined.model,
    isPending: combined.isPending,
    isFetching: combined.isFetching,
    isFetchingNextPage: combined.isFetchingNextPage,
    isError: combined.isError,
    error: combined.error,
    signature,
    pageSize,
    cursors,
    hasNextPage: combined.model.nextCursor !== null,
    loadMore,
    refresh,
  };
}
