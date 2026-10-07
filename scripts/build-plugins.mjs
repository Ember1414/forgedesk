#!/usr/bin/env node
/**
 * 构建示例插件并同步产物到 examples 目录（T6.5）。
 *
 * 用法：`node scripts/build-plugins.mjs`
 *
 * 前置：`rustup target add wasm32-wasip1`。
 * 产物（plugins/examples/<id>/plugin.wasm）**提交入库**：
 *   - 用户克隆仓库即可用"开发者模式"安装示例插件；
 *   - plugin-host 的集成冒烟测试直接加载入库产物（无需先跑构建）；
 *   - CI 会重新编译并 `git diff --exit-code` 校验"源码与产物同步"。
 */
import { spawnSync } from 'node:child_process';
import { copyFileSync, existsSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const pluginsRoot = join(repoRoot, 'plugins');
const target = join(pluginsRoot, 'target', 'wasm32-wasip1', 'release');

/**
 * 产物确定性（为什么这里要拼 RUSTFLAGS）：
 *
 * plugin.wasm 是**提交入库**的，CI 会重建并做字节级比对。而 Rust 的 panic 位置
 * （每个 unwrap/expect 的 Location）以**绝对路径字符串**写进 .rodata——`strip = true`
 * 只删 DWARF，删不掉这些字符串。于是"在哪台机器上构建"就会改变 wasm 的字节：
 * 本机 `e:\Projects\...`、CI `D:\a\forgedesk\...`、注册表 `C:\Users\<谁>\.cargo\...`。
 *
 * 解法：把所有机器相关前缀重映射成固定名字。同一前缀给两种分隔符各一条
 * （rustc 对路径分隔符的规范化行为没有稳定承诺，两条里总有一条命中，而命中的
 * 产物字符串相同）。首次重建产物时本应多出几行 diff，属于预期的一次性变化。
 */
const toSlash = (value) => value.replaceAll('\\', '/');
const remap = (prefix) => [
  `--remap-path-prefix=${prefix}=/build`,
  `--remap-path-prefix=${toSlash(prefix)}=/build`,
];
const cargoHome = resolve(
  process.env.CARGO_HOME || join(process.env.USERPROFILE ?? process.env.HOME, '.cargo'),
);
const rustflags = [...remap(pluginsRoot), ...remap(cargoHome)].join(' ');

const PLUGINS = ['commit-template', 'repo-stats', 'repo-audit'];

function run(command, args, options = {}) {
  const result = spawnSync(command, args, { stdio: 'inherit', ...options });
  if (result.status !== 0) {
    console.error(`\n[build-plugins] ${command} ${args.join(' ')} 失败（exit ${result.status}）`);
    process.exit(result.status ?? 1);
  }
}

// rustup 目标检查：给出明确的安装指引而不是让 cargo 的报错满天飞
const rustup = spawnSync('rustup', ['target', 'list', '--installed'], { encoding: 'utf8' });
if (!String(rustup.stdout).includes('wasm32-wasip1')) {
  console.error(
    '[build-plugins] 缺少 wasm32-wasip1 目标，请先执行：rustup target add wasm32-wasip1',
  );
  process.exit(1);
}

run('cargo', ['build', '--release', '--target', 'wasm32-wasip1'], {
  cwd: pluginsRoot,
  env: {
    ...process.env,
    RUSTFLAGS: [process.env.RUSTFLAGS, rustflags].filter(Boolean).join(' '),
  },
});

for (const name of PLUGINS) {
  const source = join(target, `plugin_${name.replace(/-/g, '_')}.wasm`);
  if (!existsSync(source)) {
    console.error(`[build-plugins] 未找到产物：${source}`);
    process.exit(1);
  }
  const destination = join(pluginsRoot, 'examples', name, 'plugin.wasm');
  copyFileSync(source, destination);
  console.log(`[build-plugins] ${name} → ${destination}`);
}
console.log('[build-plugins] 完成。');
