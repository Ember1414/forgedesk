/**
 * 命中检测：规则网格索引（T2.2）。
 *
 * # 为什么是网格而不是 R-tree
 *
 * 提交图的节点分布有很强的结构性：**行高固定、泳道宽固定**，节点中心永远落在
 * `(padLeft + lane*laneWidth + laneWidth/2, row*rowHeight + rowHeight/2)` 的格点上。
 * 在这种分布上，R-tree 的自适应包围盒没有任何优势——它的构建是 O(n log n)、
 * 有额外的对象分配，而规则网格是 O(n) 一次遍历、每格一个数组。
 * 十万节点下两者的查询都是 O(1) 量级，但网格的常数小一个数量级，
 * 而且**没有第三方依赖**（本任务约定不新增 npm 依赖）。
 *
 * # 为什么还需要它（直接算不就行了？）
 *
 * 单看"指针在哪一行哪一泳道"确实可以直接除出来（`rowAtY` / `laneAtX`）。
 * 但命中检测要回答的是"指针是否落在**某个节点的胶囊上**"，而胶囊是圆角的，
 * 且一个行里可能同时存在节点、ref 胶囊、折叠徽标三种可点目标。
 * 网格把"这一格附近有哪些候选"提前算好，指针移动时只需检查 1~4 个候选，
 * 而不是遍历可见的几千个节点。
 *
 * # 纯函数
 *
 * 全部函数不碰 DOM：`buildHitGrid` 吃行数据 + 度量，`hitTest*` 吃坐标。
 * 因此缩放/平移后的换算可以在单测里直接断言（jsdom 里没有 canvas）。
 */
import type { GraphRow } from '@/lib/ipc/history';

import {
  laneAtX,
  nodeRadius,
  nodeRect,
  pointInRoundedRect,
  rowAtY,
} from '@/features/history/graphGeometry';
import type { GraphMetrics, Point, Rect } from '@/features/history/graphGeometry';

/** 一个可命中的目标。 */
export interface HitTarget {
  /** 全局行号。 */
  readonly row: number;
  /** 泳道（空白处命中时为 -1）。 */
  readonly lane: number;
  /** 提交 oid（空白处命中时为 `null`）。 */
  readonly oid: string | null;
  /** 命中的是哪一类目标。 */
  readonly kind: 'node' | 'row';
}

/** 网格里的一条候选记录。 */
export interface HitEntry {
  readonly oid: string;
  readonly row: number;
  readonly lane: number;
  readonly rect: Rect;
  readonly radius: number;
}

/**
 * 规则网格索引。
 *
 * 键 = `row * columns + column`（行优先）。用一维整数键而不是 `Map<string, …>`：
 * 字符串键每次查询都要拼接与哈希，而指针移动是每帧都可能发生的高频操作。
 *
 * 为什么是**稀疏** Map 而不是稠密数组：稠密数组的长度是
 * `laneCount × rowCount`，而每个空数组在 V8 里约几十字节——十万行、六泳道
 * 就是六十万个格子（几十 MB），而其中真正有节点的格子最多只有行数的四倍。
 * 稀疏 Map 的内存与**已加载的节点数**成正比，与总行数无关。
 */
export interface HitGrid {
  readonly cellWidth: number;
  readonly cellHeight: number;
  readonly columns: number;
  readonly rows: number;
  readonly cells: ReadonlyMap<number, readonly HitEntry[]>;
}

/**
 * 建网格。
 *
 * @param laneCount 泳道数（决定列数；至少 1，避免除零）
 * @param rowCount 总行数（决定行数）
 *
 * 单元格尺寸刻意取"一个泳道宽 × 一行高"：节点直径必然小于等于它，
 * 因此一个节点最多跨越 2×2 = 4 个格子（当它正好压在格线上时）。
 * 格子再大就会让候选变多；再小则跨格数上升，两边都不划算。
 */
export function buildHitGrid(
  rows: readonly GraphRow[],
  metrics: GraphMetrics,
  laneCount: number,
  rowCount: number,
): HitGrid {
  const cellWidth = Math.max(1, metrics.laneWidth);
  const cellHeight = Math.max(1, metrics.rowHeight);
  const columns = Math.max(1, Math.ceil(laneCount));
  const rowLines = Math.max(1, Math.ceil(rowCount));
  const cells = new Map<number, HitEntry[]>();

  for (const row of rows) {
    const rect = nodeRect(metrics, row);
    const radius = nodeRadius(rect);
    const entry: HitEntry = { oid: row.oid, row: row.row, lane: row.lane, rect, radius };
    // 把节点登记进它覆盖到的所有格子（最多 4 个）
    const firstColumn = cellIndexOf(rect.x, cellWidth, columns);
    const lastColumn = cellIndexOf(rect.x + rect.w, cellWidth, columns);
    const firstLine = cellIndexOf(rect.y, cellHeight, rowLines);
    const lastLine = cellIndexOf(rect.y + rect.h, cellHeight, rowLines);
    for (let line = firstLine; line <= lastLine; line += 1) {
      for (let column = firstColumn; column <= lastColumn; column += 1) {
        const key = line * columns + column;
        const bucket = cells.get(key);
        if (bucket === undefined) {
          cells.set(key, [entry]);
        } else {
          bucket.push(entry);
        }
      }
    }
  }

  return { cellWidth, cellHeight, columns, rows: rowLines, cells };
}

/** 坐标 → 格子下标（夹到合法范围，避免越界写入）。 */
function cellIndexOf(value: number, cellSize: number, limit: number): number {
  const index = Math.floor(value / cellSize);
  return Math.min(limit - 1, Math.max(0, index));
}

/** 空格子的共享常量（避免每次未命中都分配一个新数组）。 */
const EMPTY_CELL: readonly HitEntry[] = [];

/**
 * 在网格上找指针命中的节点。
 *
 * @param point **内容坐标**（已经过 `pointerToContent` 换算，不是客户端坐标）
 * @returns 命中的节点；没有则 `null`
 */
export function hitTestNode(grid: HitGrid, point: Point): HitTarget | null {
  const column = Math.floor(point.x / grid.cellWidth);
  const line = Math.floor(point.y / grid.cellHeight);
  if (column < 0 || column >= grid.columns || line < 0 || line >= grid.rows) {
    return null;
  }
  const candidates = grid.cells.get(line * grid.columns + column) ?? EMPTY_CELL;
  let best: HitEntry | null = null;
  for (const candidate of candidates) {
    if (!pointInRoundedRect(point, candidate.rect, candidate.radius)) {
      continue;
    }
    // 多个候选重叠时取行号大的（更靠下 = 画在后面 = 视觉上在最上层）
    if (best === null || candidate.row > best.row) {
      best = candidate;
    }
  }
  if (best === null) {
    return null;
  }
  return { row: best.row, lane: best.lane, oid: best.oid, kind: 'node' };
}

/**
 * 指针所在的**行**（即使没点在节点上也返回）。
 *
 * 为什么需要它：单击一行空白处也应该选中那个提交（这是提交列表的通用预期，
 * 只有点在节点上才响应会让人以为"点不中"）；而拖动空白处要平移，
 * 两者的区分靠"是否在图列内 + 是否按住了拖动阈值"，不靠命中节点。
 *
 * @returns 行号超出已加载范围时返回 `null`
 */
export function hitTestRow(
  metrics: GraphMetrics,
  point: Point,
  rowCount: number,
): HitTarget | null {
  const row = rowAtY(metrics, point.y);
  if (row < 0 || row >= rowCount) {
    return null;
  }
  return { row, lane: laneAtX(metrics, point.x), oid: null, kind: 'row' };
}

/**
 * 客户端坐标 → 内容坐标。
 *
 * @param scrollY 滚动容器的 `scrollTop`（CSS px）
 * @param rect 画布元素的 `getBoundingClientRect()`
 * @param clientX/clientY 指针的客户端坐标
 *
 * 注意这里**不乘 scale**：`graphMetrics` 已经把 scale 乘进了行高与泳道宽，
 * 内容坐标本身就是"缩放后的坐标"，再乘一次就变成平方缩放。
 * 这条口径与 `graphGeometry` 完全一致——两个模块共用同一套换算是刻意的，
 * 否则命中框与画出来的胶囊会慢慢错开（而错位量随缩放比例变化，极难复现）。
 */
export function pointerToContent(
  scrollY: number,
  rect: { readonly left: number; readonly top: number },
  clientX: number,
  clientY: number,
): Point {
  return { x: clientX - rect.left, y: clientY - rect.top + scrollY };
}

/**
 * 完整命中检测：客户端坐标 → 目标。
 *
 * 优先返回节点（精确目标），否则退化到行（宽松目标）。
 * 调用方据 `kind` 决定语义：`node` 用于 tooltip 与右键菜单，
 * `row` 用于单击选中与拖动平移的判定。
 */
export function hitTest(
  grid: HitGrid,
  metrics: GraphMetrics,
  rowCount: number,
  scrollY: number,
  rect: { readonly left: number; readonly top: number },
  clientX: number,
  clientY: number,
): HitTarget | null {
  const point = pointerToContent(scrollY, rect, clientX, clientY);
  const node = hitTestNode(grid, point);
  if (node !== null) {
    return node;
  }
  return hitTestRow(metrics, point, rowCount);
}

/**
 * 把命中目标的行号换算成**视口** y（画 hover 卡片、滚动到选中行时用）。
 *
 * 与 `graphGeometry.contentToViewY` 的区别：这里返回的是"行顶边"，
 * 并且额外给出行的可见性判断，避免调用方各写一遍夹取逻辑。
 */
export function rowViewportTop(metrics: GraphMetrics, row: number, scrollY: number): number {
  return row * metrics.rowHeight - scrollY;
}

/** 某一行是否完全落在视口内。 */
export function isRowFullyVisible(
  metrics: GraphMetrics,
  row: number,
  scrollY: number,
  viewportHeight: number,
): boolean {
  const top = rowViewportTop(metrics, row, scrollY);
  return top >= 0 && top + metrics.rowHeight <= viewportHeight;
}
