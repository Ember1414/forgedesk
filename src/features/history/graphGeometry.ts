/**
 * 提交图的几何计算（纯函数，T2.2）。
 *
 * # 为什么几何与绘制分成两个文件
 *
 * Canvas 的绘制调用是**副作用**（改像素），在 jsdom 里根本跑不起来
 * （`getContext('2d')` 返回 `null`）。把"某个节点画在哪个矩形里""这条边走哪几个
 * 折点""缩放 1.5 倍后行高是多少"这些计算全部抽成纯函数之后：
 *   1. 它们可以被单测直接覆盖（不需要 canvas mock，也就不会有"测试通过但
 *      真实渲染错位"的假绿）；
 *   2. 命中检测（`hitTest.ts`）与渲染（`graphRenderer.ts`）共用**同一份**
 *      坐标口径。这是关键：两处各算一遍坐标，早晚会出现"点得到但画不出来"
 *      或反过来的 bug，而这类 bug 只在特定缩放比例下复现，极难排查。
 *
 * # 坐标口径（唯一）
 *
 * - **内容坐标**：以整张图的左上角为原点，行 `r` 占
 *   `[r*rowHeight, (r+1)*rowHeight)`，泳道 `l` 的中心在
 *   `padLeft + l*laneWidth + laneWidth/2`。`scale` 已经乘进 `rowHeight` /
 *   `laneWidth`（见 `graphMetrics`），因此下游函数不需要再乘一次。
 * - **视口坐标**：`viewY = contentY - scrollY`；`scrollY` 直接取滚动容器的
 *   `scrollTop`（单位是 CSS px，与内容坐标同尺度）。
 * - **CSS px**：所有几何量都是 CSS px；DPR 只在 `GraphCanvas` 里通过
 *   `ctx.scale(dpr, dpr)` 处理，几何层完全不知道 DPR 的存在。
 */
import type { GraphEdge, GraphRow } from '@/lib/ipc/history';

/** 一个点（CSS px）。 */
export interface Point {
  readonly x: number;
  readonly y: number;
}

/** 一个轴对齐矩形（CSS px）。 */
export interface Rect {
  readonly x: number;
  readonly y: number;
  readonly w: number;
  readonly h: number;
}

// ---------------------------------------------------------------- 布局常量

/**
 * scale = 1 时的基准尺寸（CSS px）。
 *
 * 28px 行高的来由：要能同时放下 16px 的节点胶囊、14px 的 ref 胶囊与
 * 一行 12px 文字，并且三者在视觉上不打架；再矮就挤，再高则一屏看不够行数。
 */
export const BASE_ROW_HEIGHT = 28;
/** 泳道宽（两条相邻泳道中心距）。 */
export const BASE_LANE_WIDTH = 22;
/** 图左侧留白：让 lane 0 的胶囊不贴边。 */
export const BASE_PAD_LEFT = 14;
/** 节点胶囊高度。 */
export const BASE_NODE_HEIGHT = 16;
/** 普通提交的胶囊宽度。 */
export const BASE_NODE_WIDTH = 22;
/** merge 提交的胶囊宽度（更宽 + 内层回声描边，表达"多个父"）。 */
export const BASE_MERGE_NODE_WIDTH = 27;
/** ref 胶囊高度（比节点矮一档，明确它是"附在节点上的标签"）。 */
export const BASE_REF_HEIGHT = 14;
/** ref 胶囊水平内边距。 */
export const BASE_REF_PADDING_X = 5;
/** ref 胶囊之间的间距。 */
export const BASE_REF_GAP = 4;
/** 折叠徽标高度。 */
export const BASE_BADGE_HEIGHT = 13;
/** 节点内首字母字号。 */
export const BASE_INITIAL_FONT = 10;
/** ref 胶囊字号。 */
export const BASE_REF_FONT = 10;
/** 徽标字号。 */
export const BASE_BADGE_FONT = 9;

/** 缩放范围（任务约定 0.5x–3x）。 */
export const MIN_SCALE = 0.5;
export const MAX_SCALE = 3;
/**
 * 视口裁剪的额外**像素**预算（上下各 200px）。
 *
 * 为什么用像素而不是行数：真正要控制的成本是"每帧多画多少东西"，而一行
 * 的高度随缩放在 14px（0.5x）到 84px（3x）之间变化。用固定行数会在放大时
 * 多画 6 倍的面积（白白吃掉帧预算），缩小时又只剩几行缓冲（滚动就露白）。
 * 固定像素预算让两种缩放下的"多余绘制量"始终一致。
 *
 * 200px 约等于半屏：滚动是连续的，缓冲半屏意味着快速滚动时也几乎看不到
 * 未绘制的空白，而每帧多画的行数被限制在十几行以内。
 */
export const OVERSCAN_PIXELS = 200;

/** 把像素预算换算成行数（至少 1 行，否则极端缩放下一滚动就露白）。 */
export function overscanRows(metrics: GraphMetrics): number {
  return Math.max(1, Math.ceil(OVERSCAN_PIXELS / Math.max(1, metrics.rowHeight)));
}

/** 缩放后的完整度量（所有下游函数只认这个对象，避免各处重复乘 scale）。 */
export interface GraphMetrics {
  readonly scale: number;
  readonly rowHeight: number;
  readonly laneWidth: number;
  readonly padLeft: number;
  readonly nodeHeight: number;
  readonly nodeWidth: number;
  readonly mergeNodeWidth: number;
  readonly refHeight: number;
  readonly refPaddingX: number;
  readonly refGap: number;
  readonly badgeHeight: number;
  readonly initialFont: number;
  readonly refFont: number;
  readonly badgeFont: number;
  /** 单线（straight / branch）的线宽。 */
  readonly lineWidth: number;
  /** merge 双轨的单轨线宽。 */
  readonly railWidth: number;
  /** merge 双轨的半间距。 */
  readonly railOffset: number;
  /** branch 虚线的 [实, 空] 长度。 */
  readonly branchDash: readonly [number, number];
  /** 折线转角半径。 */
  readonly cornerRadius: number;
}

/** 把缩放夹到 `[MIN_SCALE, MAX_SCALE]`（滚轮会给出任意倍率，不夹就会失控）。 */
export function clampScale(scale: number): number {
  if (!Number.isFinite(scale)) {
    return 1;
  }
  return Math.min(MAX_SCALE, Math.max(MIN_SCALE, scale));
}

/**
 * 由缩放算出全部度量。
 *
 * 线宽刻意**不完全等比**放大：`scale` 到 3 倍时若线宽也 3 倍，连线会糊成一团；
 * 因此线宽只跟随到 1.5 倍为止（`Math.min(scale, 1.5)`），并保证不小于 1px
 * （小于 1px 的线在非整数 DPR 上会被抗锯齿画成半透明，看起来像 bug）。
 */
export function graphMetrics(scale: number): GraphMetrics {
  const clamped = clampScale(scale);
  const strokeScale = Math.min(clamped, 1.5);
  return {
    scale: clamped,
    rowHeight: BASE_ROW_HEIGHT * clamped,
    laneWidth: BASE_LANE_WIDTH * clamped,
    padLeft: BASE_PAD_LEFT * clamped,
    nodeHeight: BASE_NODE_HEIGHT * clamped,
    nodeWidth: BASE_NODE_WIDTH * clamped,
    mergeNodeWidth: BASE_MERGE_NODE_WIDTH * clamped,
    refHeight: BASE_REF_HEIGHT * clamped,
    refPaddingX: BASE_REF_PADDING_X * clamped,
    refGap: BASE_REF_GAP * clamped,
    badgeHeight: BASE_BADGE_HEIGHT * clamped,
    initialFont: Math.max(7, BASE_INITIAL_FONT * clamped),
    refFont: Math.max(7, BASE_REF_FONT * clamped),
    badgeFont: Math.max(6, BASE_BADGE_FONT * clamped),
    lineWidth: Math.max(1, 1.75 * strokeScale),
    railWidth: Math.max(0.9, 1.1 * strokeScale),
    railOffset: Math.max(1.1, 1.6 * strokeScale),
    branchDash: [Math.max(2, 4 * clamped), Math.max(1.5, 3 * clamped)],
    cornerRadius: Math.max(2, 5 * clamped),
  };
}

// ---------------------------------------------------------------- 坐标换算

/** 泳道中心 x（内容坐标）。 */
export function laneCenterX(metrics: GraphMetrics, lane: number): number {
  return metrics.padLeft + lane * metrics.laneWidth + metrics.laneWidth / 2;
}

/** 行顶边 y（内容坐标）。 */
export function rowTop(metrics: GraphMetrics, row: number): number {
  return row * metrics.rowHeight;
}

/** 行中心 y（内容坐标）——节点与连线端点都对齐到这里。 */
export function rowCenterY(metrics: GraphMetrics, row: number): number {
  return rowTop(metrics, row) + metrics.rowHeight / 2;
}

/** 图列的宽度（CSS px）：左留白 + 全部泳道 + 右留白。 */
export function graphColumnWidth(metrics: GraphMetrics, laneCount: number): number {
  const lanes = Math.max(1, laneCount);
  return metrics.padLeft * 2 + lanes * metrics.laneWidth;
}

/** 内容总高度（CSS px）。 */
export function contentHeight(metrics: GraphMetrics, rowCount: number): number {
  return Math.max(0, rowCount) * metrics.rowHeight;
}

/** 视口 y → 内容 y。 */
export function viewToContentY(scrollY: number, viewY: number): number {
  return viewY + scrollY;
}

/** 内容 y → 视口 y。 */
export function contentToViewY(scrollY: number, contentY: number): number {
  return contentY - scrollY;
}

/** 需要绘制的行区间（含 overscan；已夹到 `[0, rowCount)`）。 */
export function visibleRowRange(
  metrics: GraphMetrics,
  scrollY: number,
  viewportHeight: number,
  rowCount: number,
  overscan: number = overscanRows(metrics),
): { readonly first: number; readonly last: number } {
  if (rowCount <= 0) {
    return { first: 0, last: -1 };
  }
  const first = Math.max(0, Math.floor(scrollY / metrics.rowHeight) - overscan);
  const lastVisible = Math.ceil((scrollY + Math.max(0, viewportHeight)) / metrics.rowHeight);
  const last = Math.min(rowCount - 1, lastVisible + overscan);
  return { first, last: Math.max(first, last) };
}

/**
 * 从**按行号升序**的行数组里切出 `[first, last]` 区间。
 *
 * 为什么用二分而不是 `filter`：`filter` 是 O(n)，而绘制是每帧都发生的事——
 * 十万行时光是筛选就比画它们还贵。行数组由 `buildGraphModel` 保证升序
 * （各页的行号区间不重叠且递增），因此二分成立。
 *
 * 不假定 `rows[i].row === i`：分页拼接理论上可能出现空洞
 * （后端因筛选而跳行时），二分对这种情况仍然正确。
 */
export function rowsInRange(
  rows: readonly GraphRow[],
  first: number,
  last: number,
): readonly GraphRow[] {
  if (rows.length === 0 || last < first) {
    return [];
  }
  const startIndex = lowerBound(rows, first);
  const endIndex = lowerBound(rows, last + 1);
  return rows.slice(startIndex, endIndex);
}

/** 第一个 `row >= target` 的下标（全部小于时返回 `rows.length`）。 */
function lowerBound(rows: readonly GraphRow[], target: number): number {
  let low = 0;
  let high = rows.length;
  while (low < high) {
    const mid = (low + high) >>> 1;
    const row = rows[mid];
    if (row !== undefined && row.row < target) {
      low = mid + 1;
    } else {
      high = mid;
    }
  }
  return low;
}

/**
 * 围绕指针缩放后新的 `scrollY`。
 *
 * 推导：指针处的内容坐标在缩放前后必须不变。
 *   `contentY = scrollY + pointerY`，缩放后 `contentY' = contentY * (new/old)`，
 *   于是 `scrollY' = contentY * (new/old) - pointerY`。
 * 不这么做的话，每次滚轮都会把视图"甩"到别处，用户会立刻迷失位置。
 */
export function zoomAround(
  scrollY: number,
  pointerY: number,
  oldScale: number,
  newScale: number,
): number {
  const from = clampScale(oldScale);
  const to = clampScale(newScale);
  if (from === 0) {
    return scrollY;
  }
  return (scrollY + pointerY) * (to / from) - pointerY;
}

// ---------------------------------------------------------------- 节点几何

/** 一行的节点胶囊矩形（内容坐标）。 */
export function nodeRect(metrics: GraphMetrics, row: GraphRow): Rect {
  const width = row.isMerge ? metrics.mergeNodeWidth : metrics.nodeWidth;
  const centerX = laneCenterX(metrics, row.lane);
  const centerY = rowCenterY(metrics, row.row);
  return {
    x: centerX - width / 2,
    y: centerY - metrics.nodeHeight / 2,
    w: width,
    h: metrics.nodeHeight,
  };
}

/** 胶囊圆角半径 = 高度的一半（两端是完整半圆，这才是"胶囊"而不是圆角矩形）。 */
export function nodeRadius(rect: Rect): number {
  return rect.h / 2;
}

/** 选中环矩形：比节点外扩一圈，让环与胶囊之间留出可见的间隙。 */
export function selectionRingRect(metrics: GraphMetrics, rect: Rect): Rect {
  const gap = Math.max(1.5, 2 * metrics.scale);
  return { x: rect.x - gap, y: rect.y - gap, w: rect.w + gap * 2, h: rect.h + gap * 2 };
}

// ---------------------------------------------------------------- 折线路径

/** 路径指令（`moveTo` / `lineTo` / `quadraticCurveTo` 的最小可测表示）。 */
export type PathCommand =
  | { readonly type: 'move'; readonly x: number; readonly y: number }
  | { readonly type: 'line'; readonly x: number; readonly y: number }
  | {
      readonly type: 'curve';
      readonly cx: number;
      readonly cy: number;
      readonly x: number;
      readonly y: number;
    };

/**
 * 把折点转成"带圆角的路径指令"。
 *
 * 为什么用二次贝塞尔而不是 `arcTo`：`arcTo` 的半径在夹角很小时会被浏览器自行
 * 缩小，且它依赖当前路径点，出错时不容易在单测里断言。二次贝塞尔以
 * "角点两侧各退 radius 处"为端点、角点本身为控制点，几何完全可预测，
 * 单测可以直接断言指令序列。
 */
export function traceRoundedPath(points: readonly Point[], radius: number): readonly PathCommand[] {
  if (points.length === 0) {
    return [];
  }
  const first = points[0];
  if (first === undefined) {
    return [];
  }
  const commands: PathCommand[] = [{ type: 'move', x: first.x, y: first.y }];
  for (let index = 1; index < points.length - 1; index += 1) {
    const corner = points[index];
    const before = points[index - 1];
    const after = points[index + 1];
    if (corner === undefined || before === undefined || after === undefined) {
      continue;
    }
    // 半径不能超过相邻段的一半，否则两段圆角会互相吃掉（画出打结的形状）
    const limit = Math.min(distance(before, corner), distance(corner, after)) / 2;
    const r = Math.max(0, Math.min(radius, limit));
    if (r === 0) {
      commands.push({ type: 'line', x: corner.x, y: corner.y });
      continue;
    }
    const start = along(corner, before, r);
    const end = along(corner, after, r);
    commands.push({ type: 'line', x: start.x, y: start.y });
    commands.push({ type: 'curve', cx: corner.x, cy: corner.y, x: end.x, y: end.y });
  }
  const last = points[points.length - 1];
  if (last !== undefined && points.length > 1) {
    commands.push({ type: 'line', x: last.x, y: last.y });
  }
  return commands;
}

/** 两点距离。 */
export function distance(a: Point, b: Point): number {
  return Math.hypot(b.x - a.x, b.y - a.y);
}

/** 从 `from` 朝 `to` 方向走 `length` 后的点（`length` 超过距离时停在 `to`）。 */
export function along(from: Point, to: Point, length: number): Point {
  const total = distance(from, to);
  if (total === 0) {
    return { x: from.x, y: from.y };
  }
  const ratio = Math.min(1, length / total);
  return { x: from.x + (to.x - from.x) * ratio, y: from.y + (to.y - from.y) * ratio };
}

/** 一条边的完整绘制几何。 */
export interface EdgeGeometry {
  readonly kind: GraphEdge['kind'];
  readonly colorIndex: number;
  /** 单轨（straight / branch）为 1 条；merge 为 2 条平行轨。 */
  readonly rails: readonly (readonly PathCommand[])[];
  readonly lineWidth: number;
  /** 空数组表示实线。 */
  readonly dash: readonly number[];
  /** 父提交不在已加载窗口内（画到最后一行下方就停，表达"继续向下"）。 */
  readonly dangling: boolean;
}

/**
 * 计算一条边的折线与画法。
 *
 * @param index oid → 行（`indexRows` 建索引）；用于把 `toOid` 换成行号。
 * @param lastRow 已加载的最大行号；父不在窗口内时线画到它下面一行就截断。
 */
export function edgeGeometry(
  metrics: GraphMetrics,
  edge: GraphEdge,
  index: ReadonlyMap<string, GraphRow>,
  lastRow: number,
): EdgeGeometry | null {
  const from = index.get(edge.fromOid);
  if (from === undefined) {
    // 孩子不在窗口内：这条边没有任何可见部分（父在孩子下方，更不可能可见）
    return null;
  }
  const x1 = laneCenterX(metrics, from.lane);
  const y1 = rowCenterY(metrics, from.row);
  const to = index.get(edge.toOid);
  const dangling = to === undefined;
  // 端点的 lane 一律以**行自己记录的 lane** 为准，而不是边上携带的快照值：
  // 跨页的边在下一页到达之前，`edge.toLane` 只是布局器给未加载父提交预留的
  // **猜测槽位**（见 `layout.rs` 的分页边界注释）；下一页真正给出该提交的 lane 之后
  // 必须以它为准，否则线会接到一条空泳道上，看起来像分支凭空消失。
  // 同页的边两者恒等（布局器就是用父行的 lane 填的 `toLane`），因此这个替换无副作用。
  const x2 = laneCenterX(metrics, to === undefined ? edge.toLane : to.lane);
  // 父在窗口外：画到最后一行下方一行的中心，形成"继续向下"的短线头。
  // 不画到内容底部，否则一个被筛选掉的父会让线贯穿整张图（那是错的）。
  const y2 = dangling ? rowCenterY(metrics, lastRow + 1) : rowCenterY(metrics, to.row);

  const colorIndex = to?.colorIndex ?? from.colorIndex;
  const base: Omit<EdgeGeometry, 'rails'> = {
    kind: edge.kind,
    colorIndex,
    lineWidth: edge.kind === 'merge' ? metrics.railWidth : metrics.lineWidth,
    dash: edge.kind === 'branch' ? [...metrics.branchDash] : [],
    dangling,
  };

  if (x1 === x2) {
    // 同泳道：一条竖线（merge 也退化成单轨——两轨重合没有信息量，只会变粗）
    return {
      ...base,
      rails: [
        traceRoundedPath(
          [
            { x: x1, y: y1 },
            { x: x2, y: y2 },
          ],
          metrics.cornerRadius,
        ),
      ],
    };
  }

  // 拐点：孩子下方半行处（贴着孩子的行边界拐弯），但不能越过父子中点，
  // 否则跨多行的边会在父亲那一侧出现倒折。
  const turnY = Math.min(y1 + metrics.rowHeight / 2, (y1 + y2) / 2);
  const railPoints = (dx: number, dy: number): readonly Point[] => [
    { x: x1 + dx, y: y1 },
    { x: x1 + dx, y: turnY + dy },
    { x: x2 + dx, y: turnY + dy },
    { x: x2 + dx, y: y2 },
  ];
  const radius = Math.max(0, metrics.cornerRadius - metrics.railOffset);

  if (edge.kind !== 'merge') {
    return {
      ...base,
      rails: [traceRoundedPath(railPoints(0, 0), metrics.cornerRadius)],
    };
  }

  // merge 双轨：竖直段左右各偏 railOffset，水平段上下各偏 railOffset。
  // 这是正交折线的"平行偏移"，两轨在任意位置的垂直间距恒为 2*railOffset，
  // 于是"这是一条合并线"在任意缩放下都看得出来（单靠颜色区分不了，
  // 因为颜色属于泳道而不属于边）。
  const offset = metrics.railOffset;
  return {
    ...base,
    rails: [
      traceRoundedPath(railPoints(-offset, -offset), radius),
      traceRoundedPath(railPoints(offset, offset), radius),
    ],
  };
}

/** 建 oid → 行 的索引（分页累积后每页各有一份，合并时以先到的为准）。 */
export function indexRows(rows: readonly GraphRow[]): ReadonlyMap<string, GraphRow> {
  const index = new Map<string, GraphRow>();
  for (const row of rows) {
    if (!index.has(row.oid)) {
      index.set(row.oid, row);
    }
  }
  return index;
}

/** 已加载的最大行号（没有行时返回 -1）。 */
export function lastRowOf(rows: readonly GraphRow[]): number {
  let last = -1;
  for (const row of rows) {
    if (row.row > last) {
      last = row.row;
    }
  }
  return last;
}

// ---------------------------------------------------------------- ref 胶囊

/** ref 的呈现类别（决定实底 / 描边 / 虚线描边）。 */
export type RefKind = 'local' | 'remote' | 'tag';

/** 一个 ref 胶囊的几何。 */
export interface RefCapsule {
  readonly rect: Rect;
  readonly radius: number;
  readonly label: string;
  readonly kind: RefKind;
}

/** 文本宽度度量函数。 */
export type TextMeasurer = (text: string, fontPx: number) => number;

/**
 * 文字度量的降级方案。
 *
 * `ctx.measureText` 在两种情况下不可用：jsdom（`getContext` 返回 `null`），
 * 以及字体尚未加载完成时（会返回按回退字体算出的宽度，导致胶囊宽度跳变）。
 * 这里给一个**保守偏宽**的估算：CJK 按 1em、其余按 0.58em。
 * 偏宽的后果是胶囊之间多一点空隙（可接受）；偏窄的后果是文字溢出胶囊（不可接受）。
 */
export function estimateTextWidth(text: string, fontPx: number): number {
  let units = 0;
  for (const char of text) {
    const code = char.codePointAt(0) ?? 0;
    // CJK 统一表意文字、全角标点与假名都是等宽全角
    const wide =
      (code >= 0x1100 && code <= 0x115f) ||
      (code >= 0x2e80 && code <= 0xa4cf) ||
      (code >= 0xac00 && code <= 0xd7a3) ||
      (code >= 0xf900 && code <= 0xfaff) ||
      (code >= 0xfe30 && code <= 0xfe6f) ||
      (code >= 0xff00 && code <= 0xff60) ||
      (code >= 0xffe0 && code <= 0xffe6);
    units += wide ? 1 : 0.58;
  }
  return units * fontPx;
}

/**
 * 拼出 Canvas 能吃的 `font` 值。
 *
 * 放在几何层（而不是渲染层）是因为**度量与绘制必须用同一个字体**：
 * `ctx.measureText` 读的是上下文上当前的 `font`，两边各拼一次就会在
 * 字号变化时静默错位（胶囊宽度按 A 字体算、文字按 B 字体画）。
 */
export function cssFont(fontStack: string, px: number): string {
  return `${Math.max(1, Math.round(px))}px ${fontStack}`;
}

/**
 * 包装一个 Canvas 上下文成度量函数；上下文不可用时自动退回估算。
 *
 * 为什么还要检查返回值：`measureText` 在某些实现里对空字符串或不可用字体
 * 返回 `width = 0`，直接拿去布局会让所有胶囊叠在同一个 x 上。
 *
 * 为什么每次度量前都要重设 `ctx.font`：`measureText` 用的是上下文上
 * **当前**的字体，而一次绘制里会依次遇到首字母/ref 胶囊/折叠徽标三种字号。
 * 不重设就会拿上一个字号的宽度去排下一个胶囊（这个错很隐蔽：
 * 只在同一行同时出现两种文字时才看得见）。
 */
export function createTextMeasurer(
  ctx: CanvasRenderingContext2D | null,
  fontStack = 'sans-serif',
): TextMeasurer {
  return (text, fontPx) => {
    if (ctx !== null) {
      ctx.font = cssFont(fontStack, fontPx);
      const measured = ctx.measureText(text).width;
      if (Number.isFinite(measured) && (measured > 0 || text.length === 0)) {
        return measured;
      }
    }
    return estimateTextWidth(text, fontPx);
  };
}

/** 一个已分类的 ref 标签。 */
export interface RefLabel {
  readonly label: string;
  readonly kind: RefKind;
}

/**
 * 在节点右侧依次排布 ref 胶囊。
 *
 * @param startX 第一个胶囊的左边缘（内容坐标）
 * @param centerY 行中心 y
 * @param maxWidth 可用宽度；超出的胶囊直接不排（不截半个胶囊——半个标签
 *   比没有标签更误导，因为用户会以为那就是全部）
 */
export function layoutRefCapsules(
  metrics: GraphMetrics,
  refs: readonly RefLabel[],
  measure: TextMeasurer,
  startX: number,
  centerY: number,
  maxWidth: number,
): readonly RefCapsule[] {
  const capsules: RefCapsule[] = [];
  let cursor = startX;
  const height = metrics.refHeight;
  const y = centerY - height / 2;
  for (const ref of refs) {
    const textWidth = measure(ref.label, metrics.refFont);
    const width = textWidth + metrics.refPaddingX * 2;
    if (cursor + width > startX + maxWidth) {
      break;
    }
    capsules.push({
      rect: { x: cursor, y, w: width, h: height },
      // 4px 圆角（不是胶囊）：与节点的半圆胶囊形成形状差异，
      // 一眼就能分清"这是提交"和"这是挂在提交上的引用"
      radius: Math.min(height / 2, 4 * metrics.scale),
      label: ref.label,
      kind: ref.kind,
    });
    cursor += width + metrics.refGap;
  }
  return capsules;
}

/** 折叠徽标的几何（"+N"，挂在 ref 胶囊之后）。 */
export interface CollapsedBadge {
  readonly rect: Rect;
  readonly radius: number;
  readonly label: string;
}

/** 排布"折叠了 N 个提交"的徽标。 */
export function layoutCollapsedBadge(
  metrics: GraphMetrics,
  count: number,
  measure: TextMeasurer,
  startX: number,
  centerY: number,
  maxWidth: number,
  format: (count: number) => string,
): CollapsedBadge | null {
  if (count <= 0) {
    return null;
  }
  const label = format(count);
  const height = metrics.badgeHeight;
  const width = measure(label, metrics.badgeFont) + metrics.refPaddingX * 2;
  if (startX + width > startX + maxWidth) {
    return null;
  }
  return {
    rect: { x: startX, y: centerY - height / 2, w: width, h: height },
    radius: height / 2,
    label,
  };
}

// ---------------------------------------------------------------- 迷你地图

/**
 * 迷你地图的一桶（按行聚合）。
 *
 * 为什么必须聚合：目标是 10 万节点仍可扫读。逐点画的话，一个 120×240 的
 * 迷你地图要塞进 10 万个点——每个点不到 1/4 像素，结果是一片均匀的噪点，
 * 既看不出结构又白烧 CPU。按行分桶后每桶只画一个色块，桶数被迷你地图的
 * 像素高度天然限制住（≤ 高度 / 2），绘制成本与仓库大小**无关**。
 */
export interface MinimapBucket {
  readonly startRow: number;
  readonly endRow: number;
  /** 桶内出现最多的颜色索引（众数）——代表"这一段主要是哪条分支"。 */
  readonly colorIndex: number;
  /** 桶内有多少行（用于决定色块的不透明度：越密越实）。 */
  readonly count: number;
  /** 桶内 hidden 行的占比（折叠区域在迷你地图上淡出，与主图口径一致）。 */
  readonly hiddenRatio: number;
}

/**
 * 把行按 `bucketRows` 分桶聚合。
 *
 * @param bucketRows 每桶覆盖的行数（≥1）
 */
export function minimapBuckets(
  rows: readonly GraphRow[],
  rowCount: number,
  bucketRows: number,
): readonly MinimapBucket[] {
  const size = Math.max(1, Math.floor(bucketRows));
  const total = Math.max(0, rowCount);
  // 聚合阶段用可变结构，最后一转成 readonly 的 MinimapBucket：
  // 对外暴露可变字段等于邀请调用方就地改，而迷你地图的桶是每帧重建的缓存。
  const buckets: MutableBucket[] = [];
  for (let start = 0; start < total; start += size) {
    buckets.push({
      startRow: start,
      endRow: Math.min(total, start + size),
      count: 0,
      hidden: 0,
      tally: new Map<number, number>(),
    });
  }
  // 一次遍历把行投进桶里（10 万行也只走一遍）
  for (const row of rows) {
    const bucket = buckets[Math.floor(row.row / size)];
    if (bucket === undefined) {
      continue;
    }
    bucket.count += 1;
    if (row.hidden) {
      bucket.hidden += 1;
    }
    bucket.tally.set(row.colorIndex, (bucket.tally.get(row.colorIndex) ?? 0) + 1);
  }
  return buckets.map((bucket) => {
    let dominant = 0;
    let best = -1;
    for (const [colorIndex, count] of bucket.tally) {
      // 平票时取下标小的：保证同一份数据两次聚合结果完全一致（不抖动）
      if (count > best || (count === best && colorIndex < dominant)) {
        best = count;
        dominant = colorIndex;
      }
    }
    return {
      startRow: bucket.startRow,
      endRow: bucket.endRow,
      colorIndex: dominant,
      count: bucket.count,
      hiddenRatio: bucket.count === 0 ? 0 : bucket.hidden / bucket.count,
    };
  });
}

/** 聚合过程中的可变中间态（不对外导出）。 */
interface MutableBucket {
  startRow: number;
  endRow: number;
  count: number;
  hidden: number;
  /** colorIndex → 出现次数（用于取众数）。 */
  tally: Map<number, number>;
}

/** 迷你地图上一个色块的矩形。 */
export interface MinimapRect {
  readonly rect: Rect;
  readonly colorIndex: number;
  /** 0..1，密度越高越实。 */
  readonly alpha: number;
}

/** 把桶映射成迷你地图上的色块矩形。 */
export function minimapRects(
  buckets: readonly MinimapBucket[],
  rowCount: number,
  width: number,
  height: number,
): readonly MinimapRect[] {
  if (rowCount <= 0 || width <= 0 || height <= 0) {
    return [];
  }
  return buckets.map((bucket) => {
    const y = (bucket.startRow / rowCount) * height;
    const h = Math.max(1, ((bucket.endRow - bucket.startRow) / rowCount) * height);
    return {
      rect: { x: 0, y, w: width, h },
      colorIndex: bucket.colorIndex,
      // 密度映射到 0.35..1：稀疏段落淡一点，避免"整条迷你地图一样亮"
      alpha:
        (0.35 + 0.65 * Math.min(1, bucket.count / Math.max(1, bucket.endRow - bucket.startRow))) *
        (1 - bucket.hiddenRatio * 0.6),
    };
  });
}

// ---------------------------------------------------------------- 命中几何

/** 点是否落在圆角矩形内（含圆角外的区域排除）。 */
export function pointInRoundedRect(point: Point, rect: Rect, radius: number): boolean {
  if (point.x < rect.x || point.x > rect.x + rect.w) {
    return false;
  }
  if (point.y < rect.y || point.y > rect.y + rect.h) {
    return false;
  }
  const r = Math.max(0, Math.min(radius, rect.w / 2, rect.h / 2));
  if (r === 0) {
    return true;
  }
  // 只在四个角区域里需要额外判定：把点与最近的角圆心比较
  const cx =
    point.x < rect.x + r ? rect.x + r : point.x > rect.x + rect.w - r ? rect.x + rect.w - r : null;
  const cy =
    point.y < rect.y + r ? rect.y + r : point.y > rect.y + rect.h - r ? rect.y + rect.h - r : null;
  if (cx === null || cy === null) {
    return true;
  }
  return Math.hypot(point.x - cx, point.y - cy) <= r;
}

/** 指针所在的全局行号（可能超出已加载范围，调用方需自行夹取）。 */
export function rowAtY(metrics: GraphMetrics, contentY: number): number {
  return Math.floor(contentY / metrics.rowHeight);
}

/** 指针所在的泳道；落在左侧留白里时返回 -1（表示"点在空白处"）。 */
export function laneAtX(metrics: GraphMetrics, contentX: number): number {
  if (contentX < metrics.padLeft) {
    return -1;
  }
  return Math.floor((contentX - metrics.padLeft) / metrics.laneWidth);
}

// ---------------------------------------------------------------- 同泳道链路

/**
 * 同泳道邻接表：oid → 同泳道的父/子 oid。
 *
 * 为什么只留同泳道的边：hover 高亮要回答的是"这个提交属于哪一条分支"，
 * 而一条分支在图上就是**同一泳道里连续的一串**。跳泳道的边（merge / branch）
 * 意味着分支已经分岔或汇合，把它算进链路会把一大片无关的提交一起点亮。
 */
export interface LaneAdjacency {
  readonly parents: ReadonlyMap<string, readonly string[]>;
  readonly children: ReadonlyMap<string, readonly string[]>;
}

/** 链路遍历的步数上限（防御环；见 `walkLaneChain`）。 */
export const CHAIN_WALK_LIMIT = 512;

/**
 * 由边集建同泳道邻接表。
 *
 * 建一次、用多次：hover 每次移到新节点都要算链路，若每次都重扫边集
 * （十万条）就会在指针移动时卡顿。调用方应在数据变化时用 `useMemo` 重建。
 *
 * 端点的泳道一律以**行自己记录的 lane** 为准（与 `edgeGeometry` 同一口径）：
 * 跨页的边上携带的 `toLane` 只是布局器给未加载父提交预留的猜测槽位。
 * 未解析的端点直接跳过——它还没到达，等下一页加载后自然会重建邻接表。
 */
export function buildLaneAdjacency(
  edges: readonly GraphEdge[],
  index: ReadonlyMap<string, GraphRow>,
): LaneAdjacency {
  const parents = new Map<string, string[]>();
  const children = new Map<string, string[]>();
  for (const edge of edges) {
    const from = index.get(edge.fromOid);
    const to = index.get(edge.toOid);
    if (from === undefined || to === undefined || from.lane !== to.lane) {
      continue;
    }
    const up = parents.get(edge.fromOid);
    if (up === undefined) {
      parents.set(edge.fromOid, [edge.toOid]);
    } else if (!up.includes(edge.toOid)) {
      up.push(edge.toOid);
    }
    const down = children.get(edge.toOid);
    if (down === undefined) {
      children.set(edge.toOid, [edge.fromOid]);
    } else if (!down.includes(edge.fromOid)) {
      down.push(edge.fromOid);
    }
  }
  return { parents, children };
}

/**
 * 从某个提交出发，沿同泳道上下两个方向走出整条链路。
 *
 * `limit` 不是性能优化而是**正确性护栏**：Git 历史本身是无环的，但前端拿到
 * 的是分页拼接出来的边集，续页过程中的瞬时状态可能凑出环（同一对 oid 的边
 * 来自两页）。没有步数上限的话，一个环就会让指针移动永久卡死主线程。
 *
 * 起点自身一定包含在结果里（哪怕它没有任何同泳道邻居），
 * 这样"高亮至少覆盖我指着的那一行"永远成立。
 */
export function walkLaneChain(
  adjacency: LaneAdjacency,
  oid: string,
  limit: number = CHAIN_WALK_LIMIT,
): ReadonlySet<string> {
  const chain = new Set<string>([oid]);
  if (limit <= 0) {
    return chain;
  }
  const queue: string[] = [oid];
  let steps = 0;
  while (queue.length > 0 && steps < limit) {
    const current = queue.shift();
    if (current === undefined) {
      break;
    }
    steps += 1;
    for (const neighbours of [adjacency.parents.get(current), adjacency.children.get(current)]) {
      if (neighbours === undefined) {
        continue;
      }
      for (const next of neighbours) {
        if (chain.has(next)) {
          continue;
        }
        chain.add(next);
        queue.push(next);
      }
    }
  }
  return chain;
}
