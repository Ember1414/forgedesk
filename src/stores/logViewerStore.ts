/**
 * 日志查看器的开关状态。
 *
 * 为什么用一个全局 store，而不是把对话框挂在出现错误的地方：
 * 日志查看器要从**两个完全不同的位置**打开——设置页（就地看着）与错误提示
 * （"查看相关日志"，此时用户可能在任意页面）。前者是内嵌组件，后者需要一个全局出口。
 * 用 store 表达"谁要求打开、要看哪个时间点附近的日志"，两端就能共用同一个对话框，
 * 不必在每个页面里各挂一份。
 */
import { create } from 'zustand';

export interface OpenLogViewerOptions {
  /** 需要高亮的时间点（Unix 毫秒）——通常是错误发生的时间。 */
  readonly nearTimestamp?: number | null;
}

export interface LogViewerState {
  /** 是否打开全局日志对话框。 */
  readonly open: boolean;
  /** 高亮锚点时间。 */
  readonly nearTimestamp: number | null;
  /** 打开日志对话框（可指定高亮时间）。 */
  openViewer(options?: OpenLogViewerOptions): void;
  /** 关闭。 */
  closeViewer(): void;
}

/** 初始状态（导出供测试复位；store 是模块级单例）。 */
export const initialLogViewerState = {
  open: false,
  nearTimestamp: null as number | null,
};

export const useLogViewerStore = create<LogViewerState>()((set) => ({
  ...initialLogViewerState,

  openViewer: (options) => {
    set({ open: true, nearTimestamp: options?.nearTimestamp ?? null });
  },

  closeViewer: () => {
    set({ open: false });
  },
}));

/** 便捷函数：不依赖 React 上下文即可打开日志查看器。 */
export function openLogViewer(options?: OpenLogViewerOptions): void {
  useLogViewerStore.getState().openViewer(options);
}
