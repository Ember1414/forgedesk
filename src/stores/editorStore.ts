/**
 * 编辑器标签状态（T5.7，Zustand，纯 UI 状态）。
 *
 * 文件内容不进 store——Monaco 的 model 自己持有；store 只保存"渲染
 * 标签栏与脏标记所需的最小投影"（打开的文件、元数据、磁盘基线）。
 */
import { create } from 'zustand';

import type { FsEol } from '@/lib/ipc';

/** 一个打开的编辑器标签。 */
export interface EditorTab {
  /** 相对仓库根的路径（POSIX 分隔符）——唯一 key。 */
  readonly path: string;
  readonly name: string;
  /** 打开时的换行符形态（保存时恢复）。 */
  readonly eol: FsEol;
  readonly hasBom: boolean;
  /** 二进制文件只读展示元信息，不进 Monaco。 */
  readonly isBinary: boolean;
  /** 打开（或最后一次保存）时的磁盘内容基线——外部变更检测的对照。 */
  readonly baseline: string | null;
  /** 磁盘内容在基线之后被外部改动过（三选一提示的触发器）。 */
  readonly dirtyDisk: boolean;
}

export interface EditorStoreState {
  readonly tabs: readonly EditorTab[];
  readonly activePath: string | null;

  openTab(tab: EditorTab): void;
  closeTab(path: string): void;
  setActive(path: string): void;
  /** 保存成功后更新基线并清脏。 */
  markSaved(path: string, baseline: string): void;
  /** 磁盘内容与基线不一致（外部修改）。 */
  markDirtyDisk(path: string): void;
  /** 用户选择"保留我的编辑"：以编辑器内容为新基线。 */
  keepMine(path: string): void;
}

export const initialEditorState = {
  tabs: [] as readonly EditorTab[],
  activePath: null as string | null,
};

export const useEditorStore = create<EditorStoreState>()((set) => ({
  ...initialEditorState,

  openTab: (tab) => {
    set((state) => {
      const exists = state.tabs.some((existing) => existing.path === tab.path);
      return {
        tabs: exists
          ? state.tabs.map((t) => (t.path === tab.path ? tab : t))
          : [...state.tabs, tab],
        activePath: tab.path,
      };
    });
  },

  closeTab: (path) => {
    set((state) => {
      const index = state.tabs.findIndex((tab) => tab.path === path);
      if (index < 0) {
        return state;
      }
      const tabs = state.tabs.filter((tab) => tab.path !== path);
      const activePath =
        state.activePath === path
          ? (tabs[Math.min(index, tabs.length - 1)]?.path ?? null)
          : state.activePath;
      return { tabs, activePath };
    });
  },

  setActive: (path) => {
    set({ activePath: path });
  },

  markSaved: (path, baseline) => {
    set((state) => ({
      tabs: state.tabs.map((tab) =>
        tab.path === path ? { ...tab, baseline, dirtyDisk: false } : tab,
      ),
    }));
  },

  markDirtyDisk: (path) => {
    set((state) => ({
      tabs: state.tabs.map((tab) => (tab.path === path ? { ...tab, dirtyDisk: true } : tab)),
    }));
  },

  keepMine: (path) => {
    set((state) => ({
      tabs: state.tabs.map((tab) => (tab.path === path ? { ...tab, dirtyDisk: false } : tab)),
    }));
  },
}));
