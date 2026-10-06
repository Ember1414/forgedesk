/**
 * 主题的应用层（T6.6）。
 *
 * 职责边界：
 *  - themeModel.ts 负责"什么是合法主题"（纯数据）；
 *  - 本文件负责"主题如何落到 DOM"：写 CSS 变量、首帧缓存、终端配色派生。
 *
 * # 为什么自定义主题要有 localStorage 直写缓存
 *
 * 内置亮/暗主题在 main.tsx 渲染前同步应用（防白闪，见 app/theme.ts）；
 * 自定义主题的持久层是 SQLite（settingsStore，异步），如果等它加载完再上色，
 * 用户每次启动都会先看到一帧内置配色。因此在**写入**自定义主题时同步镜像一份
 * 到 localStorage，启动时同步读取——SQLite 仍是真相源，缓存只是首帧投影，
 * 两者不一致时以 SQLite 为准（启动后 reconcile）。
 */

import type { ThemeColorToken, ThemeDefinition } from './themeModel';

/** 当前激活的自定义主题缓存键（值为完整 ThemeDefinition JSON；未激活时移除）。 */
export const ACTIVE_CUSTOM_THEME_CACHE_KEY = 'forgedesk.customTheme';

/** camelCase token → CSS 变量名（surfaceRaised → --fd-surface-raised）。 */
function cssVarName(token: ThemeColorToken): string {
  const kebab = token.replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`);
  return `--fd-${kebab}`;
}

/** camelCase 字体 token → CSS 变量名。 */
function cssFontVarName(key: 'ui' | 'mono'): string {
  return key === 'ui' ? '--fd-font-sans' : '--fd-font-mono';
}

/** 当前通过 inline style 挂上的自定义变量名（卸载时要逐一移除）。 */
let appliedVarNames: string[] = [];

/**
 * 应用（或清除）自定义主题的 CSS 变量。
 *
 * 仅当 `resolvedAppearance` 与主题自身外观一致时才上色：主题归属于某个外观
 * （"跟随系统"决定亮/暗后，如果当前外观不是该主题的外观，展示内置配色）。
 * 传 `theme = null` 即清除（回到内置主题）。
 */
export function applyCustomThemeColors(
  theme: ThemeDefinition | null,
  resolvedAppearance: 'light' | 'dark',
): void {
  if (typeof document === 'undefined') {
    return;
  }
  const root = document.documentElement;
  for (const name of appliedVarNames) {
    root.style.removeProperty(name);
  }
  appliedVarNames = [];

  if (theme === null || theme.appearance !== resolvedAppearance) {
    return;
  }
  for (const [token, value] of Object.entries(theme.colors)) {
    const name = cssVarName(token as ThemeColorToken);
    root.style.setProperty(name, value);
    appliedVarNames.push(name);
  }
  if (theme.fonts !== undefined) {
    for (const key of ['ui', 'mono'] as const) {
      const value = theme.fonts[key];
      if (value !== undefined) {
        const name = cssFontVarName(key);
        root.style.setProperty(name, value);
        appliedVarNames.push(name);
      }
    }
    // sizeScale 为预留字段：当前字阶为固定 px，不缩放（校验层仍接受它，见 themeModel）
  }
}

/** 从 localStorage 读取首帧缓存的自定义主题；损坏的缓存按未激活处理。 */
export function readCachedCustomTheme(): ThemeDefinition | null {
  try {
    const raw = window.localStorage.getItem(ACTIVE_CUSTOM_THEME_CACHE_KEY);
    if (raw === null) {
      return null;
    }
    const parsed: unknown = JSON.parse(raw);
    if (typeof parsed === 'object' && parsed !== null && 'id' in parsed && 'colors' in parsed) {
      return parsed as ThemeDefinition;
    }
    return null;
  } catch {
    return null;
  }
}

/** 写入（或清除）首帧缓存。启动路径依赖它，因此不能用异步存储。 */
export function writeCachedCustomTheme(theme: ThemeDefinition | null): void {
  try {
    if (theme === null) {
      window.localStorage.removeItem(ACTIVE_CUSTOM_THEME_CACHE_KEY);
    } else {
      window.localStorage.setItem(ACTIVE_CUSTOM_THEME_CACHE_KEY, JSON.stringify(theme));
    }
  } catch {
    /* 存储不可用时忽略：只影响下次启动的首帧配色，SQLite 会在加载后纠正 */
  }
}

/**
 * 由主题派生 xterm 配色（T6.6 §6：终端主题与界面主题同源）。
 *
 * `resolve` 返回 token 的最终颜色（自定义覆盖优先，缺省回退内置值）——
 * 通过注入而非内部读取 computedStyle，保持纯函数可测；真实调用方
 * 传 `getComputedStyle(document.documentElement).getPropertyValue(...)` 的包装。
 * 未覆盖的 xterm 键按语义就近映射（如 red→danger、cyan→graphLane5），
 * 这是可用的近似而非精确设计；终端侧如需微调，在主题 JSON 的 xterm 段覆盖。
 */
export function deriveXtermPalette(
  theme: Pick<ThemeDefinition, 'appearance' | 'colors' | 'xterm'>,
  resolve: (token: ThemeColorToken) => string,
): Record<string, string> {
  const mapping: readonly (readonly [string, ThemeColorToken])[] = [
    ['background', 'canvas'],
    ['foreground', 'fg'],
    ['cursor', 'brand'],
    ['cursorAccent', 'canvas'],
    ['selectionBackground', 'brandSubtle'],
    ['black', 'line'],
    ['red', 'danger'],
    ['green', 'success'],
    ['yellow', 'warning'],
    ['blue', 'info'],
    ['magenta', 'graphLane6'],
    ['cyan', 'graphLane5'],
    ['white', 'fgMuted'],
    ['brightBlack', 'fgSubtle'],
    ['brightRed', 'danger'],
    ['brightGreen', 'success'],
    ['brightYellow', 'warning'],
    ['brightBlue', 'info'],
    ['brightMagenta', 'graphLane6'],
    ['brightCyan', 'graphLane5'],
    ['brightWhite', 'fg'],
  ];
  const palette: Record<string, string> = {};
  for (const [xtermKey, token] of mapping) {
    palette[xtermKey] = theme.colors[token] ?? resolve(token);
  }
  // 显式的 xterm 段优先级最高：这是主题作者表达终端意图的出口
  if (theme.xterm !== undefined) {
    Object.assign(palette, theme.xterm);
  }
  return palette;
}
