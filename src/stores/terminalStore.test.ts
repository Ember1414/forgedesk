import { beforeEach, describe, expect, it } from 'vitest';

import { initialTerminalState, useTerminalStore } from '@/stores/terminalStore';
import type { TerminalTab } from '@/stores/terminalStore';

function makeTab(termId: string, overrides: Partial<TerminalTab> = {}): TerminalTab {
  return {
    termId,
    repoId: 1,
    shellId: 'default',
    title: '默认 Shell',
    renamed: false,
    exited: false,
    exitCode: null,
    ...overrides,
  };
}

describe('terminalStore（终端标签投影）', () => {
  beforeEach(() => {
    useTerminalStore.setState(initialTerminalState);
  });

  it('新标签立即激活，连续创建保持顺序', () => {
    useTerminalStore.getState().addTab(makeTab('term-1'));
    useTerminalStore.getState().addTab(makeTab('term-2'));
    const state = useTerminalStore.getState();
    expect(state.tabs.map((tab) => tab.termId)).toEqual(['term-1', 'term-2']);
    expect(state.activeTermId).toBe('term-2');
  });

  it('关闭当前标签后激活相邻标签（优先右侧）', () => {
    const store = useTerminalStore.getState();
    store.addTab(makeTab('term-1'));
    store.addTab(makeTab('term-2'));
    store.addTab(makeTab('term-3'));
    store.setActive('term-2');
    store.removeTab('term-2');
    const state = useTerminalStore.getState();
    expect(state.activeTermId).toBe('term-3');
    expect(state.tabs.map((tab) => tab.termId)).toEqual(['term-1', 'term-3']);
  });

  it('关闭最后一个标签后没有激活标签', () => {
    const store = useTerminalStore.getState();
    store.addTab(makeTab('term-1'));
    store.removeTab('term-1');
    expect(useTerminalStore.getState().activeTermId).toBeNull();
  });

  it('OSC 自动标题不覆盖用户重命名，但覆盖自动标题', () => {
    const store = useTerminalStore.getState();
    store.addTab(makeTab('term-1', { title: '默认 Shell' }));

    store.setTitleAuto('term-1', 'git status');
    expect(useTerminalStore.getState().tabs[0]?.title).toBe('git status');

    store.renameTab('term-1', '构建');
    store.setTitleAuto('term-1', 'npm run build');
    expect(useTerminalStore.getState().tabs[0]?.title).toBe('构建');
    expect(useTerminalStore.getState().tabs[0]?.renamed).toBe(true);
  });

  it('空白的 OSC 标题不改变标题', () => {
    const store = useTerminalStore.getState();
    store.addTab(makeTab('term-1'));
    store.setTitleAuto('term-1', '   ');
    expect(useTerminalStore.getState().tabs[0]?.title).toBe('默认 Shell');
  });

  it('退出状态与退出码写入标签', () => {
    const store = useTerminalStore.getState();
    store.addTab(makeTab('term-1'));
    store.setExited('term-1', 0);
    const tab = useTerminalStore.getState().tabs[0];
    expect(tab?.exited).toBe(true);
    expect(tab?.exitCode).toBe(0);
  });

  it('拖拽排序在全局数组上生效，越界调用不动', () => {
    const store = useTerminalStore.getState();
    store.addTab(makeTab('term-1'));
    store.addTab(makeTab('term-2'));
    store.addTab(makeTab('term-3'));

    store.moveTab(0, 2);
    expect(useTerminalStore.getState().tabs.map((tab) => tab.termId)).toEqual([
      'term-2',
      'term-3',
      'term-1',
    ]);

    useTerminalStore.getState().moveTab(5, 0);
    expect(useTerminalStore.getState().tabs.map((tab) => tab.termId)).toEqual([
      'term-2',
      'term-3',
      'term-1',
    ]);
  });
});
