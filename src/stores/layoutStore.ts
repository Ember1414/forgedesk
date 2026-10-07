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

import { isTauriRuntime, settingsSet } from '@/lib/ipc';

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
  /**
   * 详情面板尺寸（px；right=宽度 / bottom=高度）。
   * `null` = 用户没拖过，跟随各位置的默认值——不能写死一个数：
   * 同一个值当"右侧宽度"合适、当"底部高度"就离谱。
   */
  readonly detailSize: number | null;
}

/** 详情面板的缺省尺寸（与 w-72 / h-28 一致，保持升级前后观感不变）。 */
export const DETAIL_DEFAULT_WIDTH = 288;
export const DETAIL_DEFAULT_HEIGHT = 112;

export const DEFAULT_LAYOUT: LayoutState = {
  preset: 'default',
  treeWidth: 264,
  detailPanel: 'right',
  detailSize: null,
};

/** 详情面板尺寸的边界（拖拽与解析共用；同一套钳制避免"存进去读出来变了"）。 */
export const DETAIL_SIZE_LIMITS = {
  horizontal: { min: 240, max: 640 } as const,
  vertical: { min: 96, max: 480 } as const,
} as const;

/** 按方向钳制详情面板尺寸；非有限数一律回 null（跟随默认）。 */
export function clampDetailSize(
  size: number | null,
  orientation: 'horizontal' | 'vertical',
): number | null {
  if (size === null || !Number.isFinite(size)) {
    return null;
  }
  const { min, max } = DETAIL_SIZE_LIMITS[orientation];
  return Math.min(max, Math.max(min, Math.round(size)));
}

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
    // 旧版布局 JSON 没有 detailSize：缺省即 null（跟随默认），不算损坏
    const detailSizeRaw = source['detailSize'];
    const detailSize =
      typeof detailSizeRaw === 'number' ? clampDetailSize(detailSizeRaw, 'horizontal') : null;
    return { layout: { preset, treeWidth, detailPanel, detailSize }, error: null };
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
  /** 拖拽详情面板后记录尺寸（orientation 决定钳制边界）。 */
  setDetailSize(size: number, orientation: 'horizontal' | 'vertical'): void;
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

  setDetailSize: (size, orientation) => {
    set((state) => ({
      layout: { ...state.layout, detailSize: clampDetailSize(size, orientation) },
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

/**
 * 布局变化自动持久化（防抖 400ms，拖拽结束后的那一帧才落库）。
 *
 * 为什么在 store 模块里订阅而不是让每个写入口自己 persist：写入口现在有
 * 设置页、拖拽把手、预设切换，将来只会更多——任何一处忘记写库就出现
 * "重启后布局回退"。订阅是唯一覆盖全部入口的位置。
 *
 * 拖拽每帧都会 set：不防抖就是每帧一次 IPC 往返。失败静默（布局持久化
 * 是便利功能，不值得为它打断用户）。
 */
let persistTimer: ReturnType<typeof setTimeout> | null = null;

useLayoutStore.subscribe((state) => {
  if (typeof window === 'undefined' || !isTauriRuntime()) {
    return;
  }
  if (persistTimer !== null) {
    clearTimeout(persistTimer);
  }
  persistTimer = setTimeout(() => {
    persistTimer = null;
    void settingsSet('global', LAYOUT_KEY, JSON.stringify(state.layout)).catch(() => {
      // 写失败不回滚 store：内存态照常工作，下次变更会再尝试
    });
  }, 400);
});
