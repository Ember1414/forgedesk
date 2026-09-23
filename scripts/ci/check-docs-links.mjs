#!/usr/bin/env node
/**
 * 文档链接校验（M0 / T0.9）。
 *
 * 校验对象：仓库内的 Markdown 文档（README、CONTRIBUTING、AGENTS、docs/**、.github/**）。
 * 校验内容：
 *   1. 相对链接指向的**文件确实存在**（`docs/ARCHITECTURE.md`、`../AGENTS.md` 等）；
 *   2. 带锚点的链接（`API.md#logs_tail`）其锚点在目标文件里**确实存在**；
 *   3. 图片引用的路径存在（README 里的截图/图标）。
 *
 * 刻意**不**做的事：
 *   - 不访问网络：外部链接（http/https）只统计数量并打印提示，不做可达性判断。
 *     原因是 CI 需要确定性——外网抖动不该让构建失败，且未公开仓库的链接可能在外部不可见。
 *   - 不解析代码块内的内容：示例里的 `[...](...)` 不是链接。
 *
 * 锚点规则与 GitHub 一致（这也是读者点击时的真实行为）：标题转小写、去掉标点、
 * 再**把每个空白字符各自替换成一个连字符**（注意不是把连续空白合并成一个——
 * `### settings_get / settings_set` 里的斜杠被去掉后会留下两个空格，
 * GitHub 生成的锚点因此有两个连字符：`settings_get--settings_set`）。
 *
 * 用法：
 *   node scripts/ci/check-docs-links.mjs          # 校验
 *   node scripts/ci/check-docs-links.mjs --list   # 额外打印外部链接清单
 */
import { readFileSync, statSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { execFileSync } from 'node:child_process';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');
const listExternal = process.argv.includes('--list');

/**
 * 参与校验的文档。
 *
 * 用 git 列出（避免误扫 node_modules/target 等），并且**同时包含尚未提交的新文件**
 * （`--others --exclude-standard`）：否则新写的文档要等到提交后才会被校验，
 * 而"提交前就该发现断链"正是这个脚本存在的理由。
 */
function markdownFiles() {
  const output = execFileSync(
    'git',
    ['ls-files', '-z', '--cached', '--others', '--exclude-standard', '*.md'],
    { cwd: repoRoot, encoding: 'utf8' },
  );
  return [...new Set(output.split('\0').filter(Boolean))]
    .filter((path) => !path.startsWith('src/') && !path.startsWith('site/'))
    .sort();
}

/** 去掉代码块与行内代码，避免把它们内部的示例链接当成真链接。 */
function stripCodeBlocks(text) {
  return text.replace(/```[\s\S]*?```/g, '').replace(/`[^`\n]*`/g, '');
}

/**
 * 收集文档里的链接：返回 `{ kind, target }` 列表。
 *
 * 标签部分允许嵌套一层 `[...](...)`：徽章写法 `[![CI](图)](链接)` 的外层链接
 * 也要被识别出来（否则 `--list` 会少列外部链接）。
 */
function collectLinks(text) {
  const links = [];
  const pattern = /(!?)\[((?:[^[\]]|\[[^\]]*\]\([^)]*\))*)\]\(([^)\s]+)(?:\s+"[^"]*")?\)/g;
  for (const match of text.matchAll(pattern)) {
    links.push({ kind: match[1] === '!' ? 'image' : 'link', target: match[3] });
  }
  return links;
}

/** 文件里所有标题的锚点（GitHub 风格）。 */
function anchorsOf(text) {
  const anchors = new Set();
  for (const line of stripCodeBlocks(text).split(/\r?\n/)) {
    const heading = /^(#{1,6})\s+(.*)$/.exec(line);
    if (heading === null) {
      continue;
    }
    anchors.add(slugify(heading[2]));
  }
  return anchors;
}

function slugify(heading) {
  return heading
    .trim()
    .toLowerCase()
    .replace(/[^\p{L}\p{N}\s_-]/gu, '')
    .replace(/\s/g, '-');
}

const errors = [];
const externalLinks = new Set();
let checkedDocuments = 0;
let checkedLinks = 0;

const anchorCache = new Map();
function anchorsFor(absolutePath) {
  if (!anchorCache.has(absolutePath)) {
    anchorCache.set(absolutePath, anchorsOf(readFileSync(absolutePath, 'utf8')));
  }
  return anchorCache.get(absolutePath);
}

for (const relativePath of markdownFiles()) {
  const absolutePath = join(repoRoot, relativePath);
  const text = stripCodeBlocks(readFileSync(absolutePath, 'utf8'));
  checkedDocuments += 1;

  for (const { kind, target } of collectLinks(text)) {
    if (
      target.startsWith('http://') ||
      target.startsWith('https://') ||
      target.startsWith('mailto:')
    ) {
      externalLinks.add(target);
      continue;
    }
    // 纯锚点（同文件内跳转）
    if (target.startsWith('#')) {
      checkedLinks += 1;
      const anchor = target.slice(1);
      if (!anchorsFor(absolutePath).has(anchor)) {
        errors.push(`${relativePath}: 锚点不存在 "…${target}"（目标文件内没有这个标题）`);
      }
      continue;
    }

    const [pathPart, anchorPart] = target.split('#');
    // 在某些环境里路径会带前导 ./，统一去掉再解析
    const targetPath = resolve(
      dirname(absolutePath),
      decodeURIComponent(pathPart.replace(/^\.\//, '')),
    );
    checkedLinks += 1;

    const exists = (() => {
      try {
        statSync(targetPath);
        return true;
      } catch {
        return false;
      }
    })();

    if (!exists) {
      errors.push(
        `${relativePath}: ${kind === 'image' ? '图片' : '链接'}指向的文件不存在 -> ${target}`,
      );
      continue;
    }

    if (anchorPart !== undefined && anchorPart !== '' && targetPath.endsWith('.md')) {
      if (!anchorsFor(targetPath).has(anchorPart)) {
        errors.push(
          `${relativePath}: 锚点不存在 -> ${target}（目标文件内没有这个标题：${anchorPart}）`,
        );
      }
    }
  }
}

const externalNote = `（另有 ${externalLinks.size} 个外部链接未做网络校验——见脚本头部说明）`;

if (errors.length > 0) {
  console.error(`文档链接校验失败，共 ${errors.length} 处问题：`);
  for (const error of errors) {
    console.error(`  ERROR ${error}`);
  }
  console.error(externalNote);
  process.exit(1);
}

if (listExternal) {
  console.log('外部链接清单：');
  for (const url of [...externalLinks].sort()) {
    console.log(`  ${url}`);
  }
}

console.log(
  `文档链接校验通过：${checkedDocuments} 份文档、${checkedLinks} 个内部链接/图片全部有效。` +
    externalNote,
);
