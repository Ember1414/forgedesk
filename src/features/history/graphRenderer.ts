/**
 * 提交图的 Canvas 绘制（副作用层，T2.2）。
 *
 * # 这一层只做一件事：把几何结果变成像素
 *
 * 所有坐标、圆角、折点、胶囊排布都在 `graphGeometry.ts` 里算好了；
 * 本文件不出现任何"自己算坐标"的算术（唯一例外是把内容坐标平移到视口，
 * 那是一次 `ctx.translate`，不是计算）。这样做的直接收益是：
 * **命中检测与绘制不可能错位**——它们读的是同一份几何输出。
 *
 * # 为什么绘制函数接收 `CanvasLike` 而不是 `CanvasRenderingContext2D`
 *
 * jsdom 的 `getContext('2d')` 返回 `null`，所以"画对了没有"在单测里根本无法验证。
 * 把上下文收窄成一个只含所需方法的接口后，测试可以传入一个**记录调用的假上下文**，
 * 断言"隐藏行确实降了 alpha""merge 边确实描了两条轨""选中环用的是实线"。
 * 真实的 `CanvasRenderingContext2D` 结构上满足 `CanvasLike`，因此生产代码零成本。
 *
 * # 分层
 *
 * - **静态层**：连线 → 节点 → ref 胶囊 → 折叠徽标。只在数据或缩放变化时重画。
 * - **动态层**：hover 行带、同分支链路高亮、选中环、hover 环。每帧都可能重画，
 *   所以它**不画背景**（保持透明），叠在静态层之上。
 * 背景只由静态层负责；两层各是一块 canvas，由 `GraphCanvas` 持有。
 */
import type { GraphEdge, GraphRow } from '@/lib/ipc/history';

import {
  cssFont,
  edgeGeometry,
  laneCenterX,
  layoutCollapsedBadge,
  layoutRefCapsules,
  nodeRadius,
  nodeRect,
  rowCenterY,
  rowTop,
  selectionRingRect,
} from '@/features/history/graphGeometry';
import type {
  CollapsedBadge,
  EdgeGeometry,
  GraphMetrics,
  MinimapRect,
  PathCommand,
  Rect,
  RefCapsule,
  TextMeasurer,
} from '@/features/history/graphGeometry';
import type { NodeLabel } from '@/features/history/commitMeta';
import { laneColor } from '@/features/history/graphTheme';
import type { GraphTheme } from '@/features/history/graphTheme';

// ---------------------------------------------------------------- 上下文接口

/**
 * 绘制所需的最小 Canvas 表面。
 *
 * 属性类型刻意与 `CanvasRenderingContext2D` 保持一致（例如 `fillStyle` 是联合类型）：
 * 若这里写成 `string`，真实上下文反而**不能**赋值给该接口（可写属性是不变的），
 * 那就得强转，而强转正是 `no-explicit-any` 想拦住的东西。
 */
export interface CanvasLike {
  fillStyle: string | CanvasGradient | CanvasPattern;
  strokeStyle: string | CanvasGradient | CanvasPattern;
  lineWidth: number;
  globalAlpha: number;
  font: string;
  textAlign: CanvasTextAlign;
  textBaseline: CanvasTextBaseline;
  lineJoin: CanvasLineJoin;
  lineCap: CanvasLineCap;
  save(): void;
  restore(): void;
  translate(x: number, y: number): void;
  beginPath(): void;
  moveTo(x: number, y: number): void;
  lineTo(x: number, y: number): void;
  quadraticCurveTo(cx: number, cy: number, x: number, y: number): void;
  closePath(): void;
  fill(): void;
  stroke(): void;
  fillRect(x: number, y: number, w: number, h: number): void;
  clearRect(x: number, y: number, w: number, h: number): void;
  setLineDash(segments: number[]): void;
  fillText(text: string, x: number, y: number): void;
}

// ---------------------------------------------------------------- 视觉常量

/**
 * `hidden` 行的整体不透明度。
 *
 * 0.42 的来由：要低到"明显不是当前关注的内容"，又要高到仍能读出泳道走向
 * （再低就变成一片灰雾，折叠区域的拓扑信息全丢了）。
 */
export const HIDDEN_ALPHA = 0.42;

/** merge 节点内层回声描边的不透明度。 */
const MERGE_ECHO_ALPHA = 0.55;

/** 节点胶囊矮于这个高度时不画首字母（画了也是一团墨）。 */
const MIN_HEIGHT_FOR_INITIAL = 11;

/** 父提交不在窗口内时，那条截断边的不透明度（提示"线还没走完"）。 */
const DANGLING_ALPHA = 0.72;

/** 同分支链路高亮的不透明度。 */
const CHAIN_ALPHA = 0.16;

/** 链路高亮条相对泳道宽的占比（留两侧空隙，才看得出是"高亮"而不是"改色"）。 */
const CHAIN_WIDTH_RATIO = 0.62;

/**
 * 边跨度超过这个行数就归入"长边"，每帧全量扫描而不做分桶。
 *
 * 为什么这么分：一条从第 5 行连到第 5000 行的边，在任何包含其中间的窗口里
 * 都是可见的，按孩子行号分桶会漏掉它。把长边单列一张小表，
 * 短边（占绝大多数）仍能享受 O(可见) 的裁剪，长边的全扫成本与仓库大小无关
 * ——因为长边的数量取决于分支的跨度，而不是提交的总数。
 */
export const LONG_EDGE_SPAN = 64;

// ---------------------------------------------------------------- 数据形状

/** 一帧绘制共用的视口信息。 */
export interface GraphFrame {
  readonly ctx: CanvasLike;
  readonly theme: GraphTheme;
  readonly metrics: GraphMetrics;
  /** 滚动容器的 `scrollTop`（CSS px）。 */
  readonly scrollY: number;
  /** 视口宽（CSS px，不含 DPR）。 */
  readonly width: number;
  /** 视口高（CSS px）。 */
  readonly height: number;
}

/** 一次绘制的产出计数（喂给 dev 性能面板）。 */
export interface RenderStats {
  readonly nodes: number;
  readonly edges: number;
  readonly refs: number;
  readonly badges: number;
}

/** 空的绘制计数。 */
export const EMPTY_STATS: RenderStats = { nodes: 0, edges: 0, refs: 0, badges: 0 };

// ---------------------------------------------------------------- 边索引

/**
 * 边的可见性索引。
 *
 * `byRow` 按**孩子行号**分桶（短边），`longEdges` 放跨度大的边。
 * 建索引的时机是"数据变了"，而不是"每一帧"——十万条边每帧重扫一遍是
 * 拿不到 50fps 的，而建一次索引只在续页时发生。
 */
export interface RenderIndex {
  readonly byRow: readonly (readonly GraphEdge[])[];
  readonly longEdges: readonly GraphEdge[];
  readonly rowCount: number;
  readonly lastRow: number;
}

/**
 * 建边索引。
 *
 * @param rowCount 已加载的总行数（决定桶数）
 * @param lastRow 已加载的最大行号；边的父不在窗口内时用它决定截断位置
 */
export function buildRenderIndex(
  edges: readonly GraphEdge[],
  index: ReadonlyMap<string, GraphRow>,
  rowCount: number,
  lastRow: number,
): RenderIndex {
  const lines = Math.max(1, rowCount);
  const byRow: GraphEdge[][] = Array.from({ length: lines }, () => []);
  const longEdges: GraphEdge[] = [];
  for (const edge of edges) {
    const from = index.get(edge.fromOid);
    if (from === undefined) {
      // 孩子不在窗口内 → 这条边完全不可见（父一定在更下方），建索引时就丢掉
      continue;
    }
    const to = index.get(edge.toOid);
    const span = to === undefined ? Number.POSITIVE_INFINITY : Math.abs(to.row - from.row);
    if (span > LONG_EDGE_SPAN) {
      longEdges.push(edge);
      continue;
    }
    byRow[from.row]?.push(edge);
  }
  return { byRow, longEdges, rowCount, lastRow };
}

/**
 * 取出在 `[first, last]` 行窗口内可能需要绘制的边。
 *
 * 返回的是**候选**：一条边的父可能在窗口外（此时它仍要画，只是画到截断处），
 * 所以这里按孩子行号取，不做二次过滤。宁可多画几条被裁掉的线，
 * 也不要漏画——漏画表现为"分支凭空断开"，比多画难查得多。
 */
export function edgesInView(
  renderIndex: RenderIndex,
  first: number,
  last: number,
): readonly GraphEdge[] {
  const result: GraphEdge[] = [...renderIndex.longEdges];
  const from = Math.max(0, first);
  const to = Math.min(renderIndex.byRow.length - 1, last);
  for (let row = from; row <= to; row += 1) {
    const bucket = renderIndex.byRow[row];
    if (bucket !== undefined) {
      result.push(...bucket);
    }
  }
  return result;
}

// ---------------------------------------------------------------- 基础绘制

/** 清空一层（保持透明；背景由静态层单独负责）。 */
export function clearLayer(ctx: CanvasLike, width: number, height: number): void {
  ctx.clearRect(0, 0, Math.max(0, width), Math.max(0, height));
}

/** 拼出 Canvas 能吃的 `font` 值（字体串来自主题，拼接规则在几何层）。 */
export function fontFor(theme: GraphTheme, px: number): string {
  return cssFont(theme.fontStack, px);
}

/** 按路径指令描出一条折线（`traceRoundedPath` 的输出）。 */
export function applyPathCommands(ctx: CanvasLike, commands: readonly PathCommand[]): void {
  ctx.beginPath();
  for (const command of commands) {
    if (command.type === 'move') {
      ctx.moveTo(command.x, command.y);
    } else if (command.type === 'line') {
      ctx.lineTo(command.x, command.y);
    } else {
      ctx.quadraticCurveTo(command.cx, command.cy, command.x, command.y);
    }
  }
}

/**
 * 圆角矩形路径。
 *
 * 不用 `ctx.roundRect`：它进入标准较晚，而本仓库的 E2E 跑在 msedge 上、
 * 单测跑在假上下文上，手写四段二次贝塞尔既不依赖新 API，也让假上下文
 * 只需实现 `quadraticCurveTo` 一种曲线方法。
 */
export function roundedRectPath(ctx: CanvasLike, rect: Rect, radius: number): void {
  const r = Math.max(0, Math.min(radius, rect.w / 2, rect.h / 2));
  const right = rect.x + rect.w;
  const bottom = rect.y + rect.h;
  ctx.beginPath();
  if (r === 0) {
    ctx.moveTo(rect.x, rect.y);
    ctx.lineTo(right, rect.y);
    ctx.lineTo(right, bottom);
    ctx.lineTo(rect.x, bottom);
    ctx.closePath();
    return;
  }
  ctx.moveTo(rect.x + r, rect.y);
  ctx.lineTo(right - r, rect.y);
  ctx.quadraticCurveTo(right, rect.y, right, rect.y + r);
  ctx.lineTo(right, bottom - r);
  ctx.quadraticCurveTo(right, bottom, right - r, bottom);
  ctx.lineTo(rect.x + r, bottom);
  ctx.quadraticCurveTo(rect.x, bottom, rect.x, bottom - r);
  ctx.lineTo(rect.x, rect.y + r);
  ctx.quadraticCurveTo(rect.x, rect.y, rect.x + r, rect.y);
  ctx.closePath();
}

/** ref 类别 → 颜色。 */
function refColor(theme: GraphTheme, kind: RefCapsule['kind']): string {
  if (kind === 'local') {
    return theme.refLocal;
  }
  return kind === 'remote' ? theme.refRemote : theme.refTag;
}

/** 画一个 ref 胶囊（本地=实底反字，远端=描边，标签=虚线描边）。 */
function drawRefCapsule(
  ctx: CanvasLike,
  theme: GraphTheme,
  metrics: GraphMetrics,
  capsule: RefCapsule,
): void {
  const color = refColor(theme, capsule.kind);
  ctx.save();
  roundedRectPath(ctx, capsule.rect, capsule.radius);
  if (capsule.kind === 'local') {
    ctx.fillStyle = color;
    ctx.fill();
    ctx.fillStyle = theme.fgInverted;
  } else {
    // 描边胶囊：1px 边框（不随缩放无限变粗，否则小字号下边框比字还重）
    ctx.lineWidth = 1;
    ctx.strokeStyle = color;
    if (capsule.kind === 'tag') {
      ctx.setLineDash([2 * metrics.scale, 1.5 * metrics.scale]);
    }
    ctx.stroke();
    ctx.fillStyle = color;
  }
  ctx.font = fontFor(theme, metrics.refFont);
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';
  ctx.fillText(
    capsule.label,
    capsule.rect.x + capsule.rect.w / 2,
    capsule.rect.y + capsule.rect.h / 2,
  );
  ctx.restore();
}

/** 画折叠徽标（中性描边 + 次要文字，不与泳道色抢注意力）。 */
function drawCollapsedBadge(
  ctx: CanvasLike,
  theme: GraphTheme,
  metrics: GraphMetrics,
  badge: CollapsedBadge,
): void {
  ctx.save();
  roundedRectPath(ctx, badge.rect, badge.radius);
  ctx.fillStyle = theme.canvas;
  ctx.fill();
  ctx.lineWidth = 1;
  ctx.strokeStyle = theme.line;
  ctx.stroke();
  ctx.fillStyle = theme.fgMuted;
  ctx.font = fontFor(theme, metrics.badgeFont);
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';
  ctx.fillText(badge.label, badge.rect.x + badge.rect.w / 2, badge.rect.y + badge.rect.h / 2);
  ctx.restore();
}

/** 画一个节点胶囊。 */
function drawNode(
  ctx: CanvasLike,
  theme: GraphTheme,
  metrics: GraphMetrics,
  row: GraphRow,
  rect: Rect,
  label: NodeLabel | undefined,
): void {
  const color = laneColor(theme, row.colorIndex);
  const radius = nodeRadius(rect);
  const centerX = rect.x + rect.w / 2;
  const centerY = rect.y + rect.h / 2;
  ctx.save();
  if (row.hidden) {
    ctx.globalAlpha = HIDDEN_ALPHA;
  }
  roundedRectPath(ctx, rect, radius);
  if (row.hidden) {
    // 淡出的行画成"空心 + 虚线描边"：颜色信息还在（能看出属于哪条分支），
    // 但实心感消失了，扫读时视线不会被折叠区域抢走。
    ctx.lineWidth = metrics.lineWidth;
    ctx.strokeStyle = color;
    ctx.setLineDash([2.5 * metrics.scale, 2 * metrics.scale]);
    ctx.stroke();
    ctx.restore();
    return;
  }
  ctx.fillStyle = color;
  ctx.fill();
  if (row.isMerge) {
    // 内层回声描边：一个"胶囊里还有一圈"的记号，表达"这个提交有多个父"。
    // 与 merge 边的双轨呼应——同一件事在节点和线上用同一种语言说。
    const inset = Math.max(1.5, 2.5 * metrics.scale);
    const inner: Rect = {
      x: rect.x + inset,
      y: rect.y + inset,
      w: Math.max(0, rect.w - inset * 2),
      h: Math.max(0, rect.h - inset * 2),
    };
    ctx.save();
    ctx.globalAlpha = MERGE_ECHO_ALPHA;
    ctx.lineWidth = 1;
    ctx.strokeStyle = theme.fgInverted;
    roundedRectPath(ctx, inner, nodeRadius(inner));
    ctx.stroke();
    ctx.restore();
  }
  const initial = label?.initial ?? '';
  if (initial !== '' && metrics.nodeHeight >= MIN_HEIGHT_FOR_INITIAL) {
    ctx.fillStyle = theme.fgInverted;
    ctx.font = fontFor(theme, metrics.initialFont);
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    ctx.fillText(initial, centerX, centerY);
  }
  ctx.restore();
}

/** 画一条边（merge 是两条平行轨，branch 是虚线，straight 是实线）。 */
function drawEdge(ctx: CanvasLike, theme: GraphTheme, geometry: EdgeGeometry): void {
  ctx.save();
  ctx.strokeStyle = laneColor(theme, geometry.colorIndex);
  ctx.lineWidth = geometry.lineWidth;
  ctx.lineJoin = 'round';
  ctx.lineCap = 'round';
  if (geometry.dangling) {
    ctx.globalAlpha = DANGLING_ALPHA;
  }
  ctx.setLineDash([...geometry.dash]);
  for (const rail of geometry.rails) {
    applyPathCommands(ctx, rail);
    ctx.stroke();
  }
  // 虚线相位是上下文状态，不重置会污染后续绘制
  ctx.setLineDash([]);
  ctx.restore();
}

// ---------------------------------------------------------------- 静态层

/** 静态层的输入（数据 + 一帧视口信息）。 */
export interface StaticLayerInput {
  readonly frame: GraphFrame;
  /** 已裁剪到可见窗口的行（调用方负责裁剪，见 `visibleRowRange`）。 */
  readonly rows: readonly GraphRow[];
  /** 已裁剪到可见窗口的边（用 `edgesInView`）。 */
  readonly edges: readonly GraphEdge[];
  /** oid → 行；边的端点解析要用。 */
  readonly index: ReadonlyMap<string, GraphRow>;
  /** 已加载的最大行号（截断边的落点）。 */
  readonly lastRow: number;
  /** oid → 绘制文案。 */
  readonly labels: ReadonlyMap<string, NodeLabel>;
  readonly measure: TextMeasurer;
  /** ref 胶囊与徽标的可用宽度（图列右侧到文本列之间的空间）。 */
  readonly refAreaWidth: number;
  /** 折叠数 → 徽标文案（i18n 由调用方注入，本层不认识 `t`）。 */
  readonly formatCollapsed: (count: number) => string;
}

/**
 * 画静态层：背景 → 连线 → 节点 → ref 胶囊 → 折叠徽标。
 *
 * 顺序是有讲究的：连线必须在节点**下面**，否则线头会盖住胶囊的半圆端，
 * 交界处出现毛刺；ref 胶囊最后画，因为它要压在连线之上（胶囊是信息，
 * 线是背景）。
 */
export function drawStaticLayer(input: StaticLayerInput): RenderStats {
  const { frame, rows, edges, index, lastRow, labels, measure, refAreaWidth, formatCollapsed } =
    input;
  const { ctx, theme, metrics, scrollY, width, height } = frame;

  clearLayer(ctx, width, height);
  ctx.save();
  ctx.fillStyle = theme.canvas;
  ctx.fillRect(0, 0, width, height);
  ctx.restore();

  // 一次性平移到内容坐标：后续所有绘制都用 graphGeometry 给的坐标，不再减 scrollY
  ctx.save();
  ctx.translate(0, -scrollY);

  let edgeCount = 0;
  for (const edge of edges) {
    const geometry = edgeGeometry(metrics, edge, index, lastRow);
    if (geometry === null) {
      continue;
    }
    drawEdge(ctx, theme, geometry);
    edgeCount += 1;
  }

  let refCount = 0;
  let badgeCount = 0;
  for (const row of rows) {
    const rect = nodeRect(metrics, row);
    const label = labels.get(row.oid);
    drawNode(ctx, theme, metrics, row, rect, label);

    const refs = label?.refs ?? [];
    const collapsed = label?.collapsed ?? 0;
    if (refs.length === 0 && collapsed === 0) {
      continue;
    }
    // ref 区从节点右侧起排；可用宽度由调用方给（超出就不排，见 layoutRefCapsules）
    const startX = rect.x + rect.w + metrics.refGap;
    const centerY = rowCenterY(metrics, row.row);
    const capsules = layoutRefCapsules(metrics, refs, measure, startX, centerY, refAreaWidth);
    for (const capsule of capsules) {
      drawRefCapsule(ctx, theme, metrics, capsule);
    }
    refCount += capsules.length;
    if (collapsed > 0) {
      const lastCapsule = capsules[capsules.length - 1];
      const badgeX =
        lastCapsule === undefined
          ? startX
          : lastCapsule.rect.x + lastCapsule.rect.w + metrics.refGap;
      const badge = layoutCollapsedBadge(
        metrics,
        collapsed,
        measure,
        badgeX,
        centerY,
        Math.max(0, refAreaWidth - (badgeX - startX)),
        formatCollapsed,
      );
      if (badge !== null) {
        drawCollapsedBadge(ctx, theme, metrics, badge);
        badgeCount += 1;
      }
    }
  }

  ctx.restore();
  return { nodes: rows.length, edges: edgeCount, refs: refCount, badges: badgeCount };
}

// ---------------------------------------------------------------- 动态层

/** 动态层的输入。全部用 oid 表达：选中集与 hover 都来自 `graphSelectionStore`。 */
export interface DynamicLayerInput {
  readonly frame: GraphFrame;
  readonly index: ReadonlyMap<string, GraphRow>;
  /** 选中的提交（可能是多选/区间选）。 */
  readonly selectedOids: ReadonlySet<string>;
  /** 当前 hover 的提交（`null` 表示指针不在任何节点上）。 */
  readonly hoverOid: string | null;
  /** 与 hover 提交同属一条分支链路的提交（`GraphOverlay` 沿边遍历算出）。 */
  readonly chainOids: ReadonlySet<string>;
  /** 搜索命中的提交（T2.3：琥珀点线环；空集时不进入绘制分支）。 */
  readonly matchOids?: ReadonlySet<string> | undefined;
  /** 已加载的总行数（超出范围的行不画 hover 带）。 */
  readonly rowCount: number;
}

/**
 * 画动态层：hover 行带 → 同分支链路高亮 → 选中环 → hover 环。
 *
 * 不画背景（这一层是透明的叠加层），也不重画节点：
 * hover 每帧都在变，若连节点一起重画就等于放弃了分层的全部意义。
 */
export function drawDynamicLayer(input: DynamicLayerInput): void {
  const { frame, index, selectedOids, hoverOid, chainOids, matchOids, rowCount } = input;
  const { ctx, theme, metrics, scrollY, width, height } = frame;
  clearLayer(ctx, width, height);

  ctx.save();
  ctx.translate(0, -scrollY);

  // 1. hover 行带：整行底色，让"我指的是这一行"在密集的图里也毫无歧义。
  //    rowCount 的夹取不是防御性代码：命中检测允许指针落在"已加载范围之外"
  //    的行上（滚动到空白处），那时画一条行带会让人以为那里有提交。
  const hoverRow = hoverOid === null ? undefined : index.get(hoverOid);
  const hoverVisible = hoverRow !== undefined && hoverRow.row < rowCount;
  if (hoverVisible && hoverRow !== undefined) {
    ctx.save();
    ctx.fillStyle = theme.rowBand;
    ctx.fillRect(0, rowTop(metrics, hoverRow.row), width, metrics.rowHeight);
    ctx.restore();
  }

  // 2. 同分支链路高亮：沿 hover 提交的泳道画一串半透明竖条，
  //    于是"这个分支从哪儿来、到哪儿去"不用追着线看
  if (chainOids.size > 0) {
    ctx.save();
    ctx.globalAlpha = CHAIN_ALPHA;
    ctx.fillStyle = theme.hover;
    const barWidth = metrics.laneWidth * CHAIN_WIDTH_RATIO;
    for (const oid of chainOids) {
      const row = index.get(oid);
      if (row === undefined) {
        continue;
      }
      const centerX = laneCenterX(metrics, row.lane);
      ctx.fillRect(centerX - barWidth / 2, rowTop(metrics, row.row), barWidth, metrics.rowHeight);
    }
    ctx.restore();
  }

  // 2.5 搜索匹配环：警示琥珀点线。三种环的通道各不相同——选中=实线、
  //     hover=虚线、匹配=点线——任何一种单独看都可分辨，叠加时也不会互换。
  if (matchOids !== undefined && matchOids.size > 0) {
    ctx.save();
    ctx.strokeStyle = theme.match;
    ctx.lineWidth = Math.max(1.25, 1.5 * metrics.scale);
    ctx.setLineDash([1.5 * metrics.scale, 2.5 * metrics.scale]);
    for (const oid of matchOids) {
      const row = index.get(oid);
      if (row === undefined) {
        continue;
      }
      const ring = selectionRingRect(metrics, nodeRect(metrics, row));
      roundedRectPath(ctx, ring, nodeRadius(ring));
      ctx.stroke();
    }
    ctx.setLineDash([]);
    ctx.restore();
  }

  // 3. 选中环：中性墨色实线。
  //    刻意不用泳道色——选中是"状态"，泳道色是"身份"，两者混在一起时
  //    用户分不清"这个节点被选中了"还是"它属于另一条分支"。
  ctx.save();
  ctx.strokeStyle = theme.selected;
  ctx.lineWidth = Math.max(1.5, 1.75 * metrics.scale);
  ctx.setLineDash([]);
  for (const oid of selectedOids) {
    const row = index.get(oid);
    if (row === undefined) {
      continue;
    }
    const ring = selectionRingRect(metrics, nodeRect(metrics, row));
    roundedRectPath(ctx, ring, nodeRadius(ring));
    ctx.stroke();
  }
  ctx.restore();

  // 4. hover 环：品牌靛蓝虚线。与选中环的两点差异（颜色 + 虚实）是刻意的双通道
  //    编码，色觉障碍用户仍能靠虚实分辨。
  if (hoverVisible && hoverRow !== undefined) {
    const alreadySelected = hoverOid !== null && selectedOids.has(hoverOid);
    const ring = selectionRingRect(metrics, nodeRect(metrics, hoverRow));
    // 已选中时把 hover 环再往外推一圈，否则两个环完全重叠、虚线被实线吃掉
    const offset = alreadySelected ? Math.max(1.5, 2 * metrics.scale) : 0;
    const outer: Rect = {
      x: ring.x - offset,
      y: ring.y - offset,
      w: ring.w + offset * 2,
      h: ring.h + offset * 2,
    };
    ctx.save();
    ctx.strokeStyle = theme.hover;
    ctx.lineWidth = Math.max(1.25, 1.5 * metrics.scale);
    ctx.setLineDash([3 * metrics.scale, 2 * metrics.scale]);
    roundedRectPath(ctx, outer, nodeRadius(outer));
    ctx.stroke();
    ctx.setLineDash([]);
    ctx.restore();
  }

  ctx.restore();
}

// ---------------------------------------------------------------- 迷你地图

/** 迷你地图的输入。 */
export interface MinimapInput {
  readonly ctx: CanvasLike;
  readonly theme: GraphTheme;
  readonly rects: readonly MinimapRect[];
  readonly width: number;
  readonly height: number;
  readonly rowCount: number;
  /** 视口首行（含小数，用于画出精确的可视框）。 */
  readonly viewportStartRow: number;
  readonly viewportEndRow: number;
}

/** 迷你地图可视框的填充不透明度。 */
const MINIMAP_VIEWPORT_FILL_ALPHA = 0.12;
/** 迷你地图可视框的边框不透明度。 */
const MINIMAP_VIEWPORT_EDGE_ALPHA = 0.55;

/**
 * 画迷你地图：按行聚合的色块 + 可视框。
 *
 * 色块来自 `minimapRects`（已经做过密度→透明度映射），本函数只负责上色。
 * 绘制成本与仓库大小**无关**（桶数被像素高度限制住），这是十万节点下
 * 迷你地图仍然可以常开的原因。
 *
 * 可视框用**中性前景色**而不是某个泳道色：泳道色会被误读成
 * "这一段属于这条分支"，而可视框是**视图状态**，与数据无关
 * （与选中环不用泳道色是同一条理由）。低透明度填充 + 1px 边框，
 * 既能看出范围，又不会把色块盖掉——色块是迷你地图的全部信息。
 */
export function drawMinimap(input: MinimapInput): void {
  const { ctx, theme, rects, width, height, rowCount, viewportStartRow, viewportEndRow } = input;
  clearLayer(ctx, width, height);
  if (rowCount <= 0) {
    return;
  }
  ctx.save();
  ctx.fillStyle = theme.canvas;
  ctx.fillRect(0, 0, width, height);
  for (const block of rects) {
    ctx.save();
    ctx.globalAlpha = Math.max(0, Math.min(1, block.alpha));
    ctx.fillStyle = laneColor(theme, block.colorIndex);
    ctx.fillRect(block.rect.x, block.rect.y, block.rect.w, block.rect.h);
    ctx.restore();
  }
  const top = Math.max(0, (viewportStartRow / rowCount) * height);
  const bottom = Math.min(height, (viewportEndRow / rowCount) * height);
  const boxHeight = Math.max(1, bottom - top);
  ctx.fillStyle = theme.fg;
  ctx.globalAlpha = MINIMAP_VIEWPORT_FILL_ALPHA;
  ctx.fillRect(0, top, width, boxHeight);
  ctx.globalAlpha = MINIMAP_VIEWPORT_EDGE_ALPHA;
  ctx.strokeStyle = theme.fg;
  ctx.lineWidth = 1;
  ctx.setLineDash([]);
  // 只画上下两条边：左右边框贴着迷你地图的边，画上去像渲染毛刺
  ctx.fillRect(0, top, width, 1);
  ctx.fillRect(0, top + boxHeight - 1, width, 1);
  ctx.restore();
}
