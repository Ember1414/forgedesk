/**
 * Monaco 主题：跟随应用主题（含自定义主题色板）。
 *
 * # 为什么不用 `theme="vs-dark"`
 *
 * 写死一个主题等于"外观设置管不到编辑器"：亮色用户看到的是一块黑框，
 * 切主题时编辑器纹丝不动（2026-10-08 反馈的"它的背景只有黑色吗"）。
 *
 * # 做法：把设计令牌读出来喂给 Monaco
 *
 * 不新造第二套颜色真相源——直接用 `--fd-*` 令牌（`src/ui/tokens.css`）：
 *   - 应用切 light/dark → 令牌值变 → 重新 defineTheme；
 *   - 用户选了带色板的自定义主题 → 那些 `--fd-*` 被覆盖 → 编辑器跟着变。
 *
 * 因此"编辑器配色"永远与界面一致，且**只有一处**定义。
 */
import type { Monaco } from '@monaco-editor/react';

import { currentResolvedTheme, onResolvedThemeChange } from '@/app/theme';
import { subscribeActiveTheme } from '@/features/themes/activeCustomTheme';

/** 主题名（单一名，每次变化时重新定义，避免注册表里堆积）。 */
export const MONACO_THEME_NAME = 'forgedesk';

/** `defineTheme` 的数据形状（从 Monaco 实例的类型里取，避免额外 import 类型命名空间）。 */
export type MonacoThemeData = Parameters<Monaco['editor']['defineTheme']>[1];

/** 读一个 CSS 变量的当前值（未定义时用兜底值，保证离线/测试环境也能出主题）。 */
function readToken(styles: CSSStyleDeclaration, name: string, fallback: string): string {
  const value = styles.getPropertyValue(name).trim();
  return value === '' ? fallback : value;
}

/** 依据当前生效的设计令牌构造 Monaco 主题。 */
export function buildMonacoTheme(): MonacoThemeData {
  const dark = currentResolvedTheme() === 'dark';
  // 兜底值只在"没有样式表"的环境（单测 jsdom）里用到，因此不必与主题完全一致
  const styles =
    typeof window === 'undefined' ? null : window.getComputedStyle(document.documentElement);
  const token = (name: string, fallback: string): string =>
    styles === null ? fallback : readToken(styles, name, fallback);

  const canvas = token('--fd-canvas', dark ? '#111418' : '#ffffff');
  const surface = token('--fd-surface', dark ? '#171b21' : '#f7f8fa');
  const sunken = token('--fd-surface-sunken', dark ? '#0d1013' : '#eef0f3');
  const line = token('--fd-line', dark ? '#2a313a' : '#dfe3e8');
  const fg = token('--fd-fg', dark ? '#e6e9ee' : '#14181d');
  const fgMuted = token('--fd-fg-muted', dark ? '#a7b0bb' : '#4c5561');
  const fgSubtle = token('--fd-fg-subtle', dark ? '#7c8794' : '#6b7480');
  const brand = token('--fd-brand', dark ? '#7aa2f7' : '#2f5fd0');
  const brandSubtle = token('--fd-brand-subtle', dark ? '#1d2a44' : '#e6ecfb');
  const success = token('--fd-success', dark ? '#5fd08a' : '#1f7a45');
  const warning = token('--fd-warning', dark ? '#e0b153' : '#8a6100');
  const danger = token('--fd-danger', dark ? '#ef7a7a' : '#b3261e');
  const info = token('--fd-info', dark ? '#78b7e8' : '#1a5fa8');
  const spark = token('--fd-spark', dark ? '#d0a2f0' : '#7a3fa8');

  return {
    base: dark ? 'vs-dark' : 'vs',
    inherit: true,
    rules: [
      { token: '', foreground: fg.replace('#', '') },
      { token: 'comment', foreground: fgSubtle.replace('#', ''), fontStyle: 'italic' },
      { token: 'keyword', foreground: brand.replace('#', '') },
      { token: 'operator', foreground: fgMuted.replace('#', '') },
      { token: 'string', foreground: success.replace('#', '') },
      { token: 'number', foreground: spark.replace('#', '') },
      { token: 'constant', foreground: spark.replace('#', '') },
      { token: 'type', foreground: info.replace('#', '') },
      { token: 'type.identifier', foreground: info.replace('#', '') },
      { token: 'identifier', foreground: fg.replace('#', '') },
      { token: 'function', foreground: brand.replace('#', '') },
      { token: 'tag', foreground: danger.replace('#', '') },
      { token: 'attribute.name', foreground: warning.replace('#', '') },
      { token: 'attribute.value', foreground: success.replace('#', '') },
      { token: 'delimiter', foreground: fgMuted.replace('#', '') },
      // diff 视图（编辑器内的对比用不到，但保持一致）
      { token: 'string.diff', foreground: success.replace('#', '') },
    ],
    colors: {
      'editor.background': canvas,
      'editor.foreground': fg,
      'editorLineNumber.foreground': fgSubtle,
      'editorLineNumber.activeForeground': fg,
      'editorCursor.foreground': brand,
      'editor.selectionBackground': brandSubtle,
      'editor.inactiveSelectionBackground': brandSubtle,
      'editor.lineHighlightBackground': sunken,
      'editor.lineHighlightBorder': '#00000000',
      'editorGutter.background': canvas,
      'editorIndentGuide.background1': line,
      'editorIndentGuide.activeBackground1': fgSubtle,
      'editorWhitespace.foreground': line,
      'editorWidget.background': surface,
      'editorWidget.border': line,
      'editorSuggestWidget.background': surface,
      'editorSuggestWidget.selectedBackground': brandSubtle,
      'editorHoverWidget.background': surface,
      'editorHoverWidget.border': line,
      'editorBracketMatch.background': brandSubtle,
      'editorBracketMatch.border': brand,
      'editorError.foreground': danger,
      'editorWarning.foreground': warning,
      'editorOverviewRuler.border': '#00000000',
      'scrollbarSlider.background': line,
      'scrollbarSlider.hoverBackground': fgSubtle,
      'scrollbarSlider.activeBackground': fgMuted,
      'minimap.background': canvas,
      // 查找高亮：默认色在浅色主题下几乎看不见
      'editor.findMatchBackground': brandSubtle,
      'editor.findMatchHighlightBackground': brandSubtle,
      // 差异编辑（内联对比）也跟随主题
      'diffEditor.insertedTextBackground': 'transparent',
      'diffEditor.removedTextBackground': 'transparent',
      'diffEditor.insertedLineBackground': 'transparent',
      'diffEditor.removedLineBackground': 'transparent',
    },
  };
}

/** 已挂载的 Monaco 实例（变化时用它重新定义主题；未挂载时为 null）。 */
let mountedMonaco: Monaco | null = null;

/**
 * 编辑器的等宽字体栈。
 *
 * Monaco 只接受真实的字体列表（`fontFamily` 是 CSS 值，但不解析 `var()` 之外的
 * 层叠），因此把令牌里的值读出来传给它——保证编辑器与终端/代码块用的是同一套字体。
 */
export function monacoFontFamily(): string {
  if (typeof window === 'undefined') {
    return 'monospace';
  }
  const value = window
    .getComputedStyle(document.documentElement)
    .getPropertyValue('--fd-font-mono')
    .trim();
  return value === '' ? 'monospace' : value;
}

/**
 * 订阅"主题可能变了"的两种来源：
 *   - `onResolvedThemeChange`：light / dark 解析结果变化（含 system 跟随）；
 *   - `subscribeActiveTheme`：用户选了另一个主题（自定义色板被换上/卸下）。
 *
 * 返回退订函数；两者都触发"重新 defineTheme + setTheme"。
 */
export function subscribeMonacoThemeChanges(onChange: () => void): () => void {
  const offResolved = onResolvedThemeChange(onChange);
  const offActive = subscribeActiveTheme(onChange);
  return () => {
    offResolved();
    offActive();
  };
}

/**
 * 把当前主题应用到 Monaco。
 *
 * 传 `monaco` 表示"刚从 `onMount` 拿到实例"；不传则复用上次的实例
 * （主题变化时由订阅触发）。
 */
export function applyMonacoTheme(monaco?: Monaco): void {
  const instance = monaco ?? mountedMonaco;
  if (instance === undefined || instance === null) {
    return;
  }
  mountedMonaco = instance;
  instance.editor.defineTheme(MONACO_THEME_NAME, buildMonacoTheme());
  instance.editor.setTheme(MONACO_THEME_NAME);
}
