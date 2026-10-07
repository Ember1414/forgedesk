#!/usr/bin/env node
/**
 * 版本号同步 / 校验（M7 / T7.1）。
 *
 * # 为什么需要它
 *
 * 版本号写在三个地方：`src-tauri/tauri.conf.json`、`package.json`、
 * `Cargo.toml`（`[workspace.package]`），而 `Cargo.lock` 会跟着 cargo 走。
 * 手工改三处的结局必然是某次发版漏掉一处——产物名、关于页与 crate 版本互相不一致，
 * 而且要到打包之后才会发现。把"真相源 + 同步"变成一个命令，就没有漏掉的余地。
 *
 * # 真相源
 *
 * `tauri.conf.json` 的 `version` 是唯一真相源（打包器读的就是它）。
 * `--check` 校验三处一致；给定版本号则写三处并刷新 `Cargo.lock`。
 *
 * # 为什么用文本替换而不是 JSON 解析后重写
 *
 * 重写会把注释、键顺序与数组换行风格一起改掉，随后 prettier / rustfmt 会再报一次
 * "格式不一致"。这里只改**那一行版本值**，其余字节保持原样（Cargo.toml 尤其重要：
 * 它有多处 `version = "..."` 出现在内联表里，必须只在 `[workspace.package]` 段落内替换）。
 *
 * 用法：
 *   node scripts/setup/sync-version.mjs --check       # 校验三处一致
 *   node scripts/setup/sync-version.mjs 1.0.0         # 写入三处并刷新 Cargo.lock
 */

import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');
const TAURI_CONFIG = join(repoRoot, 'src-tauri', 'tauri.conf.json');
const PACKAGE_JSON = join(repoRoot, 'package.json');
const CARGO_TOML = join(repoRoot, 'Cargo.toml');
const SEMVER = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/;

function fail(message) {
  console.error(message);
  process.exit(2);
}

/** 读取 tauri.conf.json 的版本（真相源）。 */
function tauriVersion() {
  const match = readFileSync(TAURI_CONFIG, 'utf8').match(/"version"\s*:\s*"([^"]*)"/);
  if (match === null) {
    fail('tauri.conf.json 里找不到 "version" 字段。');
  }
  return match[1];
}

/** 读取 package.json 的版本。 */
function packageVersion() {
  const match = readFileSync(PACKAGE_JSON, 'utf8').match(/"version"\s*:\s*"([^"]*)"/);
  if (match === null) {
    fail('package.json 里找不到 "version" 字段。');
  }
  return match[1];
}

/**
 * 读取 Cargo.toml `[workspace.package]` 段落的版本。
 *
 * 只在段落内匹配：`tauri = { version = "2", ... }` 这类内联表也含 `version = "..."`，
 * 全局正则会改坏依赖声明。
 */
function cargoVersion() {
  const lines = readFileSync(CARGO_TOML, 'utf8').split(/\r?\n/);
  const start = lines.findIndex((line) => line.trim() === '[workspace.package]');
  if (start === -1) {
    fail('Cargo.toml 里找不到 [workspace.package] 段落。');
  }
  for (let index = start + 1; index < lines.length; index += 1) {
    const line = lines[index];
    if (line.trimStart().startsWith('[')) {
      break; // 进入下一个段落
    }
    const match = line.match(/^version\s*=\s*"([^"]*)"/);
    if (match !== null) {
      return match[1];
    }
  }
  fail('Cargo.toml 的 [workspace.package] 段落里找不到 version。');
}

/** 只替换 tauri.conf.json 中第一处 `"version": "..."`。 */
function writeTauriVersion(version) {
  const text = readFileSync(TAURI_CONFIG, 'utf8');
  writeFileSync(TAURI_CONFIG, text.replace(/("version"\s*:\s*)"[^"]*"/, `$1"${version}"`));
}

/** 只替换 package.json 中第一处 `"version": "..."`。 */
function writePackageVersion(version) {
  const text = readFileSync(PACKAGE_JSON, 'utf8');
  writeFileSync(PACKAGE_JSON, text.replace(/("version"\s*:\s*)"[^"]*"/, `$1"${version}"`));
}

/** 只替换 Cargo.toml `[workspace.package]` 段落内的 version 行。 */
function writeCargoVersion(version) {
  const lines = readFileSync(CARGO_TOML, 'utf8').split(/\r?\n/);
  const start = lines.findIndex((line) => line.trim() === '[workspace.package]');
  if (start === -1) {
    fail('Cargo.toml 里找不到 [workspace.package] 段落。');
  }
  for (let index = start + 1; index < lines.length; index += 1) {
    if (lines[index].trimStart().startsWith('[')) {
      break;
    }
    if (/^version\s*=\s*"[^"]*"/.test(lines[index])) {
      lines[index] = `version = "${version}"`;
      writeFileSync(CARGO_TOML, lines.join('\n'));
      return;
    }
  }
  fail('Cargo.toml 的 [workspace.package] 段落里找不到 version。');
}

/** 刷新 Cargo.lock（改 workspace 版本后 lock 会过期，`--locked` 会失败）。 */
function refreshCargoLock() {
  try {
    execFileSync('cargo', ['metadata', '--format-version', '1', '--offline'], {
      cwd: repoRoot,
      stdio: 'ignore',
      maxBuffer: 64 * 1024 * 1024,
    });
    return true;
  } catch {
    return false;
  }
}

const argv = process.argv.slice(2);
if (argv.length === 0) {
  fail('用法：sync-version.mjs --check | <版本号>（例如 1.0.0）');
}

if (argv[0] === '--check') {
  const versions = {
    'tauri.conf.json': tauriVersion(),
    'package.json': packageVersion(),
    'Cargo.toml': cargoVersion(),
  };
  const distinct = new Set(Object.values(versions));
  for (const [file, version] of Object.entries(versions)) {
    console.log(`  ${file.padEnd(18)} ${version}`);
  }
  if (distinct.size !== 1) {
    console.error(
      `版本号不一致：${[...distinct].join(' vs ')}。运行 pnpm version:sync <版本> 修复。`,
    );
    process.exit(1);
  }
  console.log(`版本号一致：${[...distinct][0]}`);
  process.exit(0);
}

const version = argv[0];
if (!SEMVER.test(version)) {
  fail(`版本号不符合 SemVer：${version}`);
}

writeTauriVersion(version);
writePackageVersion(version);
writeCargoVersion(version);

const lockRefreshed = refreshCargoLock();
console.log(`版本号已同步为 ${version}：tauri.conf.json / package.json / Cargo.toml`);
console.log(
  lockRefreshed
    ? 'Cargo.lock 已刷新。'
    : '警告：Cargo.lock 未能自动刷新（cargo 不可用？）——请手动运行 cargo metadata 后再提交。',
);
console.log('别忘了更新 CHANGELOG.md 并打 tag（见 docs/RELEASE.md）。');
