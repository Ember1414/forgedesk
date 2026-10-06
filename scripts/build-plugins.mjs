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

run('cargo', ['build', '--release', '--target', 'wasm32-wasip1'], { cwd: pluginsRoot });

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
