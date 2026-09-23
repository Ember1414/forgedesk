#!/usr/bin/env node
/**
 * 把 .github/workflows 下所有 `uses: owner/repo@ref` 的 ref 解析成
 * 完整的 commit SHA，并就地改写文件。（供应链加固，见 AGENTS.md §8）
 *
 * 何时运行：docs/OPEN-SOURCE-READINESS.md 中「固定 action 到 commit SHA」一项。
 * 需要网络访问 api.github.com（未认证时 60 次/小时，本仓库用量约 6-8 次）。
 *
 * 用法：
 *   node scripts/ci/pin-actions.mjs --dry-run    # 只打印将要做的替换
 *   node scripts/ci/pin-actions.mjs              # 实际改写文件
 *
 * 注意：本脚本不会自动执行 —— 它属于"转公开前的审计动作"，需要人工确认结果。
 */
import { readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');
const workflowsDir = join(repoRoot, '.github', 'workflows');
const dryRun = process.argv.includes('--dry-run');

/** 匹配 `owner/repo@ref`（只处理 @ref 部分不是 40 位 SHA 的情况） */
const USES_PATTERN = /(uses:\s*)([A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+)@([A-Za-z0-9_./-]+)/g;

/** 已解析过的 ref 缓存，避免同一 action 重复请求 API */
const resolvedCache = new Map();

/**
 * 通过 GitHub API 把 ref 解析为 commit SHA。
 * 使用 /commits/{ref} 端点（对分支与 tag 都适用，且返回的是 commit 本身）。
 */
async function resolveRef(action, ref) {
  const key = `${action}@${ref}`;
  const cached = resolvedCache.get(key);
  if (cached !== undefined) {
    return cached;
  }

  const url = `https://api.github.com/repos/${action}/commits/${encodeURIComponent(ref)}`;
  const response = await fetch(url, {
    headers: {
      accept: 'application/vnd.github+json',
      'user-agent': 'forgedesk-pin-actions',
    },
  });

  if (!response.ok) {
    const remaining = response.headers.get('x-ratelimit-remaining');
    const hint =
      remaining === '0'
        ? '（GitHub API 未认证请求已达速率上限，请稍后重试，或设置 GITHUB_TOKEN 环境变量）'
        : '';
    throw new Error(`无法解析 ${key}：HTTP ${response.status} ${response.statusText} ${hint}`);
  }

  const body = await response.json();
  const sha = body?.sha;
  if (typeof sha !== 'string' || !/^[0-9a-f]{40}$/.test(sha)) {
    throw new Error(`无法从响应中取得 ${key} 的 commit SHA`);
  }

  resolvedCache.set(key, sha);
  return sha;
}

const files = readdirSync(workflowsDir).filter(
  (name) => name.endsWith('.yml') || name.endsWith('.yaml'),
);

let totalReplacements = 0;

for (const fileName of files) {
  const path = join(workflowsDir, fileName);
  const original = readFileSync(path, 'utf8');

  const matches = [...original.matchAll(USES_PATTERN)].filter((match) => {
    const ref = match[3];
    return ref !== undefined && !/^[0-9a-f]{40}$/.test(ref);
  });

  if (matches.length === 0) {
    console.log(`${fileName}: 无需改动（已是 SHA 或无用例）`);
    continue;
  }

  let updated = original;

  for (const match of matches) {
    const [whole, prefix, action, ref] = match;
    if (prefix === undefined || action === undefined || ref === undefined) {
      continue;
    }

    const sha = await resolveRef(action, ref);
    const replacement = `${prefix}${action}@${sha} # ${ref}`;
    updated = updated.replace(whole, replacement);
    totalReplacements += 1;
    console.log(`${fileName}: ${action}@${ref} → ${sha}`);
  }

  if (!dryRun) {
    writeFileSync(path, updated, 'utf8');
  }
}

console.log('');
console.log(
  `${dryRun ? '[dry-run] 将改写' : '已改写'} ${files.length} 个文件，共 ${totalReplacements} 处 action 引用。`,
);
if (dryRun) {
  console.log('去掉 --dry-run 以实际写入。');
}
