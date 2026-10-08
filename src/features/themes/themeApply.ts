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
  if (theme.motion !== undefined) {
    for (const key of ['fast', 'base', 'slow'] as const) {
      const value = theme.motion[key];
      if (value !== undefined) {
        const name = `--fd-duration-${key}`;
        root.style.setProperty(name, value);
        appliedVarNames.push(name);
      }
    }
  }
}

/** 主题画廊预览要用的令牌（画一个迷你界面所需的全部颜色）。 */
export const PREVIEW_TOKENS = [
  'canvas',
  'surface',
  'surfaceSunken',
  'line',
  'fg',
  'fgMuted',
  'fgSubtle',
  'brand',
  'success',
  'warning',
  'danger',
] as const;

const paletteCache = new Map<'light' | 'dark', Record<string, string>>();

/**
 * 读出**某个外观**下的真实令牌值。
 *
 * 为什么不能直接读 `document.documentElement`：那只会给出*当前*外观的颜色，
 * 于是画廊里的暗色主题卡片会显示成亮色配色（"预览与实际不符"）。
 * 这里用一个离屏元素挂上 `data-theme`，让 CSS 自己算出该外观的令牌——
 * 不复制任何色值，`tokens.css` 仍是唯一真相源。
 */
export function resolveAppearancePalette(appearance: 'light' | 'dark'): Record<string, string> {
  const cached = paletteCache.get(appearance);
  if (cached !== undefined) {
    return cached;
  }
  if (typeof document === 'undefined') {
    return {};
  }
  // 优先从样式表读：`tokens.css` 的 `:root` / `[data-theme='dark']` 规则是真相源。
  // 这很重要——自定义主题的变量是 inline 挂在 <html> 上的，离屏探测元素会**继承**
  // 到它们，于是"基础主题"的卡片会显示成自定义主题的颜色（预览与实际不符）。
  const fromSheets = paletteFromStyleSheets(appearance);
  const palette =
    Object.keys(fromSheets).length >= PREVIEW_TOKENS.length
      ? fromSheets
      : { ...paletteFromProbe(appearance), ...fromSheets };
  paletteCache.set(appearance, palette);
  return palette;
}

/** 跨源样式表读不到规则；同源（Tauri 自定义协议）都能读。 */
function readStyleRules(sheet: CSSStyleSheet): CSSRuleList | null {
  try {
    return sheet.cssRules;
  } catch {
    return null;
  }
}

/** 直接读 CSSOM：只取定义该外观的规则里的 `--fd-*`。 */
function paletteFromStyleSheets(appearance: 'light' | 'dark'): Record<string, string> {
  const wanted = appearance === 'dark' ? ["[data-theme='dark']", '[data-theme="dark"]'] : [':root'];
  const palette: Record<string, string> = {};
  for (const sheet of Array.from(document.styleSheets)) {
    const rules = readStyleRules(sheet);
    if (rules === null) {
      continue;
    }
    for (const rule of Array.from(rules)) {
      if (!(rule instanceof CSSStyleRule)) {
        continue;
      }
      if (!wanted.some((selector) => rule.selectorText.includes(selector))) {
        continue;
      }
      for (const property of Array.from(rule.style)) {
        if (property.startsWith('--fd-')) {
          palette[property.slice('--fd-'.length)] = rule.style.getPropertyValue(property).trim();
        }
      }
    }
  }
  return palette;
}

/**
 * 离屏探测：让 CSS 自己算出该外观的令牌值（CSSOM 拿不到足够令牌时的兜底，
 * 例如样式由 JS 动态注入且规则不可枚举的场景）。
 */
function paletteFromProbe(appearance: 'light' | 'dark'): Record<string, string> {
  const palette: Record<string, string> = {};
  const probe = document.createElement('div');
  probe.setAttribute('data-theme', appearance);
  probe.setAttribute('aria-hidden', 'true');
  probe.style.cssText =
    'position:absolute;left:-9999px;top:0;width:0;height:0;pointer-events:none;';
  document.body.appendChild(probe);
  const styles = window.getComputedStyle(probe);
  for (const token of PREVIEW_TOKENS) {
    palette[token] = styles.getPropertyValue(cssVarName(token)).trim();
  }
  probe.remove();
  return palette;
}

/** 主题在画廊预览里的最终配色（主题覆盖优先，其余取该外观的令牌）。 */
export function resolveThemePalette(theme: ThemeDefinition): Record<string, string> {
  const base = resolveAppearancePalette(theme.appearance);
  const merged: Record<string, string> = { ...base };
  for (const [token, value] of Object.entries(theme.colors)) {
    if (value !== undefined) {
      merged[token] = value;
    }
  }
  return merged;
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
