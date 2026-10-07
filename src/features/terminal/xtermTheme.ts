/**
 * xterm 主题映射（T5.2）：从设计 token（`src/ui/tokens.css` 的 `--fd-*`）派生。
 *
 * 为什么不写死两套主题色：任务的意图是"主题跟随应用主题"——token 层已经在
 * 明暗两套下各有一份通过 WCAG 校验的值，xterm 直接读当前生效的 CSS 变量，
 * 主题切换（含"跟随系统"）就自动跟随，不需要第二份真相源。
 *
 * ANSI 16 色（black..brightWhite）没有对应的语义 token：它们是终端的**固定
 * 调色板**（用户脚本与 CLI 约定的 8/16 色），语义 token 里没有等价物。
 * 这里给出两套原创取值（明/暗），只保证与背景的对比度与色相区分度，
 * 不复刻任何发行版主题的配色。
 */
import type { ITheme } from '@xterm/xterm';

/**
 * 明色 ANSI 调色板（背景为浅色时的 16 色）。全部 16 色对白色背景 ≥ 4.5:1（WCAG AA）。
 *
 * 两个刻意的反直觉取值：
 * - white 是中灰而不是浅灰（浅灰在白底上不可读）；
 * - brightWhite 是深炭色：cmd.exe 的默认前景就是 brightWhite，浅色主题下若按
 *   "最亮"取值，整个 cmd 会话的输出都会几乎不可见——浅色主题把 brightWhite
 *   映射为深色是 One Half Light 等主流浅色终端主题的通行做法。
 */
const ANSI_LIGHT: readonly string[] = [
  '#3d434f', // black
  '#b3261e', // red
  '#12704a', // green
  '#9a6206', // yellow
  '#1d65c1', // blue
  '#7d42cf', // magenta
  '#0d6b78', // cyan
  '#6b7280', // white
  '#565d6b', // brightBlack
  '#d43f37', // brightRed
  '#178556', // brightGreen
  '#9a6f10', // brightYellow
  '#2b74c9', // brightBlue
  '#8a54d1', // brightMagenta
  '#0f7f90', // brightCyan
  '#2f3540', // brightWhite
];

/** 暗色 ANSI 调色板（背景为深色时的 16 色）。 */
const ANSI_DARK: readonly string[] = [
  '#4a505c', // black
  '#e0675e', // red
  '#4bb47f', // green
  '#d3a13c', // yellow
  '#6aa3e8', // blue
  '#a883e8', // magenta
  '#45b3c4', // cyan
  '#a8aeb8', // white
  '#7d8390', // brightBlack
  '#ef8a82', // brightRed
  '#6fca93', // brightGreen
  '#e3b964', // brightYellow
  '#8db9f0', // brightBlue
  '#bfa0f0', // brightMagenta
  '#67c8d6', // brightCyan
  '#dfe3ea', // brightWhite
];

/** `--fd-font-mono` 读不到或为空时使用的具体字体栈（不含 canvas 无法解析的值）。 */
const FALLBACK_MONO_FONT = "'Cascadia Code', 'JetBrains Mono', Consolas, 'Courier New', monospace";

/**
 * 解析出 canvas 可用的等宽字体栈。
 *
 * 为什么不能把 `var(--fd-font-mono, …)` 直接交给 xterm：xterm 会把 fontFamily
 * 拼进 canvas 2D 的 `ctx.font`，而 `var()` 在那里不是合法值——整条声明被静默
 * 忽略，字形回退到 `10px sans-serif`，但格子尺寸却按 DOM 度量的 13px 排布，
 * 结果就是"字符特别小且不是等宽字体"。`ui-monospace` 这类 CSS 系统关键字在
 * canvas 字体简写里同样不可靠，一并剔除。
 */
export function resolveTerminalFontFamily(): string {
  const raw = getComputedStyle(document.documentElement).getPropertyValue('--fd-font-mono').trim();
  if (raw === '') {
    return FALLBACK_MONO_FONT;
  }
  const families = raw
    .split(',')
    .map((family) => family.trim())
    .filter((family) => family !== '' && !/^ui-mono(?:space)?$/i.test(family));
  return families.length > 0 ? families.join(', ') : FALLBACK_MONO_FONT;
}

/** 读取当前主题下的 xterm 颜色（每次调用实时取值，含明暗切换后的新值）。 */
export function deriveXtermTheme(): ITheme {
  const root = document.documentElement;
  const style = getComputedStyle(root);
  const value = (name: string, fallback: string): string =>
    style.getPropertyValue(name).trim() || fallback;
  const isDark = root.getAttribute('data-theme') === 'dark';
  // noUncheckedIndexedAccess：索引访问是 string | undefined，
  // 调色板是本文件内的字面量数组，长度恒为 16——这里取一次非空断言并集中付账
  const ansi = (isDark ? ANSI_DARK : ANSI_LIGHT) as readonly string[] & { length: 16 };
  const at = (index: number): string => ansi[index] ?? '#888888';

  return {
    background: value('--fd-surface', isDark ? '#171a20' : '#ffffff'),
    foreground: value('--fd-fg', isDark ? '#eceef2' : '#14161a'),
    cursor: value('--fd-fg', '#888888'),
    cursorAccent: value('--fd-surface', '#ffffff'),
    selectionBackground: value('--fd-brand-subtle', isDark ? '#2a2f4a' : '#eef0fe'),
    selectionForeground: value('--fd-fg', '#14161a'),
    black: at(0),
    red: at(1),
    green: at(2),
    yellow: at(3),
    blue: at(4),
    magenta: at(5),
    cyan: at(6),
    white: at(7),
    brightBlack: at(8),
    brightRed: at(9),
    brightGreen: at(10),
    brightYellow: at(11),
    brightBlue: at(12),
    brightMagenta: at(13),
    brightCyan: at(14),
    brightWhite: at(15),
  };
}
