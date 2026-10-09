#!/usr/bin/env node
/**
 * 把包管理器清单对齐到一次真实发布（M8 / T8.3–T8.5）。
 *
 * # 为什么以 SHA256SUMS 为唯一输入
 *
 * 发布时 `releases/SHA256SUMS` 是"这次到底发了什么"的权威记录（由
 * `make-checksums.mjs` 从最终产物生成、经 GPG 签名）。如果清单里的版本/hash
 * 从别处来（手工填、从 tag 推），就会出现"清单指向的资产与发布不一致"——
 * 那是供应链事故的经典形态。这里只做一件事：让清单与那份文件逐字一致。
 *
 * # 用法
 *
 *   node scripts/ci/update-packaging.mjs --sums releases/SHA256SUMS          # 写入
 *   node scripts/ci/update-packaging.mjs --check --sums releases/SHA256SUMS  # 只校验
 *
 * --check 在清单与发布不一致时以非零码退出，供本地与 CI 使用。
 * 某个平台的产物不在校验和文件里时，对应渠道整体跳过（不半更新）。
 *
 * # 范围
 *
 * 只更新"自有渠道"的清单：Scoop、Winget、Homebrew Cask。
 * Flathub/Snap/AUR 未启用（Linux 打包链按 ADR-005 暂缓），因此不在范围。
 */
import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');

function parseArgs(argv) {
  const options = { sums: join(repoRoot, 'releases', 'SHA256SUMS'), check: false };
  for (let index = 0; index < argv.length; index += 1) {
    const key = argv[index];
    switch (key) {
      case '--sums':
        options.sums = resolve(argv[index + 1] ?? '');
        index += 1;
        break;
      case '--check':
        options.check = true;
        break;
      default:
        console.error(`未知参数：${key}`);
        process.exit(2);
    }
  }
  return options;
}

/** 解析 `<hash>  <文件名>` 行（make-checksums.mjs 的稳定格式）。 */
function parseSums(path) {
  const map = new Map();
  for (const rawLine of readFileSync(path, 'utf8').split(/\r?\n/)) {
    const line = rawLine.trim();
    if (line === '') {
      continue;
    }
    const match = /^([0-9a-fA-F]{64})\s{2}(.+)$/.exec(line);
    if (match === null) {
      console.error(`无法解析校验和行：${line}`);
      process.exit(1);
    }
    map.set(match[2], match[1].toLowerCase());
  }
  return map;
}

/** 从产物名里提取版本号（发布产物命名：ForgeDesk_<版本>_<平台>_<架构>...）。 */
function detectVersion(names) {
  const versions = new Set();
  for (const name of names) {
    const match = /^ForgeDesk_(\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.]+)?)_/.exec(name);
    if (match !== null) {
      versions.add(match[1]);
    }
  }
  if (versions.size === 0) {
    console.error('SHA256SUMS 里没有可识别的 ForgeDesk 产物名。');
    process.exit(1);
  }
  if (versions.size > 1) {
    console.error(`SHA256SUMS 里出现了多个版本号：${[...versions].join('、')}`);
    process.exit(1);
  }
  return [...versions][0];
}

/**
 * 每个渠道的替换规则。
 *
 * `requires`：该渠道依赖的产物；缺失时整体跳过（避免"版本更新了但 hash 还是旧的"）。
 * `label: null` 的规则不参与 --check（值无法从校验和推导，如 ReleaseDate）。
 */
function buildTargets(version, sums, today) {
  const portableName = `ForgeDesk_${version}_windows_x64_portable.zip`;
  const exeName = `ForgeDesk_${version}_windows_x64.exe`;
  const dmgName = `ForgeDesk_${version}_macos_universal.dmg`;
  const releaseBase = `https://github.com/Ember1414/forgedesk/releases/download/v${version}`;
  const tagUrl = `https://github.com/Ember1414/forgedesk/releases/tag/v${version}`;

  return [
    {
      path: join(repoRoot, 'packaging', 'scoop', 'forgedesk.json'),
      requires: [portableName],
      rules: [
        { label: 'version', pattern: /("version":\s*")[^"]+(")/, replacement: `$1${version}$2` },
        {
          label: 'portable url',
          pattern: /releases\/download\/v[^/]+\/ForgeDesk_[^"]+_windows_x64_portable\.zip/,
          replacement: `releases/download/v${version}/${portableName}`,
        },
        {
          label: 'portable hash',
          pattern: /("hash":\s*")[0-9a-f]{64}(")/,
          replacement: `$1${sums.get(portableName)}$2`,
        },
      ],
    },
    {
      path: join(repoRoot, 'packaging', 'winget', 'ForgeDesk.ForgeDesk.yaml'),
      requires: [exeName],
      rules: [
        {
          label: 'version',
          pattern: /^PackageVersion: .+$/m,
          replacement: `PackageVersion: ${version}`,
        },
      ],
    },
    {
      path: join(repoRoot, 'packaging', 'winget', 'ForgeDesk.ForgeDesk.installer.yaml'),
      requires: [exeName],
      rules: [
        {
          label: 'version',
          pattern: /^PackageVersion: .+$/m,
          replacement: `PackageVersion: ${version}`,
        },
        {
          label: 'installer url',
          pattern: /^(\s*)InstallerUrl: .+$/m,
          replacement: `$1InstallerUrl: ${releaseBase}/${exeName}`,
        },
        {
          label: 'installer sha256',
          pattern: /^(\s*)InstallerSha256: [0-9a-fA-F]{64}$/m,
          replacement: `$1InstallerSha256: ${sums.get(exeName).toUpperCase()}`,
        },
        {
          label: null,
          pattern: /^ReleaseDate: \d{4}-\d{2}-\d{2}$/m,
          replacement: `ReleaseDate: ${today}`,
        },
      ],
    },
    {
      path: join(repoRoot, 'packaging', 'winget', 'ForgeDesk.ForgeDesk.locale.en-US.yaml'),
      requires: [exeName],
      rules: [
        {
          label: 'version',
          pattern: /^PackageVersion: .+$/m,
          replacement: `PackageVersion: ${version}`,
        },
        {
          label: 'release notes url',
          pattern: /^ReleaseNotesUrl: .+$/m,
          replacement: `ReleaseNotesUrl: ${tagUrl}`,
        },
      ],
    },
    {
      path: join(repoRoot, 'packaging', 'winget', 'ForgeDesk.ForgeDesk.locale.zh-CN.yaml'),
      requires: [exeName],
      rules: [
        {
          label: 'version',
          pattern: /^PackageVersion: .+$/m,
          replacement: `PackageVersion: ${version}`,
        },
        {
          label: 'release notes url',
          pattern: /^ReleaseNotesUrl: .+$/m,
          replacement: `ReleaseNotesUrl: ${tagUrl}`,
        },
      ],
    },
    {
      path: join(repoRoot, 'packaging', 'homebrew', 'forgedesk.rb'),
      requires: [dmgName],
      rules: [
        { label: 'version', pattern: /^(\s*version ")[^"]+(")$/m, replacement: `$1${version}$2` },
        {
          label: 'dmg sha256',
          pattern: /^(\s*sha256 ")[0-9a-f]{64}(")$/m,
          replacement: `$1${sums.get(dmgName)}$2`,
        },
      ],
    },
  ];
}

// ------------------------------------------------------------------ 主流程

const options = parseArgs(process.argv.slice(2));

if (!existsSync(options.sums)) {
  console.error(`找不到校验和文件：${options.sums}`);
  console.error('先用发布产物生成（make-checksums.mjs），或从 Release 下载 SHA256SUMS。');
  process.exit(1);
}

const sums = parseSums(options.sums);
const version = detectVersion([...sums.keys()]);
const today = new Date().toISOString().slice(0, 10);

let drifted = false;
for (const target of buildTargets(version, sums, today)) {
  const relative = target.path.slice(repoRoot.length + 1);
  const missing = target.requires.filter((name) => !sums.has(name));
  if (missing.length > 0) {
    console.warn(`⚠ ${relative}：缺少产物（${missing.join('、')}），跳过该渠道`);
    continue;
  }

  const original = readFileSync(target.path, 'utf8');
  let updated = original;
  const changed = [];
  const checkDiffs = [];

  for (const rule of target.rules) {
    if (!rule.pattern.test(updated)) {
      if (rule.label !== null) {
        checkDiffs.push(`${rule.label}（未找到可替换内容）`);
      }
      continue;
    }
    const next = updated.replace(rule.pattern, rule.replacement);
    if (next === updated) {
      continue;
    }
    changed.push(rule.label ?? 'release date');
    if (rule.label !== null) {
      checkDiffs.push(rule.label);
    }
    updated = next;
  }

  if (options.check) {
    if (checkDiffs.length > 0) {
      drifted = true;
      console.error(`✗ ${relative} 与发布不一致：${checkDiffs.join('；')}`);
    } else {
      console.log(`✓ ${relative} 已对齐 v${version}`);
    }
    continue;
  }

  if (changed.length === 0) {
    console.log(`✓ ${relative} 已对齐 v${version}`);
    continue;
  }
  writeFileSync(target.path, updated);
  console.log(`↑ ${relative} 已更新（${changed.join('、')}） → v${version}`);
}

if (options.check && drifted) {
  console.error('\n清单与发布不一致。运行（不带 --check）以就地更新：');
  console.error(`  node scripts/ci/update-packaging.mjs --sums ${options.sums}`);
  process.exit(1);
}

console.log(options.check ? '\n全部清单与发布一致。' : '\n清单更新完成。');
