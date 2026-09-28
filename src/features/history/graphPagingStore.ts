/**
 * 分页游标状态（Zustand，T2.2）。
 *
 * # 为什么"游标列表"是 UI 状态
 *
 * 后端只告诉我们"下一页从第 N 行开始"（`HistoryPage.nextCursor`），
 * 但**已经加载了哪几页**是前端自己的事：用户滚到底部才加载下一页，
 * 这是视图行为，后端既不知道也不该知道。于是它归 Zustand。
 *
 * 数据本身（提交与布局）仍然只在 Query 缓存里，一页一条记录、键是
 * `logKey(repoId, cursor, signature)`。这里只存"我请求过哪些 cursor"，
 * 相当于一个目录：目录 + 缓存 = 累积视图。
 *
 * # 为什么不用 `useInfiniteQuery`
 *
 * `useInfiniteQuery` 确实能自动累积，但它把**所有页塞进同一条缓存记录**：
 *  1. 键的形状只能是 `[key, repoId, filters]`，无法表达"第 3 页"。而
 *     `@/lib/queryKeys` 的 `logKey(repoId, cursor, filters)` 是仓库既定约定，
 *     `repoChanged.ts` 的前缀失效也依赖它；
 *  2. 一次 `repo:changed` 会让整条无限查询重新串行拉取**已累积的全部页**
 *     （用户翻了 50 页就是 50 次 IPC），而按页分键时 TanStack 只重取当前
 *     挂载着的那些页，且各页互相独立、可以并发；
 *  3. 无法丢弃远处的页（十万行的仓库里翻到很深时，前面的页应该可以被淘汰）。
 *
 * # 为什么按"筛选签名"分桶
 *
 * 改一次筛选条件就是一份全新的历史：老的页码在新结果集里没有意义
 * （第 200 行在另一组筛选下是完全不同的提交）。按签名分桶后，
 * 切回上一组筛选还能看到当时累积到的深度（缓存没被淘汰就不用重拉），
 * 而不会把两套结果的页混在一起——混起来的表现是"图里凭空多出一段提交"。
 */
import { create } from 'zustand';

/** 首页游标（后端 `cursor` 缺省即 0，这里显式写出来便于阅读）。 */
export const FIRST_CURSOR = 0;

/** 未登记过的签名默认只有首页（模块级常量，保证 Zustand 选择器的引用稳定）。 */
export const FIRST_PAGE_CURSORS: readonly number[] = [FIRST_CURSOR];

export interface GraphPagingState {
  /** 筛选签名 → 已加载页的游标列表（升序、去重）。 */
  readonly cursorsBySignature: Readonly<Record<string, readonly number[]>>;
  /** 某个签名下的游标列表（没登记过就是 `[0]`）。 */
  cursorsFor(signature: string): readonly number[];
  /**
   * 追加一页。
   *
   * @returns 是否真的追加了（重复的、或比当前最后一页更靠前的游标会被拒绝）。
   *   返回布尔值而不是 void：调用方（滚动哨兵）据此决定要不要继续尝试，
   *   否则一次抖动就会排上十几个重复请求。
   */
  appendCursor(signature: string, cursor: number): boolean;
  /** 丢弃某个签名的累积进度（筛选被"重置"时用）。 */
  resetSignature(signature: string): void;
  /** 丢弃全部（切换仓库、测试复位）。 */
  resetAll(): void;
}

/** 初始状态（导出供测试复位）。 */
export const initialGraphPagingState = {
  cursorsBySignature: {} as Readonly<Record<string, readonly number[]>>,
};

export const useGraphPagingStore = create<GraphPagingState>()((set, get) => ({
  ...initialGraphPagingState,

  cursorsFor: (signature) => get().cursorsBySignature[signature] ?? FIRST_PAGE_CURSORS,

  appendCursor: (signature, cursor) => {
    if (!Number.isFinite(cursor) || cursor < 0) {
      return false;
    }
    const current = get().cursorsFor(signature);
    const last = current[current.length - 1] ?? FIRST_CURSOR;
    // 只接受"比当前最后一页更靠后"的游标：
    // 数据在翻页之间发生变化时，后端给出的 nextCursor 可能落回已加载的区间，
    // 那时追加会让两页覆盖同一批全局行号（图上表现为提交重复出现）。
    if (cursor <= last || current.includes(cursor)) {
      return false;
    }
    set({
      cursorsBySignature: { ...get().cursorsBySignature, [signature]: [...current, cursor] },
    });
    return true;
  },

  resetSignature: (signature) => {
    const next = { ...get().cursorsBySignature };
    delete next[signature];
    set({ cursorsBySignature: next });
  },

  resetAll: () => {
    set({ cursorsBySignature: {} });
  },
}));
