/**
 * 内置主题（T6.6 §2）。
 *
 * ForgeDesk Light / Dark 的 colors 为空：内置两套外观的**唯一真相是 tokens.css**，
 * 这里只提供元数据与"空覆盖"（应用空覆盖 = 清除自定义变量，回到 tokens.css）。
 * 若在这里复制一份色值，切主题就会出现第三份真相，迟早漂移。
 *
 * Sandstone Dawn / Pine Nocturne 为本项目原创配色与命名（红线 R3：
 * 不得复刻竞品或知名 IDE 主题），完整给出 colors，导入的自定义主题同理。
 */

import type { ThemeDefinition } from './themeModel';

/** 内置主题列表（顺序即画廊展示顺序）。 */
export const BUILTIN_THEMES: readonly ThemeDefinition[] = [
  {
    id: 'forgedesk-light',
    name: 'ForgeDesk Light',
    appearance: 'light',
    version: '1.0.0',
    colors: {},
  },
  {
    id: 'forgedesk-dark',
    name: 'ForgeDesk Dark',
    appearance: 'dark',
    version: '1.0.0',
    colors: {},
  },
  {
    id: 'sandstone-dawn',
    name: 'Sandstone Dawn',
    appearance: 'light',
    version: '1.0.0',
    colors: {
      canvas: '#f6f1e7',
      surface: '#fdfaf3',
      surfaceRaised: '#fffdf8',
      surfaceSunken: '#ede5d3',
      line: '#e0d7c2',
      lineStrong: '#c3b591',
      fg: '#2b2416',
      fgMuted: '#5f543c',
      fgSubtle: '#7a6e52',
      fgInverted: '#fdfaf3',
      brand: '#9a3412',
      brandHover: '#7c2d12',
      brandFg: '#fdfaf3',
      brandSubtle: '#f4e5d0',
      spark: '#b45309',
      success: '#166534',
      warning: '#92400e',
      danger: '#b91c1c',
      info: '#1e40af',
    },
  },
  {
    id: 'pine-nocturne',
    name: 'Pine Nocturne',
    appearance: 'dark',
    version: '1.0.0',
    colors: {
      canvas: '#0c1512',
      surface: '#12201b',
      surfaceRaised: '#182a22',
      surfaceSunken: '#081009',
      line: '#233830',
      lineStrong: '#374f42',
      fg: '#e7efe9',
      fgMuted: '#a2b9ab',
      fgSubtle: '#869e8f',
      fgInverted: '#0c1512',
      brand: '#6ee7b7',
      brandHover: '#93efc9',
      brandFg: '#07130d',
      brandSubtle: '#122a1f',
      spark: '#fbbf24',
      success: '#4ade80',
      warning: '#fbbf24',
      danger: '#f87171',
      info: '#7dd3fc',
    },
  },
];

/** 按外观取默认主题（非法主题回退时的目标，T6.6 §3）。 */
export function builtinThemeFor(appearance: 'light' | 'dark'): ThemeDefinition {
  const fallback = BUILTIN_THEMES.at(appearance === 'dark' ? 1 : 0);
  if (fallback !== undefined) {
    return fallback;
  }
  // BUILTIN_THEMES 是常量且两个外观各有一条：这一支只兜底防御性编程
  return {
    id: 'forgedesk-light',
    name: 'ForgeDesk Light',
    appearance: 'light',
    version: '1.0.0',
    colors: {},
  };
}

/** 按 id 找内置主题。 */
export function findBuiltinTheme(id: string): ThemeDefinition | undefined {
  return BUILTIN_THEMES.find((theme) => theme.id === id);
}
