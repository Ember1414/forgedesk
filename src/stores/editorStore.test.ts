import { beforeEach, describe, expect, it } from 'vitest';
import {
  editorDocumentId,
  initialEditorState,
  useEditorStore,
  type EditorTab,
} from '@/stores/editorStore';

function tab(repoId: number): EditorTab {
  return {
    repoId,
    path: 'src/main.rs',
    name: 'main.rs',
    eol: 'lf',
    hasBom: false,
    isBinary: false,
    baseline: `repo ${repoId}`,
    draft: `repo ${repoId}`,
    dirtyDisk: false,
  };
}

beforeEach(() => useEditorStore.setState(initialEditorState));

describe('仓库文档会话', () => {
  it('同名文件按仓库隔离内容、活动标签与 URI', () => {
    const store = useEditorStore.getState();
    store.openTab(tab(1));
    store.openTab(tab(2));
    store.setDraft(1, 'src/main.rs', 'draft');
    const state = useEditorStore.getState();
    expect(state.tabs.map((item) => item.draft)).toEqual(['draft', 'repo 2']);
    expect(state.activePaths).toEqual({ 1: 'src/main.rs', 2: 'src/main.rs' });
    expect(editorDocumentId(1, 'src/main.rs')).not.toBe(editorDocumentId(2, 'src/main.rs'));
  });
  it('重复打开不覆盖草稿，保存中继续输入也不会清掉新内容', () => {
    const store = useEditorStore.getState();
    store.openTab(tab(1));
    store.setDraft(1, 'src/main.rs', 'new draft');
    store.openTab(tab(1));
    store.markSaved(1, 'src/main.rs', 'older saved value');
    expect(useEditorStore.getState().tabs[0]).toMatchObject({
      draft: 'new draft',
      baseline: 'older saved value',
    });
  });
  it('关闭一个仓库的文档不关闭另一个仓库的同名文档', () => {
    const store = useEditorStore.getState();
    store.openTab(tab(1));
    store.openTab(tab(2));
    store.closeTab(1, 'src/main.rs');
    expect(useEditorStore.getState().tabs.map((item) => item.repoId)).toEqual([2]);
    expect(useEditorStore.getState().activePaths).toEqual({ 2: 'src/main.rs' });
  });
});
