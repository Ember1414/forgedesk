import { describe, expect, it } from 'vitest';

import {
  contrastRatio,
  contrastReport,
  parseColor,
  relativeLuminance,
  validateThemeJson,
} from './themeModel';
import { BUILTIN_THEMES } from './builtinThemes';

function validTheme(): Record<string, unknown> {
  return {
    id: 'midnight-test',
    name: 'Midnight Test',
    appearance: 'dark',
    version: '1.0.0',
    colors: { canvas: '#101418', fg: '#e6e9ee' },
  };
}

describe('主题 JSON 校验', () => {
  it('接受一份只覆盖部分 token 的合法主题', () => {
    const result = validateThemeJson(validTheme());
    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.theme.id).toBe('midnight-test');
      expect(result.theme.colors.canvas).toBe('#101418');
    }
  });

  it('拒绝非对象输入', () => {
    const result = validateThemeJson('nope');
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.errors[0]?.code).toBe('notAnObject');
    }
  });

  it('拒绝非法 id（大写、空、超长）', () => {
    for (const id of ['Midnight', '', 'x'.repeat(65), '带中文']) {
      const input = { ...validTheme(), id };
      const result = validateThemeJson(input);
      expect(result.ok).toBe(false);
      if (!result.ok) {
        expect(result.errors.some((e) => e.field === 'id')).toBe(true);
      }
    }
  });

  it('拒绝占用内置主题 id', () => {
    const result = validateThemeJson({ ...validTheme(), id: 'forgedesk-light' });
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.errors.some((e) => e.code === 'builtinId')).toBe(true);
    }
  });

  it('拒绝空白或超长的名称', () => {
    for (const name of ['   ', 'x'.repeat(101)]) {
      const result = validateThemeJson({ ...validTheme(), name });
      expect(result.ok).toBe(false);
      if (!result.ok) {
        expect(result.errors.some((e) => e.field === 'name')).toBe(true);
      }
    }
  });

  it('拒绝非 light/dark 的外观', () => {
    const result = validateThemeJson({ ...validTheme(), appearance: 'blue' });
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.errors.some((e) => e.field === 'appearance')).toBe(true);
    }
  });

  it('拒绝非法版本号', () => {
    for (const version of ['1', '1.0', 'v1.0.0', '']) {
      const result = validateThemeJson({ ...validTheme(), version });
      expect(result.ok).toBe(false);
      if (!result.ok) {
        expect(result.errors.some((e) => e.field === 'version')).toBe(true);
      }
    }
  });

  it('拒绝白名单之外的颜色 token（拼错必须报错而不是静默忽略）', () => {
    const result = validateThemeJson({
      ...validTheme(),
      colors: { canvasx: '#101418' },
    });
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(
        result.errors.some((e) => e.code === 'unknownToken' && e.field === 'colors.canvasx'),
      ).toBe(true);
    }
  });

  it('拒绝非法颜色值（非十六进制/rgb）', () => {
    const result = validateThemeJson({
      ...validTheme(),
      colors: { canvas: 'blue' },
    });
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.errors.some((e) => e.code === 'invalidColor')).toBe(true);
    }
  });

  it('拒绝非法的字体与字号缩放设置', () => {
    const result = validateThemeJson({
      ...validTheme(),
      fonts: { ui: '   ', sizeScale: 3 },
    });
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.errors.some((e) => e.field === 'fonts.ui')).toBe(true);
      expect(result.errors.some((e) => e.field === 'fonts.sizeScale')).toBe(true);
    }
  });

  it('拒绝 xterm 段的未知键与非法颜色', () => {
    const result = validateThemeJson({
      ...validTheme(),
      xterm: { background: '#101418', notAKey: '#000000' },
    });
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.errors.some((e) => e.field === 'xterm.notAKey')).toBe(true);
    }
  });

  it('一次返回全部错误而不是首个错误', () => {
    const result = validateThemeJson({ id: 'X', name: '', appearance: 'nope', version: 'bad' });
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.errors.length).toBeGreaterThanOrEqual(4);
    }
  });

  it('接受 rgb() 形式的颜色（scrim 等需要透明度）', () => {
    const result = validateThemeJson({
      ...validTheme(),
      colors: { scrim: 'rgb(15 17 21 / 0.45)' },
    });
    expect(result.ok).toBe(true);
  });

  it('接受合法的动效时长（ms / s 两种写法）', () => {
    const result = validateThemeJson({
      ...validTheme(),
      motion: { fast: '90ms', base: '0.2s' },
    });
    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.theme.motion).toEqual({ fast: '90ms', base: '0.2s' });
    }
  });

  it('拒绝非法或过大的动效时长（防止主题把界面变卡）', () => {
    const result = validateThemeJson({
      ...validTheme(),
      motion: { fast: 'soon', base: '5000ms', slow: '320ms' },
    });
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.errors.some((e) => e.field === 'motion.fast')).toBe(true);
      // 单值上限 2000ms：1 秒的过渡已经很难用，5 秒是错误量级
      expect(result.errors.some((e) => e.field === 'motion.base')).toBe(true);
      expect(result.errors.some((e) => e.field === 'motion.slow')).toBe(false);
    }
  });
});

describe('颜色解析与对比度', () => {
  it('解析 #hex 短写与 6 位写法', () => {
    expect(parseColor('#fff')).toEqual({ r: 255, g: 255, b: 255 });
    expect(parseColor('#101418')).toEqual({ r: 0x10, g: 0x14, b: 0x18 });
    expect(parseColor('nope')).toBeNull();
  });

  it('黑与白对比度为 21:1，相同颜色为 1:1', () => {
    expect(contrastRatio('#000000', '#ffffff')).toBeCloseTo(21, 1);
    expect(contrastRatio('#808080', '#808080')).toBeCloseTo(1, 5);
  });

  it('相对亮度介于 0 与 1 之间且白色最高', () => {
    const black = relativeLuminance(parseColor('#000000') ?? { r: 0, g: 0, b: 0 });
    const white = relativeLuminance(parseColor('#ffffff') ?? { r: 255, g: 255, b: 255 });
    expect(black).toBeCloseTo(0, 5);
    expect(white).toBeCloseTo(1, 5);
  });

  it('内置主题不出现不达标色对；空覆盖的内置主题全部继承', () => {
    for (const theme of BUILTIN_THEMES) {
      for (const row of contrastReport(theme)) {
        expect(row.state).not.toBe('fail');
        if (theme.colors.fg === undefined) {
          expect(row.state).toBe('inherited');
        }
      }
    }
  });

  it('自定义主题的达标与不达标色对分别标记', () => {
    const good = contrastReport({
      appearance: 'dark',
      colors: { fg: '#f2f4f8', canvas: '#101418', brand: '#4f46e5', brandFg: '#ffffff' },
    });
    expect(good.find((row) => row.label === 'body')?.state).toBe('pass');
    expect(good.find((row) => row.label === 'button')?.state).toBe('pass');

    // 按钮底色与文字对比不足（白字配浅色按钮）
    const bad = contrastReport({
      appearance: 'light',
      colors: { fg: '#14161a', canvas: '#f7f8fa', brand: '#cbd5f5', brandFg: '#ffffff' },
    });
    expect(bad.find((row) => row.label === 'button')?.state).toBe('fail');
  });
});
