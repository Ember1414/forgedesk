#!/usr/bin/env node
/**
 * 扫描 src 下未被豁免文件中的颜色字面量（T6.6 §5）。
 *
 * 为什么要有这个脚本：主题系统的前提是"所有颜色经 CSS 变量"。
 * 一个组件里手写的 #4f46e5 不会立刻坏，但它绕过了主题切换——
 * 用户换了主题，那块颜色纹丝不动。与其靠评审肉眼，不如让 CI 拦。
 *
 * 白名单（颜色"定义"允许出现字面量，颜色"使用"不允许）：
 *   - src/ui/tokens.css（唯一的 token 真相源）
 *   - src/features/themes/**（主题定义、派生与它们的测试）
 *   - e2e/**（E2E 断言视觉值时需要比较具体颜色）
 *
 * 退出码：发现未豁免的颜色字面量时为 1，并逐条列出文件与行号。
 */
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');
const SCAN_ROOTS = ['src', 'e2e'].map((part) => join(repoRoot, part));
const EXTENSIONS = new Set(['.ts', '.tsx', '.css']);

const ALLOWLIST_DIR_PREFIXES = [join(repoRoot, 'src', 'features', 'themes'), join(repoRoot, 'e2e')];
const ALLOWLIST_FILES = [join(repoRoot, 'src', 'ui', 'tokens.css')];

// 颜色字面量：#hex（3-8 位）、rgb()/rgba()、hsl()/hsla()、oklch()/oklab()、
// color()，以及 Tailwind 调色板类（bg-slate-800 这类"具体色名+数字阶"组合）
const COLOR_PATTERNS = [
  /#[0-9a-fA-F]{3,8}\b/g,
  /\b(?:rgba?|hsla?|oklch|oklab|color)\s*\(/g,
  /\b(?:bg|text|border|ring|fill|stroke|from|via|to|shadow)-(?:slate|gray|zinc|neutral|stone|red|orange|amber|yellow|lime|green|emerald|teal|cyan|sky|blue|indigo|violet|purple|fuchsia|pink|rose)-\d{2,3}\b/g,
];

function isAllowlisted(path) {
  return (
    ALLOWLIST_FILES.includes(path) ||
    ALLOWLIST_DIR_PREFIXES.some(
      (prefix) =>
        path === prefix || path.startsWith(`${prefix}\\`) || path.startsWith(`${prefix}/`),
    )
  );
}

function* walk(dir) {
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    const stat = statSync(full);
    if (stat.isDirectory()) {
      if (entry === 'node_modules' || entry === 'target' || entry === '__dev__') {
        continue; // __dev__ 是开发专用页面，不进生产构建（CODING_STYLE §3.1）
      }
      yield* walk(full);
    } else if (EXTENSIONS.has(entry.slice(entry.lastIndexOf('.')))) {
      yield full;
    }
  }
}

const violations = [];
for (const root of SCAN_ROOTS) {
  let exists = true;
  try {
    statSync(root);
  } catch {
    exists = false;
  }
  if (!exists) {
    continue;
  }
  for (const file of walk(root)) {
    if (isAllowlisted(file)) {
      continue;
    }
    const text = readFileSync(file, 'utf8');
    const lines = text.split(/\r?\n/);
    lines.forEach((line, index) => {
      for (const pattern of COLOR_PATTERNS) {
        pattern.lastIndex = 0;
        if (pattern.test(line)) {
          violations.push({
            file: relative(repoRoot, file),
            line: index + 1,
            text: line.trim().slice(0, 120),
          });
          break; // 每行只报一次，避免同一条命中多个模式刷屏
        }
      }
    });
  }
}

if (violations.length > 0) {
  console.error(`发现 ${violations.length} 处未豁免的颜色字面量：`);
  for (const violation of violations.slice(0, 50)) {
    console.error(`  ${violation.file}:${violation.line}  ${violation.text}`);
  }
  if (violations.length > 50) {
    console.error(`  …其余 ${violations.length - 50} 处略`);
  }
  console.error('\n请改用 src/ui/tokens.css 的语义 token（如 bg-surface / text-fg-muted）。');
  console.error('颜色定义请放在 tokens.css 或 src/features/themes/ 下的主题定义文件。');
  process.exit(1);
}
console.log('[PASS] 颜色字面量扫描：未发现未豁免的颜色字面量。');
