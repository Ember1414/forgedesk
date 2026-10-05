import { beforeEach, describe, expect, it } from 'vitest';

import { scanOscTitle } from '@/features/terminal/manager';
import { initialTerminalState, useTerminalStore } from '@/stores/terminalStore';

function encode(text: string): Uint8Array {
  return new TextEncoder().encode(text);
}

describe('scanOscTitle（OSC 标题 → 标签标题）', () => {
  beforeEach(() => {
    useTerminalStore.setState(initialTerminalState);
    useTerminalStore.getState().addTab({
      termId: 'term-1',
      repoId: 1,
      shellId: 'default',
      title: '默认 Shell',
      renamed: false,
      exited: false,
      exitCode: null,
    });
  });

  it('BEL 结尾的 OSC 0 标题更新标签', () => {
    scanOscTitle('term-1', encode('\x1b]0;git status\x07'));
    expect(useTerminalStore.getState().tabs[0]?.title).toBe('git status');
  });

  it('ST 结尾的 OSC 2 标题更新标签', () => {
    scanOscTitle('term-1', encode('\x1b]2;npm run build\x1b\\'));
    expect(useTerminalStore.getState().tabs[0]?.title).toBe('npm run build');
  });

  it('普通输出不含标题序列时不改标题', () => {
    scanOscTitle('term-1', encode('hello 中文 🎉\r\n'));
    expect(useTerminalStore.getState().tabs[0]?.title).toBe('默认 Shell');
  });

  it('没有 ESC 的块直接短路（性能路径）', () => {
    scanOscTitle('term-1', encode('a'.repeat(10_000)));
    expect(useTerminalStore.getState().tabs[0]?.title).toBe('默认 Shell');
  });
});
