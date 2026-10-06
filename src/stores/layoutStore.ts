/**
 * 布局状态（T5.10）：持久化为 JSON 存 settings 表（`ui.layout`）。
 *
 * # 容错（任务书：非法 JSON → 回退默认并提示）
 *
 * settings 表的值由调用方（设置页加载路径）传给 [`parseLayout`]——
 * 任何解析失败/形状不符都回退默认并返回错误信息，由调用方 toast。
 *
 * # 预设
 *
 * Default（平衡）/ review（大 diff：详情置底）/ terminal（导航到终端页）/
 * editor（编辑器模式：树收窄）。预设只改**本 store 的字段**并持久化；
 * "终端模式"额外导航（由调用方处理，store 不耦合路由）。
 */
import { create } from 'zustand';

/** 布局预设名。 */
export const LAYOUT_PRESETS = ['default', 'review', 'editor'] as const;
export type LayoutPreset = (typeof LAYOUT_PRESETS)[number];

/** 布局快照（持久化形状）。 */
export interface LayoutState {
  readonly preset: LayoutPreset;
  /** 文件树宽度（px，200..=480）。 */
  readonly treeWidth: number;
  /** 详情面板位置（复用 uiStore 的语义，持久化在此避免双写）。 */
  readonly detailPanel: 'right' | 'bottom' | 'hidden';
}

export const DEFAULT_LAYOUT: LayoutState = {
  preset: 'default',
  treeWidth: 264,
  detailPanel: 'right',
};

/** 解析持久化值；非法 → 默认 + 错误消息（调用方提示"布局已重置"）。 */
export function parseLayout(raw: string | undefined): {
  layout: LayoutState;
  error: string | null;
} {
  if (raw === undefined || raw === '') {
    return { layout: { ...DEFAULT_LAYOUT }, error: null };
  }
  try {
    const parsed: unknown = JSON.parse(raw);
    if (typeof parsed !== 'object' || parsed === null) {
      return { layout: { ...DEFAULT_LAYOUT }, error: 'layout is not an object' };
    }
    const source = parsed as Record<string, unknown>;
    const preset = LAYOUT_PRESETS.includes(source['preset'] as LayoutPreset)
      ? (source['preset'] as LayoutPreset)
      : DEFAULT_LAYOUT.preset;
    const treeWidthRaw = source['treeWidth'];
    const treeWidth =
      typeof treeWidthRaw === 'number' && Number.isFinite(treeWidthRaw)
        ? Math.min(480, Math.max(200, Math.round(treeWidthRaw)))
        : DEFAULT_LAYOUT.treeWidth;
    const detailPanel =
      source['detailPanel'] === 'bottom' ||
      source['detailPanel'] === 'hidden' ||
      source['detailPanel'] === 'right'
        ? source['detailPanel']
        : DEFAULT_LAYOUT.detailPanel;
    return { layout: { preset, treeWidth, detailPanel }, error: null };
  } catch (error) {
    return {
      layout: { ...DEFAULT_LAYOUT },
      error: error instanceof Error ? error.message : String(error),
    };
  }
}

export const LAYOUT_KEY = 'ui.layout';

export interface LayoutStoreState {
  readonly layout: LayoutState;
  /** 布局损坏提示（设置页显示一次）。 */
  readonly corrupted: boolean;
  setLayout(layout: LayoutState): void;
  setPreset(preset: LayoutPreset): void;
  setTreeWidth(width: number): void;
  resetToDefault(): void;
  /** 从 settings 表载入（App 启动调用一次）。 */
  hydrate(raw: string | undefined): void;
  markCorruptionHandled(): void;
}

export const initialLayoutState = {
  layout: { ...DEFAULT_LAYOUT },
  corrupted: false,
};

export const useLayoutStore = create<LayoutStoreState>()((set) => ({
  ...initialLayoutState,

  setLayout: (layout) => {
    set({ layout });
  },

  setPreset: (preset) => {
    set((state) => ({ layout: { ...state.layout, preset } }));
  },

  setTreeWidth: (width) => {
    set((state) => ({
      layout: { ...state.layout, treeWidth: Math.min(480, Math.max(200, Math.round(width))) },
    }));
  },

  resetToDefault: () => {
    set({ layout: { ...DEFAULT_LAYOUT } });
  },

  hydrate: (raw) => {
    const { layout, error } = parseLayout(raw);
    set({ layout, corrupted: error !== null });
  },

  markCorruptionHandled: () => {
    set({ corrupted: false });
  },
}));
