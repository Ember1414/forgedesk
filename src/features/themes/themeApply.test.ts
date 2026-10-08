import { afterEach, describe, expect, it } from 'vitest';

import {
  ACTIVE_CUSTOM_THEME_CACHE_KEY,
  applyCustomThemeColors,
  deriveXtermPalette,
  readCachedCustomTheme,
  writeCachedCustomTheme,
} from './themeApply';
import { BUILTIN_THEMES } from './builtinThemes';
import type { ThemeDefinition } from './themeModel';

const DARK_THEME: ThemeDefinition = {
  id: 'night-ink',
  name: 'Night Ink',
  appearance: 'dark',
  version: '1.0.0',
  colors: { canvas: '#101418', brand: '#8b93f8' },
};

describe('自定义主题的 DOM 应用', () => {
  afterEach(() => {
    // 卸载残留的 inline 变量，避免用例顺序影响结果
    applyCustomThemeColors(null, 'dark');
    window.localStorage.removeItem(ACTIVE_CUSTOM_THEME_CACHE_KEY);
  });

  it('把覆盖的 token 写成 --fd-* inline 变量（camelCase 转 kebab）', () => {
    applyCustomThemeColors(DARK_THEME, 'dark');

    const root = document.documentElement;
    expect(root.style.getPropertyValue('--fd-canvas')).toBe('#101418');
    expect(root.style.getPropertyValue('--fd-brand')).toBe('#8b93f8');
  });

  it('当前外观与主题外观不一致时不套用（外观归属原则）', () => {
    applyCustomThemeColors(DARK_THEME, 'light');

    const root = document.documentElement;
    expect(root.style.getPropertyValue('--fd-canvas')).toBe('');
  });

  it('清除主题时移除此前写入的全部变量（包括字体）', () => {
    const withFont: ThemeDefinition = {
      ...DARK_THEME,
      fonts: { mono: 'JetBrains Mono, monospace' },
    };
    applyCustomThemeColors(withFont, 'dark');
    expect(document.documentElement.style.getPropertyValue('--fd-font-mono')).not.toBe('');

    applyCustomThemeColors(null, 'dark');
    expect(document.documentElement.style.getPropertyValue('--fd-canvas')).toBe('');
    expect(document.documentElement.style.getPropertyValue('--fd-font-mono')).toBe('');
  });

  it('切换主题时先清掉上一份的变量，不留叠加残留', () => {
    applyCustomThemeColors(DARK_THEME, 'dark');
    const other: ThemeDefinition = { ...DARK_THEME, id: 'other', colors: { fg: '#ffffff' } };
    applyCustomThemeColors(other, 'dark');

    const root = document.documentElement;
    // 上一主题的 canvas 被清掉，不再是 inline 值
    expect(root.style.getPropertyValue('--fd-canvas')).toBe('');
    expect(root.style.getPropertyValue('--fd-fg')).toBe('#ffffff');
  });
});

describe('首帧缓存', () => {
  afterEach(() => {
    window.localStorage.removeItem(ACTIVE_CUSTOM_THEME_CACHE_KEY);
  });

  it('写入后能同步读回同一主题', () => {
    writeCachedCustomTheme(DARK_THEME);
    expect(readCachedCustomTheme()?.id).toBe('night-ink');
  });

  it('清除后读取为 null；损坏的 JSON 也按未激活处理', () => {
    writeCachedCustomTheme(null);
    expect(readCachedCustomTheme()).toBeNull();

    window.localStorage.setItem(ACTIVE_CUSTOM_THEME_CACHE_KEY, '{broken');
    expect(readCachedCustomTheme()).toBeNull();
  });
});

describe('非颜色维度（字体 / 动效）', () => {
  afterEach(() => {
    applyCustomThemeColors(null, 'light');
  });

  it('动效时长会被写到 --fd-duration-*，清除时移除', () => {
    const theme = {
      id: 'slow-motion',
      name: 'Slow Motion',
      appearance: 'light' as const,
      version: '1.0.0',
      colors: {},
      motion: { fast: '180ms', base: '260ms' },
    };

    applyCustomThemeColors(theme, 'light');

    const root = document.documentElement;
    expect(root.style.getPropertyValue('--fd-duration-fast')).toBe('180ms');
    expect(root.style.getPropertyValue('--fd-duration-base')).toBe('260ms');
    // 未声明的档位不写（继续用 tokens.css 的值）
    expect(root.style.getPropertyValue('--fd-duration-slow')).toBe('');

    applyCustomThemeColors(null, 'light');
    expect(root.style.getPropertyValue('--fd-duration-fast')).toBe('');
  });

  it('外观与主题不一致时不落任何变量（动效也不落）', () => {
    applyCustomThemeColors(
      {
        id: 'dark-only',
        name: 'Dark Only',
        appearance: 'dark',
        version: '1.0.0',
        colors: { canvas: '#000000' },
        motion: { fast: '300ms' },
      },
      'light',
    );

    expect(document.documentElement.style.getPropertyValue('--fd-duration-fast')).toBe('');
  });
});

describe('xterm 配色派生', () => {
  it('未覆盖的键从 resolve 取内置值，显式 xterm 段优先级最高', () => {
    const palette = deriveXtermPalette(
      { appearance: 'dark', colors: { canvas: '#101418' }, xterm: { cursor: '#ff8800' } },
      (token) => (token === 'canvas' ? 'INHERITED-CANVAS' : `BUILTIN-${token}`),
    );

    expect(palette['background']).toBe('#101418'); // 主题覆盖优先
    expect(palette['foreground']).toBe('BUILTIN-fg'); // 缺失 token 走 resolve
    expect(palette['red']).toBe('BUILTIN-danger'); // 语义就近映射
    expect(palette['cursor']).toBe('#ff8800'); // 显式 xterm 段最高
  });

  it('内置主题（colors 为空）也能派生出完整终端配色', () => {
    const dark = BUILTIN_THEMES.at(1);
    expect(dark).toBeDefined();
    if (dark === undefined) {
      return;
    }
    const palette = deriveXtermPalette(dark, (token) => `BUILTIN-${token}`);
    expect(palette['background']).toBe('BUILTIN-canvas');
    expect(Object.keys(palette).length).toBeGreaterThanOrEqual(21);
  });
});
