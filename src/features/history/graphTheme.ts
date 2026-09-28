/**
 * Canvas 用的配色：从 CSS 变量读出设计 token（T2.2）。
 *
 * # 为什么需要这一层
 *
 * Canvas 2D 的 `fillStyle` / `strokeStyle` 只接受**具体颜色字符串**，
 * 写 `var(--fd-brand)` 是无效的（不像 DOM 那样由样式系统解析）。
 * 而 `docs/CODING_STYLE.md` §3.5 明令禁止在组件里硬编码颜色——
 * 于是唯一的出路是运行时用 `getComputedStyle` 把 token 读出来。
 *
 * 这样做的额外好处：主题切换（`<html data-theme>`）时重新读一次即可，
 * Canvas 与 DOM 永远用同一份色值，不会出现"切了暗色但图还是亮色"。
 *
 * # 降级策略
 *
 * `readGraphTheme` 是**纯函数**（接收一个取值回调），因此可以在没有 DOM 的
 * 环境（单测、SSR）里直接测。取不到值时兜底到**另一个 token**，
 * 而不是写死一个十六进制色值——宁可颜色不够好看，也不能在 TS 里出现字面色值，
 * 否则 `check:contrast` 的护栏就被绕过去了。
 */
import type { RefKind } from '@/features/history/graphGeometry';

/** 泳道调色板长度；必须与后端 `domain::history::layout::PALETTE_SIZE` 一致。 */
export const LANE_PALETTE_SIZE = 8;

/** Canvas 渲染需要的全部颜色（全部是已解析的具体色值字符串）。 */
export interface GraphTheme {
  /** 8 色泳道调色板，下标即 `GraphRow.colorIndex`。 */
  readonly lanes: readonly string[];
  /** 本地分支胶囊。 */
  readonly refLocal: string;
  /** 远端分支胶囊。 */
  readonly refRemote: string;
  /** 标签胶囊。 */
  readonly refTag: string;
  /** 选中环（中性墨色，实线）。 */
  readonly selected: string;
  /** hover 环（品牌靛蓝，虚线）。 */
  readonly hover: string;
  /** 画布底色。 */
  readonly canvas: string;
  /** 行带底色（hover 整行高亮）。 */
  readonly rowBand: string;
  /** 连线在"淡化"状态下的颜色（hidden 行）。 */
  readonly line: string;
  /** 主文字色（折叠徽标里的数字等）。 */
  readonly fg: string;
  /** 次要文字色。 */
  readonly fgMuted: string;
  /** 实底胶囊上的反色文字。 */
  readonly fgInverted: string;
  /**
   * Canvas 用的字体栈（CSS `font-family` 值）。
   *
   * 与颜色同理：`ctx.font` 只接受具体字符串，写 `var(--fd-font-sans)` 无效。
   * 从 token 读出来才能保证图里的首字母与 DOM 里的正文是同一副字体，
   * 否则中英文混排时两者的字宽节奏会对不上。
   */
  readonly fontStack: string;
}

/** 取值回调的形状（`CSSStyleDeclaration.getPropertyValue` 兼容）。 */
export type CssVarReader = (name: string) => string;

/**
 * 从 CSS 变量解析出 Canvas 配色。
 *
 * @param read 取值回调，传入不带 `--` 前缀的变量名（如 `fd-graph-lane-0`）。
 */
export function readGraphTheme(read: CssVarReader): GraphTheme {
  // 兜底链：graph 专用色 → 通用语义色。
  // 之所以兜到 `fd-fg-subtle` 这类**一定存在**的 token，是因为它在 T0.3 就被
  // `check:contrast` 守着；万一将来有人删了 graph token，图会变成灰调而不是崩掉。
  const fallback = read('fd-fg-subtle').trim() || 'transparent';
  const pick = (name: string): string => read(name).trim() || fallback;

  const lanes: string[] = [];
  for (let index = 0; index < LANE_PALETTE_SIZE; index += 1) {
    lanes.push(pick(`fd-graph-lane-${index}`));
  }

  return {
    lanes,
    refLocal: pick('fd-graph-ref-local'),
    refRemote: pick('fd-graph-ref-remote'),
    refTag: pick('fd-graph-ref-tag'),
    selected: pick('fd-graph-selected'),
    hover: pick('fd-graph-hover'),
    canvas: pick('fd-canvas'),
    rowBand: pick('fd-surface-sunken'),
    line: pick('fd-line'),
    fg: pick('fd-fg'),
    fgMuted: pick('fd-fg-muted'),
    fgInverted: pick('fd-fg-inverted'),
    // 兜底用 CSS 通用族名（不是具体字体，也不是颜色，不构成硬编码）
    fontStack: read('fd-font-sans').trim() || 'sans-serif',
  };
}

/**
 * 读取当前文档的 Canvas 配色。
 *
 * 单独一个函数（而不是在组件里直接写 `getComputedStyle`）：
 * 这样"依赖 DOM 的那一行"只有一处，纯函数 `readGraphTheme` 可以被完整单测。
 */
export function readGraphThemeFromDocument(): GraphTheme {
  const style = getComputedStyle(document.documentElement);
  return readGraphTheme((name) => style.getPropertyValue(`--${name}`));
}

/**
 * 按 `colorIndex` 取泳道色。
 *
 * 取模而不是直接下标：后端的 `color_index = lane % PALETTE_SIZE` 已经是 0..7，
 * 但布局若来自更早的版本（或夹具写错）越界时，`noUncheckedIndexedAccess`
 * 会给出 `undefined`——那会让 `fillStyle = undefined` 静默变成透明。
 * 兜到 0 号色至少保证"看得见"。
 */
export function laneColor(theme: GraphTheme, colorIndex: number): string {
  const index = ((colorIndex % LANE_PALETTE_SIZE) + LANE_PALETTE_SIZE) % LANE_PALETTE_SIZE;
  return theme.lanes[index] ?? theme.lanes[0] ?? 'transparent';
}

// ---------------------------------------------------------------- DOM 侧的同一批 token

/**
 * 泳道背景色工具类（下标 = `colorIndex`）。
 *
 * 为什么写成**字面量数组**而不是 `` `bg-graph-lane-${i}` ``：
 * Tailwind 4 靠扫描源码里的完整字符串生成工具类，拼接出来的类名
 * 永远不会被扫到，运行时就是一个不存在的类（颜色静默丢失）。
 * 八行重复是可读性与正确性之间的必要代价。
 *
 * 为什么 DOM 侧也要泳道色：列表模式与详情面板里的分支色点、
 * hover 卡片里的色条，必须与 Canvas 上的节点同色，否则用户无法把
 * "图里那个蓝色的圈"与"列表里那个蓝色的点"对应起来。
 */
export const LANE_BG_CLASSES: readonly string[] = [
  'bg-graph-lane-0',
  'bg-graph-lane-1',
  'bg-graph-lane-2',
  'bg-graph-lane-3',
  'bg-graph-lane-4',
  'bg-graph-lane-5',
  'bg-graph-lane-6',
  'bg-graph-lane-7',
];

/** 泳道前景（文字/边框）色工具类，与 `LANE_BG_CLASSES` 一一对应。 */
export const LANE_TEXT_CLASSES: readonly string[] = [
  'text-graph-lane-0',
  'text-graph-lane-1',
  'text-graph-lane-2',
  'text-graph-lane-3',
  'text-graph-lane-4',
  'text-graph-lane-5',
  'text-graph-lane-6',
  'text-graph-lane-7',
];

/** 把任意整数归一到 `0..LANE_PALETTE_SIZE-1`（与 `laneColor` 同一口径）。 */
function laneIndex(colorIndex: number): number {
  return ((colorIndex % LANE_PALETTE_SIZE) + LANE_PALETTE_SIZE) % LANE_PALETTE_SIZE;
}

/** 按 `colorIndex` 取泳道背景类（越界兜到 0 号，理由同 `laneColor`）。 */
export function laneBgClass(colorIndex: number): string {
  return LANE_BG_CLASSES[laneIndex(colorIndex)] ?? LANE_BG_CLASSES[0] ?? 'bg-graph-lane-0';
}

/** 按 `colorIndex` 取泳道文字/边框类。 */
export function laneTextClass(colorIndex: number): string {
  return LANE_TEXT_CLASSES[laneIndex(colorIndex)] ?? LANE_TEXT_CLASSES[0] ?? 'text-graph-lane-0';
}

/**
 * ref 芯片的 DOM 样式（与 Canvas 侧的三种画法对应）。
 *
 * Canvas 靠"实底 / 实线描边 / 虚线描边"区分三种 ref。DOM 侧沿用同一套编码：
 * local 实底反字、remote 实线描边、tag 虚线描边——两边完全一致，
 * 用户把图上看到的胶囊与列表里的芯片对应起来时不需要重新学一套规则。
 * 色相也共用同一批 token（local 酒红 / remote 钢青 / tag 青铜）。
 *
 * 同样必须是字面量（见 `LANE_BG_CLASSES` 的理由）。
 */
export const REF_CHIP_CLASSES: Readonly<Record<RefKind, string>> = {
  local: 'border-transparent bg-graph-ref-local text-fg-inverted',
  remote: 'border-graph-ref-remote bg-surface text-graph-ref-remote',
  tag: 'border-graph-ref-tag border-dashed bg-surface text-graph-ref-tag',
};

/** 按 ref 类型取芯片类。 */
export function refChipClass(kind: RefKind): string {
  return REF_CHIP_CLASSES[kind];
}
