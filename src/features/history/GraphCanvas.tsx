/**
 * 提交图画布（T2.2）。
 *
 * # 这个组件负责什么
 *
 * 绘制与视口：DPR 适配、静态/动态两层 canvas、视口裁剪、围绕指针缩放、
 * 拖拽平移、迷你地图、rAF 合并重绘。它**不理解 Git 语义**，也不决定
 * "选中意味着什么"——那些在 `GraphOverlay` 与 `graphSelectionStore` 里。
 *
 * # 为什么用一块滚动容器 + sticky 视口，而不是自己实现滚动
 *
 * 自己接管滚动（把 scrollTop 变成 React state）意味着每滚一像素都要重渲染，
 * 而且会丢掉原生滚动条、触摸板惯性、无障碍滚动这些浏览器已经做对的东西。
 * 这里的做法是：一个高度等于内容总高的占位 div 撑出真实滚动区，里面放一个
 * `position: sticky; top: 0` 的视口，canvas 与文本行都在视口内、按 `-scrollTop`
 * 偏移。滚动完全交给浏览器，我们只在 `scroll` 事件里重绘 canvas（命令式，
 * 不走 React）并改写文本列的 `transform`。
 *
 * # 为什么 hover 不走 React state
 *
 * 指针移动是每秒几十次的高频事件。hover 状态写进 `graphSelectionStore`，
 * canvas 通过 store 订阅在 rAF 里重画**动态层**，本组件不重渲染。
 * 只有缩放、迷你地图开关、选中集变化（都是低频）才走 React 渲染。
 *
 * 组件整体套了 `memo`：`GraphOverlay` 会因为 hover 卡片而重渲染，
 * 但传下来的 props 全是稳定引用/字符串，因此本组件会直接跳过。
 *
 * # 无障碍
 *
 * - 静态 canvas 是 `role="img"` + 摘要标签：它传达的是"整张图的形状"，
 *   逐行内容与文本列完全重复，让读屏软件念两遍只会更吵。动态 canvas 是
 *   纯装饰（选中环/hover 环），`aria-hidden`。
 * - 交互层是 `role="listbox"` + `aria-multiselectable`，每行 `role="option"`
 *   带 `aria-selected` / `aria-posinset` / `aria-setsize`。只有可视行在 DOM 里，
 *   不给 setsize 读屏软件会以为总共就那几十条。
 * - 焦点落在 listbox 上，活动项用 `aria-activedescendant` 指出去——这样键盘
 *   导航不会把焦点从滚动容器里挪走（`role="option"` 自己拿焦点会导致
 *   浏览器把滚动锚定到选项上，与我们自己的滚动控制打架）。
 * - 需要"表格"语义（列头、按列朗读）时请切到列表模式，那边是 `role="grid"`。
 *
 * # 横向溢出的取舍
 *
 * 泳道很多且放大到 3x 时，图列可能比视口宽。这里选择**裁掉**而不是加横向
 * 滚动条：横向滚动会让 sticky 视口的宽度计算与文本列的右对齐同时复杂化，
 * 而用户真正的出路是缩小（0.5x 能装下 3 倍的泳道）。文本列始终右对齐可见。
 */
import {
  memo,
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import type { CSSProperties, KeyboardEvent as ReactKeyboardEvent, ReactNode } from 'react';
import type {
  MouseEvent as ReactMouseEvent,
  PointerEvent as ReactPointerEvent,
  UIEvent as ReactUIEvent,
} from 'react';

import type { GraphRow } from '@/lib/ipc/history';

import { cn } from '@/lib/utils';

import {
  buildLaneAdjacency,
  clampScale,
  contentHeight,
  createTextMeasurer,
  graphColumnWidth,
  graphMetrics,
  minimapBuckets,
  minimapRects,
  nodeRect,
  visibleRowRange,
  walkLaneChain,
  zoomAround,
} from '@/features/history/graphGeometry';
import type { MinimapRect } from '@/features/history/graphGeometry';
import {
  buildRenderIndex,
  drawDynamicLayer,
  drawMinimap,
  drawStaticLayer,
  edgesInView,
} from '@/features/history/graphRenderer';
import { buildHitGrid, hitTest } from '@/features/history/hitTest';
import type { HitTarget } from '@/features/history/hitTest';
import type { RowText } from '@/features/history/commitMeta';
import { laneBgClass, refChipClass } from '@/features/history/graphTheme';
import { useGraphTheme } from '@/features/history/useGraphTheme';
import {
  useGraphSelectionStore,
  useSelectedOidSet,
  ZOOM_STEP,
} from '@/features/history/graphSelectionStore';
import type { SelectionModifiers } from '@/features/history/graphSelectionStore';
import type { GraphModel } from '@/features/history/useGraphQuery';
import { createFpsMeter, readHeapMb, recordGraphPerf } from '@/features/history/graphPerfStore';

// ---------------------------------------------------------------- 常量

/** 迷你地图尺寸（CSS px）。宽度刻意窄：它是概览，不是第二个视图。 */
const MINIMAP_WIDTH = 120;
const MINIMAP_HEIGHT = 200;
/** 迷你地图距视口右下角的留白。 */
const MINIMAP_MARGIN = 12;
/** 迷你地图里一个色块的最小像素高（桶数因此被限制在 100 以内）。 */
const MINIMAP_MIN_BLOCK_PX = 2;

/** 拖拽多少像素之后才算"平移"而不是"单击"（手抖容忍度）。 */
const PAN_THRESHOLD_PX = 4;

/** 距底部多近时请求下一页（提前一点，用户不会看到"卡住"）。 */
const LOAD_MORE_THRESHOLD_PX = 600;

/** 文本列里 ref 胶囊区的预留宽度（基准 px，随 scale 缩放）。 */
const REF_AREA_BASE_WIDTH = 176;

/** 文本列的 DOM 窗口额外行数（比 canvas 的像素预算小：DOM 行贵得多）。 */
const DOM_OVERSCAN_ROWS = 6;

/** hover 卡片与节点之间的水平间距。 */
const HOVER_CARD_GAP = 8;

// ---------------------------------------------------------------- 对外形状

/** hover 卡片需要的信息（坐标是**视口**内的 CSS px）。 */
export interface HoverTarget {
  readonly oid: string;
  /** 该行的文案；分页边界上提交可能还没到，那时是 `undefined`。 */
  readonly text: RowText | undefined;
  /** 节点右边缘的视口 x（卡片贴在节点右侧，而不是跟着指针跑）。 */
  readonly x: number;
  /** 行中心的视口 y。 */
  readonly y: number;
}

export interface GraphCanvasProps {
  readonly model: GraphModel;
  /** 与 `model.rows` 同序、同长度的 DOM 文案（调用方注入 i18n 后算好）。 */
  readonly texts: readonly RowText[];
  /** 图区域的无障碍名称（listbox 的名字）。 */
  readonly listLabel: string;
  /** 静态 canvas 的替代文本摘要（如"提交图：1284 个提交，6 条泳道"）。 */
  readonly canvasLabel: string;
  readonly minimapLabel: string;
  /** 折叠徽标文案（i18n 注入；本组件不认识 `t`）。 */
  readonly formatCollapsed: (count: number) => string;
  /** 一行的完整可读文案（`aria-label`，含 oid / 作者 / 时间）。 */
  readonly rowLabel: (text: RowText) => string;
  /** 单击 / Enter 选中一行。 */
  readonly onActivate: (oid: string, modifiers: SelectionModifiers) => void;
  /** Ctrl/Cmd+A：全选已加载的提交。 */
  readonly onSelectAll: () => void;
  /** hover 目标变化（`null` = 指针离开任何提交）。 */
  readonly onHoverTarget: (target: HoverTarget | null) => void;
  /** 滚到底部附近时请求下一页。 */
  readonly onNeedMore: () => void;
  /**
   * 搜索命中的提交（T2.3）：动态层为它们画琥珀点线环。
   * 引用稳定由调用方保证（useMemo），空集 = 无高亮。
   */
  readonly matchOids?: ReadonlySet<string> | undefined;
  /**
   * 滚动跳转请求（T2.3 的"上一处 / 下一处"）。`token` 递增以保证同一行
   * 重复请求也能再次触发 effect；行会被滚到视口中部（不是顶端，上下文
   * 可见）。画布与列表模式各自处理自己的滚动，这里只管画布。
   */
  readonly scrollToRow?: { readonly row: number; readonly token: number } | null | undefined;
  readonly className?: string;
}

// ---------------------------------------------------------------- DPR

/** 读设备像素比（非浏览器环境或异常值时退回 1）。 */
function readDevicePixelRatio(): number {
  if (typeof window === 'undefined') {
    return 1;
  }
  const value = window.devicePixelRatio;
  return Number.isFinite(value) && value > 0 ? value : 1;
}

/**
 * 订阅设备像素比变化。
 *
 * 为什么要订阅：Windows 上把窗口从 100% 缩放的显示器拖到 150% 的显示器时
 * `devicePixelRatio` 会变。不重设 canvas 的后备缓冲区，图就会突然糊掉
 * （位图被拉伸），而这个 bug 只有多显示器用户能复现。
 *
 * 用 `(resolution: Ndppx)` 媒体查询监听是标准做法：`devicePixelRatio` 本身
 * 没有事件，而媒体查询在值变化时会触发 `change`。依赖里带 `dpr` 是必需的——
 * 每次变化都要换成监听"下一个值"的查询。
 */
function useDevicePixelRatio(): number {
  const [dpr, setDpr] = useState(readDevicePixelRatio);
  useEffect(() => {
    if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') {
      return;
    }
    const media = window.matchMedia(`(resolution: ${dpr}dppx)`);
    const onChange = (): void => {
      setDpr(readDevicePixelRatio());
    };
    media.addEventListener('change', onChange);
    return () => {
      media.removeEventListener('change', onChange);
    };
  }, [dpr]);
  return dpr;
}

// ---------------------------------------------------------------- 文本列

interface TextRowProps {
  readonly text: RowText;
  readonly rowHeight: number;
  readonly selected: boolean;
  readonly active: boolean;
  readonly label: string;
  readonly rowCount: number;
  readonly id: string;
}

/**
 * 一行文本。
 *
 * `aria-posinset` / `aria-setsize` 用**全局行号**而不是它在窗口里的下标：
 * 读屏软件要报的是"第 3021 条，共 12840 条"。
 */
const TextRow = memo(function TextRow({
  text,
  rowHeight,
  selected,
  active,
  label,
  rowCount,
  id,
}: TextRowProps) {
  const style: CSSProperties = {
    position: 'absolute',
    top: text.row * rowHeight,
    left: 0,
    right: 0,
    height: rowHeight,
  };
  return (
    <div
      id={id}
      role="option"
      aria-selected={selected}
      aria-posinset={text.row + 1}
      aria-setsize={rowCount}
      aria-label={label}
      data-oid={text.oid}
      data-row={text.row}
      className={cn(
        'flex items-center gap-2 pr-2',
        selected ? 'bg-surface-raised' : active ? 'bg-surface-sunken' : undefined,
        text.hidden ? 'opacity-60' : undefined,
      )}
      style={style}
    >
      <span
        aria-hidden="true"
        className={cn('h-2 w-2 shrink-0 rounded-full', laneBgClass(text.colorIndex))}
      />
      <span className="min-w-0 flex-1 truncate text-13 text-fg">{text.subject}</span>
      {text.refs.map((ref) => (
        <span
          key={`${ref.kind}:${ref.label}`}
          className={cn('shrink-0 rounded-xs border px-1 text-12', refChipClass(ref.kind))}
        >
          {ref.label}
        </span>
      ))}
      {text.collapsed > 0 ? (
        <span className="shrink-0 rounded-xs border border-line px-1 text-12 text-fg-muted">
          +{text.collapsed}
        </span>
      ) : null}
      <span className="w-32 shrink-0 truncate text-12 text-fg-muted">{text.author}</span>
      <span className="w-20 shrink-0 text-right text-12 text-fg-subtle tabular-nums">
        {text.time}
      </span>
    </div>
  );
});

interface TextColumnProps {
  readonly rows: readonly RowText[];
  readonly rowHeight: number;
  readonly selectedOids: ReadonlySet<string>;
  readonly activeOid: string | null;
  readonly rowCount: number;
  readonly rowLabel: (text: RowText) => string;
  readonly idPrefix: string;
}

/** 文本列（memo：滚动期间只有 `rows` 变，选中集不变时整列跳过重渲染）。 */
const TextColumn = memo(function TextColumn({
  rows,
  rowHeight,
  selectedOids,
  activeOid,
  rowCount,
  rowLabel,
  idPrefix,
}: TextColumnProps) {
  return (
    <>
      {rows.map((text) => (
        <TextRow
          key={text.oid}
          text={text}
          rowHeight={rowHeight}
          selected={selectedOids.has(text.oid)}
          active={text.oid === activeOid}
          label={rowLabel(text)}
          rowCount={rowCount}
          id={`${idPrefix}${text.oid}`}
        />
      ))}
    </>
  );
});

// ---------------------------------------------------------------- 主组件

/** `aria-activedescendant` 用的行 id 前缀。 */
const ROW_ID_PREFIX = 'fd-graph-row-';

function GraphCanvasInner(props: GraphCanvasProps): ReactNode {
  const {
    model,
    texts,
    listLabel,
    canvasLabel,
    minimapLabel,
    formatCollapsed,
    rowLabel,
    onActivate,
    onSelectAll,
    onHoverTarget,
    onNeedMore,
    matchOids,
    scrollToRow,
    className,
  } = props;

  const theme = useGraphTheme();
  const scale = useGraphSelectionStore((state) => state.scale);
  const minimapOpen = useGraphSelectionStore((state) => state.minimapOpen);
  const selectedOids = useSelectedOidSet();

  const containerRef = useRef<HTMLDivElement | null>(null);
  const staticCanvasRef = useRef<HTMLCanvasElement | null>(null);
  const dynamicCanvasRef = useRef<HTMLCanvasElement | null>(null);
  const minimapCanvasRef = useRef<HTMLCanvasElement | null>(null);
  const textColumnRef = useRef<HTMLDivElement | null>(null);

  /** 需要重画的层（rAF 合并用）。只在事件与回调里写，不在渲染期。 */
  const dirtyRef = useRef({ static: true, dynamic: true, minimap: true });
  const rafRef = useRef<number | null>(null);
  const fpsRef = useRef(createFpsMeter());
  /** 链路高亮缓存：只有 hover 目标变了才重新走图。 */
  const chainCacheRef = useRef<{ oid: string | null; chain: ReadonlySet<string> }>({
    oid: null,
    chain: new Set<string>(),
  });
  /** 待落地的"围绕指针缩放"（在 layout effect 里写回 scrollTop）。 */
  const pendingZoomRef = useRef<{
    prevScrollY: number;
    pointerY: number;
    prevScale: number;
    nextScale: number;
  } | null>(null);
  /** 拖拽平移的起点。 */
  const panRef = useRef<{
    pointerId: number;
    startY: number;
    startScroll: number;
    moved: boolean;
  } | null>(null);
  /** 上一次触发续页时的滚动高度（避免同一位置反复请求）。 */
  const needMoreAtRef = useRef(-1);
  /** 上一次上报给 overlay 的 hover 目标（去重，避免每次移动都触发上层渲染）。 */
  const lastHoverRef = useRef<{ oid: string; x: number; y: number } | null>(null);

  const [viewport, setViewport] = useState({ width: 0, height: 0 });
  const [activeIndex, setActiveIndex] = useState(-1);
  const [rowWindow, setRowWindow] = useState({ first: 0, last: -1 });
  const dpr = useDevicePixelRatio();

  // `useId` 的原文含冒号，拼进 id 后虽然合法（HTML5 允许除空白外的任意字符），
  // 但一旦将来有人用 CSS 选择器或 `querySelector` 找这些行就会踩转义的坑。
  // 这里只留字母数字，代价是几行正则。
  const rawId = useId();
  const rowIdPrefix = `${ROW_ID_PREFIX}${rawId.replace(/[^a-zA-Z0-9]/g, '')}-`;

  const metrics = useMemo(() => graphMetrics(scale), [scale]);
  const totalHeight = contentHeight(metrics, model.rowCount);
  const graphWidth = graphColumnWidth(metrics, model.laneCount);

  /** 行号 → `texts` 里的下标（分页拼接理论上可能出现行号空洞，因此不能假定相等）。 */
  const positionByRow = useMemo(() => {
    const map = new Map<number, number>();
    model.rows.forEach((row, position) => {
      map.set(row.row, position);
    });
    return map;
  }, [model.rows]);

  const hasRefs = useMemo(() => {
    for (const label of model.labels.values()) {
      if (label.refs.length > 0) {
        return true;
      }
    }
    return false;
  }, [model.labels]);
  const refAreaWidth = hasRefs ? Math.round(REF_AREA_BASE_WIDTH * metrics.scale) : 0;
  const textLeft = graphWidth + refAreaWidth;

  const renderIndex = useMemo(
    () => buildRenderIndex(model.edges, model.index, model.rowCount, model.lastRow),
    [model.edges, model.index, model.rowCount, model.lastRow],
  );
  const adjacency = useMemo(
    () => buildLaneAdjacency(model.edges, model.index),
    [model.edges, model.index],
  );
  const hitGrid = useMemo(
    () => buildHitGrid(model.rows, metrics, model.laneCount, model.rowCount),
    [model.rows, metrics, model.laneCount, model.rowCount],
  );
  const minimapBlocks: readonly MinimapRect[] = useMemo(() => {
    if (!minimapOpen || model.rowCount <= 0) {
      return [];
    }
    // 桶数上限 = 像素高 / 最小块高。桶再多也画不出更多像素，
    // 反而会因为不足 1px 而被抗锯齿糊成一片。
    const maxBuckets = Math.max(1, Math.floor(MINIMAP_HEIGHT / MINIMAP_MIN_BLOCK_PX));
    const bucketRows = Math.max(1, Math.ceil(model.rowCount / maxBuckets));
    const buckets = minimapBuckets(model.rows, model.rowCount, bucketRows);
    return minimapRects(buckets, model.rowCount, MINIMAP_WIDTH, MINIMAP_HEIGHT);
  }, [minimapOpen, model.rows, model.rowCount]);

  // 活动项越界时退化成"无活动项"，而不是把 state 写回去——
  // 那会在渲染期触发更新（`react-hooks/set-state-in-render` 是 error 级）。
  const safeActiveIndex = activeIndex >= 0 && activeIndex < texts.length ? activeIndex : -1;
  const activeText = safeActiveIndex >= 0 ? texts[safeActiveIndex] : undefined;
  const activeOid = activeText?.oid ?? null;
  const activeDescendant = activeOid === null ? undefined : `${rowIdPrefix}${activeOid}`;

  /** 窗口内的文本行（按行号取，空洞自然跳过）。 */
  const visibleTexts = useMemo(() => {
    const result: RowText[] = [];
    for (let row = Math.max(0, rowWindow.first); row <= rowWindow.last; row += 1) {
      const position = positionByRow.get(row);
      if (position === undefined) {
        continue;
      }
      const text = texts[position];
      if (text !== undefined) {
        result.push(text);
      }
    }
    return result;
  }, [positionByRow, texts, rowWindow.first, rowWindow.last]);

  // ------------------------------------------------------------ 重绘

  const paint = useCallback(() => {
    rafRef.current = null;
    const container = containerRef.current;
    const staticCanvas = staticCanvasRef.current;
    const dynamicCanvas = dynamicCanvasRef.current;
    if (container === null || staticCanvas === null || dynamicCanvas === null) {
      return;
    }
    const { width, height } = viewport;
    if (width <= 0 || height <= 0) {
      return;
    }
    const staticCtx = staticCanvas.getContext('2d');
    const dynamicCtx = dynamicCanvas.getContext('2d');
    // jsdom 里 getContext 返回 null；此时静默跳过，组件仍然可以挂载与被断言。
    if (staticCtx === null || dynamicCtx === null) {
      return;
    }
    const frameStart = performance.now();
    const scrollY = container.scrollTop;
    const dirty = dirtyRef.current;
    const hoverOid = useGraphSelectionStore.getState().hoverOid;

    const cached = chainCacheRef.current;
    let chain = cached.chain;
    if (cached.oid !== hoverOid) {
      chain = hoverOid === null ? new Set<string>() : walkLaneChain(adjacency, hoverOid);
      chainCacheRef.current = { oid: hoverOid, chain };
    }

    // DPR 只在这里出现：几何层与绘制层全程用 CSS px。
    staticCtx.setTransform(dpr, 0, 0, dpr, 0, 0);
    dynamicCtx.setTransform(dpr, 0, 0, dpr, 0, 0);

    let staticMs: number | null = null;
    if (dirty.static) {
      const started = performance.now();
      const range = visibleRowRange(metrics, scrollY, height, model.rowCount);
      const first = Math.max(0, range.first);
      const rows: GraphRow[] = [];
      for (let row = first; row <= range.last; row += 1) {
        const position = positionByRow.get(row);
        const entry = position === undefined ? undefined : model.rows[position];
        if (entry !== undefined) {
          rows.push(entry);
        }
      }
      drawStaticLayer({
        frame: { ctx: staticCtx, theme, metrics, scrollY, width, height },
        rows,
        edges: edgesInView(renderIndex, range.first, range.last),
        index: model.index,
        lastRow: model.lastRow,
        labels: model.labels,
        measure: createTextMeasurer(staticCtx, theme.fontStack),
        refAreaWidth,
        formatCollapsed,
      });
      dirty.static = false;
      staticMs = performance.now() - started;
    }

    let dynamicMs: number | null = null;
    if (dirty.dynamic) {
      const started = performance.now();
      drawDynamicLayer({
        frame: { ctx: dynamicCtx, theme, metrics, scrollY, width, height },
        index: model.index,
        selectedOids,
        hoverOid,
        chainOids: chain,
        matchOids,
        rowCount: model.rowCount,
      });
      dirty.dynamic = false;
      dynamicMs = performance.now() - started;
    }

    if (dirty.minimap) {
      const minimapCtx = minimapCanvasRef.current?.getContext('2d') ?? null;
      if (minimapCtx !== null) {
        minimapCtx.setTransform(dpr, 0, 0, dpr, 0, 0);
        drawMinimap({
          ctx: minimapCtx,
          theme,
          rects: minimapBlocks,
          width: MINIMAP_WIDTH,
          height: MINIMAP_HEIGHT,
          rowCount: model.rowCount,
          viewportStartRow: scrollY / metrics.rowHeight,
          viewportEndRow: (scrollY + height) / metrics.rowHeight,
        });
      }
      dirty.minimap = false;
    }

    const fps = fpsRef.current.push(frameStart);
    const heapMb = readHeapMb();
    recordGraphPerf({
      rowCount: model.rowCount,
      nodeCount: model.rows.length,
      edgeCount: model.edges.length,
      laneCount: model.laneCount,
      scale: metrics.scale,
      ...(staticMs === null ? {} : { staticDrawMs: staticMs }),
      ...(dynamicMs === null ? {} : { dynamicDrawMs: dynamicMs }),
      ...(fps === null ? {} : { fps }),
      ...(heapMb === null ? {} : { heapMb }),
    });
  }, [
    viewport,
    dpr,
    metrics,
    theme,
    model,
    positionByRow,
    renderIndex,
    adjacency,
    selectedOids,
    matchOids,
    refAreaWidth,
    formatCollapsed,
    minimapBlocks,
  ]);

  const schedule = useCallback(
    (layers?: {
      readonly static?: boolean;
      readonly dynamic?: boolean;
      readonly minimap?: boolean;
    }) => {
      const dirty = dirtyRef.current;
      if (layers === undefined) {
        dirty.static = true;
        dirty.dynamic = true;
        dirty.minimap = true;
      } else {
        dirty.static = dirty.static || layers.static === true;
        dirty.dynamic = dirty.dynamic || layers.dynamic === true;
        dirty.minimap = dirty.minimap || layers.minimap === true;
      }
      if (rafRef.current !== null) {
        return;
      }
      rafRef.current = requestAnimationFrame(paint);
    },
    [paint],
  );

  // 影响像素的东西变了（数据 / 缩放 / 主题 / 视口 / 选中集）就整体重画。
  // hover 不走这里：它由下面的 store 订阅单独触发，避免 React 渲染。
  useLayoutEffect(() => {
    schedule();
  }, [schedule]);

  // 卸载时取消未执行的 rAF，否则会在已卸载的 canvas 上绘制。
  useEffect(() => {
    return () => {
      if (rafRef.current !== null) {
        cancelAnimationFrame(rafRef.current);
        rafRef.current = null;
      }
    };
  }, []);

  // 订阅 store：hover / 选中 / 比较基准变化时只重画动态层。
  useEffect(() => {
    let previous = useGraphSelectionStore.getState();
    return useGraphSelectionStore.subscribe((next) => {
      if (
        next.hoverOid !== previous.hoverOid ||
        next.selectedOids !== previous.selectedOids ||
        next.compareBaseOid !== previous.compareBaseOid
      ) {
        schedule({ dynamic: true });
      }
      previous = next;
    });
  }, [schedule]);

  // ------------------------------------------------------------ 尺寸与缩放

  useEffect(() => {
    const element = containerRef.current;
    if (element === null || typeof ResizeObserver === 'undefined') {
      return;
    }
    const observer = new ResizeObserver((entries) => {
      const box = entries[0]?.contentRect;
      if (box === undefined) {
        return;
      }
      const width = Math.round(box.width);
      const height = Math.round(box.height);
      // 尺寸没变时返回原对象：ResizeObserver 会因为我们的 sticky 视口改高
      // 而再次回调，返回新对象就会形成"回调 → 渲染 → 回调"的死循环。
      setViewport((prev) =>
        prev.width === width && prev.height === height ? prev : { width, height },
      );
    });
    observer.observe(element);
    return () => {
      observer.disconnect();
    };
  }, []);

  // 首屏与尺寸/数据变化时也要给出可见行窗口。
  //
  // 为什么不能只靠 `handleScroll`：文本列的行是按 `rowWindow` 窗口渲染的，而窗口
  // 只在滚动回调里更新。于是**内容不足一屏（没有滚动条）时永远收不到滚动事件**，
  // 窗口停在下限 `{ first: 0, last: -1 }`——历史页上只剩泳道图与 ref 标签，一条
  // 提交信息都不显示；提交多到能滚动时，也要等用户先滚一下文字才出现。
  // 2026-10-08 由官网截图流程暴露（截图里 15 条提交的页面没有任何提交文字）。
  //
  // 这里复刻 `handleScroll` 的同一段计算（同参数、同 overscan），保证"挂载即等价
  // 于滚到当前位置"，不引入第二套窗口语义。
  useEffect(() => {
    const container = containerRef.current;
    const height = container === null ? viewport.height : container.clientHeight;
    if (height <= 0) {
      return;
    }
    const range = visibleRowRange(
      metrics,
      container === null ? 0 : container.scrollTop,
      height,
      model.rowCount,
      DOM_OVERSCAN_ROWS,
    );
    setRowWindow((prev) => (prev.first === range.first && prev.last === range.last ? prev : range));
  }, [metrics, model.rowCount, viewport.height]);

  // canvas 后备缓冲区 = CSS 尺寸 × DPR。少了这一步，高分屏上整张图都是糊的。
  useLayoutEffect(() => {
    const width = Math.max(1, Math.round(viewport.width * dpr));
    const height = Math.max(1, Math.round(viewport.height * dpr));
    const staticCanvas = staticCanvasRef.current;
    const dynamicCanvas = dynamicCanvasRef.current;
    const minimapCanvas = minimapCanvasRef.current;
    if (staticCanvas !== null) {
      staticCanvas.width = width;
      staticCanvas.height = height;
    }
    if (dynamicCanvas !== null) {
      dynamicCanvas.width = width;
      dynamicCanvas.height = height;
    }
    if (minimapCanvas !== null) {
      minimapCanvas.width = Math.max(1, Math.round(MINIMAP_WIDTH * dpr));
      minimapCanvas.height = Math.max(1, Math.round(MINIMAP_HEIGHT * dpr));
    }
    schedule();
  }, [viewport.width, viewport.height, dpr, schedule]);

  // 缩放落地：新 scale 已经进了 DOM（内容高度变了），此时才能把 scrollTop 写到
  // "指针下的内容不动"的位置。放在 layout effect 里是为了赶在浏览器绘制之前——
  // 否则用户会看到一帧跳变。
  useLayoutEffect(() => {
    const pending = pendingZoomRef.current;
    const container = containerRef.current;
    if (pending === null || container === null) {
      return;
    }
    pendingZoomRef.current = null;
    container.scrollTop = Math.max(
      0,
      zoomAround(pending.prevScrollY, pending.pointerY, pending.prevScale, pending.nextScale),
    );
  }, [scale]);

  /** 以视口内某点为锚缩放（滚轮与快捷键共用）。 */
  const zoomAt = useCallback((pointerY: number, factor: number) => {
    const container = containerRef.current;
    if (container === null) {
      return;
    }
    const store = useGraphSelectionStore.getState();
    const prevScale = store.scale;
    const nextScale = clampScale(prevScale * factor);
    if (nextScale === prevScale) {
      return;
    }
    pendingZoomRef.current = {
      prevScrollY: container.scrollTop,
      pointerY,
      prevScale,
      nextScale,
    };
    store.setScale(nextScale);
  }, []);

  // ctrl/cmd + 滚轮 = 缩放。必须用**原生**监听器：React 把 wheel 注册成 passive，
  // `preventDefault()` 在里面无效，页面会一边缩放一边滚动。
  useEffect(() => {
    const element = containerRef.current;
    if (element === null) {
      return;
    }
    const handler = (event: WheelEvent): void => {
      if (!event.ctrlKey && !event.metaKey) {
        return;
      }
      event.preventDefault();
      const rect = element.getBoundingClientRect();
      zoomAt(event.clientY - rect.top, event.deltaY < 0 ? ZOOM_STEP : 1 / ZOOM_STEP);
    };
    element.addEventListener('wheel', handler, { passive: false });
    return () => {
      element.removeEventListener('wheel', handler);
    };
  }, [zoomAt]);

  // ------------------------------------------------------------ 命中与指针

  /** 客户端坐标 → 该行的 oid（节点命中优先，否则按行号兜底）。 */
  const oidAt = useCallback(
    (clientX: number, clientY: number): string | null => {
      const container = containerRef.current;
      if (container === null) {
        return null;
      }
      const rect = container.getBoundingClientRect();
      const target: HitTarget | null = hitTest(
        hitGrid,
        metrics,
        model.rowCount,
        container.scrollTop,
        rect,
        clientX,
        clientY,
      );
      if (target === null) {
        return null;
      }
      if (target.oid !== null) {
        return target.oid;
      }
      // 点在一行的空白处也算点中那一行：只有节点能点会让人觉得"点不中"，
      // 而提交列表的通用预期是整行可点。
      const position = positionByRow.get(target.row);
      return position === undefined ? null : (model.rows[position]?.oid ?? null);
    },
    [hitGrid, metrics, model.rowCount, model.rows, positionByRow],
  );

  /** 上报 hover 目标（按 oid + 位置去重，避免每次指针移动都渲染上层）。 */
  const reportHover = useCallback(
    (oid: string | null) => {
      const store = useGraphSelectionStore.getState();
      if (oid !== store.hoverOid) {
        store.setHoverOid(oid);
      }
      if (oid === null) {
        lastHoverRef.current = null;
        setActiveIndex(-1);
        onHoverTarget(null);
        return;
      }
      const container = containerRef.current;
      const row = model.index.get(oid);
      if (container === null || row === undefined) {
        return;
      }
      const nodeBox = nodeRect(metrics, row);
      // x 是内容坐标，但容器没有横向滚动也没有边框/内边距，
      // 因此它同时就是 sticky 视口（= 卡片定位基准）里的 x。
      const x = nodeBox.x + nodeBox.w + HOVER_CARD_GAP;
      const y = nodeBox.y + nodeBox.h / 2 - container.scrollTop;
      const previous = lastHoverRef.current;
      if (previous !== null && previous.oid === oid && previous.x === x && previous.y === y) {
        return;
      }
      lastHoverRef.current = { oid, x, y };
      const position = positionByRow.get(row.row);
      setActiveIndex((prev) => (prev === position ? prev : (position ?? -1)));
      onHoverTarget({ oid, text: position === undefined ? undefined : texts[position], x, y });
    },
    [model.index, metrics, positionByRow, texts, onHoverTarget],
  );

  const handlePointerMove = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      const pan = panRef.current;
      if (pan !== null && pan.pointerId === event.pointerId) {
        const deltaY = event.clientY - pan.startY;
        if (!pan.moved && Math.abs(deltaY) > PAN_THRESHOLD_PX) {
          pan.moved = true;
        }
        if (pan.moved) {
          const container = containerRef.current;
          if (container !== null) {
            container.scrollTop = pan.startScroll - deltaY;
          }
          return;
        }
      }
      reportHover(oidAt(event.clientX, event.clientY));
    },
    [oidAt, reportHover],
  );

  const handlePointerDown = useCallback((event: ReactPointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) {
      return;
    }
    const container = containerRef.current;
    if (container === null) {
      return;
    }
    panRef.current = {
      pointerId: event.pointerId,
      startY: event.clientY,
      startScroll: container.scrollTop,
      moved: false,
    };
    // 捕获指针：否则拖到容器外就收不到 move/up，平移会"卡住"。
    // jsdom 没有这个方法，因此先探测。
    const target = event.currentTarget;
    if (typeof target.setPointerCapture === 'function') {
      target.setPointerCapture(event.pointerId);
    }
  }, []);

  const handlePointerUp = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      const pan = panRef.current;
      panRef.current = null;
      if (pan === null || pan.pointerId !== event.pointerId) {
        return;
      }
      const target = event.currentTarget;
      if (
        typeof target.releasePointerCapture === 'function' &&
        typeof target.hasPointerCapture === 'function' &&
        target.hasPointerCapture(event.pointerId)
      ) {
        target.releasePointerCapture(event.pointerId);
      }
      if (pan.moved) {
        return;
      }
      const oid = oidAt(event.clientX, event.clientY);
      if (oid === null) {
        // 点在最后一行之外的空白处：清空选择（与文件管理器一致，
        // 不清会让人以为还选着刚才那批）。
        useGraphSelectionStore.getState().clearSelection();
        reportHover(null);
        return;
      }
      onActivate(oid, {
        additive: event.ctrlKey || event.metaKey,
        range: event.shiftKey,
      });
    },
    [oidAt, onActivate, reportHover],
  );

  const handlePointerLeave = useCallback(() => {
    panRef.current = null;
    reportHover(null);
  }, [reportHover]);

  const handleContextMenu = useCallback(
    (event: ReactMouseEvent<HTMLDivElement>) => {
      // 右键也要更新 hover 目标：`GraphOverlay` 用它决定菜单内容与是否弹菜单。
      // 键盘触发的 contextmenu（菜单键）clientX/Y 为 0，此时保持现有 hover 不动。
      if (event.clientX !== 0 || event.clientY !== 0) {
        reportHover(oidAt(event.clientX, event.clientY));
      }
    },
    [oidAt, reportHover],
  );

  const handleScroll = useCallback(
    (event: ReactUIEvent<HTMLDivElement>) => {
      const container = event.currentTarget;
      const scrollY = container.scrollTop;
      // 文本列用命令式 transform 跟随滚动：走 state 会让每滚一像素都重渲染。
      const column = textColumnRef.current;
      if (column !== null) {
        column.style.transform = `translateY(${-scrollY}px)`;
      }
      dirtyRef.current.static = true;
      dirtyRef.current.dynamic = true;
      dirtyRef.current.minimap = true;
      if (rafRef.current === null) {
        rafRef.current = requestAnimationFrame(paint);
      }
      const range = visibleRowRange(
        metrics,
        scrollY,
        viewport.height,
        model.rowCount,
        DOM_OVERSCAN_ROWS,
      );
      setRowWindow((prev) =>
        prev.first === range.first && prev.last === range.last ? prev : range,
      );
      // 续页去重用"上次触发时的滚动高度"：数据到达后 scrollHeight 会变，
      // 于是同一个位置不会被反复请求。
      const distanceToEnd = container.scrollHeight - scrollY - container.clientHeight;
      if (
        distanceToEnd <= LOAD_MORE_THRESHOLD_PX &&
        needMoreAtRef.current !== container.scrollHeight
      ) {
        needMoreAtRef.current = container.scrollHeight;
        onNeedMore();
      }
    },
    [paint, metrics, viewport.height, model.rowCount, onNeedMore],
  );

  // ------------------------------------------------------------ 键盘

  const scrollRowIntoView = useCallback(
    (row: number) => {
      const container = containerRef.current;
      if (container === null) {
        return;
      }
      const top = row * metrics.rowHeight;
      const bottom = top + metrics.rowHeight;
      if (top < container.scrollTop) {
        container.scrollTop = top;
      } else if (bottom > container.scrollTop + container.clientHeight) {
        container.scrollTop = bottom - container.clientHeight;
      }
    },
    [metrics.rowHeight],
  );

  const moveActive = useCallback(
    (delta: number, modifiers: SelectionModifiers) => {
      if (texts.length === 0) {
        return;
      }
      // 还没有活动项时，↓ 选第一行、↑ 选最后一行（与列表控件的惯例一致）。
      const current = safeActiveIndex >= 0 ? safeActiveIndex : delta > 0 ? -1 : 0;
      const next = Math.min(texts.length - 1, Math.max(0, current + delta));
      const text = texts[next];
      if (text === undefined) {
        return;
      }
      setActiveIndex(next);
      scrollRowIntoView(text.row);
      onActivate(text.oid, modifiers);
    },
    [texts, safeActiveIndex, scrollRowIntoView, onActivate],
  );

  const handleKeyDown = useCallback(
    (event: ReactKeyboardEvent<HTMLDivElement>) => {
      const modifiers: SelectionModifiers = {
        additive: event.ctrlKey || event.metaKey,
        range: event.shiftKey,
      };
      const pageRows = Math.max(1, Math.floor(viewport.height / metrics.rowHeight));
      switch (event.key) {
        case 'ArrowDown':
          event.preventDefault();
          moveActive(1, modifiers);
          break;
        case 'ArrowUp':
          event.preventDefault();
          moveActive(-1, modifiers);
          break;
        case 'PageDown':
          event.preventDefault();
          moveActive(pageRows, modifiers);
          break;
        case 'PageUp':
          event.preventDefault();
          moveActive(-pageRows, modifiers);
          break;
        case 'Home':
          event.preventDefault();
          moveActive(-texts.length - 1, modifiers);
          break;
        case 'End':
          event.preventDefault();
          moveActive(texts.length + 1, modifiers);
          break;
        case 'Enter':
        case ' ': {
          event.preventDefault();
          const text = safeActiveIndex >= 0 ? texts[safeActiveIndex] : undefined;
          if (text !== undefined) {
            onActivate(text.oid, modifiers);
          }
          break;
        }
        case '+':
        case '=':
          event.preventDefault();
          zoomAt(viewport.height / 2, ZOOM_STEP);
          break;
        case '-':
        case '_':
          event.preventDefault();
          zoomAt(viewport.height / 2, 1 / ZOOM_STEP);
          break;
        case '0':
          event.preventDefault();
          useGraphSelectionStore.getState().setScale(1);
          break;
        case 'a':
          // 只拦截带修饰键的 Ctrl/Cmd+A；裸 a 留给浏览器与未来的快捷键。
          if (event.ctrlKey || event.metaKey) {
            event.preventDefault();
            onSelectAll();
          }
          break;
        default:
          break;
      }
    },
    [
      moveActive,
      texts,
      safeActiveIndex,
      viewport.height,
      metrics.rowHeight,
      zoomAt,
      onSelectAll,
      onActivate,
    ],
  );

  // ------------------------------------------------------------ 滚动跳转（T2.3 搜索）

  // token 变化即触发：同一行重复请求也要滚（用户可能连按两次"下一处"中间
  // 手动挪了滚动条）。滚到**视口中部**——顶端对齐会让目标行贴着上边框，
  // 看不到它的上下文。
  useEffect(() => {
    if (scrollToRow === null || scrollToRow === undefined) {
      return;
    }
    const container = containerRef.current;
    if (container === null) {
      return;
    }
    const top = scrollToRow.row * metrics.rowHeight;
    container.scrollTop = Math.max(0, top - Math.max(0, viewport.height - metrics.rowHeight) / 2);
  }, [scrollToRow, metrics.rowHeight, viewport.height]);

  // ------------------------------------------------------------ 迷你地图

  /** 把指针在迷你地图上的 y 换算成滚动位置（按下与拖动共用）。 */
  const scrollToMinimapRatio = useCallback(
    (clientY: number) => {
      const container = containerRef.current;
      const canvas = minimapCanvasRef.current;
      if (container === null || canvas === null || model.rowCount <= 0) {
        return;
      }
      const rect = canvas.getBoundingClientRect();
      const ratio = Math.min(1, Math.max(0, (clientY - rect.top) / Math.max(1, rect.height)));
      container.scrollTop = Math.max(
        0,
        ratio * model.rowCount * metrics.rowHeight - viewport.height / 2,
      );
    },
    [model.rowCount, metrics.rowHeight, viewport.height],
  );

  // 按下即跳 + 按住拖动跟随（T2.3：可拖动跳转）。setPointerCapture 让指针
  // 移出画布后 move 事件仍打到画布上，松手才停止。
  const handleMinimapPointerDown = useCallback(
    (event: ReactPointerEvent<HTMLCanvasElement>) => {
      if (model.rowCount <= 0) {
        return;
      }
      event.preventDefault();
      event.currentTarget.setPointerCapture(event.pointerId);
      scrollToMinimapRatio(event.clientY);
    },
    [model.rowCount, scrollToMinimapRatio],
  );

  const handleMinimapPointerMove = useCallback(
    (event: ReactPointerEvent<HTMLCanvasElement>) => {
      // 只在按住主键拖动时跟随；悬停（无按键）不滚动
      if (event.buttons & 1) {
        scrollToMinimapRatio(event.clientY);
      }
    },
    [scrollToMinimapRatio],
  );

  // ------------------------------------------------------------ 渲染

  const canvasStyle: CSSProperties = { width: viewport.width, height: viewport.height };

  return (
    <div
      ref={containerRef}
      className={cn(
        'relative min-h-0 flex-1 overflow-x-hidden overflow-y-auto bg-canvas',
        className,
      )}
      onScroll={handleScroll}
    >
      <div className="relative" style={{ height: totalHeight }}>
        <div className="sticky top-0 overflow-hidden" style={{ height: viewport.height }}>
          <canvas
            ref={staticCanvasRef}
            role="img"
            aria-label={canvasLabel}
            className="absolute inset-0"
            style={canvasStyle}
          />
          <canvas
            ref={dynamicCanvasRef}
            aria-hidden="true"
            className="pointer-events-none absolute inset-0"
            style={canvasStyle}
          />
          <div
            role="listbox"
            aria-label={listLabel}
            aria-multiselectable="true"
            aria-activedescendant={activeDescendant}
            tabIndex={0}
            data-testid="graph-hit-layer"
            className={cn(
              'absolute inset-0 select-none outline-none',
              'focus-visible:ring-2 focus-visible:ring-brand focus-visible:ring-inset',
            )}
            onPointerMove={handlePointerMove}
            onPointerDown={handlePointerDown}
            onPointerUp={handlePointerUp}
            onPointerCancel={handlePointerLeave}
            onPointerLeave={handlePointerLeave}
            onContextMenu={handleContextMenu}
            onKeyDown={handleKeyDown}
          >
            {/* 中间两层是纯布局容器；role="presentation" 让读屏软件把 option
                当成 listbox 的直接子项（ARIA 要求 option 的父级是 listbox，
                否则整棵树的可选择性会被忽略）。 */}
            <div
              role="presentation"
              className="absolute inset-y-0 right-0"
              style={{ left: textLeft }}
            >
              <div
                role="presentation"
                ref={textColumnRef}
                className="absolute inset-0 will-change-transform"
              >
                <TextColumn
                  rows={visibleTexts}
                  rowHeight={metrics.rowHeight}
                  selectedOids={selectedOids}
                  activeOid={activeOid}
                  rowCount={model.rowCount}
                  rowLabel={rowLabel}
                  idPrefix={rowIdPrefix}
                />
              </div>
            </div>
          </div>
          {minimapOpen ? (
            <canvas
              ref={minimapCanvasRef}
              role="img"
              aria-label={minimapLabel}
              data-testid="graph-minimap"
              className="absolute cursor-pointer rounded-sm border border-line bg-surface shadow-sm"
              style={{
                right: MINIMAP_MARGIN,
                bottom: MINIMAP_MARGIN,
                width: MINIMAP_WIDTH,
                height: MINIMAP_HEIGHT,
              }}
              onPointerDown={handleMinimapPointerDown}
              onPointerMove={handleMinimapPointerMove}
            />
          ) : null}
        </div>
      </div>
    </div>
  );
}

export const GraphCanvas = memo(GraphCanvasInner);
