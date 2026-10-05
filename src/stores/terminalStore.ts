/**
 * 终端标签页状态（Zustand，纯 UI 状态）。
 *
 * 边界（AGENTS.md §6「单一真相源」）：标签的开合、激活、标题是**界面状态**；
 * 会话本身活在后端进程里（`TerminalRegistry`），xterm 实例活在
 * `src/features/terminal/manager.ts` 的模块级注册表里——本 store 只保存
 * "渲染标签栏所需的最小投影"。Git 状态不在这里。
 *
 * 会话持久化（T5.2 任务书）：应用重启后**不恢复**会话（后端 shell 进程已死，
 * 假恢复比不恢复更糟）；"在新标签中恢复上次命令"由 manager 的命令历史承担。
 */
import { create } from 'zustand';

/** 一个终端标签的投影。 */
export interface TerminalTab {
  /** 后端会话 id（`term-<n>`）。 */
  readonly termId: string;
  /** 归属仓库（存储层记录 id）。 */
  readonly repoId: number;
  /** shell 选项 id（`default` / `pwsh` / `gitbash` …）。 */
  readonly shellId: string;
  /** 标签标题：shell 名 / OSC 标题 / 用户重命名，优先级递增。 */
  readonly title: string;
  /** 用户重命名后，OSC 标题不再覆盖。 */
  readonly renamed: boolean;
  /** 会话是否已退出（退出后标签保留，供查看回滚缓冲）。 */
  readonly exited: boolean;
  /** 退出码（null = 后端拿不到）。 */
  readonly exitCode: number | null;
}

export interface TerminalStoreState {
  readonly tabs: readonly TerminalTab[];
  readonly activeTermId: string | null;

  addTab(tab: TerminalTab): void;
  /** 关闭标签（不调用后端——后端 `term_close` 由调用方负责）。 */
  removeTab(termId: string): void;
  setActive(termId: string): void;
  renameTab(termId: string, title: string): void;
  /** OSC 标题到达时的自动更新（用户重命名过的标签不被覆盖）。 */
  setTitleAuto(termId: string, title: string): void;
  setExited(termId: string, exitCode: number | null): void;
  /** 拖拽排序：把 from 位置的标签移到 to 位置。 */
  moveTab(from: number, to: number): void;
}

/** 初始状态（导出供测试复位；store 是模块级单例）。 */
export const initialTerminalState = {
  tabs: [] as readonly TerminalTab[],
  activeTermId: null as string | null,
};

export const useTerminalStore = create<TerminalStoreState>()((set) => ({
  ...initialTerminalState,

  addTab: (tab) => {
    set((state) => ({
      tabs: [...state.tabs, tab],
      // 新标签立即激活：用户点 "+" 就是要用新终端
      activeTermId: tab.termId,
    }));
  },

  removeTab: (termId) => {
    set((state) => {
      const index = state.tabs.findIndex((tab) => tab.termId === termId);
      if (index < 0) {
        return state;
      }
      const tabs = state.tabs.filter((tab) => tab.termId !== termId);
      let activeTermId = state.activeTermId;
      if (activeTermId === termId) {
        // 关掉当前标签后激活相邻的（优先右边，其次左边）
        activeTermId = tabs[Math.min(index, tabs.length - 1)]?.termId ?? null;
      }
      return { tabs, activeTermId };
    });
  },

  setActive: (termId) => {
    set({ activeTermId: termId });
  },

  renameTab: (termId, title) => {
    set((state) => ({
      tabs: state.tabs.map((tab) =>
        tab.termId === termId ? { ...tab, title, renamed: true } : tab,
      ),
    }));
  },

  setTitleAuto: (termId, title) => {
    set((state) => ({
      tabs: state.tabs.map((tab) =>
        tab.termId === termId && !tab.renamed && title.trim() !== ''
          ? { ...tab, title: title.trim() }
          : tab,
      ),
    }));
  },

  setExited: (termId, exitCode) => {
    set((state) => ({
      tabs: state.tabs.map((tab) =>
        tab.termId === termId ? { ...tab, exited: true, exitCode } : tab,
      ),
    }));
  },

  moveTab: (from, to) => {
    set((state) => {
      if (
        from === to ||
        from < 0 ||
        to < 0 ||
        from >= state.tabs.length ||
        to >= state.tabs.length
      ) {
        return state;
      }
      const tabs = [...state.tabs];
      const moved = tabs.splice(from, 1)[0];
      if (!moved) {
        return state;
      }
      tabs.splice(to, 0, moved);
      return { tabs };
    });
  },
}));
