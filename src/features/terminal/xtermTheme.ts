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

/** 明色 ANSI 调色板（背景为浅色时的 16 色）。 */
const ANSI_LIGHT: readonly string[] = [
  '#3d434f', // black
  '#b3261e', // red
  '#12704a', // green
  '#9a6206', // yellow
  '#1d65c1', // blue
  '#7d42cf', // magenta
  '#0d6b78', // cyan
  '#8b9099', // white
  '#565d6b', // brightBlack
  '#d43f37', // brightRed
  '#1a8f5c', // brightGreen
  '#b57d13', // brightYellow
  '#3b82d9', // brightBlue
  '#9a6ad9', // brightMagenta
  '#1a97ab', // brightCyan
  '#c8cdd6', // brightWhite
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
