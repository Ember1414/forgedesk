#!/usr/bin/env node
/**
 * 生成发布构建用的 Tauri 配置覆盖文件（`release-config.json`）（M7 / T7.3）。
 *
 * # 为什么要覆盖而不是直接写进 tauri.conf.json
 *
 * 更新源（endpoints）与公钥都是**发布配置**，而且有两条硬约束：
 *
 * 1. `bundle.createUpdaterArtifacts` 一旦开启，Tauri 就要求 `plugins.updater.pubkey`
 *    存在——而**公钥只有发布方才有**。源码自编译、开发构建与贡献者的本地构建都
 *    不该被这条要求拦住（`update_check` 正是为此返回 `configured: false`）。
 * 2. endpoints 指向云托管的清单地址；在托管落地之前把它写进仓库，会让开发构建
 *    从"静默无更新源"变成"每次检查都网络失败"，把一条不该出现的错误提示塞给用户。
 *
 * 因此：**仓库里的配置保持"没有更新源"**，发布流水线用 `tauri build --config <本文件>`
 * 在构建时叠加这两项。副作用是"发布产物有更新能力、开发产物没有"这件事变得可见
 * （见 `docs/RELEASE.md` §4.1）。
 *
 * 用法：
 *   node scripts/ci/make-build-config.mjs \
 *     --pubkey <公钥内容> --channel stable \
 *     [--base-url https://forgedesk.pages.dev] --out release-config.json
 *
 * `--print` 只打印结果不落盘（本地看一眼它到底注入了什么）。
 */
import { writeFileSync } from 'node:fs';
import { resolve } from 'node:path';

/** 渠道 → 清单路径段。两侧必须一致：这里改了，`docs/RELEASE.md` 与 Pages 目录也要改。 */
const CHANNELS = new Set(['stable', 'beta']);

function parseArgs(argv) {
  const options = {
    pubkey: undefined,
    channel: 'stable',
    baseUrl: 'https://forgedesk.pages.dev',
    out: 'release-config.json',
    print: false,
  };
  for (let index = 0; index < argv.length; index += 1) {
    const key = argv[index];
    switch (key) {
      case '--pubkey':
        options.pubkey = argv[index + 1];
        index += 1;
        break;
      case '--channel':
        options.channel = argv[index + 1];
        index += 1;
        break;
      case '--base-url':
        options.baseUrl = argv[index + 1];
        index += 1;
        break;
      case '--out':
        options.out = argv[index + 1];
        index += 1;
        break;
      case '--print':
        options.print = true;
        break;
      default:
        console.error(`未知参数：${key}`);
        process.exit(2);
    }
  }

  if (options.pubkey === undefined || options.pubkey.trim() === '') {
    console.error(
      '缺少 --pubkey（更新签名公钥）。\n' +
        '它由 Secrets 提供；缺失时说明这个仓库还没准备好发布，流水线应当选择跳过而不是发出未签名的更新。',
    );
    process.exit(2);
  }
  if (!CHANNELS.has(options.channel)) {
    console.error(`未知的 --channel：${options.channel}（可取值：${[...CHANNELS].join(' / ')}）`);
    process.exit(2);
  }
  return options;
}

const options = parseArgs(process.argv.slice(2));

// endpoint 模板：Tauri 会替换 {{target}} / {{arch}} / {{current_version}}。
// 这里**只用 {{target}}**：路径里已经带了渠道，版本由清单内容回答，
// 把 current_version 写进路径会让静态托管需要为每个版本准备一个文件（漏传一次就断更）。
const endpoint = `${options.baseUrl.replace(/\/+$/, '')}/updates/${options.channel}/{{target}}.json`;

const config = {
  bundle: {
    // 开启后 Tauri 会为安装包生成 .sig（updater 的验签材料）
    createUpdaterArtifacts: true,
  },
  plugins: {
    updater: {
      pubkey: options.pubkey.trim(),
      endpoints: [endpoint],
    },
  },
};

const json = `${JSON.stringify(config, null, 2)}\n`;

if (options.print) {
  process.stdout.write(json);
} else {
  writeFileSync(resolve(options.out), json, 'utf8');
  console.error(`已写入 ${options.out}`);
  // 只回显渠道与 endpoint（**不回显公钥全文**：日志会长期留存，公钥虽公开，
  // 但把一长串 base64 打进日志毫无排查价值，只会淹没真正的信息）
  console.error(`  channel:  ${options.channel}`);
  console.error(`  endpoint: ${endpoint}`);
}
