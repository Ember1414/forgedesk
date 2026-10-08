import type { Monaco } from '@monaco-editor/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import {
  applyMonacoTheme,
  buildMonacoTheme,
  MONACO_THEME_NAME,
  monacoFontFamily,
} from '@/features/editor/monacoTheme';

/**
 * Monaco 主题的两个契约：
 *   1. **跟随应用外观**——写死 `vs-dark` 时亮色用户看到的是一块黑框
 *      （2026-10-08 反馈"它的背景只有黑色吗"）；
 *   2. 主题数据来自设计令牌，因此不能出现"第二套颜色真相源"。
 */
function fakeMonaco(): Pick<Monaco, 'editor'> {
  return {
    editor: {
      defineTheme: vi.fn(),
      setTheme: vi.fn(),
    },
  } as unknown as Pick<Monaco, 'editor'>;
}

afterEach(() => {
  document.documentElement.removeAttribute('data-theme');
});

describe('Monaco 主题', () => {
  it('applyMonacoTheme 定义并套用同一个主题名', () => {
    const monaco = fakeMonaco();

    applyMonacoTheme(monaco as unknown as Monaco);

    expect(monaco.editor.defineTheme).toHaveBeenCalledTimes(1);
    expect(monaco.editor.defineTheme).toHaveBeenCalledWith(
      MONACO_THEME_NAME,
      expect.objectContaining({ rules: expect.any(Array), colors: expect.any(Object) }),
    );
    expect(monaco.editor.setTheme).toHaveBeenCalledWith(MONACO_THEME_NAME);
  });

  it('未挂载实例时不抛错（主题变化早于首次渲染的边界）', () => {
    expect(() => {
      applyMonacoTheme();
    }).not.toThrow();
  });

  it('base 跟随解析后的外观', () => {
    document.documentElement.setAttribute('data-theme', 'dark');
    expect(buildMonacoTheme().base).toBe('vs-dark');

    document.documentElement.setAttribute('data-theme', 'light');
    expect(buildMonacoTheme().base).toBe('vs');
  });

  it('关键字 / 背景等关键色位都有值（不依赖样式表也能出主题）', () => {
    const theme = buildMonacoTheme();
    expect(theme.colors?.['editor.background']).toBeTruthy();
    expect(theme.rules?.some((rule) => rule.token === 'keyword')).toBe(true);
  });

  it('等宽字体取自令牌（令牌缺失时回退 monospace）', () => {
    expect(monacoFontFamily()).not.toBe('');
  });
});
