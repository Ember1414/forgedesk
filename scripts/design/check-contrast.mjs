#!/usr/bin/env node
/**
 * 校验 **应用语义 token（src/ui/tokens.css）** 与 **官网配色（site/index.html）**
 * 的 WCAG 对比度。
 *
 * 为什么要有这个脚本：设计 token 的"可读性"很容易在后续调色时被无声破坏。
 * 把对比度变成 CI 可判定的门禁，而不是一句"看着还行"。
 *
 * 为什么把官网也纳入（M7 / T7.9）：官网是用户最先看到的东西，也是**唯一没有应用
 * 主题系统兜底**的界面——它的颜色直接写在页面里。只守应用、不守官网，等于把
 * "最深的第一印象"留在门禁之外。
 *
 * 退出码：存在不达标项时为 1。
 */
import { readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');
const TOKENS_PATH = join(repoRoot, 'src', 'ui', 'tokens.css');
const SITE_PATH = join(repoRoot, 'site', 'index.html');

// ---------- 解析 token ----------

/** 从 CSS 文本中提取指定选择器块内的 --fd-<name>: #rrggbb; 变量。 */
function parseTokenBlock(css, selector) {
  const selectorIndex = css.indexOf(selector);
  if (selectorIndex === -1) {
    throw new Error(`tokens.css 中未找到选择器：${selector}`);
  }
  const open = css.indexOf('{', selectorIndex);
  const close = css.indexOf('}', open);
  if (open === -1 || close === -1) {
    throw new Error(`选择器 ${selector} 的块不完整`);
  }
  const body = css.slice(open + 1, close);
  const vars = {};
  const re = /--fd-([a-z0-9-]+)\s*:\s*(#[0-9a-fA-F]{6})\s*;/g;
  for (const match of body.matchAll(re)) {
    vars[match[1]] = match[2].toLowerCase();
  }
  return vars;
}

/**
 * 提取官网页面里的**全部** `:root { … }` 变量块（官网不带 `--fd-` 前缀）。
 *
 * 约定：第一个是亮色，第二个（位于 `@media (prefers-color-scheme: dark)` 内）是暗色。
 * 少一个块就报错——官网的深色主题是"用户系统设置为深色时看到的样子"，
 * 它不能被悄悄删掉而不被发现。
 */
function parseSiteThemes(html) {
  const bodies = [];
  const re = /:root\s*\{([^}]*)\}/g;
  for (const match of html.matchAll(re)) {
    const vars = {};
    for (const varMatch of match[1].matchAll(/--([a-z0-9-]+)\s*:\s*(#[0-9a-fA-F]{6})\s*;/g)) {
      vars[varMatch[1]] = varMatch[2].toLowerCase();
    }
    bodies.push(vars);
  }
  if (bodies.length !== 2) {
    throw new Error(
      `site/index.html 里应当有且只有两个 :root 色值块（亮色 + 暗色），实际 ${bodies.length} 个。`,
    );
  }
  return [
    { theme: 'site 亮色', vars: bodies[0] },
    { theme: 'site 暗色', vars: bodies[1] },
  ];
}

// ---------- WCAG 相对亮度与对比度 ----------

function channelToLinear(value) {
  const s = value / 255;
  return s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
}

function relativeLuminance(hex) {
  const r = parseInt(hex.slice(1, 3), 16);
  const g = parseInt(hex.slice(3, 5), 16);
  const b = parseInt(hex.slice(5, 7), 16);
  return 0.2126 * channelToLinear(r) + 0.7152 * channelToLinear(g) + 0.0722 * channelToLinear(b);
}

function contrastRatio(a, b) {
  const la = relativeLuminance(a);
  const lb = relativeLuminance(b);
  const [hi, lo] = la > lb ? [la, lb] : [lb, la];
  return (hi + 0.05) / (lo + 0.05);
}

// ---------- 检查项 ----------

/** 文字类：WCAG AA 正文要求 4.5:1 */
const TEXT_PAIRS = [
  ['fg', 'canvas', '正文 / 画布'],
  ['fg', 'surface', '正文 / 面板'],
  ['fg', 'surface-raised', '正文 / 浮起面板'],
  ['fg-muted', 'canvas', '次要文字 / 画布'],
  ['fg-muted', 'surface', '次要文字 / 面板'],
  ['fg-subtle', 'surface', '弱化文字 / 面板'],
  ['fg-inverted', 'brand', '反色文字 / 品牌色'],
  ['brand', 'surface', '品牌色文字 / 面板'],
  ['brand', 'canvas', '品牌色文字 / 画布'],
  ['spark', 'surface', '强调色 / 面板'],
  ['success', 'surface', '成功色 / 面板'],
  ['warning', 'surface', '警告色 / 面板'],
  ['danger', 'surface', '危险色 / 面板'],
  ['info', 'surface', '信息色 / 面板'],
];

/** 非文本类：焦点环等需 >= 3:1（WCAG 1.4.11） */
const NON_TEXT_PAIRS = [['brand', 'canvas', '焦点环 / 画布']];

/**
 * 官网实际出现的文字/背景组合。
 *
 * 官网没有语义 token 层，因此这里逐条列出页面里真实存在的配对
 * （每加一处新的文字底色，都要在这里补一行——否则它就是"没人守着的文字"）。
 * 页面上所有文字都在 12–16px，按 WCAG 的"非大号文字"处理，一律 4.5:1。
 */
const SITE_TEXT_PAIRS = [
  ['fg', 'canvas', '正文 / 画布'],
  ['fg', 'surface', '正文 / 面板'],
  ['fg', 'surface-sunken', '正文 / 代码块'],
  ['fg-muted', 'canvas', '次要文字 / 画布'],
  ['fg-muted', 'surface', '次要文字 / 面板'],
  ['fg-muted', 'surface-sunken', '次要文字 / 代码块'],
  ['link', 'surface', '链接与按钮 / 面板'],
  ['link', 'canvas', '链接与按钮 / 画布'],
];

/** 仅报告不判定：分隔线属于装饰性元素，不适用 1.4.11 */
const INFO_PAIRS = [
  ['line', 'surface', '分隔线 / 面板'],
  ['line-strong', 'surface', '强分隔线 / 面板'],
];

const TEXT_MIN = 4.5;
const NON_TEXT_MIN = 3;

// ---------- 执行 ----------

const css = readFileSync(TOKENS_PATH, 'utf8');
const themes = [
  { name: 'light（亮色）', vars: parseTokenBlock(css, ':root') },
  { name: 'dark（暗色）', vars: parseTokenBlock(css, "[data-theme='dark']") },
];

let failures = 0;
const rows = [];

for (const { name, vars } of themes) {
  for (const [fg, bg, label] of TEXT_PAIRS) {
    assertVars(name, vars, fg, bg);
    rows.push({ theme: name, label, ratio: contrastRatio(vars[fg], vars[bg]), min: TEXT_MIN });
  }
  for (const [fg, bg, label] of NON_TEXT_PAIRS) {
    assertVars(name, vars, fg, bg);
    rows.push({ theme: name, label, ratio: contrastRatio(vars[fg], vars[bg]), min: NON_TEXT_MIN });
  }
  for (const [fg, bg, label] of INFO_PAIRS) {
    assertVars(name, vars, fg, bg);
    rows.push({ theme: name, label, ratio: contrastRatio(vars[fg], vars[bg]), min: null });
  }
}

// 官网（site/index.html）：同一套阈值，但变量名不带 --fd- 前缀
for (const { theme, vars } of parseSiteThemes(readFileSync(SITE_PATH, 'utf8'))) {
  for (const [fg, bg, label] of SITE_TEXT_PAIRS) {
    for (const name of [fg, bg]) {
      if (vars[name] === undefined) {
        throw new Error(`官网主题「${theme}」缺少变量 --${name}（site/index.html）`);
      }
    }
    rows.push({ theme, label, ratio: contrastRatio(vars[fg], vars[bg]), min: TEXT_MIN });
  }
}

for (const row of rows) {
  const status = row.min === null ? 'INFO' : row.ratio >= row.min ? 'PASS' : 'FAIL';
  if (status === 'FAIL') failures += 1;
  const threshold = row.min === null ? '  -  ' : row.min.toFixed(1);
  console.log(
    `${status}  ${row.theme.padEnd(14)} ${row.label.padEnd(24)} ${row.ratio.toFixed(2).padStart(6)} : 1   (阈值 ${threshold})`,
  );
}

console.log('');
if (failures > 0) {
  console.error(
    `对比度校验失败：${failures} 项低于阈值。请调整 src/ui/tokens.css 或 site/index.html 中的色值。`,
  );
  process.exit(1);
}
console.log(
  `对比度校验通过：共检查 ${rows.length} 项（含官网配色；WCAG AA，正文 4.5:1 / 非文本 3:1）。`,
);

function assertVars(themeName, vars, ...names) {
  for (const name of names) {
    if (vars[name] === undefined) {
      throw new Error(`主题 ${themeName} 缺少 token --fd-${name}`);
    }
  }
}
