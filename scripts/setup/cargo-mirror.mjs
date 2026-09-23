#!/usr/bin/env node
/**
 * 在本地生成 `.cargo/config.toml`，把 crates.io 指向国内镜像。
 *
 * 为什么是「本地生成」而不是「提交到仓库」：
 *   Cargo 源替换是**编译机器网络环境**的属性，不是项目的属性。
 *   如果把它提交进仓库，GitHub Actions 上的 runner（位于海外）也会被迫去访问
 *   国内镜像，结果是从"慢"变成"更慢甚至超时"。
 *   因此该文件被 .gitignore 排除，需要的人自己生成。
 *
 * 用法：
 *   node scripts/setup/cargo-mirror.mjs              # 使用 rsproxy.cn（大陆推荐）
 *   node scripts/setup/cargo-mirror.mjs --tuna       # 使用清华 TUNA 镜像
 *   node scripts/setup/cargo-mirror.mjs --ustc       # 使用中科大镜像
 *   node scripts/setup/cargo-mirror.mjs --remove     # 删除本地镜像配置，回到官方源
 */
import { mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');
const configPath = join(repoRoot, '.cargo', 'config.toml');

const MIRRORS = {
  rsproxy: {
    index: 'sparse+https://rsproxy.cn/index/',
    label: 'rsproxy.cn（字节跳动 Rust 团队维护）',
  },
  tuna: {
    index: 'sparse+https://mirrors.tuna.tsinghua.edu.cn/crates.io-index/',
    label: '清华 TUNA',
  },
  ustc: { index: 'sparse+https://mirrors.ustc.edu.cn/crates.io-index/', label: '中科大 USTC' },
};

const args = process.argv.slice(2);

if (args.includes('--remove')) {
  rmSync(configPath, { force: true });
  console.log(`已删除 ${configPath}，cargo 将恢复使用官方 crates.io。`);
  process.exit(0);
}

const selected = args.includes('--tuna') ? 'tuna' : args.includes('--ustc') ? 'ustc' : 'rsproxy';
const mirror = MIRRORS[selected];

const content = `# 本文件由 scripts/setup/cargo-mirror.mjs 生成，已被 .gitignore 排除。
#
# 作用：把 crates.io 指向镜像源，避免在部分网络环境下依赖拉取超时。
# 当前镜像：${mirror.label}
#
# 恢复官方源：node scripts/setup/cargo-mirror.mjs --remove
# 注意：本文件属于「本机环境配置」，不要提交到仓库（否则会影响 CI runner）。

[source.crates-io]
replace-with = "mirror"

[source.mirror]
registry = "${mirror.index}"

[net]
# 使用系统 git 拉取依赖（对代理与 SSH 配置更友好）
git-fetch-with-cli = true
retry = 3
`;

mkdirSync(dirname(configPath), { recursive: true });
writeFileSync(configPath, content, 'utf8');

console.log(`已写入 ${configPath}`);
console.log(`  镜像：${mirror.label}`);
