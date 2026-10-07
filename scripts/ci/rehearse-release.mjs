#!/usr/bin/env node
/**
 * 发布脚本链的**本地演练**（M7 / T7.3）。
 *
 * # 为什么需要它
 *
 * `release.yml` 是"从没跑过的流水线"：它要等密钥、等 Pages 项目、等一个真 tag 才有机会
 * 第一次执行。而它调用的那串脚本（归一化命名 → 便携版 zip → 合并校验和 → updater 清单）
 * 是**纯本地逻辑**，完全可以用假产物在几十毫秒内跑一遍。把"参数写错 / 顺序写错 /
 * 路径写错"这类问题提前在这里暴露，比在真正发版时才发现要便宜得多。
 *
 * 覆盖：脚本之间的**接口与顺序**（含 updater 清单的形状与 target 白名单）。
 * 不覆盖：`tauri build` 本身、真实签名（Ed25519 / GPG）、GitHub Release、Pages 部署——
 * 这些都要真凭据与真环境，见 `docs/RELEASE.md` §4.1 的"首次发布时必须核对的两件事"。
 *
 * 用法：
 *   node scripts/ci/rehearse-release.mjs [--keep]
 *
 * 演练在 `target/release-rehearsal/` 下进行（已被 .gitignore 覆盖）；
 * 默认结束后清理，`--keep` 保留现场以便人工翻看产物。
 */
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');
const keep = process.argv.includes('--keep');

const work = join(repoRoot, 'target', 'release-rehearsal');
const bundleDir = join(work, 'bundle');
const releasesDir = join(work, 'releases');
const updatesDir = join(work, 'updates');

const version = JSON.parse(
  readFileSync(join(repoRoot, 'src-tauri', 'tauri.conf.json'), 'utf8'),
).version;

/** 跑一段 Node 脚本，失败即整体失败（演练的意义就在于"任一步失败都要看得见"）。 */
function run(label, scriptPath, args) {
  process.stdout.write(`▶ ${label}\n`);
  execFileSync(process.execPath, [join(repoRoot, 'scripts', 'ci', scriptPath), ...args], {
    cwd: repoRoot,
    stdio: 'inherit',
  });
}

function fail(message) {
  console.error(`\n✗ 演练失败：${message}`);
  process.exit(1);
}

function expectFile(path, label) {
  if (!existsSync(path)) {
    fail(`缺少${label}：${path}`);
  }
}

rmSync(work, { recursive: true, force: true });

// ---- 1. 造一棵与 `tauri build` 输出形状一致的假 bundle 树 -------------------
// 文件名刻意沿用 Tauri 的真实命名（带语言后缀的 msi、带 -setup 的 nsis），
// 因为 `rename-bundles.mjs` 的解析逻辑正是针对这些形状写的。
mkdirSync(join(bundleDir, 'msi'), { recursive: true });
mkdirSync(join(bundleDir, 'nsis'), { recursive: true });
mkdirSync(join(work, 'release'), { recursive: true });

writeFileSync(join(bundleDir, 'msi', `ForgeDesk_${version}_x64_en-US.msi`), 'rehearsal-msi\n');
writeFileSync(
  join(bundleDir, 'nsis', `ForgeDesk_${version}_x64-setup.exe`),
  'rehearsal-nsis-installer\n',
);
writeFileSync(
  join(bundleDir, 'nsis', `ForgeDesk_${version}_x64-setup.exe.sig`),
  'rehearsal-signature-base64\n',
);
writeFileSync(join(work, 'release', 'forgedesk.exe'), 'rehearsal-exe\n');

// ---- 2. 按 release.yml 的顺序跑同一条脚本链 ---------------------------------
run('归一化产物名（rename-bundles）', 'rename-bundles.mjs', [
  '--out',
  releasesDir,
  '--sha256',
  '--arch',
  'x64',
  '--bundle-dir',
  bundleDir,
]);

run('生成便携版 zip（make-portable）', 'make-portable.mjs', [
  '--out',
  releasesDir,
  '--exe',
  join(work, 'release', 'forgedesk.exe'),
  '--arch',
  'x64',
]);

run('合并校验和（make-checksums）', 'make-checksums.mjs', ['--dir', releasesDir]);

writeFileSync(join(work, 'notes.md'), '## 演练\n\n- 这条说明是演练生成的\n');
run('生成 updater 清单（make-updater-manifest）', 'make-updater-manifest.mjs', [
  '--version',
  version,
  '--target',
  'windows-x86_64',
  '--signature',
  join(bundleDir, 'nsis', `ForgeDesk_${version}_x64-setup.exe.sig`),
  '--url',
  `https://github.com/example/forgedesk/releases/download/v${version}/ForgeDesk_${version}_windows_x64.exe`,
  '--notes',
  join(work, 'notes.md'),
  '--pub-date',
  '2026-01-01T00:00:00Z',
  '--out',
  join(updatesDir, 'stable', 'windows-x86_64.json'),
]);

run('生成 Release Notes（release-notes）', 'release-notes.mjs', [
  '--version',
  version,
  '--out',
  join(work, 'notes-from-git.md'),
]);

// ---- 3. 断言：产物齐、清单形状对 -------------------------------------------
// 归一化后的名字：Tauri 的 `…-setup.exe` 会被规整成 `…_windows_x64.exe`
// （release.yml 里那个 glob 必须按这个形状写——这里正是钉住它的地方）
expectFile(join(releasesDir, `ForgeDesk_${version}_windows_x64.msi`), '归一化后的 msi');
expectFile(join(releasesDir, `ForgeDesk_${version}_windows_x64.exe`), 'NSIS 安装器');
expectFile(join(releasesDir, `ForgeDesk_${version}_windows_x64_portable.zip`), '便携版 zip');
expectFile(join(releasesDir, 'SHA256SUMS'), '合并后的 SHA256SUMS');

const sums = readFileSync(join(releasesDir, 'SHA256SUMS'), 'utf8').trim().split('\n');
if (sums.length !== 3) {
  fail(`SHA256SUMS 应有 3 行（msi / nsis / 便携版 zip），实际 ${sums.length} 行`);
}
for (const line of sums) {
  if (!/^[0-9a-f]{64} {2}\S+$/.test(line)) {
    fail(`SHA256SUMS 行格式不符合 sha256sum 约定：${line}`);
  }
}

const manifestPath = join(updatesDir, 'stable', 'windows-x86_64.json');
const manifest = JSON.parse(readFileSync(manifestPath, 'utf8'));
if (manifest.version !== version) {
  fail(`清单版本号不符：${manifest.version} ≠ ${version}`);
}
const entry = manifest.platforms?.['windows-x86_64'];
if (entry === undefined) {
  fail('清单里缺少 windows-x86_64 这一段');
}
if (typeof entry.signature !== 'string' || entry.signature === '') {
  fail('清单里的 signature 为空（客户端会拒绝安装）');
}
if (!entry.url.endsWith(`ForgeDesk_${version}_windows_x64.exe`)) {
  fail(`清单里的 url 没有指向 NSIS 安装器：${entry.url}`);
}

if (!keep) {
  rmSync(work, { recursive: true, force: true });
}

console.log('\n✓ 演练通过：全部脚本按 release.yml 的顺序串起来可用。');
console.log('  已断言：产物命名、SHA256SUMS 的 3 行与格式、清单的版本/签名/URL 形状。');
console.log('  **未覆盖**（需要真凭据与真环境）：tauri build、Ed25519 与 GPG 签名、');
console.log('  GitHub Release 创建、Cloudflare Pages 部署。核对清单见 docs/RELEASE.md §4.1。');
if (keep) {
  console.log(`  现场保留在：${work}`);
}
