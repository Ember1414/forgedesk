import { describe, expect, it } from 'vitest';

import {
  detectConflicts,
  displayShortcut,
  evaluateWhen,
  hasPlatformMod,
  matchesEvent,
  normalizeShortcut,
  shortcutsToMarkdown,
} from '@/lib/shortcutKeys';

describe('normalizeShortcut（键规范化）', () => {
  it('修饰键顺序无关，输出统一为 Mod/Shift/Alt + 键名', () => {
    expect(normalizeShortcut('shift+ctrl+p')).toBe('Mod+Shift+P');
    expect(normalizeShortcut('Ctrl+Shift+P')).toBe('Mod+Shift+P');
    expect(normalizeShortcut('mod+s')).toBe('Mod+S');
    expect(normalizeShortcut('F5')).toBe('F5');
  });

  it('单字母键名大写，多余空白被剔除', () => {
    expect(normalizeShortcut(' ctrl + s ')).toBe('Mod+S');
    expect(normalizeShortcut('ctrl+e')).toBe('Mod+E');
  });
});

describe('matchesEvent（事件命中）', () => {
  const base = {
    key: 'p',
    ctrlKey: false,
    metaKey: false,
    shiftKey: false,
    altKey: false,
  };

  it('Mod+Shift+P 在平台修饰键 + Shift 按下时命中', () => {
    const event = { ...base, key: 'P', shiftKey: true, ...({} as object), ...({} as object) };
    const withMod = { ...event, ctrlKey: true, metaKey: true };
    // Windows（Ctrl 平台）下 ctrlKey=true；macOS 下 metaKey=true——测试里两个都给
    expect(matchesEvent('Mod+Shift+P', withMod)).toBe(true);
  });

  it('修饰键不满足时不命中', () => {
    expect(matchesEvent('Mod+Shift+P', base)).toBe(false);
    expect(matchesEvent('Mod+Shift+P', { ...base, key: 'P', shiftKey: true })).toBe(false);
  });

  it('多按修饰键不命中（严格匹配）', () => {
    expect(matchesEvent('F5', { ...base, key: 'F5', ctrlKey: true })).toBe(false);
    expect(matchesEvent('F5', { ...base, key: 'F5' })).toBe(true);
  });

  it('字母键大小写不敏感', () => {
    expect(matchesEvent('Mod+S', { ...base, key: 'S', ctrlKey: true, metaKey: true })).toBe(true);
    expect(matchesEvent('Mod+S', { ...base, key: 's', ctrlKey: true, metaKey: true })).toBe(true);
  });
});

describe('detectConflicts（冲突检测）', () => {
  it('同键 + when 交集 → 冲突', () => {
    const conflicts = detectConflicts([
      { id: 'a', normalizedKey: 'Mod+S', when: ['repoOpen'] },
      { id: 'b', normalizedKey: 'Mod+S', when: ['repoOpen', 'editorActive'] },
    ]);
    expect(conflicts).toHaveLength(1);
    expect(conflicts[0]?.firstId).toBe('a');
    expect(conflicts[0]?.secondId).toBe('b');
  });

  it('同键 + when 无交集 → 不冲突', () => {
    const conflicts = detectConflicts([
      { id: 'a', normalizedKey: 'Mod+S', when: ['repoOpen'] },
      { id: 'b', normalizedKey: 'Mod+S', when: ['paletteOpen'] },
    ]);
    expect(conflicts).toHaveLength(0);
  });

  it('同键 + 双方全局 → 冲突', () => {
    const conflicts = detectConflicts([
      { id: 'a', normalizedKey: 'F5', when: [] },
      { id: 'b', normalizedKey: 'F5', when: [] },
    ]);
    expect(conflicts).toHaveLength(1);
  });

  it('无键命令与异键命令不冲突', () => {
    const conflicts = detectConflicts([
      { id: 'a', normalizedKey: null, when: [] },
      { id: 'b', normalizedKey: 'Mod+S', when: [] },
      { id: 'c', normalizedKey: null, when: [] },
    ]);
    expect(conflicts).toHaveLength(0);
  });
});

describe('evaluateWhen（when 求值）', () => {
  it('全部 tag 命中才可用', () => {
    const context = new Set(['repoOpen', 'editorActive']);
    expect(evaluateWhen(['repoOpen'], context)).toBe(true);
    expect(evaluateWhen(['repoOpen', 'editorActive'], context)).toBe(true);
    expect(evaluateWhen(['repoOpen', 'paletteOpen'], context)).toBe(false);
    expect(evaluateWhen([], context)).toBe(true);
  });
});

describe('展示与导出', () => {
  it('displayShortcut 把 Mod 换成平台修饰键', () => {
    expect(displayShortcut('Mod+Shift+P')).toMatch(/(Cmd|Ctrl)\+Shift\+P/);
  });

  it('shortcutsToMarkdown 生成表格', () => {
    const md = shortcutsToMarkdown(
      [
        { title: '打开命令面板', normalized: 'Mod+Shift+P' },
        { title: '刷新', normalized: null },
      ],
      { keyColumn: '快捷键', actionColumn: '功能' },
    );
    expect(md).toContain('| Mod+Shift+P | 打开命令面板 |'.replace('Mod', displayShortcut('Mod')));
    expect(md).toContain('| — | 刷新 |');
  });

  it('hasPlatformMod 识别平台修饰键', () => {
    expect(hasPlatformMod('Mod+S')).toBe(true);
    expect(hasPlatformMod('F5')).toBe(false);
  });
});
