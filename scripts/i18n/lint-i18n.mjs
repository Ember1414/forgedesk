#!/usr/bin/env node
/**
 * i18n 硬编码文案检查（T0.6）。
 *
 * 为什么需要它：AGENTS.md §6 规定"所有用户可见文案走 i18n key"，
 * 但这条约定靠人记是不可靠的——提交一条硬编码中文的界面，测试与 lint 都不会拦。
 * 等到 M6 做中英完整覆盖时，才发现有一堆字符串散落在组件里，返工成本极高
 * （PLAN.md 风险表把这条列为高概率风险）。
 *
 * 判定规则（务实优先，避免误报淹没真实问题）：
 *   1. 只检查 `src/**` 下的 .ts/.tsx；
 *   2. 跳过测试文件（`*.test.ts(x)`、`__tests__/**`）：测试里出现中文是**应该**的，
 *      它断言的就是中文文案本身；
 *   3. 先剥掉注释（行注释、块注释），再找中日韩字符（CJK）。
 *      命中时若同一行在命中位置之前出现 `t(`（即文案确实被翻译函数包着），则放行；
 *   4. 豁免：文件首部含 `// i18n-ignore-file`（开发专用页面），
 *      或该行含 `// i18n-ignore`（单行豁免，必须带理由）。
 *
 * 已知局限（诚实的边界）：
 *   - 它是**文本扫描**而不是 AST 分析：`t(` 跨行书写、或中文出现在正则/模板里
 *     可能误判。当前代码风格（key 调用单行写完）下不触发；一旦出现误报，
 *     请在那一行加 `// i18n-ignore` 而不是放宽全局规则。
 *   - 它不检查"key 是否存在"——那属于 key 覆盖检查（i18n.test.ts 已覆盖中英对齐）。
 *
 * 用法：node scripts/i18n/lint-i18n.mjs
 */
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');
const srcDir = join(repoRoot, 'src');

/** 中日韩统一表意文字 + 常见中文标点（用于识别"用户可见中文"）。 */
const CJK_PATTERN = /[\u4e00-\u9fff\u3000-\u303f\uff00-\uffef]/;

const FILE_EXCLUSIONS = [
  /\.test\.tsx?$/,
  /\.spec\.tsx?$/,
  /[\\/]__tests__[\\/]/,
  /[\\/]__mocks__[\\/]/,
];

const problems = [];
let checkedFiles = 0;
let exemptFiles = 0;

/** 递归收集待检查文件。 */
function collectFiles(directory) {
  const entries = readdirSync(directory);
  const files = [];
  for (const entry of entries) {
    const fullPath = join(directory, entry);
    if (statSync(fullPath).isDirectory()) {
      files.push(...collectFiles(fullPath));
      continue;
    }
    if (!/\.tsx?$/.test(entry)) {
      continue;
    }
    const relativePath = relative(repoRoot, fullPath).split(sep).join('/');
    if (FILE_EXCLUSIONS.some((pattern) => pattern.test(relativePath))) {
      continue;
    }
    files.push(fullPath);
  }
  return files;
}

/**
 * 把注释替换成等长空白，保留行列位置。
 *
 * 为什么要保留长度：报错要给出准确的行号，报错行里若被截断就难以定位。
 * 为什么必须处理字符串状态：`"https://x"` 里的 `//` 不是注释开始，
 * 直接按行删除 `//` 之后的内容会把代码切坏、产生误报。
 */
function stripComments(source) {
  let output = '';
  let state = 'code'; // code | line-comment | block-comment | single | double | template

  for (let index = 0; index < source.length; index += 1) {
    const char = source[index];
    const next = source[index + 1];

    switch (state) {
      case 'code':
        if (char === '/' && next === '/') {
          state = 'line-comment';
          output += '  ';
          index += 1;
          continue;
        }
        if (char === '/' && next === '*') {
          state = 'block-comment';
          output += '  ';
          index += 1;
          continue;
        }
        if (char === "'") {
          state = 'single';
        } else if (char === '"') {
          state = 'double';
        } else if (char === '`') {
          state = 'template';
        }
        output += char;
        continue;

      case 'line-comment':
        if (char === '\n') {
          state = 'code';
          output += '\n';
          continue;
        }
        output += ' ';
        continue;

      case 'block-comment':
        if (char === '*' && next === '/') {
          state = 'code';
          output += '  ';
          index += 1;
          continue;
        }
        output += char === '\n' ? '\n' : ' ';
        continue;

      default: {
        // 字符串内部：保留内容（中文就可能在这里），但识别结束符
        if (char === '\\') {
          output += char + (next ?? '');
          index += 1;
          continue;
        }
        if (
          (state === 'single' && char === "'") ||
          (state === 'double' && char === '"') ||
          (state === 'template' && char === '`')
        ) {
          state = 'code';
        }
        output += char;
      }
    }
  }

  return output;
}

function checkFile(fullPath) {
  const source = readFileSync(fullPath, 'utf8');
  const relativePath = relative(repoRoot, fullPath).split(sep).join('/');

  const header = source.split('\n').slice(0, 6).join('\n');
  if (header.includes('i18n-ignore-file')) {
    exemptFiles += 1;
    return;
  }

  checkedFiles += 1;
  const stripped = stripComments(source);
  const strippedLines = stripped.split('\n');
  // 逐行豁免要读**原文**：注释已被剥离，标记本身不会出现在 stripped 行里
  const originalLines = source.split('\n');

  strippedLines.forEach((line, index) => {
    const match = CJK_PATTERN.exec(line);
    if (match === null) {
      return;
    }
    // 命中位置之前出现 t( 视为"已走 i18n"
    const before = line.slice(0, match.index);
    if (/(^|[^\w.])t\(/.test(before)) {
      return;
    }
    if ((originalLines[index] ?? '').includes('i18n-ignore')) {
      return;
    }
    problems.push({
      file: relativePath,
      line: index + 1,
      snippet: line.trim(),
    });
  });
}

if (!statSync(srcDir).isDirectory()) {
  console.error('未找到 src 目录。');
  process.exit(1);
}

for (const file of collectFiles(srcDir)) {
  checkFile(file);
}

console.log(
  `已检查 ${checkedFiles} 个源文件（豁免 ${exemptFiles} 个开发专用页面，跳过测试文件）。`,
);

if (problems.length > 0) {
  console.error('');
  console.error('发现未走 i18n 的用户可见文案：');
  for (const problem of problems) {
    console.error(`  ${problem.file}:${problem.line}`);
    console.error(`    ${problem.snippet}`);
  }
  console.error('');
  console.error('处理方式：');
  console.error('  1. 把文案移入 src/lib/i18n/locales/<lang>/*.json，组件里改用 t(...)；');
  console.error('  2. 开发专用页面（不进入生产构建）在文件首部加 // i18n-ignore-file；');
  console.error('  3. 个别无法翻译的内容（如日志、协议常量）在本行加 // i18n-ignore 并写明理由。');
  console.error('');
  console.error(`i18n 检查失败：${problems.length} 处硬编码文案。`);
  process.exit(1);
}

console.log('i18n 检查通过：未发现未走 key 的用户可见文案。');
