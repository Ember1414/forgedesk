#!/usr/bin/env node
/**
 * 组装 Cloudflare Pages 的部署目录（site + 更新清单）。
 *
 * # 为什么必须把这件事收口到一个脚本
 *
 * Cloudflare Pages 的 Direct Upload 是**目录快照**：这次没带上的文件会被删除。
 * 而站点（`site/`）与更新清单（`updates/`）托管在**同一个 Pages 项目**里
 * （ADR-003 的 Direct Upload 方案），于是任何一边单独部署都会抹掉另一边。
 * 历史实现（release.yml 里手写的 bash）有三个缺口：
 *
 *   ① 跨渠道保留的文件名只列了 3/7：实测客户端在 Windows 上请求的是 `windows.json`
 *      这个别名（见 release.yml 的注释），而它不在保留列表里 —— stable 一发布，
 *      beta 的 macOS/Windows 清单就消失；
 *   ② `curl` 的网络失败与"文件确实不存在"被合并处理，默认动作是**删除**
 *      （fail-open：一次网络抖动等于删掉另一个渠道的清单）；
 *   ③ 站点自身的部署（pages.yml）完全不做补齐 —— 改一行站点文案，
 *      线上 `updates/**` 全没，所有客户端更新检查失败。
 *
 * 本脚本把链路收到一处，规则只有一条：**宁可中止部署，不可删除清单**。
 *
 *   - 先把 `site/` 复制进 staging；
 *   - 再把 stable/beta 两个渠道的**全部已知清单文件**从线上拉回；
 *   - 最后用本次发布产出的本地 `updates/` 覆盖（新版本优先）；
 *   - 任何网络失败 / 超时 / 非预期响应 / 内容校验不过 → 非零退出（部署中止）。
 *
 * 用法：
 *   node scripts/ci/stage-pages.mjs --out <staging> [--channel stable|beta] [--local-updates <目录>]
 *
 * 测试便捷性：`--origin` 可覆盖站点源（默认 https://forgedesk.pages.dev），
 * 用于本机验证"拉取失败必须中止"等分支。
 */
import { cpSync, existsSync, mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');

/** 更新清单的渠道。两个渠道都要补齐——发布只影响其中一个。 */
const CHANNELS = ['stable', 'beta'];

/**
 * 每个渠道下**实际会被写入**的文件全集（少一个都会造成某个平台断更）。
 *
 * 来源逐项核对：
 *   - `windows.json` / `windows-x86_64.json`：release.yml 写 Windows 清单时同时写两个别名
 *     （插件在 Windows 上实测请求 `windows.json`）；
 *   - `darwin.json` / `darwin-aarch64.json` / `darwin-x86_64.json`：macOS universal 同理；
 *   - `SHA256SUMS` / `SHA256SUMS.asc`：下载页同源渲染校验和用（`.asc` 可选）。
 */
const CHANNEL_FILES = [
  'windows.json',
  'windows-x86_64.json',
  'darwin.json',
  'darwin-aarch64.json',
  'darwin-x86_64.json',
  'SHA256SUMS',
  'SHA256SUMS.asc',
];

function parseArgs(argv) {
  const options = {
    out: undefined,
    channel: undefined,
    localUpdates: undefined,
    origin: 'https://forgedesk.pages.dev',
  };
  for (let index = 0; index < argv.length; index += 1) {
    const key = argv[index];
    const next = argv[index + 1];
    switch (key) {
      case '--out':
        options.out = resolve(next ?? '');
        index += 1;
        break;
      case '--channel':
        options.channel = next;
        index += 1;
        break;
      case '--local-updates':
        options.localUpdates = resolve(next ?? '');
        index += 1;
        break;
      case '--origin':
        options.origin = (next ?? '').replace(/\/$/, '');
        index += 1;
        break;
      default:
        console.error(`未知参数：${key}`);
        process.exit(2);
    }
  }
  if (options.out === undefined) {
    console.error('缺少 --out <目录>。');
    process.exit(2);
  }
  if (options.channel !== undefined && !CHANNELS.includes(options.channel)) {
    console.error(`--channel 只支持 ${CHANNELS.join(' | ')}，收到：${options.channel}`);
    process.exit(2);
  }
  return options;
}

/** 内容校验：清单必须是可解析 JSON，校验和文件必须是标准 `sha256sum` 行。 */
function validateContent(fileName, text) {
  if (text.trim() === '') {
    return '内容为空';
  }
  if (fileName.endsWith('.json')) {
    try {
      JSON.parse(text);
    } catch {
      return '不是合法 JSON';
    }
    return null;
  }
  if (fileName.startsWith('SHA256SUMS') && fileName.endsWith('.asc')) {
    return text.includes('BEGIN PGP SIGNATURE') ? null : '不是 PGP 分离签名';
  }
  if (fileName.startsWith('SHA256SUMS')) {
    const lines = text.trim().split(/\r?\n/);
    const bad = lines.find((line) => !/^[0-9a-f]{64}\s{2}.+$/.test(line));
    return bad === undefined ? null : `校验和行格式不符：${bad.slice(0, 60)}`;
  }
  return null;
}

/**
 * 拉取一个线上文件，返回三种结果之一：
 *
 *   { kind: 'ok', text }      —— 存在且内容通过校验（调用方负责写盘）
 *   { kind: 'missing' }       —— 确实不存在（404，或 Pages 的 HTML 兜底）
 *   { kind: 'abort', reason } —— **未知状态**（网络错误/超时/5xx/内容不合规）→ 必须中止
 *
 * 关键：'missing' 只用于"确定不存在"，其余一律 abort。旧实现把这两类合并成
 * "跳过（=删除）"，一次抖动就能删掉另一个渠道的清单。
 */
async function fetchRemote(url) {
  const attempts = 3;
  for (let attempt = 1; attempt <= attempts; attempt += 1) {
    let response;
    try {
      response = await fetch(url, {
        redirect: 'follow',
        signal: AbortSignal.timeout(20_000),
        headers: { 'cache-control': 'no-cache' },
      });
    } catch (error) {
      if (attempt < attempts) {
        await new Promise((done) => setTimeout(done, attempt * 1000));
        continue;
      }
      return {
        kind: 'abort',
        reason: `请求失败：${error instanceof Error ? error.message : error}`,
      };
    }

    if (response.status === 404) {
      return { kind: 'missing' };
    }
    if (response.status !== 200) {
      // 5xx/403 等：既不能当作"不存在"，也不能当作"完好"
      return { kind: 'abort', reason: `HTTP ${response.status}` };
    }

    const contentType = (response.headers.get('content-type') ?? '').toLowerCase();
    const text = await response.text();
    // Cloudflare Pages 对不存在的路径返回 200 + 首页 HTML（实测），这算"不存在"；
    // 但 content-type 是 HTML 而路径明明是 .json/.asc，说明内容不对，仍按缺失去处理
    if (contentType.includes('text/html') || text.trimStart().startsWith('<!DOCTYPE')) {
      return { kind: 'missing' };
    }

    const problem = validateContent(new URL(url).pathname.split('/').pop() ?? '', text);
    if (problem !== null) {
      return { kind: 'abort', reason: `内容校验未通过：${problem}` };
    }
    return { kind: 'ok', text };
  }
  return { kind: 'abort', reason: '重试耗尽' };
}

// ------------------------------------------------------------------ 主流程

const options = parseArgs(process.argv.slice(2));
const siteDir = join(repoRoot, 'site');
if (!existsSync(siteDir)) {
  console.error(`找不到站点目录：${siteDir}`);
  process.exit(1);
}

/**
 * 复制 site/ 但**排除 `site/updates/<渠道>/**`**。
 *
 * 为什么：`site/updates/` 被 .gitignore 忽略，但它里面分两类东西——
 *   - `gpg-pubkey.asc`：build-site.mjs 的正经产物，必须部署；
 *   - `updates/stable/**` 等渠道目录：开发机上一次本地演练留下的**过期残留**
 *     （实测本机就有 v1.0.0 的 SHA256SUMS）。带上它们会把旧清单发到线上。
 * 清单的正确来源只有两个：线上拉回（历史渠道）与本次发布（--local-updates）。
 */
function siteFilter(source) {
  const parts = relative(siteDir, source).split(sep);
  if (parts[0] === 'updates' && parts[1] !== undefined && CHANNELS.includes(parts[1])) {
    return false;
  }
  return true;
}

// 从干净目录开始：staging 里混入上一次的残留会静默发布过期文件
rmSync(options.out, { recursive: true, force: true });
mkdirSync(options.out, { recursive: true });
cpSync(siteDir, options.out, { recursive: true, filter: siteFilter });

const preserved = [];
const skippedChannel = options.channel;
for (const channel of CHANNELS) {
  for (const file of CHANNEL_FILES) {
    // 当前发布渠道的新文件由 --local-updates 提供，不做线上拉回（省请求也更清晰）
    if (channel === skippedChannel) {
      continue;
    }
    const url = `${options.origin}/updates/${channel}/${file}`;
    const result = await fetchRemote(url);
    if (result.kind === 'abort') {
      console.error(`✗ 无法确认 ${url} 的状态（${result.reason}）。`);
      console.error('  为避免把线上清单删掉，本次部署已中止。请重试工作流。');
      process.exit(1);
    }
    if (result.kind === 'missing') {
      continue;
    }
    const target = join(options.out, 'updates', channel, file);
    mkdirSync(dirname(target), { recursive: true });
    writeFileSync(target, result.text);
    preserved.push(`updates/${channel}/${file}`);
  }
}

let localCount = 0;
if (options.localUpdates !== undefined) {
  if (!existsSync(options.localUpdates)) {
    console.error(`--local-updates 指向的目录不存在：${options.localUpdates}`);
    process.exit(1);
  }
  // 本地（本次发布）的文件后写：同名文件以新版本为准
  cpSync(options.localUpdates, join(options.out, 'updates'), { recursive: true });
  localCount = CHANNEL_FILES.length;
}

console.log(`staging 目录已组装：${options.out}`);
console.log(`  站点文件：site/ 全量`);
console.log(
  `  线上保留：${preserved.length} 个文件${preserved.length > 0 ? `（${preserved.join('、')}）` : ''}`,
);
if (options.localUpdates !== undefined) {
  console.log(`  本次发布覆盖：updates/${options.channel}/（最多 ${localCount} 个文件）`);
}
