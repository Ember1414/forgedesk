#!/usr/bin/env node
/**
 * 汇总一个目录下全部发布产物的 SHA256，生成 `SHA256SUMS`（M7 / T7.3，D8.5 复用）。
 *
 * # 为什么需要它（`rename-bundles.mjs --sha256` 不够用）
 *
 * `rename-bundles.mjs` 只对自己的产物（安装器）求校验和，而 Windows 的发布集里
 * 还有**便携版 zip**（由 `make-portable.mjs` 生成）——用户最常下载的就是它。
 * 两份清单并存会让"校验哪一个"变成一道考题，因此发布前统一重算一次：
 * **一个目录，一份 SHA256SUMS**。
 *
 * # 刻意排除哪些文件
 *
 * - `SHA256SUMS` / `SHA256SUMS.asc`：清单自身与它的签名（后者无法自校验）；
 * - `*.sig`：updater 的 Ed25519 签名。它由 updater 在安装前做**密码学校验**，
 *   纳入 sha256sum 只会让清单随"重新签名"漂移，却不增加任何验证能力；
 * - `*.sha256`：`make-portable.mjs` 为单文件写的重复清单。
 *
 * # 确定性
 *
 * 按文件名排序、固定结尾换行、哈希用**小写十六进制全量 64 位**（与 `sha256sum` 一致）。
 * 同一天重跑应逐字节一致——否则"校验和变了"会被误读成"产物被改过"。
 *
 * 用法：
 *   node scripts/ci/make-checksums.mjs --dir releases
 */
import { createHash } from 'node:crypto';
import { readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';

/** 不参与校验和的文件（理由见文件头）。 */
const ALWAYS_EXCLUDED = new Set(['SHA256SUMS', 'SHA256SUMS.asc']);
const EXCLUDED_SUFFIXES = ['.sig', '.sha256'];

function parseArgs(argv) {
  const options = { dir: undefined };
  for (let index = 0; index < argv.length; index += 1) {
    if (argv[index] === '--dir') {
      options.dir = argv[index + 1];
      index += 1;
    } else {
      console.error(`未知参数：${argv[index]}`);
      process.exit(2);
    }
  }
  if (options.dir === undefined) {
    console.error('缺少 --dir <目录>（发布产物所在目录）。');
    process.exit(2);
  }
  return options;
}

/** 该文件名是否参与校验和。 */
function isChecksummed(fileName) {
  if (ALWAYS_EXCLUDED.has(fileName)) {
    return false;
  }
  return !EXCLUDED_SUFFIXES.some((suffix) => fileName.endsWith(suffix));
}

const { dir } = parseArgs(process.argv.slice(2));
const absolute = resolve(dir);

let entries;
try {
  entries = readdirSync(absolute);
} catch (error) {
  console.error(`无法读取目录 ${absolute}：${error.message}`);
  process.exit(1);
}

// 子目录意味着"这一级不是扁平的产物集"——发布目录约定是扁平的，
// 静默忽略会让"少上传了一个文件"变成发布后才发现的问题，所以直接失败。
const subdirectories = entries.filter((name) => statSync(join(absolute, name)).isDirectory());
if (subdirectories.length > 0) {
  console.error(`发布目录里出现子目录（约定为扁平）：${subdirectories.join('、')}`);
  process.exit(1);
}

const files = entries.filter(isChecksummed).sort();
if (files.length === 0) {
  console.error(`目录里没有被校验的产物（全部被排除或目录为空）：${absolute}`);
  process.exit(1);
}

const lines = files.map((name) => {
  const hash = createHash('sha256')
    .update(readFileSync(join(absolute, name)))
    .digest('hex');
  console.log(`${hash.slice(0, 16)}…  ${name}`);
  return `${hash}  ${name}`;
});

writeFileSync(join(absolute, 'SHA256SUMS'), `${lines.join('\n')}\n`, 'utf8');
console.log('');
console.log(`已写入 ${join(absolute, 'SHA256SUMS')}（${lines.length} 项）`);
