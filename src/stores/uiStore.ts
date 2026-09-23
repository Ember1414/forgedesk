/**
 * UI 状态（Zustand）。
 *
 * 边界（AGENTS.md §6「单一真相源」）：
 *   - 这里只放**纯界面**状态：侧栏折叠、详情面板位置、主题、当前仓库 id。
 *   - Git 的服务端状态（工作区变更、提交列表…）一律走 TanStack Query，不进 store。
 *   - 后台任务虽然由后端产生，但前端只维护「展示所需的最小投影」，见 jobStore。
 */
import { create } from 'zustand';

import { applyThemeMode, readThemeMode, writeThemeMode } from '@/app/theme';
import type { ThemeMode } from '@/app/theme';

/** 详情面板位置：右侧 / 底部 / 隐藏（repo 相关页面共用）。 */
export const DETAIL_PANEL_POSITIONS = ['right', 'bottom', 'hidden'] as const;
export type DetailPanelPosition = (typeof DETAIL_PANEL_POSITIONS)[number];

export interface UiState {
  /** 用户手工折叠的侧栏（与"窗口过窄"导致的自动折叠相互独立）。 */
  readonly sidebarCollapsed: boolean;
  readonly detailPanel: DetailPanelPosition;
  readonly themeMode: ThemeMode;
  /** 当前打开的仓库 id；null 表示未打开任何仓库。 */
  readonly currentRepoId: string | null;

  toggleSidebar(): void;
  setSidebarCollapsed(collapsed: boolean): void;
  setDetailPanel(position: DetailPanelPosition): void;
  setThemeMode(mode: ThemeMode): void;
  setCurrentRepoId(repoId: string | null): void;
}

/**
 * 初始值。
 *
 * 导出它是为了让测试可以在用例之间复位（Zustand 的 store 是模块级单例，
 * 不复位会让"用例通过顺序"影响结果，属于典型的假绿来源）。
 */
export const initialUiState = {
  sidebarCollapsed: false,
  detailPanel: 'right' as DetailPanelPosition,
  themeMode: readThemeMode(),
  currentRepoId: null,
} satisfies Pick<UiState, 'sidebarCollapsed' | 'detailPanel' | 'themeMode' | 'currentRepoId'>;

export const useUiStore = create<UiState>()((set) => ({
  ...initialUiState,

  toggleSidebar: () => {
    set((state) => ({ sidebarCollapsed: !state.sidebarCollapsed }));
  },
  setSidebarCollapsed: (collapsed) => {
    set({ sidebarCollapsed: collapsed });
  },
  setDetailPanel: (position) => {
    set({ detailPanel: position });
  },
  setThemeMode: (mode) => {
    // 先落盘与写 DOM，再更新 store：这样即使 React 尚未渲染，界面主题也已生效
    writeThemeMode(mode);
    applyThemeMode(mode);
    set({ themeMode: mode });
  },
  setCurrentRepoId: (repoId) => {
    set({ currentRepoId: repoId });
  },
}));
