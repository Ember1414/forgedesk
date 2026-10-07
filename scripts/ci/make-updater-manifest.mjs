#!/usr/bin/env node
/**
 * 生成 updater 清单（`latest.json`）（M7 / T7.1 / T7.3）。
 *
 * # 清单是自动更新的"契约"
 *
 * 已发布的应用里硬编码了公钥与更新地址，它会下载这份清单、比对版本、下载清单里
 * 那个 URL 指向的包、用清单里的 `signature` **验签**。因此清单由三段信息构成：
 *
 *   { version, notes, pub_date, platforms: { "<target>": { signature, url } } }
 *
 * `target` 用 Tauri 的取值（`windows-x86_64` / `darwin-aarch64` / `linux-x86_64`…），
 * 与 `endpoints` 模板里的 `{{target}}` 必须逐字一致——写错一个字符的后果是
 * "新版本永远检测不到"，而它在客户端表现为"没有更新"（最难排查的一类故障）。
 * 所以这里对 target 做**白名单校验**，宁可构建失败也不发出一个错误的清单。
 *
 * # 路径约定（静态托管）
 *
 * 清单发布到 `updates/<channel>/<target>.json`。**不带版本号**：静态托管无法做
 * "没有更新就返回 204"的协商，把版本写进路径会让每次发版都要新建一个文件，
 * 而漏传一次就是全量用户收不到更新。updater 自己会比对版本，旧清单不会触发升级。
 *
 * 若该文件已存在且传了 `--merge`，则只更新本次 `--target` 这一段：
 * M8 的三平台发布会各自上传自己那一段，谁都不会覆盖别人。
 *
 * 用法：
 *   node scripts/ci/make-updater-manifest.mjs --version 0.7.1 --target windows-x86_64 \
 *     --signature <包.sig> --url <包的下载地址> [--notes <文件>] [--pub-date <ISO8601>] \
 *     [--merge] --out <latest.json>
 */
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';

/** Tauri 认可的 target 取值（与 `endpoints` 的 `{{target}}` 同源）。 */
const KNOWN_TARGETS = new Set([
  'windows-x86_64',
  'windows-aarch64',
  'windows-i686',
  'darwin-x86_64',
  'darwin-aarch64',
  'linux-x86_64',
  'linux-aarch64',
  'linux-i686',
]);

function parseArgs(argv) {
  const options = {
    version: undefined,
    target: undefined,
    signature: undefined,
    url: undefined,
    notes: undefined,
    pubDate: undefined,
    merge: false,
    out: undefined,
  };
  for (let index = 0; index < argv.length; index += 1) {
    const key = argv[index];
    switch (key) {
      case '--version':
        options.version = argv[index + 1];
        index += 1;
        break;
      case '--target':
        options.target = argv[index + 1];
        index += 1;
        break;
      case '--signature':
        options.signature = argv[index + 1];
        index += 1;
        break;
      case '--url':
        options.url = argv[index + 1];
        index += 1;
        break;
      case '--notes':
        options.notes = argv[index + 1];
        index += 1;
        break;
      case '--pub-date':
        options.pubDate = argv[index + 1];
        index += 1;
        break;
      case '--merge':
        options.merge = true;
        break;
      case '--out':
        options.out = argv[index + 1];
        index += 1;
        break;
      default:
        console.error(`未知参数：${key}`);
        process.exit(2);
    }
  }

  const missing = ['version', 'target', 'signature', 'url', 'out'].filter(
    (key) => options[key] === undefined,
  );
  if (missing.length > 0) {
    console.error(`缺少参数：${missing.map((key) => `--${key}`).join('、')}`);
    process.exit(2);
  }
  return options;
}

const options = parseArgs(process.argv.slice(2));

if (!KNOWN_TARGETS.has(options.target)) {
  console.error(
    `未知的 --target：${options.target}\n` +
      `必须是 Tauri 的取值之一：${[...KNOWN_TARGETS].join('、')}\n` +
      '（它必须与 tauri.conf.json 里 updater endpoints 的 {{target}} 逐字一致）',
  );
  process.exit(1);
}

// 版本号不在这里定义"合法"，但必须非空且不含空白：空版本会让 updater 永远认为"已是最新"
if (options.version.trim() === '' || /\s/.test(options.version)) {
  console.error(`--version 非法：${JSON.stringify(options.version)}`);
  process.exit(1);
}

/**
 * 读文本文件；**缺文件时给出可读错误**而不是让 ENOENT 抛栈。
 *
 * 这条路径上的每一次失败都表现为"流水线红在了一个看不出原因的地方"，
 * 而真正的原因（签名没产出、notes 路径写错）只有一行字之遥。
 */
function readTextOrExit(path, label) {
  try {
    return readFileSync(resolve(path), 'utf8');
  } catch (error) {
    console.error(`无法读取${label}：${path} —— ${error.message}`);
    process.exit(1);
  }
}

const signature = readTextOrExit(options.signature, '签名文件').trim();
if (signature === '') {
  console.error(`签名文件为空：${options.signature}（未签名的清单会被客户端拒绝安装）`);
  process.exit(1);
}

const notes =
  options.notes === undefined ? undefined : readTextOrExit(options.notes, '说明文件').trim();

let platforms = {};
if (options.merge && existsSync(resolve(options.out))) {
  const existing = JSON.parse(readFileSync(resolve(options.out), 'utf8'));
  if (existing.platforms !== undefined && typeof existing.platforms === 'object') {
    platforms = existing.platforms;
  }
  console.error(`--merge：保留已存在的 ${Object.keys(platforms).length} 个平台条目`);
}

platforms[options.target] = { signature, url: options.url };

const manifest = {
  version: options.version,
  ...(notes === undefined || notes === '' ? {} : { notes }),
  pub_date: options.pubDate ?? new Date().toISOString(),
  platforms,
};

// 输出目录由本脚本保证存在：清单的路径形如 `updates/<渠道>/<target>.json`，
// 而那个子目录在第一次发布时并不存在——把"先 mkdir"的责任推给每个调用方，
// 迟早会有一次在发版当天以 ENOENT 收场（本脚本的本地演练就撞上过这个）
const outPath = resolve(options.out);
mkdirSync(dirname(outPath), { recursive: true });

// 缩进 + 结尾换行：清单是要被人读、被人 diff 的（排障时第一眼看的就是它）
writeFileSync(outPath, `${JSON.stringify(manifest, null, 2)}\n`, 'utf8');
console.log(`已写入 ${options.out}`);
console.log(`  version: ${manifest.version}`);
console.log(`  platforms: ${Object.keys(platforms).join('、')}`);
console.log(`  pub_date: ${manifest.pub_date}`);
