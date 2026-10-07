#!/usr/bin/env node
/**
 * 官网落地页的自检（M7 / T7.9）。
 *
 * # 为什么这页值得一个脚本
 *
 * `site/index.html` 是仓库里**唯一没有自动化覆盖的面向用户的代码**
 * （`src/` 有 vitest、`crates/` 有 cargo test、其余脚本由 CI 跑）。它却直接决定
 * 用户能不能下载到正确的东西：其中一段内联脚本要读取发布清单、再按命名约定拼出
 * 下载地址——拼错一个字符的后果是"官网上的下载按钮 404"，而发现它的时机往往是
 * 用户已经在看官网了。
 *
 * # 做法
 *
 * 用 jsdom 把页面**真的加载起来**（含内联脚本），注入假的 `fetch`，断言两种状态：
 *
 *   1. 没有清单（尚未发布 / 404 / 托管方返回 HTML 兜底页）→ 保持"尚无可用版本"，
 *      且**不出现任何下载链接**（宁可什么都不给，也不给一个必然 404 的入口）；
 *   2. 有清单 → 出现 4 个链接，且地址指向同一次发布的约定产物名。
 *
 * 第 2 条的清单夹具**由 `make-updater-manifest.mjs` 真实生成**（而不是手写一份）：
 * 这样"页面拼出的资产名"与"发布流程写出的清单"必须同时对得上，任一环节改名都会在这里红。
 *
 * 与 e2e（playwright）的分工：e2e 测应用窗口，这里测静态页；不启浏览器，快且无依赖外部服务。
 *
 * 用法：node scripts/ci/check-site.mjs
 */
import { execFileSync } from 'node:child_process';
import { mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { JSDOM } from 'jsdom';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');
const html = readFileSync(join(repoRoot, 'site', 'index.html'), 'utf8');
const version = JSON.parse(
  readFileSync(join(repoRoot, 'src-tauri', 'tauri.conf.json'), 'utf8'),
).version;

let failures = 0;
function assert(condition, message) {
  if (condition) {
    console.log(`  ✓ ${message}`);
    return;
  }
  failures += 1;
  console.error(`  ✗ ${message}`);
}

/** 载入页面并跑完内联脚本（含它发起的那个 fetch）。 */
async function loadPage(fetchImpl) {
  const dom = new JSDOM(html, {
    runScripts: 'dangerously',
    url: 'https://forgedesk.pages.dev/',
    beforeParse(window) {
      window.fetch = fetchImpl;
      // jsdom 不实现剪贴板；页面在缺失时会走"复制失败，请手动选择"分支，
      // 这里给一个实现以免噪声，但**不断言**复制结果（那是浏览器行为）
      Object.defineProperty(window.navigator, 'clipboard', {
        value: { writeText: () => Promise.resolve() },
        configurable: true,
      });
    },
  });
  // 让 fetch 的 Promise 链跑完（页面用 .then/.catch，没有 await 点）
  await new Promise((resolveTick) => {
    setTimeout(resolveTick, 0);
  });
  return dom;
}

/** 收集页面上的下载链接（href 列表）。 */
function downloadHrefs(document) {
  return [...document.querySelectorAll('#downloads a')].map((anchor) =>
    anchor.getAttribute('href'),
  );
}

// ---- 状态 1：没有清单（尚未发布） ------------------------------------------
console.log('状态 1：没有发布清单（404）');
{
  const dom = await loadPage(() => Promise.resolve({ ok: false }));
  const { document } = dom.window;
  const status = document.getElementById('status').textContent;
  const downloads = document.getElementById('downloads');
  const lead = document.getElementById('download-lead').textContent;

  assert(status.includes('尚无可用版本'), `状态如实显示「尚无可用版本」（实际：${status.trim()}）`);
  assert(downloads.hidden, '下载区保持隐藏（不给出必然 404 的入口）');
  assert(downloadHrefs(document).length === 0, '页面上没有任何下载链接');
  assert(lead.includes('尚未发布'), '引导文案说明"尚未发布"');
  assert(
    document.getElementById('windows-note').hidden,
    '没有版本时不显示 SmartScreen 提示（那条提示此时没有指代对象）',
  );
  dom.window.close();
}

// ---- 状态 2：有清单（用真实生成器造夹具） ----------------------------------
console.log('状态 2：有发布清单（夹具由 make-updater-manifest.mjs 生成）');
{
  const work = join(repoRoot, 'target', 'site-check');
  rmSync(work, { recursive: true, force: true });
  mkdirSync(work, { recursive: true });

  const signaturePath = join(work, 'fake.sig');
  const manifestPath = join(work, 'windows-x86_64.json');
  writeFileSync(signaturePath, 'rehearsal-signature\n');
  execFileSync(
    process.execPath,
    [
      join(repoRoot, 'scripts', 'ci', 'make-updater-manifest.mjs'),
      '--version',
      version,
      '--target',
      'windows-x86_64',
      '--signature',
      signaturePath,
      '--url',
      `https://github.com/Ember1414/forgedesk/releases/download/v${version}/ForgeDesk_${version}_windows_x64.exe`,
      '--pub-date',
      '2026-10-07T00:00:00Z',
      '--out',
      manifestPath,
    ],
    { cwd: repoRoot, stdio: 'ignore' },
  );
  const manifest = JSON.parse(readFileSync(manifestPath, 'utf8'));
  const base = `https://github.com/Ember1414/forgedesk/releases/download/v${version}`;

  const dom = await loadPage(() =>
    Promise.resolve({ ok: true, json: () => Promise.resolve(manifest) }),
  );
  const { document } = dom.window;
  const hrefs = downloadHrefs(document);

  assert(
    document.getElementById('status').textContent.includes(`v${version}`),
    `状态显示最新版本 v${version}`,
  );
  assert(!document.getElementById('downloads').hidden, '下载区展开');
  assert(hrefs.length === 4, `4 个下载入口（实际 ${hrefs.length}）`);
  for (const [expected, label] of [
    [`${base}/ForgeDesk_${version}_windows_x64.exe`, 'NSIS 安装器'],
    [`${base}/ForgeDesk_${version}_windows_x64.msi`, 'MSI 安装包'],
    [`${base}/ForgeDesk_${version}_windows_x64_portable.zip`, '便携版 zip'],
    [`${base}/SHA256SUMS`, '校验和文件'],
  ]) {
    assert(hrefs.includes(expected), `${label}指向 ${expected.split('/').pop()}`);
  }
  assert(
    document.getElementById('windows-note').textContent.includes('SmartScreen'),
    '显示 SmartScreen 提示（未购买代码签名证书的如实说明）',
  );
  dom.window.close();

  rmSync(work, { recursive: true, force: true });
}

if (failures > 0) {
  console.error(`\n官网页面自检失败：${failures} 项。`);
  process.exit(1);
}
console.log('\n官网页面自检通过（无清单 / 有清单两种状态）。');
