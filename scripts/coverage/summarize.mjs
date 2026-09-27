#!/usr/bin/env node
/**
 * 覆盖率汇总（T1.12）。
 *
 * 输入是 `llvm-cov report` 的文本输出（默认 `target/cov-report.txt`），
 * 输出是**按 crate 聚合**的行覆盖率与总体行覆盖率。
 *
 * 为什么需要这么一个脚本：
 *
 * 1. 门禁的阈值是按 crate 定的（`domain` 与 `git-engine` ≥ 85%、workspace 整体 ≥ 60%），
 *    而 `llvm-cov report` 只给"每个文件 + 一行 TOTAL"。手工加总既慢又容易看错；
 * 2. 覆盖率数字进验收报告时要**可复现**：谁都能跑一次并得到同一组数字，
 *    比在文档里抄一份两个月前的百分比可信。
 *
 * 用法：
 *   node scripts/coverage/summarize.mjs [report.txt]
 * 生成 report 的完整命令见 `docs/acceptance/M1.md` 的"覆盖率"一节。
 */
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

/** 阈值：与 T1.12 的任务定义一致（domain 与 git-engine 更高）。 */
const THRESHOLDS = {
  'crates/domain/': 85,
  'crates/git-engine/': 85,
  '*': 60,
};

const reportPath = resolve(process.argv[2] ?? 'target/cov-report.txt');
const text = readFileSync(reportPath, 'utf8');

/**
 * 解析一行：`<filename>  <13 个数字/百分比列>`。
 *
 * 文件名里可能有空格，因此从**右边**取 13 列，剩下的都是路径。
 * 列顺序：Regions, MissedRegions, RegionCover, Functions, MissedFunctions,
 * Executed, Lines, MissedLines, LineCover, Branches, MissedBranches, BranchCover…
 * —— 不同 LLVM 版本列数略有差异，因此只认"末尾那个百分比是行覆盖"这一条：
 * 这里取的是**第 9 列**（Lines/Missed/LinesCover 的第二项与第三项）。
 */
const ROW_PATTERN =
  /^(?<filename>.+?)\s+(?<regions>\d+)\s+(?<missedRegions>\d+)\s+(?<regionCover>[\d.]+%|-)\s+(?<functions>\d+)\s+(?<missedFunctions>\d+)\s+(?<executed>[\d.]+%|-)\s+(?<lines>\d+)\s+(?<missedLines>\d+)\s+(?<lineCover>[\d.]+%|-)\s+(?<branches>\d+)\s+(?<missedBranches>\d+)\s+(?<branchCover>[\d.]+%|-)\s*$/;

function parseLine(line) {
  const match = ROW_PATTERN.exec(line.trim());
  if (match?.groups === undefined) {
    return null;
  }
  const { filename, lines, missedLines } = match.groups;
  return { filename, lines: Number(lines), missedLines: Number(missedLines) };
}

const rows = [];
for (const line of text.split('\n')) {
  const parsed = parseLine(line);
  if (parsed !== null && parsed.filename !== 'TOTAL' && !parsed.filename.startsWith('Files')) {
    rows.push(parsed);
  }
}

if (rows.length === 0) {
  // 没有数据行 = 报告格式不对（例如把 stderr 也重定向进了文件）：把前几行打出来，
  // 让下一个人一眼看出问题，而不是对着"0 行覆盖率 100%"发愣
  const preview = text
    .split('\n')
    .filter((line) => line.trim() !== '')
    .slice(0, 3)
    .map((line) => `  [${line.slice(0, 140)}]`)
    .join('\n');
  process.stderr.write(`报告里没有可解析的数据行：${reportPath}\n前几行：\n${preview}\n`);
  process.exit(2);
}

/** 把路径归到它所属的 crate（`crates/<name>/`）。 */
function crateOf(filename) {
  const normalized = filename.replace(/\\/g, '/');
  const match = normalized.match(/^crates\/([^/]+)\//);
  if (match !== null) {
    return `crates/${match[1]}/`;
  }
  if (normalized.startsWith('src-tauri/')) {
    return 'src-tauri/';
  }
  return 'other';
}

const totals = new Map();
for (const row of rows) {
  const crate = crateOf(row.filename);
  const current = totals.get(crate) ?? { lines: 0, missed: 0 };
  current.lines += row.lines;
  current.missed += row.missedLines;
  totals.set(crate, current);
}

const onlyCrates = rows.filter((row) => !row.filename.replace(/\\/g, '/').startsWith('src-tauri/'));
const overall = onlyCrates.reduce(
  (sum, row) => ({ lines: sum.lines + row.lines, missed: sum.missed + row.missedLines }),
  { lines: 0, missed: 0 },
);

function percent(bucket) {
  return bucket.lines === 0 ? 100 : ((bucket.lines - bucket.missed) / bucket.lines) * 100;
}

const failures = [];
process.stdout.write(`覆盖率报告：${reportPath}\n\n`);
process.stdout.write('crate                        行数    未覆盖    行覆盖率   阈值\n');
for (const [crate, bucket] of [...totals].sort()) {
  const threshold = THRESHOLDS[crate] ?? null;
  const cover = percent(bucket);
  const mark = threshold !== null && cover < threshold ? '✗' : ' ';
  process.stdout.write(
    `${mark} ${crate.padEnd(26)}${String(bucket.lines).padStart(6)}${String(bucket.missed).padStart(9)}` +
      `${cover.toFixed(2).padStart(10)}%${threshold === null ? '' : String(threshold).padStart(7)}\n`,
  );
  if (threshold !== null && cover < threshold) {
    failures.push(`${crate} 行覆盖率 ${cover.toFixed(2)}% < ${threshold}%`);
  }
}

const overallCover = percent(overall);
const overallMark = overallCover < THRESHOLDS['*'] ? '✗' : ' ';
process.stdout.write(
  `\n${overallMark} 总体（不含 src-tauri）      ${String(overall.lines).padStart(6)}` +
    `${String(overall.missed).padStart(9)}${overallCover.toFixed(2).padStart(10)}%` +
    `${String(THRESHOLDS['*']).padStart(7)}\n`,
);
if (overallCover < THRESHOLDS['*']) {
  failures.push(`整体行覆盖率 ${overallCover.toFixed(2)}% < ${THRESHOLDS['*']}%`);
}

if (failures.length > 0) {
  process.stdout.write(`\n未达标：\n${failures.map((item) => `  - ${item}`).join('\n')}\n`);
  process.exit(1);
}
process.stdout.write('\n全部达标。\n');
