#!/usr/bin/env node
/**
 * 生成 Windows 便携版 zip（M7 / T7.4）。
 *
 * # 为什么需要便携版
 *
 * 零成本方案（红线 R5）没有付费代码签名证书，安装器会触发 SmartScreen。
 * 便携版 zip 解压即用，**不经过安装器**，是最不容易被拦的分发形态；
 * 也是 Scoop/清单类渠道之外"只想试一下"的用户成本最低的入口。
 *
 * # 为什么自己写 ZIP 而不是引入依赖
 *
 * 需要写 zip 的只有这一个场景，而引入 archiver/jszip 会为它带来一棵依赖树
 * （供应链面变大，红线 R5/R8 的精神是能不加就不加）。ZIP 的"store/deflate +
 * 中央目录"格式足够简单，用 Node 内建的 `zlib.deflateRawSync` 即可完成，
 * 且产物可以用系统解压器验证（本仓库的做法：生成后用 Expand-Archive 回读）。
 *
 * # 产物
 *
 *   <out>/ForgeDesk_<version>_windows_<arch>_portable.zip
 *   <out>/ForgeDesk_<version>_windows_<arch>_portable.zip.sha256   （`<hash>  <文件名>`）
 *
 * zip 内包含：
 *   ForgeDesk.exe          可执行文件（由 --exe 指定或自动探测 release 产物）
 *   LICENSE                Apache-2.0 全文
 *   README-portable.txt    使用与校验说明（免安装、SmartScreen、校验方法）
 *
 * 用法：
 *   node scripts/ci/make-portable.mjs --out <目录> [--exe <forgedesk.exe>] [--arch x64|arm64]
 */

import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { deflateRawSync } from 'node:zlib';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');
const ZIP_LOCAL_SIG = 0x04034b50;
const ZIP_CENTRAL_SIG = 0x02014b50;
const ZIP_EOCD_SIG = 0x06054b50;
/** 通用位标记：bit 11 = 文件名为 UTF-8（中文机器名/路径下更安全）。 */
const FLAG_UTF8 = 0x0800;
/** 压缩方法：8 = deflate。 */
const METHOD_DEFLATE = 8;

function parseArgs(argv) {
  const options = { out: undefined, exe: undefined, arch: undefined };
  for (let index = 0; index < argv.length; index += 1) {
    const key = argv[index];
    const next = argv[index + 1];
    switch (key) {
      case '--out':
        options.out = next;
        index += 1;
        break;
      case '--exe':
        options.exe = next;
        index += 1;
        break;
      case '--arch':
        options.arch = next;
        index += 1;
        break;
      default:
        console.error(`未知参数：${key}`);
        process.exit(2);
    }
  }
  if (options.out === undefined) {
    console.error('缺少 --out <目录>（便携版 zip 的输出目录）。');
    process.exit(2);
  }
  if (options.arch !== undefined && !['x64', 'arm64'].includes(options.arch)) {
    console.error(`--arch 只支持 x64 | arm64，收到：${options.arch}`);
    process.exit(2);
  }
  return options;
}

/** 从 tauri.conf.json 读版本号（单一真相源，不在这里再写一份）。 */
function readVersion() {
  const config = JSON.parse(readFileSync(join(repoRoot, 'src-tauri', 'tauri.conf.json'), 'utf8'));
  if (typeof config.version !== 'string' || config.version === '') {
    console.error('tauri.conf.json 缺少 version 字段。');
    process.exit(1);
  }
  return config.version;
}

/** 探测 release 可执行文件（workspace 根 target 优先，其次是 src-tauri 下的历史布局）。 */
function findExe() {
  const candidates = [
    join(repoRoot, 'target', 'release', 'forgedesk.exe'),
    join(repoRoot, 'src-tauri', 'target', 'release', 'forgedesk.exe'),
  ];
  const found = candidates.find((path) => existsSync(path));
  if (found === undefined) {
    console.error(`未找到 release 可执行文件（找过：${candidates.join('、')}）。`);
    console.error('请先运行：pnpm tauri build（或在 CI 里跑完 release 构建）。');
    process.exit(1);
  }
  return found;
}

// ---------------------------------------------------------------- ZIP 写入

/** CRC32 查表（ZIP 用的是 IEEE 802.3 多项式，反射形式 0xEDB88320）。 */
const CRC_TABLE = (() => {
  const table = new Uint32Array(256);
  for (let index = 0; index < 256; index += 1) {
    let value = index;
    for (let bit = 0; bit < 8; bit += 1) {
      value = value & 1 ? 0xedb88320 ^ (value >>> 1) : value >>> 1;
    }
    table[index] = value >>> 0;
  }
  return table;
})();

function crc32(buffer) {
  let crc = 0xffffffff;
  for (const byte of buffer) {
    crc = CRC_TABLE[(crc ^ byte) & 0xff] ^ (crc >>> 8);
  }
  return (crc ^ 0xffffffff) >>> 0;
}

/** MS-DOS 时间/日期编码（ZIP 头用的就是这套）。 */
function dosDateTime(date) {
  const year = Math.max(1980, date.getFullYear());
  const time =
    ((date.getHours() << 11) | (date.getMinutes() << 5) | (date.getSeconds() >> 1)) & 0xffff;
  const day = (((year - 1980) << 9) | ((date.getMonth() + 1) << 5) | date.getDate()) & 0xffff;
  return { time, day };
}

/**
 * 把若干 { name, data } 条目打成 ZIP（deflate）。
 *
 * 只实现这一个场景需要的部分：全部用 deflate、无 zip64（产物远小于 4GB）、
 * 无注释/无附加字段。返回可直接写盘的 Buffer。
 */
function createZip(entries, now = new Date()) {
  const { time, day } = dosDateTime(now);
  const localParts = [];
  const centralParts = [];
  let offset = 0;

  for (const entry of entries) {
    const nameBytes = Buffer.from(entry.name, 'utf8');
    const raw = entry.data;
    const deflated = deflateRawSync(raw, { level: 9 });
    // 压不动就退回 store：小文件/已压缩数据上 deflate 反而更大
    const useDeflate = deflated.length < raw.length;
    const payload = useDeflate ? deflated : raw;
    const method = useDeflate ? METHOD_DEFLATE : 0;
    const crc = crc32(raw);

    const local = Buffer.alloc(30);
    local.writeUInt32LE(ZIP_LOCAL_SIG, 0);
    local.writeUInt16LE(20, 4); // 解压所需版本 2.0
    local.writeUInt16LE(FLAG_UTF8, 6);
    local.writeUInt16LE(method, 8);
    local.writeUInt16LE(time, 10);
    local.writeUInt16LE(day, 12);
    local.writeUInt32LE(crc, 14);
    local.writeUInt32LE(payload.length, 18);
    local.writeUInt32LE(raw.length, 22);
    local.writeUInt16LE(nameBytes.length, 26);
    local.writeUInt16LE(0, 28); // 附加字段长度

    localParts.push(local, nameBytes, payload);

    const central = Buffer.alloc(46);
    central.writeUInt32LE(ZIP_CENTRAL_SIG, 0);
    central.writeUInt16LE(20, 4); // 生成方版本（0x0014 = 2.0，DOS）
    central.writeUInt16LE(20, 6); // 解压所需版本
    central.writeUInt16LE(FLAG_UTF8, 8);
    central.writeUInt16LE(method, 10);
    central.writeUInt16LE(time, 12);
    central.writeUInt16LE(day, 14);
    central.writeUInt32LE(crc, 16);
    central.writeUInt32LE(payload.length, 20);
    central.writeUInt32LE(raw.length, 24);
    central.writeUInt16LE(nameBytes.length, 28);
    central.writeUInt16LE(0, 30); // 附加字段
    central.writeUInt16LE(0, 32); // 注释
    central.writeUInt16LE(0, 34); // 起始磁盘
    central.writeUInt16LE(0, 36); // 内部属性
    central.writeUInt32LE(0x20, 38); // 外部属性：置 DOS 归档位
    central.writeUInt32LE(offset, 42); // 本地头偏移

    centralParts.push(central, nameBytes);
    offset += local.length + nameBytes.length + payload.length;
  }

  const centralSize = centralParts.reduce((total, part) => total + part.length, 0);
  const eocd = Buffer.alloc(22);
  eocd.writeUInt32LE(ZIP_EOCD_SIG, 0);
  eocd.writeUInt16LE(0, 4); // 当前磁盘
  eocd.writeUInt16LE(0, 6); // 中央目录起始磁盘
  eocd.writeUInt16LE(entries.length, 8);
  eocd.writeUInt16LE(entries.length, 10);
  eocd.writeUInt32LE(centralSize, 12);
  eocd.writeUInt32LE(offset, 16);
  eocd.writeUInt16LE(0, 20); // 注释长度

  return Buffer.concat([...localParts, ...centralParts, eocd]);
}

/** 便携版说明文件内容（中文为主，附一行英文；Windows 记事本可读）。 */
function portableReadme(version) {
  return [
    `ForgeDesk ${version} — 便携版 / portable build`,
    '',
    '用法：解压到任意目录，双击 ForgeDesk.exe 运行（无需安装，无需管理员权限）。',
    'Usage: unzip anywhere and run ForgeDesk.exe. No installer, no admin rights.',
    '',
    '说明：',
    '  1. 应用需要系统已安装 Git 命令行（用于写操作）与 WebView2 运行时',
    '     （Windows 10/11 通常已预装；缺失时请安装 Microsoft Edge WebView2 Runtime）。',
    '  2. 未使用付费代码签名证书，首次运行可能提示“Windows 已保护你的电脑”。',
    '     如确认下载来源可信：点“更多信息”→“仍要运行”。',
    '  3. 校验下载完整性（PowerShell）：',
    '       Get-FileHash .\\ForgeDesk_%VERSION%_windows_%ARCH%_portable.zip -Algorithm SHA256',
    '     与发布页的 SHA256SUMS（或同名 .sha256 文件）比对。',
    '',
    '隐私：不收集代码内容、无遥测、无 AI 推理；凭据只存系统凭据库。详见应用内“设置 → 隐私”。',
    '',
    '许可：Apache License 2.0（见同目录 LICENSE）。',
    'ForgeDesk is an independent project and is not affiliated with the Git project,',
    'the Software Freedom Conservancy, GitHub, Inc., or the Tauri project.',
    '',
  ]
    .join('\r\n')
    .replaceAll('%VERSION%', version);
}

// ---------------------------------------------------------------- 主流程

const options = parseArgs(process.argv.slice(2));
const version = readVersion();
const arch = options.arch ?? (process.arch === 'arm64' ? 'arm64' : 'x64');
const exePath = options.exe === undefined ? findExe() : resolve(options.exe);
const outDir = resolve(options.out);

if (!existsSync(exePath) || !statSync(exePath).isFile()) {
  console.error(`--exe 指向的不是一个文件：${exePath}`);
  process.exit(1);
}

const licensePath = join(repoRoot, 'LICENSE');
if (!existsSync(licensePath)) {
  console.error(`缺少 LICENSE 文件：${licensePath}`);
  process.exit(1);
}

const baseName = `ForgeDesk_${version}_windows_${arch}_portable`;
const zipPath = join(outDir, `${baseName}.zip`);

const zip = createZip([
  { name: 'ForgeDesk.exe', data: readFileSync(exePath) },
  { name: 'LICENSE', data: readFileSync(licensePath) },
  { name: 'README-portable.txt', data: Buffer.from(portableReadme(version), 'utf8') },
]);

mkdirSync(outDir, { recursive: true });
writeFileSync(zipPath, zip);

const digest = createHash('sha256').update(zip).digest('hex');
writeFileSync(join(outDir, `${baseName}.zip.sha256`), `${digest}  ${baseName}.zip\n`);

console.log(`便携版已生成：${zipPath}`);
console.log(`  exe    : ${exePath}`);
console.log(`  大小   : ${(zip.length / 1024 / 1024).toFixed(1)} MB`);
console.log(`  sha256 : ${digest}`);
