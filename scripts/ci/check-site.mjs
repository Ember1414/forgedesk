#!/usr/bin/env node
/**
 * 官网落地页的自检（M7 / T7.9）。
 *
 * # 为什么这页值得一个脚本
 *
 * `site/index.html` 是仓库里**唯一没有自动化覆盖的面向用户的代码**
 * （`src/` 有 vitest、`crates/` 有 cargo test、其余脚本由 CI 跑）。它却直接决定
 * 用户能不能下载到正确的东西：其中一段内联脚本要读取发布清单、按命名约定拼出
 * 下载地址、还要把同源发布的校验和渲染出来——拼错一个字符的后果是"官网上的
 * 下载按钮 404"或"页面上显示的校验和是错的"，而发现它的时机往往是用户已经在看官网了。
 *
 * # 做法
 *
 * 用 jsdom 把页面**真的加载起来**（含内联脚本），注入按 URL 分发的假 `fetch`，断言三种状态：
 *
 *   1. 没有清单（尚未发布 / 404 / 托管方返回 HTML 兜底页）→ 保持"尚无可用版本"，
 *      **不出现任何下载链接**，也不显示校验和；
 *   2. 有清单但没有校验和文件 → 下载入口正常，校验和区保持隐藏（静态校验步骤仍可用）；
 *   3. 有清单也有校验和 → 校验和区展开，且**显示的值与流水线写出的 SHA256SUMS 逐字一致**。
 *
 * 第 2/3 条的夹具都由真实脚本生成（`make-updater-manifest.mjs` / `make-checksums.mjs`），
 * 而不是手写：这样"页面显示的东西"与"发布流程写出的东西"必须同时对得上，
 * 任一环节改名或改格式都会在这里红。
 *
 * 与 e2e（playwright）的分工：e2e 测应用窗口，这里测静态页；不启浏览器、不需要网络。
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

const MANIFEST_URL = '/updates/stable/windows-x86_64.json';
const CHECKSUMS_URL = '/updates/stable/SHA256SUMS';

let failures = 0;
function assert(condition, message) {
  if (condition) {
    console.log(`  ✓ ${message}`);
    return;
  }
  failures += 1;
  console.error(`  ✗ ${message}`);
}

/** 跑一个仓库内的脚本（夹具一律由真实脚本生成）。 */
function runScript(scriptPath, args) {
  execFileSync(process.execPath, [join(repoRoot, 'scripts', 'ci', scriptPath), ...args], {
    cwd: repoRoot,
    stdio: 'ignore',
  });
}

/** 让挂起的 Promise 链跑完（页面里是 .then/.catch，没有 await 点）。 */
function flush() {
  return new Promise((resolveTick) => {
    setTimeout(resolveTick, 0);
  });
}

/**
 * 载入页面并跑完内联脚本。
 *
 * `routes` 是 `URL → { status, body }`；未列出的 URL 一律 404——页面必须自己扛住。
 */
async function loadPage(routes) {
  const fakeFetch = (url) =>
    Promise.resolve(
      routes[String(url)] === undefined
        ? { ok: false, status: 404 }
        : { ok: true, status: 200, ...routes[String(url)] },
    );

  const dom = new JSDOM(html, {
    runScripts: 'dangerously',
    url: 'https://forgedesk.pages.dev/',
    beforeParse(window) {
      window.fetch = fakeFetch;
      // jsdom 不实现剪贴板；页面在缺失时会走"复制失败，请手动选择"分支
      Object.defineProperty(window.navigator, 'clipboard', {
        value: { writeText: () => Promise.resolve() },
        configurable: true,
      });
    },
  });
  // 清单 → 渲染 → 再拉校验和，是两跳 Promise：刷两轮微/宏任务才稳
  await flush();
  await flush();
  return dom;
}

/** 页面上的下载链接（href 列表）。 */
function downloadHrefs(document) {
  return [...document.querySelectorAll('#downloads a')].map((anchor) =>
    anchor.getAttribute('href'),
  );
}

// ---- 夹具：清单与校验和都由真实脚本生成 ------------------------------------
const work = join(repoRoot, 'target', 'site-check');
rmSync(work, { recursive: true, force: true });
mkdirSync(join(work, 'releases'), { recursive: true });

const signaturePath = join(work, 'fake.sig');
const manifestPath = join(work, 'windows-x86_64.json');
writeFileSync(signaturePath, 'rehearsal-signature\n');
runScript('make-updater-manifest.mjs', [
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
]);
const manifest = JSON.parse(readFileSync(manifestPath, 'utf8'));

// 假产物 → 真 SHA256SUMS（与流水线同一个生成器、同一套命名约定）
const assetNames = [
  `ForgeDesk_${version}_windows_x64.exe`,
  `ForgeDesk_${version}_windows_x64.msi`,
  `ForgeDesk_${version}_windows_x64_portable.zip`,
];
for (const name of assetNames) {
  writeFileSync(join(work, 'releases', name), `rehearsal-${name}\n`);
}
runScript('make-checksums.mjs', ['--dir', join(work, 'releases')]);
const checksums = readFileSync(join(work, 'releases', 'SHA256SUMS'), 'utf8');
const base = `https://github.com/Ember1414/forgedesk/releases/download/v${version}`;

// ---- 状态 1：没有清单（尚未发布） ------------------------------------------
console.log('状态 1：没有发布清单（404）');
{
  const dom = await loadPage({});
  const { document } = dom.window;
  const status = document.getElementById('status').textContent;

  assert(status.includes('尚无可用版本'), `状态如实显示「尚无可用版本」（实际：${status.trim()}）`);
  assert(document.getElementById('downloads').hidden, '下载区保持隐藏（不给出必然 404 的入口）');
  assert(downloadHrefs(document).length === 0, '页面上没有任何下载链接');
  assert(
    document.getElementById('download-lead').textContent.includes('尚未发布'),
    '引导文案说明"尚未发布"',
  );
  assert(
    document.getElementById('windows-note').hidden,
    '没有版本时不显示 SmartScreen 提示（那条提示此时没有指代对象）',
  );
  assert(document.getElementById('checksums').hidden, '校验和区保持隐藏');
  dom.window.close();
}

// ---- 状态 2：有清单、没有校验和文件 ----------------------------------------
console.log('状态 2：有发布清单，但没有校验和文件');
{
  const dom = await loadPage({
    [MANIFEST_URL]: { json: () => Promise.resolve(manifest) },
  });
  const { document } = dom.window;
  const hrefs = downloadHrefs(document);

  assert(
    document.getElementById('status').textContent.includes(`v${version}`),
    `状态显示最新版本 v${version}`,
  );
  assert(!document.getElementById('downloads').hidden, '下载区展开');
  assert(hrefs.length === 4, `4 个下载入口（实际 ${hrefs.length}）`);
  assert(
    document.getElementById('windows-note').textContent.includes('SmartScreen'),
    '显示 SmartScreen 提示（未购买代码签名证书的如实说明）',
  );
  assert(
    document.getElementById('checksums').hidden,
    '缺少 SHA256SUMS 时校验和区仍隐藏（不显示空内容）',
  );
  dom.window.close();
}

// ---- 状态 3：有清单也有校验和 ---------------------------------------------
console.log('状态 3：有发布清单与校验和');
{
  const dom = await loadPage({
    [MANIFEST_URL]: { json: () => Promise.resolve(manifest) },
    [CHECKSUMS_URL]: { text: () => Promise.resolve(checksums) },
  });
  const { document } = dom.window;
  const hrefs = downloadHrefs(document);

  assert(hrefs.length === 4, `4 个下载入口（实际 ${hrefs.length}）`);
  for (const [expected, label] of [
    [`${base}/ForgeDesk_${version}_windows_x64.exe`, 'NSIS 安装器'],
    [`${base}/ForgeDesk_${version}_windows_x64.msi`, 'MSI 安装包'],
    [`${base}/ForgeDesk_${version}_windows_x64_portable.zip`, '便携版 zip'],
    [`${base}/SHA256SUMS`, '校验和文件'],
  ]) {
    assert(hrefs.includes(expected), `${label}指向 ${expected.split('/').pop()}`);
  }

  assert(!document.getElementById('checksums').hidden, '校验和区展开');
  const rendered = document.getElementById('checksum-lines').textContent;
  assert(rendered === checksums.trim(), '页面显示的校验和与流水线写出的 SHA256SUMS 逐字一致');
  assert(
    assetNames.every((name) => rendered.includes(name)),
    '校验和覆盖全部三个 Windows 产物',
  );
  dom.window.close();
}

rmSync(work, { recursive: true, force: true });

if (failures > 0) {
  console.error(`\n官网页面自检失败：${failures} 项。`);
  process.exit(1);
}
console.log('\n官网页面自检通过（无清单 / 有清单无校验和 / 有清单有校验和 三种状态）。');
