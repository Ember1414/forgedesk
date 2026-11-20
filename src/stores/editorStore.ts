/** 编辑器文档会话：仓库与路径共同寻址，草稿独立于路由挂载生命周期。 */
import { create } from 'zustand';
import type { FsEol } from '@/lib/ipc';

export interface EditorTab {
  readonly repoId: number;
  readonly path: string;
  readonly name: string;
  readonly eol: FsEol;
  readonly hasBom: boolean;
  readonly isBinary: boolean;
  readonly baseline: string | null;
  readonly draft: string | null;
  readonly dirtyDisk: boolean;
}

/** Monaco URI 包含仓库身份，避免不同仓库的同名文件共享 model。 */
export function editorDocumentId(repoId: number, path: string): string {
  return `forgedesk://repository/${repoId}/${path.split('/').map(encodeURIComponent).join('/')}`;
}

export interface EditorStoreState {
  readonly tabs: readonly EditorTab[];
  readonly activePaths: Readonly<Record<number, string>>;
  openTab(tab: EditorTab): void;
  closeTab(repoId: number, path: string): void;
  setActive(repoId: number, path: string): void;
  setDraft(repoId: number, path: string, content: string): void;
  markSaved(repoId: number, path: string, baseline: string): void;
  markDirtyDisk(repoId: number, path: string): void;
  keepMine(repoId: number, path: string, baseline: string): void;
}

export const initialEditorState = {
  tabs: [] as readonly EditorTab[],
  activePaths: {} as Readonly<Record<number, string>>,
};

export const useEditorStore = create<EditorStoreState>()((set) => {
  const update = (repoId: number, path: string, change: Partial<EditorTab>): void => {
    set((state) => ({
      tabs: state.tabs.map((tab) =>
        tab.repoId === repoId && tab.path === path ? { ...tab, ...change } : tab,
      ),
    }));
  };
  return {
    ...initialEditorState,
    openTab: (tab) => {
      set((state) => ({
        // 再次点击文件树只激活文档，不能用磁盘内容覆盖已有草稿。
        tabs: state.tabs.some((item) => item.repoId === tab.repoId && item.path === tab.path)
          ? state.tabs
          : [...state.tabs, tab],
        activePaths: { ...state.activePaths, [tab.repoId]: tab.path },
      }));
    },
    closeTab: (repoId, path) => {
      set((state) => {
        const repoTabs = state.tabs.filter((tab) => tab.repoId === repoId);
        const index = repoTabs.findIndex((tab) => tab.path === path);
        const remaining = repoTabs.filter((tab) => tab.path !== path);
        const activePaths = { ...state.activePaths };
        if (activePaths[repoId] === path) {
          const next = remaining[Math.min(index, remaining.length - 1)];
          if (next) activePaths[repoId] = next.path;
          else delete activePaths[repoId];
        }
        return {
          tabs: state.tabs.filter((tab) => tab.repoId !== repoId || tab.path !== path),
          activePaths,
        };
      });
    },
    setActive: (repoId, path) => {
      set((state) => ({ activePaths: { ...state.activePaths, [repoId]: path } }));
    },
    setDraft: (repoId, path, draft) => update(repoId, path, { draft }),
    markSaved: (repoId, path, baseline) => update(repoId, path, { baseline, dirtyDisk: false }),
    markDirtyDisk: (repoId, path) => update(repoId, path, { dirtyDisk: true }),
    keepMine: (repoId, path, baseline) => update(repoId, path, { baseline, dirtyDisk: false }),
  };
});
