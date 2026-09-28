/**
 * 提交图的 UI 状态（Zustand，T2.2）。
 *
 * # 为什么这些状态归 Zustand，而不是 TanStack Query
 *
 * 判据是"**谁是真相源**"：提交数据、泳道布局、下一页游标的真相源在后端，
 * 归 Query（见 `useGraphQuery.ts`）；而"选中了哪几个提交""缩放多少""是图还是列表"
 * 的真相源在用户手上，后端既不知道也不该知道，归 Zustand。
 * 把选中集塞进 Query 缓存会有两个具体后果：一是它没有 queryFn，只能靠
 * `setQueryData` 硬写，等于把缓存当成全局变量用；二是 `repo:changed` 的
 * 前缀失效会把用户的选中状态一起清掉（那是数据失效，不该动视图）。
 *
 * # 为什么是模块级单例而不是页面内 useState
 *
 * 图列（Canvas）、文本列（列表行）、右键菜单、详情面板、迷你地图是**五个不同的
 * 组件子树**，它们要读同一份选中集。用 props 逐层传递会把 `HistoryPage`
 * 变成一个纯粹的中转站；而 Context 在这里没有额外收益（没有"同一页面两份不同
 * 选中状态"的场景）。与仓库既有的 `uiStore` / `logViewerStore` 同一取舍。
 *
 * # 关于 `selectedOids` 用数组而不是 Set
 *
 * Zustand 的浅比较与 React 的重渲染都依赖**引用稳定性**：动作里每次
 * `new Set(...)` 与 `[...]` 都会换引用，这一点两者相同；但数组额外带来两个好处：
 *  1. 顺序即行序，"第一个选中项"是确定的（详情面板与"设为比较基准"要用）；
 *  2. 可以直接被 `useMemo(() => new Set(oids), [oids])` 转成查询用的集合，
 *     而 Set 无法结构化比较，测试里断言也不如数组直观。
 */
import { useMemo } from 'react';

import { create } from 'zustand';

import { clampScale } from '@/features/history/graphGeometry';

/** 视图模式：图（Canvas）或列表（可访问的网格）。 */
export type GraphViewMode = 'graph' | 'list';

/** 一次点击带上的修饰键（由组件从事件里翻译过来，store 不认识 DOM 事件）。 */
export interface SelectionModifiers {
  /** Ctrl / Cmd：加选或取消选择。 */
  readonly additive: boolean;
  /** Shift：从锚点到目标的区间选。 */
  readonly range: boolean;
}

/** 无修饰键（导出供组件复用，避免每次点击都新建一个对象）。 */
export const NO_MODIFIERS: SelectionModifiers = { additive: false, range: false };

/** 缩放步进（滚轮一格 = 1.2 倍；太小感觉不到，太大则会跳过用户想要的档位）。 */
export const ZOOM_STEP = 1.2;

/** 迷你地图默认关闭：它在小屏上占地方，而多数浏览场景用不到。 */
const DEFAULT_MINIMAP_OPEN = false;

export interface GraphSelectionState {
  /** 选中的提交 oid，按行序排列（可能为空）。 */
  readonly selectedOids: readonly string[];
  /** Shift 区间选的锚点（最后一次单击/Ctrl 点击的提交）。 */
  readonly anchorOid: string | null;
  /** 比较基准（右键菜单"设为比较基准"；T2.4 的差异视图会消费它）。 */
  readonly compareBaseOid: string | null;
  /** 指针悬停的提交（`null` 表示不在任何节点上）。 */
  readonly hoverOid: string | null;
  /** 详情面板显示的提交（`null` 表示面板关闭）。 */
  readonly detailOid: string | null;
  /**
   * 详情面板是否钉住（T2.4 的"钉住 / 跟随选中"两模式）。
   *
   * 钉住后 `select` 仍正常更新选中集，但**不再改写 `detailOid`**——面板冻结在
   * 当前提交上，用户可以放心地点别处对照。关闭钉住时面板立即回到跟随模式
   * （显示当前选中的第一个提交）。
   */
  readonly detailPinned: boolean;
  readonly viewMode: GraphViewMode;
  readonly scale: number;
  readonly minimapOpen: boolean;

  /**
   * 选中一个提交。
   *
   * @param order 当前**行序**的 oid 列表（区间选与排序都靠它；
   *   传空数组表示"顺序未知"，此时区间选退化成单选）
   */
  select(oid: string, modifiers: SelectionModifiers, order: readonly string[]): void;
  /** 选中一批（Ctrl+A 全选；同样按 `order` 归一化顺序）。 */
  selectMany(oids: readonly string[], order: readonly string[]): void;
  clearSelection(): void;
  setHoverOid(oid: string | null): void;
  setCompareBase(oid: string | null): void;
  setDetailOid(oid: string | null): void;
  /** 切换钉住模式（钉住后 select 不再改写 detailOid）。 */
  setDetailPinned(pinned: boolean): void;
  setViewMode(mode: GraphViewMode): void;
  /** 设定缩放（自动夹到 0.5x–3x）。 */
  setScale(scale: number): void;
  /** 相对缩放（滚轮用；同样夹取）。 */
  zoomBy(factor: number): void;
  toggleMinimap(): void;
  /** 复位视图（缩放回 1、列表回到图模式、迷你地图关掉；不动选中集）。 */
  resetView(): void;
}

/** 初始状态（导出供测试复位；store 是模块级单例）。 */
export const initialGraphSelectionState = {
  selectedOids: [] as readonly string[],
  anchorOid: null as string | null,
  compareBaseOid: null as string | null,
  hoverOid: null as string | null,
  detailOid: null as string | null,
  detailPinned: false,
  viewMode: 'graph' as GraphViewMode,
  scale: 1,
  minimapOpen: DEFAULT_MINIMAP_OPEN,
};

/**
 * 按行序归一化一组 oid。
 *
 * 为什么必须归一化：Ctrl 多选时用户可能从下往上点，`selectedOids` 的顺序就会是
 * 点击顺序。而"详情面板显示第一个选中项""区间选的起点"都要求一个**与点击顺序无关**
 * 的确定顺序，否则同样的选择会因为点击路径不同而表现出不同结果（测试也会随机失败）。
 *
 * `order` 里没有的 oid 排在最后（数据刚刷新、选择还没被清掉的瞬时情况），
 * 保持稳定顺序而不是直接丢弃——丢弃会让用户看到选中项凭空消失。
 */
export function orderByRow(oids: readonly string[], order: readonly string[]): readonly string[] {
  if (order.length === 0) {
    return [...oids];
  }
  const rank = new Map<string, number>();
  order.forEach((oid, index) => {
    if (!rank.has(oid)) {
      rank.set(oid, index);
    }
  });
  return [...oids].sort((left, right) => {
    const leftRank = rank.get(left) ?? Number.MAX_SAFE_INTEGER;
    const rightRank = rank.get(right) ?? Number.MAX_SAFE_INTEGER;
    return leftRank - rightRank;
  });
}

/**
 * 锚点到目标之间的 oid 区间（含两端），按行序返回。
 *
 * 任一端不在 `order` 里时返回 `null`：这发生在筛选条件变了、或那一页已经被
 * 丢弃之后。此时调用方应退化成单选——**宁可少选，也不能选错一片**
 * （用一个过期锚点算出的区间会选中一批用户从没打算选的提交）。
 */
export function oidRange(
  anchorOid: string,
  targetOid: string,
  order: readonly string[],
): readonly string[] | null {
  const from = order.indexOf(anchorOid);
  const to = order.indexOf(targetOid);
  if (from < 0 || to < 0) {
    return null;
  }
  return order.slice(Math.min(from, to), Math.max(from, to) + 1);
}

export const useGraphSelectionStore = create<GraphSelectionState>()((set, get) => ({
  ...initialGraphSelectionState,

  select: (oid, modifiers, order) => {
    const state = get();
    // 钉住时面板冻结在当前提交上：选中照常更新，详情不动（见 detailPinned 的说明）
    const nextDetail = state.detailPinned ? state.detailOid : oid;
    if (modifiers.range && state.anchorOid !== null) {
      const range = oidRange(state.anchorOid, oid, order);
      if (range !== null) {
        // 区间选**替换**而不是追加：与文件管理器、GitHub Desktop 的 Shift 语义一致。
        // 锚点保持不变，于是"按住 Shift 上下移动"能连续调整区间末端。
        set({ selectedOids: range, detailOid: nextDetail });
        return;
      }
    }
    if (modifiers.additive) {
      const selected = state.selectedOids.includes(oid)
        ? state.selectedOids.filter((existing) => existing !== oid)
        : [...state.selectedOids, oid];
      set({ selectedOids: orderByRow(selected, order), anchorOid: oid, detailOid: nextDetail });
      return;
    }
    set({ selectedOids: [oid], anchorOid: oid, detailOid: nextDetail });
  },

  selectMany: (oids, order) => {
    set({ selectedOids: orderByRow(oids, order) });
  },

  clearSelection: () => {
    set({ selectedOids: [], anchorOid: null });
  },

  setHoverOid: (oid) => {
    // 悬停是每帧都可能变化的高频状态：值没变就不要触发订阅者重渲染。
    // 画布的重绘走的是命令式路径（不依赖 React 渲染），这里的短路只是为了
    // 不让 hover 卡片与文本列跟着抖动。
    if (get().hoverOid === oid) {
      return;
    }
    set({ hoverOid: oid });
  },

  setCompareBase: (oid) => {
    set({ compareBaseOid: oid });
  },

  setDetailOid: (oid) => {
    set({ detailOid: oid });
  },

  setDetailPinned: (pinned) => {
    set({ detailPinned: pinned });
  },

  setViewMode: (mode) => {
    set({ viewMode: mode });
  },

  setScale: (scale) => {
    set({ scale: clampScale(scale) });
  },

  zoomBy: (factor) => {
    set({ scale: clampScale(get().scale * factor) });
  },

  toggleMinimap: () => {
    set({ minimapOpen: !get().minimapOpen });
  },

  resetView: () => {
    set({ scale: 1, viewMode: 'graph', minimapOpen: DEFAULT_MINIMAP_OPEN });
  },
}));

/**
 * 选中集的 Set 视图（渲染层与命中检测要按 oid 做 O(1) 查询）。
 *
 * 单独一个 hook 而不是让每个组件各写一遍 `useMemo`：五个子树都需要同一个集合，
 * 各写一遍就会有五份 Set（内存与 GC 都白烧），而且漏写 memo 的那份会每次渲染
 * 都新建，让下游的 `useEffect` 依赖失效、无谓地重画 Canvas。
 */
export function useSelectedOidSet(): ReadonlySet<string> {
  const selectedOids = useGraphSelectionStore((state) => state.selectedOids);
  return useMemo(() => new Set(selectedOids), [selectedOids]);
}
