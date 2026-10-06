#!/usr/bin/env node
/**
 * i18n key 完整性检查（T6.7）。
 *
 * 三项断言（任一失败退出码 1）：
 *   1. zh-CN 与 en-US 的 key 集合完全一致（不多不少）；
 *   2. 无空文案（值 trim 后非空）；
 *   3. 插值占位符一致（`{{name}}` 两侧必须出现同一组占位符）。
 *
 * 与 i18n:lint 的分工：lint 管"源码里没写死文案"，本脚本管"两个语言包互相对得上"。
 * （另：src/lib/i18n/i18n.test.ts 在 Vitest 里也有类似断言——CI 双保险，
 * 本脚本让贡献者在本地不跑测试也能快速自检。）
 */
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');
const LOCALES_ROOT = join(repoRoot, 'src', 'lib', 'i18n', 'locales');
const LOCALES = ['zh-CN', 'en-US'];

const PLACEHOLDER = /\{\{([^}]+)\}\}/g;

/** 递归收集 { key: 文案 }（叶子是字符串）。 */
function flatten(object, prefix = '', out = new Map()) {
  for (const [key, value] of Object.entries(object)) {
    const path = prefix === '' ? key : `${prefix}.${key}`;
    if (value !== null && typeof value === 'object') {
      flatten(value, path, out);
    } else {
      out.set(path, String(value));
    }
  }
  return out;
}

function loadLocale(locale) {
  const root = join(LOCALES_ROOT, locale);
  const files = [];
  const walk = (dir) => {
    for (const entry of readdirSync(dir)) {
      const full = join(dir, entry);
      if (statSync(full).isDirectory()) {
        walk(full);
      } else if (entry.endsWith('.json')) {
        files.push(full);
      }
    }
  };
  walk(root);
  const keys = new Map();
  for (const file of files) {
    const namespace = relative(root, file)
      .replace(/\.json$/, '')
      .replaceAll('\\', '/');
    const data = JSON.parse(readFileSync(file, 'utf8'));
    for (const [key, value] of flatten(data)) {
      keys.set(`${namespace}:${key}`, { value, file: relative(repoRoot, file) });
    }
  }
  return keys;
}

const locales = Object.fromEntries(LOCALES.map((locale) => [locale, loadLocale(locale)]));
const problems = [];

// 1) key 集合一致
const [zhKeys, enKeys] = [locales['zh-CN'], locales['en-US']];
for (const key of zhKeys.keys()) {
  if (!enKeys.has(key)) {
    problems.push(`en-US 缺少 key：${key}`);
  }
}
for (const key of enKeys.keys()) {
  if (!zhKeys.has(key)) {
    problems.push(`zh-CN 缺少 key：${key}（en-US 独有）`);
  }
}

// 2) 空文案 + 3) 占位符一致
for (const locale of LOCALES) {
  for (const [key, { value, file }] of locales[locale]) {
    if (value.trim() === '') {
      problems.push(`${locale} 空文案：${key}（${file}）`);
    }
  }
}
for (const [key, { value }] of zhKeys) {
  const en = enKeys.get(key);
  if (en === undefined) {
    continue; // 已在 1) 里报过
  }
  const placeholders = (text) => new Set([...text.matchAll(PLACEHOLDER)].map((m) => m[1].trim()));
  const zhSet = placeholders(value);
  const enSet = placeholders(en.value);
  for (const name of zhSet) {
    if (!enSet.has(name)) {
      problems.push(`占位符不一致：${key} —— zh-CN 有 {{${name}}}，en-US 没有`);
    }
  }
  for (const name of enSet) {
    if (!zhSet.has(name)) {
      problems.push(`占位符不一致：${key} —— en-US 有 {{${name}}}，zh-CN 没有`);
    }
  }
}

if (problems.length > 0) {
  console.error(`i18n:check 发现 ${problems.length} 个问题：`);
  for (const problem of problems.slice(0, 50)) {
    console.error(`  - ${problem}`);
  }
  if (problems.length > 50) {
    console.error(`  …其余 ${problems.length - 50} 个略`);
  }
  process.exit(1);
}
console.log(
  `[PASS] i18n key 完整性：${LOCALES.join('/')} 各 ${zhKeys.size} 个 key，无缺失、无空文案、占位符一致。`,
);
