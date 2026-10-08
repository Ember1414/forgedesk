#!/usr/bin/env node
/**
 * 基准回归门禁（T2.9）：把本次探针结果与入库基线比较，退化 > 10% 即失败。
 *
 * 用法：
 *   node scripts/perf/check-regression.mjs <results-dir>
 *
 * - `<results-dir>` 里有若干 `<fixture>.json`（perf_probe 的一行 JSON 输出）。
 * - 基线文件 `scripts/perf/baseline-ci.json` **不存在**时：本次结果成为基线
 *   （写在该路径下），退出 0 并打印提示——工作流只读权限，入仓提交由维护者
 *   下载 artifact 完成。
 * - 基线存在时：逐夹具、逐指标比较，`current > baseline * (1 + 0.10)` 的
 *   指标全部列出并以非零退出；任一指标缺失（基线有、本次没有）也视为失败，
 *   探针悄悄少量一个数字不该被当成"没退化"。
 *
 * 阈值为什么是 10%：任务书 T2.9 的约定。CI runner 是共享资源，绝对值会漂，
 * 但同一 runner 池的相对波动通常远小于 10%（夹具与口径都是确定的）。
 */
import {
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  statSync,
  writeFileSync,
} from 'node:fs';
import { join, resolve } from 'node:path';

const BASELINE_PATH = resolve('scripts/perf/baseline-ci.json');
const THRESHOLD = 0.1;

const resultsDir = process.argv[2];
if (resultsDir === undefined) {
  process.stderr.write('用法：node scripts/perf/check-regression.mjs <results-dir>\n');
  process.exit(2);
}
// 结果目录必须真的存在：此前不存在时 readdirSync 抛未捕获异常，堆栈里只有
// ENOENT，看不出"是路径写错了"还是"探针没产出"（2026-10-08 的 Nightly 失败）
if (!existsSync(resultsDir) || !statSync(resultsDir).isDirectory()) {
  const message = `结果目录不存在或不是目录：${resultsDir}`;
  process.stderr.write(`${message}\n`);
  process.stdout.write(`::error title=性能门禁::${message}\n`);
  process.exit(2);
}

// 结果目录 → { 夹具名: { 指标: 数值 } }
const current = {};
for (const file of readdirSync(resultsDir)) {
  if (!file.endsWith('.json')) {
    continue;
  }
  const name = file.replace(/\.json$/, '');
  current[name] = JSON.parse(readFileSync(join(resultsDir, file), 'utf8'));
}
if (Object.keys(current).length === 0) {
  process.stderr.write(`结果目录为空：${resultsDir}\n`);
  process.exit(2);
}

if (!existsSync(BASELINE_PATH)) {
  mkdirSync(resolve('scripts/perf'), { recursive: true });
  writeFileSync(BASELINE_PATH, JSON.stringify(current, null, 2) + '\n');
  process.stdout.write(
    '未找到基线，本次结果已写入 scripts/perf/baseline-ci.json（仅存在于本次运行的 artifact）。\n' +
      '请下载 perf-results artifact，把该文件拷进仓库并提交——从下一次运行起回归门禁生效。\n',
  );
  process.exit(0);
}

const baseline = JSON.parse(readFileSync(BASELINE_PATH, 'utf8'));
const regressions = [];

for (const [fixture, metrics] of Object.entries(current)) {
  const base = baseline[fixture];
  if (base === undefined) {
    regressions.push(`${fixture}: 基线中没有这个夹具（基线需要更新）`);
    continue;
  }
  for (const [metric, value] of Object.entries(metrics)) {
    if (value === null || value === undefined) {
      continue; // 本次没量到的指标（如未传 --deep-cursor）不参与比较
    }
    const baseValue = base[metric];
    if (baseValue === null || baseValue === undefined) {
      regressions.push(`${fixture}.${metric}: 基线缺这一项（基线需要更新）`);
      continue;
    }
    if (typeof value !== 'number' || typeof baseValue !== 'number') {
      continue; // 非数值字段（watchSeconds 等）不比较
    }
    if (value > baseValue * (1 + THRESHOLD)) {
      const percent = (((value - baseValue) / baseValue) * 100).toFixed(1);
      regressions.push(
        `${fixture}.${metric}: ${baseValue} → ${value}（+${percent}%，超过 ${THRESHOLD * 100}% 门禁）`,
      );
    }
  }
}

if (regressions.length > 0) {
  process.stderr.write('性能回归门禁失败：\n');
  for (const line of regressions) {
    process.stderr.write(`  - ${line}\n`);
  }
  // 同时发成 GitHub 注解：**日志 API 不允许匿名读取**，而注解可以。
  // 否则"哪个指标退化了、退化多少"永远只有登录 GitHub 的人能看到
  // （2026-10-08 的 Nightly 失败就是这样被卡住的）。
  for (const line of regressions) {
    process.stdout.write(`::error title=性能回归::${line}\n`);
  }
  process.exit(1);
}
process.stdout.write('性能回归门禁通过：所有指标都在基线 10% 以内。\n');
