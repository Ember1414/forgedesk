#!/usr/bin/env node
/**
 * 归一化 Tauri 打包产物名并生成 SHA256SUMS（M0 / T0.11）。
 *
 * # 为什么要改名
 *
 * Tauri 各平台打包器的默认命名互不相同，而且有的带语言后缀、有的没有版本号：
 *
 *   ForgeDesk_0.0.1_x64_en-US.msi        （WiX，带语言）
 *   ForgeDesk_0.0.1_x64-setup.exe        （NSIS）
 *   forgedesk_0.0.1_amd64.deb            （全小写）
 *   forgedesk-0.0.1-1.x86_64.rpm         （rpm 风格）
 *   ForgeDesk.app.tar.gz                 （.app 归档：既没版本也没架构）
 *
 * Release 附件名要被包管理器清单（Homebrew/Scoop/Winget/AUR…）引用，
 * 名字不稳定 = 每次发版都要改一遍清单。因此统一成：
 *
 *   ForgeDesk_<version>_<target>_<arch>.<ext>
 *
 *   target ∈ windows | macos | linux   （按包类型判定，与文件名拼写无关）
 *   arch   ∈ x64 | arm64 | x86         （从文件名推断，推断不出用 --arch 兜底）
 *
 * # SHA256SUMS
 *
 * 三平台各自生成自己产物的校验和文件（`<hash>  <文件名>`，与 sha256sum -c 兼容）；
 * 发布时（M7/T7.3）再把三份合并为一份总的 SHA256SUMS 并附 GPG 签名。
 *
 * 用法：
 *   node scripts/ci/rename-bundles.mjs --out <目录> [--profile release] [--sha256] [--arch x64|arm64|x86]
 *                                      [--bundle-dir <目录>]
 *
 * --bundle-dir 覆盖自动探测的 bundle 目录：本地验证本脚本时不想真的跑一次
 * pnpm tauri build（要几分钟），就造一棵假的 bundle 树指给它。
 *
 * 找不到任何产物时**报错退出**：CI 里的"上传产物"步骤绝不能对着空目录成功。
 */
import { createHash } from 'node:crypto';
import {
  cpSync,
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  statSync,
  writeFileSync,
} from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');

/** 解析命令行参数（手写就够：选项只有五个，不值得为此加依赖）。 */
function parseArgs(argv) {
  const options = {
    out: undefined,
    profile: 'release',
    sha256: false,
    arch: undefined,
    bundleDir: undefined,
  };
  for (let index = 0; index < argv.length; index += 1) {
    const key = argv[index];
    const next = argv[index + 1];
    switch (key) {
      case '--out':
        options.out = next;
        index += 1;
        break;
      case '--profile':
        options.profile = next;
        index += 1;
        break;
      case '--sha256':
        options.sha256 = true;
        break;
      case '--arch':
        options.arch = next;
        index += 1;
        break;
      case '--bundle-dir':
        options.bundleDir = next;
        index += 1;
        break;
      default:
        console.error(`未知参数：${key}`);
        process.exit(2);
    }
  }
  if (options.out === undefined) {
    console.error('缺少 --out <目录>（归一化产物的输出目录）。');
    process.exit(2);
  }
  if (options.arch !== undefined && !['x64', 'arm64', 'x86'].includes(options.arch)) {
    console.error(`--arch 只支持 x64 | arm64 | x86，收到：${options.arch}`);
    process.exit(2);
  }
  return options;
}

/** 从 tauri.conf.json 读版本号（单一真相源：不在这里再写一份）。 */
function readVersion() {
  const config = JSON.parse(readFileSync(join(repoRoot, 'src-tauri', 'tauri.conf.json'), 'utf8'));
  const version = config.version;
  if (typeof version !== 'string' || version === '') {
    console.error('tauri.conf.json 缺少 version 字段。');
    process.exit(1);
  }
  return version;
}

/**
 * 定位 bundle 输出根目录。
 *
 * ForgeDesk 是 cargo workspace 成员，cargo 把 target 放在 workspace 根；
 * 但不要写死：用 `cargo metadata` 拿真实路径，拿不到（或没有 cargo）再退回两个候选位置。
 */
function findBundleRoot(profile) {
  const candidates = [];
  try {
    const metadata = JSON.parse(
      execFileSync('cargo', ['metadata', '--no-deps', '--format-version', '1'], {
        cwd: repoRoot,
        encoding: 'utf8',
        maxBuffer: 16 * 1024 * 1024,
      }),
    );
    candidates.push(join(metadata.target_directory, profile, 'bundle'));
  } catch {
    // 没有 cargo（例如只装了 Node 的环境）：退回约定位置
  }
  candidates.push(join(repoRoot, 'target', profile, 'bundle'));
  candidates.push(join(repoRoot, 'src-tauri', 'target', profile, 'bundle'));

  const found = candidates.find((path) => existsSync(path));
  if (found === undefined) {
    console.error(`未找到 bundle 目录（找过：${candidates.join('、')}）。`);
    console.error('请先运行 pnpm tauri build。');
    process.exit(1);
  }
  return found;
}

/** 递归收集目录下所有文件。 */
function walk(directory) {
  const files = [];
  for (const entry of readdirSync(directory)) {
    const path = join(directory, entry);
    if (statSync(path).isDirectory()) {
      files.push(...walk(path));
    } else {
      files.push(path);
    }
  }
  return files;
}

/** 按扩展名判定平台；不属于任何打包产物的文件返回 null（直接跳过）。 */
function platformOf(fileName) {
  if (fileName.endsWith('.msi') || fileName.endsWith('.exe')) return 'windows';
  if (fileName.endsWith('.AppImage') || fileName.endsWith('.deb') || fileName.endsWith('.rpm')) {
    return 'linux';
  }
  if (fileName.endsWith('.dmg') || fileName.endsWith('.app.tar.gz')) return 'macos';
  return null;
}

/** 从文件名推断架构（推断不出返回 null，由调用方决定是否用 --arch 兜底）。 */
function archOf(fileName) {
  const lowered = fileName.toLowerCase();
  if (/(^|[^a-z0-9])(x64|x86_64|amd64)([^a-z0-9]|$)/.test(lowered)) return 'x64';
  if (/(^|[^a-z0-9])(aarch64|arm64)([^a-z0-9]|$)/.test(lowered)) return 'arm64';
  if (/(^|[^a-z0-9])(i686|i386)([^a-z0-9]|$)/.test(lowered)) return 'x86';
  return null;
}

/** 保留打包产物特有的扩展名（.app.tar.gz 是三段，不能只取最后一段）。 */
function extensionOf(fileName) {
  if (fileName.endsWith('.app.tar.gz')) return '.app.tar.gz';
  const dot = fileName.lastIndexOf('.');
  return dot === -1 ? '' : fileName.slice(dot);
}

function sha256Of(path) {
  return createHash('sha256').update(readFileSync(path)).digest('hex');
}

const options = parseArgs(process.argv.slice(2));
const version = readVersion();
const bundleRoot =
  options.bundleDir !== undefined ? resolve(options.bundleDir) : findBundleRoot(options.profile);
const outDir = resolve(options.out);

/** 收集产物：{ sourcePath, normalized }。 */
const artifacts = [];
for (const sourcePath of walk(bundleRoot)) {
  const fileName = sourcePath.split(/[\\/]/).pop();
  const platform = platformOf(fileName);
  if (platform === null) {
    continue; // bundle 目录里还有 metadata.plist 之类的非产物文件
  }

  const arch = archOf(fileName) ?? options.arch;
  if (arch === undefined || arch === null) {
    console.error(
      `无法从文件名推断架构，且未提供 --arch 兜底：${fileName}\n` +
        '（例如 macOS 的 .app.tar.gz 不带架构信息，CI 里请传 --arch）',
    );
    process.exit(1);
  }

  const extension = extensionOf(fileName);
  const normalized = `ForgeDesk_${version}_${platform}_${arch}${extension}`;
  artifacts.push({ sourcePath, normalized });
}

if (artifacts.length === 0) {
  console.error(`bundle 目录里没有找到任何打包产物：${bundleRoot}`);
  process.exit(1);
}

mkdirSync(outDir, { recursive: true });

const checksums = [];
for (const { sourcePath, normalized } of artifacts) {
  const destination = join(outDir, normalized);
  cpSync(sourcePath, destination);
  console.log(`${normalized}  <-  ${relativeToRepo(sourcePath)}`);
  if (options.sha256) {
    checksums.push(`${sha256Of(destination)}  ${normalized}`);
  }
}

if (options.sha256) {
  // 固定结尾换行 + 按文件名排序：同一天重跑生成的文件应当逐字节一致
  const content = `${checksums.sort().join('\n')}\n`;
  writeFileSync(join(outDir, 'SHA256SUMS'), content, 'utf8');
  console.log(`SHA256SUMS（${checksums.length} 项）`);
}

function relativeToRepo(path) {
  return path.slice(repoRoot.length + 1);
}
