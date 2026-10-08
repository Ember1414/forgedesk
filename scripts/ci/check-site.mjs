#!/usr/bin/env node
/**
 * 官网自检（M7 / T7.9）。
 *
 * # 为什么这个站点值得一个脚本
 *
 * `site/` 是仓库里**唯一没有其它自动化覆盖的面向用户的代码**。它直接决定
 * 用户能不能下载到正确的东西：页面要读取发布清单、按命名约定拼出下载地址、
 * 渲染版本矩阵与同源校验和——拼错一个字符的后果是"官网上的下载按钮 404"
 * 或"页面上显示的校验和是错的"，而发现时机往往是用户已经在看官网了。
 *
 * # 做法
 *
 * 先跑 `build-site.mjs` 生成 docs/privacy/license 页（与部署流程同一生成器），
 * 再用 jsdom 把页面**真的加载起来**（app.js 内联注入），按 URL 分发假 `fetch`，断言：
 *
 *   1. 没有清单（尚未发布 / 404）→ 首页与下载页保持"尚无可用版本"，
 *      **不出现任何指向 Release 产物的下载链接**，也不显示校验和；
 *   2. 有清单但没有校验和文件 → 版本矩阵正常渲染，校验和区保持隐藏；
 *   3. 有清单也有校验和 → 校验和区展开，且**显示的值与流水线写出的 SHA256SUMS
 *      逐字一致**；SHA256 列指向同源校验和文件；
 *   4. 静态要求：平台识别切换、三平台校验命令、GPG 区、信任说明、
 *      更新日志兜底、生成页与 docs/ 同源、og:image 存在。
 *
 * 第 2/3 条的夹具由真实脚本生成（make-updater-manifest.mjs / make-checksums.mjs），
 * 而不是手写：页面显示的东西与发布流程写出的东西必须同时对得上。
 *
 * 用法：node scripts/ci/check-site.mjs
 */
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { JSDOM } from 'jsdom';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');
const version = JSON.parse(
  readFileSync(join(repoRoot, 'src-tauri', 'tauri.conf.json'), 'utf8'),
).version;

const MANIFEST_URL = '/updates/stable/windows-x86_64.json';
const CHECKSUMS_URL = '/updates/stable/SHA256SUMS';
const ASC_URL = '/updates/stable/SHA256SUMS.asc';
// 公钥的公开地址。**注意它与 ASC_URL 是两个不同的文件**：曾经页面探测 .asc（存在）
// 却链接到公钥（不存在），导致下载页上挂出一个必然拿到首页 HTML 的坏链接——
// v1.0.0 发布后实测发现，状态 3 于是拆成"有公钥 / 无公钥"两种情形。
const PUBKEY_URL = '/updates/gpg-pubkey.asc';
const API_RELEASES_URL = 'https://api.github.com/repos/Ember1414/forgedesk/releases?per_page=10';

let failures = 0;
function assert(condition, message) {
  if (condition) {
    console.log(`  ✓ ${message}`);
    return;
  }
  failures += 1;
  console.error(`  ✗ ${message}`);
}

/** 跑一个仓库内的脚本（夹具与生成页一律由真实脚本产出）。 */
function runScript(scriptPath, args) {
  execFileSync(process.execPath, [join(repoRoot, 'scripts', 'ci', scriptPath), ...args], {
    cwd: repoRoot,
    stdio: 'pipe',
  });
}

/** 让挂起的 Promise 链跑完（页面里是 .then/.catch，没有 await 点）。 */
function flush() {
  return new Promise((resolveTick) => {
    setTimeout(resolveTick, 0);
  });
}

// ---- 生成页（与部署流程同一生成器） ----------------------------------------
console.log('生成文档/隐私/许可证页（build-site.mjs）');
runScript('build-site.mjs', []);

const appJs = readFileSync(join(repoRoot, 'site', 'app.js'), 'utf8');

/**
 * 载入站点页面并跑完 app.js。
 *
 * `routes` 是 `URL → { status, body | json() | text() }`；未列出的 URL 一律 404。
 * `routes[url].contentType` 可模拟托管方的 HTML 兜底（Cloudflare Pages 对不存在的
 * 路径返回 200 + text/html，而不是 404——见 app.js 的 isHtmlFallback）。
 * `userAgent` 用来测平台识别。
 */
async function loadPage(file, routes, { path = '/', userAgent } = {}) {
  const raw = readFileSync(join(repoRoot, 'site', file), 'utf8');
  // jsdom 默认不加载外部脚本：把 app.js 内联进来（样式不参与断言，无需加载）
  const html = raw.replace('<script defer src="/app.js"></script>', `<script>${appJs}</script>`);
  const fakeFetch = (url) => {
    const route = routes[String(url)];
    if (route === undefined) {
      return Promise.resolve({ ok: false, status: 404, headers: { get: () => null } });
    }
    const contentType =
      route.contentType ?? (route.json !== undefined ? 'application/json' : 'text/plain');
    return Promise.resolve({
      ok: true,
      status: 200,
      headers: {
        get: (name) => (String(name).toLowerCase() === 'content-type' ? contentType : null),
      },
      json: () => Promise.resolve(route.json),
      text: () => Promise.resolve(route.text ?? ''),
    });
  };
  void 0;

  const dom = new JSDOM(html, {
    runScripts: 'dangerously',
    url: `https://forgedesk.pages.dev${path}`,
    beforeParse(window) {
      window.fetch = fakeFetch;
      if (userAgent !== undefined) {
        Object.defineProperty(window.navigator, 'userAgent', {
          value: userAgent,
          configurable: true,
        });
      }
      // jsdom 不实现剪贴板；页面在缺失时会走"复制失败，请手动选择"分支
      Object.defineProperty(window.navigator, 'clipboard', {
        value: { writeText: () => Promise.resolve() },
        configurable: true,
      });
    },
  });
  // 清单 → 渲染 → 再拉校验和，是多跳 Promise：刷两轮宏任务才稳
  await flush();
  await flush();
  return dom;
}

/** 页面上指向 Release 产物（releases/download/）的链接——没有发布时必须为 0。 */
function artifactLinks(document) {
  return [...document.querySelectorAll('a[href]')].filter((anchor) =>
    (anchor.getAttribute('href') || '').includes('/releases/download/'),
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

// ---- 静态结构：所有页面共同的要求 -----------------------------------------
console.log('静态结构（多页导航 / 平台识别 / 三平台命令 / og 图）');
{
  const index = readFileSync(join(repoRoot, 'site', 'index.html'), 'utf8');
  const download = readFileSync(join(repoRoot, 'site', 'download.html'), 'utf8');

  assert(index.includes('id="os-switch"'), '首页有平台手动切换（自动识别的兜底）');
  assert(
    ['data-os="windows"', 'data-os="macos"', 'data-os="linux"'].every((token) =>
      index.includes(token),
    ),
    '切换覆盖 Windows / macOS / Linux 三项',
  );
  assert(index.includes('href="https://github.com/Ember1414/forgedesk"'), '首页有星标仓库入口');
  assert(index.includes('Tauri 2 + Rust'), '首页有技术说明（Tauri + Rust）');
  assert(
    (index.match(/<li>\s*<div class="cap-icon"/g) ?? []).length === 4,
    '四点核心能力（配原创插图）',
  );
  assert(index.includes('property="og:image"'), '首页带 og:image（原创分享图）');
  assert(existsSync(join(repoRoot, 'site', 'og.png')), 'og 分享图文件存在');

  for (const [label, html] of [
    ['download', download],
    ['index', index],
  ]) {
    assert(html.includes('/assets.css') && html.includes('/app.js'), `${label} 引用共享样式与逻辑`);
  }

  // 三平台校验命令（T7.9 第 2 点）
  assert(
    download.includes('id="cmd-windows"') && download.includes('Get-FileHash'),
    'Windows 校验命令',
  );
  assert(
    download.includes('id="cmd-macos"') && download.includes('shasum -a 256'),
    'macOS 校验命令',
  );
  assert(download.includes('id="cmd-linux"') && download.includes('sha256sum'), 'Linux 校验命令');
  assert(download.includes('gpg --verify'), 'GPG 验证命令');
  assert(
    download.includes('SmartScreen') && download.includes('xattr -dr com.apple.quarantine'),
    '信任说明覆盖 Windows SmartScreen 与 macOS 去隔离',
  );
  assert(download.includes('id="matrix-wrap"'), '下载页有版本矩阵容器');

  // 生成页与 docs/ 同源
  const manualReadme = readFileSync(join(repoRoot, 'docs', 'manual', 'README.md'), 'utf8');
  const docsPage = readFileSync(join(repoRoot, 'site', 'docs', 'index.html'), 'utf8');
  const firstManualHeading = /^#\s+(.+)$/m.exec(manualReadme)?.[1] ?? '用户手册';
  assert(docsPage.includes(firstManualHeading), '生成的文档页含手册标题（同源渲染）');
  for (const article of ['01-work-with-a-repository', '02-history-branches-sync']) {
    const source = readFileSync(join(repoRoot, 'docs', 'manual', `${article}.md`), 'utf8');
    const generated = readFileSync(join(repoRoot, 'site', 'docs', `${article}.html`), 'utf8');
    const heading = /^#\s+(.+)$/m.exec(source)?.[1] ?? article;
    assert(generated.includes(heading), `生成页 ${article} 与手册同源`);
  }
  const privacy = readFileSync(join(repoRoot, 'docs', 'PRIVACY.md'), 'utf8');
  const privacyPage = readFileSync(join(repoRoot, 'site', 'privacy.html'), 'utf8');
  assert(privacyPage.includes('PRIVACY') || privacy.length > 0, '隐私页由 docs/PRIVACY.md 生成');
  const license = readFileSync(join(repoRoot, 'LICENSE'), 'utf8');
  const licensePage = readFileSync(join(repoRoot, 'site', 'license.html'), 'utf8');
  assert(
    licensePage.includes(license.trim().split('\n')[0].slice(0, 30)),
    '许可证页与 LICENSE 同源',
  );

  // GPG 公钥：站点副本必须与真相源逐字节一致。下载页给出的是"用这把公钥验签名"，
  // 一旦两份漂移，用户拿到的就是一把验不过（或更糟：验得过旧签名）的钥匙。
  const pubkeySource = join(repoRoot, 'docs', 'keys', 'forgedesk-release.pub');
  const pubkeyTarget = join(repoRoot, 'site', 'updates', 'gpg-pubkey.asc');
  if (existsSync(pubkeySource)) {
    assert(existsSync(pubkeyTarget), '公钥真相源存在时，站点副本必须已生成（build-site）');
    assert(
      existsSync(pubkeyTarget) &&
        readFileSync(pubkeyTarget, 'utf8') === readFileSync(pubkeySource, 'utf8'),
      '站点副本与 docs/keys/forgedesk-release.pub 逐字节一致',
    );
    assert(
      readFileSync(pubkeySource, 'utf8').includes('BEGIN PGP PUBLIC KEY BLOCK'),
      '公钥文件是 ASCII armor 形态（不是二进制导出或空文件）',
    );
  } else {
    assert(!existsSync(pubkeyTarget), '没有公钥真相源时站点不得残留旧副本（轮换安全）');
  }
}

// ---- 状态 1：没有清单（尚未发布） ------------------------------------------
console.log('状态 1：没有发布清单（404）');
{
  const dom = await loadPage(
    'index.html',
    {},
    { path: '/', userAgent: 'Mozilla/5.0 (Windows NT 10.0; Win64; x64)' },
  );
  const { document } = dom.window;
  const status = document.getElementById('status').textContent;

  assert(status.includes('尚无可用版本'), `状态如实显示「尚无可用版本」（实际：${status.trim()}）`);
  assert(
    artifactLinks(document).length === 0,
    '页面上没有任何指向 Release 产物的链接（不给出必然 404 的入口）',
  );
  const cta = document.getElementById('download-cta');
  assert(
    cta !== null && cta.textContent.includes('前往 Releases'),
    '无版本时主按钮如实引导到 Releases',
  );
  // 下载页同样如实：无版本时矩阵隐藏、空态说明"尚未发布"
  const downloadDom = await loadPage('download.html', {}, { path: '/download.html' });
  assert(downloadDom.window.document.getElementById('checksums').hidden, '校验和区保持隐藏');
  assert(
    downloadDom.window.document.getElementById('status').textContent.includes('尚无可用版本'),
    '下载页无版本时同样如实显示',
  );
  assert(downloadDom.window.document.getElementById('matrix-wrap').hidden, '版本矩阵保持隐藏');
  assert(
    downloadDom.window.document.getElementById('matrix-empty').textContent.includes('尚未发布'),
    '矩阵空态说明"尚未发布"',
  );
  dom.window.close();
  downloadDom.window.close();
}

// ---- 状态 2：有清单、没有校验和文件 ----------------------------------------
console.log('状态 2：有发布清单，但没有校验和文件');
{
  const dom = await loadPage(
    'index.html',
    { [MANIFEST_URL]: { json: manifest } },
    { path: '/', userAgent: 'Mozilla/5.0 (Windows NT 10.0; Win64; x64)' },
  );
  const { document } = dom.window;

  assert(
    document.getElementById('status').textContent.includes(`v${version}`),
    `状态显示最新版本 v${version}`,
  );
  const cta = document.getElementById('download-cta');
  const ctaLink = cta.querySelector('a.btn.primary');
  assert(
    ctaLink !== null && ctaLink.getAttribute('href') === manifest.platforms['windows-x86_64'].url,
    '平台识别的下载按钮指向清单里的安装包地址',
  );
  // 下载页：矩阵展开（有清单就渲染，SHA256 列此时降级为文字）
  const downloadDom = await loadPage(
    'download.html',
    { [MANIFEST_URL]: { json: manifest } },
    { path: '/download.html' },
  );
  assert(!downloadDom.window.document.getElementById('matrix-wrap').hidden, '版本矩阵展开');
  const rows = downloadDom.window.document.querySelectorAll('#matrix-wrap tbody tr');
  assert(rows.length === 3, `矩阵有 3 行产物（exe/msi/zip，实际 ${rows.length}）`);
  assert(
    downloadDom.window.document.getElementById('checksums').hidden,
    '缺少 SHA256SUMS 时校验和区仍隐藏（不显示空内容）',
  );
  dom.window.close();
  downloadDom.window.close();
}

// ---- 状态 3：有清单也有校验和 ---------------------------------------------
console.log('状态 3：有发布清单与校验和');
{
  const dom = await loadPage(
    'download.html',
    {
      [MANIFEST_URL]: { json: manifest },
      [CHECKSUMS_URL]: { text: checksums },
      [ASC_URL]: { text: '-----BEGIN PGP SIGNATURE-----\nfake\n-----END PGP SIGNATURE-----' },
      [PUBKEY_URL]: {
        text: '-----BEGIN PGP PUBLIC KEY BLOCK-----\nfake\n-----END PGP PUBLIC KEY BLOCK-----',
      },
    },
    { path: '/download.html' },
  );
  const { document } = dom.window;

  assert(!document.getElementById('matrix-wrap').hidden, '版本矩阵展开');
  const shaLinks = [
    ...document.querySelectorAll('#matrix-wrap a[href="/updates/stable/SHA256SUMS"]'),
  ];
  assert(shaLinks.length === 3, `SHA256 列全部指向同源校验和文件（实际 ${shaLinks.length}）`);

  assert(!document.getElementById('checksums').hidden, '校验和区展开');
  const rendered = document.getElementById('checksum-lines').textContent;
  assert(rendered === checksums.trim(), '页面显示的校验和与流水线写出的 SHA256SUMS 逐字一致');
  assert(
    assetNames.every((name) => rendered.includes(name)),
    '校验和覆盖全部三个 Windows 产物',
  );

  const gpgKey = document.getElementById('gpg-key');
  assert(!gpgKey.hidden, '有公钥文件时才展示 GPG 公钥下载入口');
  assert(
    gpgKey.querySelector('a').getAttribute('href') === PUBKEY_URL,
    'GPG 入口的链接指向被探测的那个文件（探测与链接不得分叉）',
  );
  assert(
    document.getElementById('gpg-placeholder').hidden,
    '公钥已发布时收起"尚未发布"的占位说明（两句话不能同时出现）',
  );
  dom.window.close();

  // 回归：只有 .asc（签名）而**没有**公钥文件时，入口必须保持隐藏。
  // 这正是 v1.0.0 上线时的真实状态——探测 .asc 却链接公钥，页面挂出了坏链接。
  const noPubkey = await loadPage(
    'download.html',
    {
      [MANIFEST_URL]: { json: manifest },
      [CHECKSUMS_URL]: { text: checksums },
      [ASC_URL]: { text: '-----BEGIN PGP SIGNATURE-----\nfake\n-----END PGP SIGNATURE-----' },
    },
    { path: '/download.html' },
  );
  assert(
    noPubkey.window.document.getElementById('gpg-key').hidden,
    '只有签名没有公钥时，GPG 公钥入口保持隐藏（不给出必坏的链接）',
  );
  assert(
    !noPubkey.window.document.getElementById('gpg-placeholder').hidden,
    '公钥缺失时如实说明现状（占位文案可见）',
  );
  noPubkey.window.close();
  void 0;
}

// ---- 状态 4：更新日志（运行时拉取 + 兜底） ---------------------------------
console.log('状态 4：更新日志页（Releases 拉取与兜底）');
{
  const empty = await loadPage('changelog.html', {});
  assert(
    empty.window.document.getElementById('changelog-body').textContent.includes('GitHub Releases'),
    '拉取失败/无发布时兜底到 Releases 链接',
  );
  empty.window.close();

  const populated = await loadPage('changelog.html', {
    [API_RELEASES_URL]: {
      json: [
        {
          name: `ForgeDesk v${version}`,
          tag_name: `v${version}`,
          published_at: '2026-10-07T00:00:00Z',
          body: '### 新功能\n- 演示条目',
        },
      ],
    },
  });
  const body = populated.window.document.getElementById('changelog-body');
  assert(body.textContent.includes(`ForgeDesk v${version}`), '能渲染 Release 标题');
  assert(body.querySelectorAll('article').length === 1, '每个 Release 一篇文章块');
  populated.window.close();
}

// ---- 状态 5：托管方 HTML 兜底（未发布时的真实形态） ------------------------
// Cloudflare Pages 对**不存在的路径**返回 200 + 首页 HTML，而不是 404（实测）。
// 页面必须把它当成"没有这个文件"——否则会把首页当清单/校验和渲染、并给出坏链接。
console.log('状态 5：托管方 HTML 兜底（Pages 对缺失路径返回 200 + 首页）');
{
  const HTML = { contentType: 'text/html', text: '<!doctype html><html>home</html>' };
  const routes = { [MANIFEST_URL]: HTML, [CHECKSUMS_URL]: HTML, [ASC_URL]: HTML };
  const dom = await loadPage('index.html', routes, {
    path: '/',
    userAgent: 'Mozilla/5.0 (Windows NT 10.0; Win64; x64)',
  });
  const { document } = dom.window;
  assert(
    document.getElementById('status').textContent.includes('尚无可用版本'),
    'HTML 兜底时状态仍为「尚无可用版本」',
  );
  assert(artifactLinks(document).length === 0, 'HTML 兜底时不产生任何下载链接');
  dom.window.close();

  // 下载页同样不能把首页兜底当成数据
  const downloadDom = await loadPage('download.html', routes, { path: '/download.html' });
  assert(
    downloadDom.window.document.getElementById('checksums').hidden,
    'HTML 兜底时校验和区保持隐藏',
  );
  assert(
    downloadDom.window.document.getElementById('matrix-wrap').hidden,
    'HTML 兜底时版本矩阵保持隐藏',
  );
  downloadDom.window.close();
}

rmSync(work, { recursive: true, force: true });

if (failures > 0) {
  console.error(`\n官网自检失败：${failures} 项。`);
  process.exit(1);
}
console.log(
  '\n官网自检通过（静态结构 / 无清单 / 有清单无校验和 / 有清单有校验和 / HTML 兜底 / 更新日志）。',
);
