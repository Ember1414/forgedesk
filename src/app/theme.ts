/**
 * 主题解析与应用（M0 / T0.4）。
 *
 * 为什么单独成一个模块而不是塞进 store：主题要满足两个约束——
 *   1. **首帧就正确**：入口在渲染 React 之前就要把 `data-theme` 写到 <html>，
 *      否则暗色用户会先看到一帧白屏（闪白）。这需要不依赖 React 的同步函数。
 *   2. **只有一处真相**：store、设置页、开发预览页都读写同一份逻辑，避免各写一份
 *      localStorage key 与"系统主题"判断（T0.3 的预览页曾自行读写 'forgedesk.theme'）。
 *
 * CSS 只处理确定的 light / dark 两种状态（见 src/ui/tokens.css 的约定），
 * 'system' 在这里被解析成具体值后再写入 <html data-theme>。
 */

/** 存储键。T0.3 的预览页已使用同名键，这里保持兼容。 */
export const THEME_STORAGE_KEY = 'forgedesk.theme';

export const THEME_MODES = ['light', 'dark', 'system'] as const;
export type ThemeMode = (typeof THEME_MODES)[number];

/** 实际写入 DOM 的主题（不含 'system'）。 */
export type ResolvedTheme = 'light' | 'dark';

const DARK_QUERY = '(prefers-color-scheme: dark)';

/** 系统是否偏好暗色（非浏览器环境或无法探测时按亮色处理）。 */
export function prefersDarkScheme(): boolean {
  if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') {
    return false;
  }
  return window.matchMedia(DARK_QUERY).matches;
}

export function isThemeMode(value: string): value is ThemeMode {
  return (THEME_MODES as readonly string[]).includes(value);
}

/** 把模式解析为最终主题。 */
export function resolveTheme(mode: ThemeMode): ResolvedTheme {
  if (mode === 'system') {
    return prefersDarkScheme() ? 'dark' : 'light';
  }
  return mode;
}

/** 读取用户选择的模式；缺省为 'system'（首次启动跟随系统最不容易出错）。 */
export function readThemeMode(): ThemeMode {
  try {
    const stored = window.localStorage.getItem(THEME_STORAGE_KEY);
    return stored !== null && isThemeMode(stored) ? stored : 'system';
  } catch {
    return 'system';
  }
}

export function writeThemeMode(mode: ThemeMode): void {
  try {
    window.localStorage.setItem(THEME_STORAGE_KEY, mode);
  } catch {
    /* 存储不可用时忽略：仅影响下次启动的初始主题 */
  }
}

/**
 * 把模式应用到 <html>，并同步 `color-scheme`
 * （后者决定原生控件与滚动条的配色，漏掉会在暗色下出现白色滚动条）。
 *
 * 解析结果变化后通知订阅者（T6.6：自定义主题的颜色归属某个外观，
 * resolved 值变化时必须重新套用/卸载自定义变量）。
 */
export function applyThemeMode(mode: ThemeMode): ResolvedTheme {
  const resolved = resolveTheme(mode);
  if (typeof document !== 'undefined') {
    const root = document.documentElement;
    root.setAttribute('data-theme', resolved);
    root.style.colorScheme = resolved;
  }
  for (const listener of resolvedListeners) {
    listener(resolved);
  }
  return resolved;
}

type ResolvedListener = (resolved: ResolvedTheme) => void;
const resolvedListeners: ResolvedListener[] = [];

/** 订阅解析结果变化（light/dark 切换，含 system 模式下的自动切换）。返回退订函数。 */
export function onResolvedThemeChange(listener: ResolvedListener): () => void {
  resolvedListeners.push(listener);
  return () => {
    const index = resolvedListeners.indexOf(listener);
    if (index >= 0) {
      resolvedListeners.splice(index, 1);
    }
  };
}

/** 当前解析后的外观（读 <html data-theme>；尚未初始化时按亮色处理）。 */
export function currentResolvedTheme(): ResolvedTheme {
  if (typeof document === 'undefined') {
    return 'light';
  }
  return document.documentElement.getAttribute('data-theme') === 'dark' ? 'dark' : 'light';
}

/**
 * 订阅系统外观变化。仅在 mode === 'system' 时需要调用。
 * 返回取消订阅函数；环境不支持时返回空函数，调用方无需分支。
 */
export function watchSystemTheme(onChange: (theme: ResolvedTheme) => void): () => void {
  if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') {
    return () => undefined;
  }
  const media = window.matchMedia(DARK_QUERY);
  const handler = (event: MediaQueryListEvent): void => {
    onChange(event.matches ? 'dark' : 'light');
  };
  media.addEventListener('change', handler);
  return () => {
    media.removeEventListener('change', handler);
  };
}
