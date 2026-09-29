#!/usr/bin/env node
/**
 * 性能基线用的仓库夹具生成器（T1.12 第 4 条）。
 *
 * 为什么要生成而不是克隆：性能基线要的是**可复现的规模**（100 / 1k / 10k /
 * 50k / 100k 提交），而克隆来的仓库既不可复现（内容会变），也不允许把别人的
 * 代码放进验收流程。`git fast-import` 能在几十秒内造出十万提交，
 * 且不产生任何工作区文件——量的是"读历史/读状态"的开销，不是磁盘 IO。
 *
 * 三种模式（可组合）：
 *   node scripts/perf/gen-repo.mjs <目录> --commits 100000   # N 次提交的历史
 *   node scripts/perf/gen-repo.mjs <目录> --untracked 10000  # 1 次提交 + N 个未跟踪文件
 *   node scripts/perf/gen-repo.mjs <目录> --commits 100000 --branches 200
 *     # 主链之外再开 N 条分支（每条 2 个提交，每 4 条合并回 main 一次），
 *     # 量的是"分支多 tips"对引用枚举、日志与布局的影响
 *
 * 生成的仓库带固定的提交时间（基准时间 + 序号）与固定的作者，
 * 因此同一个 N 每次生成的提交 oid 完全相同。
 */
import { execFileSync } from 'node:child_process';
import { mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';

const [targetArg, ...rest] = process.argv.slice(2);
if (targetArg === undefined) {
  process.stderr.write(
    '用法：node scripts/perf/gen-repo.mjs <目录> [--commits N] [--untracked N] [--branches N]\n',
  );
  process.exit(2);
}

const target = resolve(targetArg);
const commitsArg = readNumber(rest, '--commits', 0);
const untrackedArg = readNumber(rest, '--untracked', 0);
const modifiedArg = readNumber(rest, '--modified', 0);
const branchesArg = readNumber(rest, '--branches', 0);
if (branchesArg > 0 && commitsArg === 0) {
  process.stderr.write('--branches 需要与 --commits 一起使用：分支要从主链的提交上分出来\n');
  process.exit(2);
}

function readNumber(args, flag, fallback) {
  const index = args.indexOf(flag);
  if (index === -1) {
    return fallback;
  }
  const value = Number(args[index + 1]);
  if (!Number.isFinite(value) || value < 0) {
    process.stderr.write(`${flag} 需要一个非负整数\n`);
    process.exit(2);
  }
  return value;
}

rmSync(target, { recursive: true, force: true });
mkdirSync(target, { recursive: true });

function git(args, options = {}) {
  return execFileSync('git', args, { cwd: target, stdio: 'pipe', ...options });
}

git(['init', '--initial-branch=main']);
git(['config', 'user.name', 'Perf Fixture']);
git(['config', 'user.email', 'perf@example.invalid']);
git(['config', 'core.autocrlf', 'false']);

if (commitsArg > 0) {
  const baseTime = 1_600_000_000;
  let stream = 'reset refs/heads/main\n';
  let clock = 0;

  for (let index = 1; index <= commitsArg; index += 1) {
    const message = `perf commit ${index}\n`;
    clock += 1;
    stream += `commit refs/heads/main\nmark :${index}\n`;
    stream += `committer Perf Fixture <perf@example.invalid> ${baseTime + clock} +0000\n`;
    stream += `data ${Buffer.byteLength(message)}\n${message}`;
    // 每次提交改一个文件：文件数少、提交数多，正是"历史规模"这一类
    stream += `M 100644 inline src/file-${index % 32}.txt\n`;
    const content = `line ${index}\n`;
    stream += `data ${Buffer.byteLength(content)}\n${content}`;
  }

  // 分支与合并在**同一个 fast-import 流**里追加：mark 只在单个流内有效，
  // 拆两个流就得靠解析 rev-list 拿真实 oid，成本与出错面都大。
  // 形状：第 i 条分支从主链均匀取的分叉点长出 2 个提交；每 4 条把该分支
  // 合并回 main（两父 merge，给布局器喂多父节点）。mark 分配：
  //   :1..:N            主链
  //   :N+1..:N+2B       各分支的两个提交（分支 i 占 N+2i+1、N+2i+2）
  //   :N+2B+1..         依序的 merge 提交
  const mergeMarkBase = commitsArg + branchesArg * 2;
  let mainTipMark = commitsArg;
  let mergeCount = 0;
  for (let index = 0; index < branchesArg; index += 1) {
    const forkIndex = Math.max(1, Math.floor(((index + 1) * commitsArg) / (branchesArg + 1)));
    const branchName = `perf/branch-${index}`;
    const firstMark = commitsArg + index * 2 + 1;
    const secondMark = firstMark + 1;
    for (let offset = 0; offset < 2; offset += 1) {
      const mark = firstMark + offset;
      clock += 1;
      const message = `perf branch ${index} commit ${offset}\n`;
      stream += `commit refs/heads/${branchName}\nmark :${mark}\n`;
      stream += `committer Perf Fixture <perf@example.invalid> ${baseTime + clock} +0000\n`;
      stream += `data ${Buffer.byteLength(message)}\n${message}`;
      stream += `from :${offset === 0 ? forkIndex : firstMark}\n`;
      stream += `M 100644 inline src/branch-${index}-${offset}.txt\n`;
      const content = `branch ${index} offset ${offset}\n`;
      stream += `data ${Buffer.byteLength(content)}\n${content}`;
    }
    if (index % 4 === 3) {
      mergeCount += 1;
      clock += 1;
      const mark = mergeMarkBase + mergeCount;
      const message = `perf merge branch ${index}\n`;
      stream += `commit refs/heads/main\nmark :${mark}\n`;
      stream += `committer Perf Fixture <perf@example.invalid> ${baseTime + clock} +0000\n`;
      stream += `data ${Buffer.byteLength(message)}\n${message}`;
      stream += `from :${mainTipMark}\n`;
      stream += `merge :${secondMark}\n`;
      stream += `M 100644 inline src/merge-${index}.txt\n`;
      const content = `merge branch ${index}\n`;
      stream += `data ${Buffer.byteLength(content)}\n${content}`;
      mainTipMark = mark;
    }
  }

  // **一次喂完**：分多次调用会在每次开头重新 `reset` 同一个分支，
  // 第二次起就被 git 判成非快进（"new tip does not contain..."）而失败。
  // 十万提交的流约 20MB，一次喂进去比"分批 + 事后改 ref"简单得多。
  git(['fast-import', '--quiet'], { input: stream, maxBuffer: 256 * 1024 * 1024 });
  git(['reset', '--hard', 'main']);
  git(['gc', '--quiet']);
}

if (modifiedArg > 0) {
  // 一次提交里放 N 个文件，再在**工作区**全部改一行：这正是 M1 验收里
  // "10k 变更文件"的形状（已跟踪 + 内容变化），也是 git 与我们的 stat 缓存
  // 都要走的那条路径
  let stream = 'reset refs/heads/main\ncommit refs/heads/main\nmark :1\n';
  stream += 'committer Perf Fixture <perf@example.invalid> 1600000000 +0000\n';
  const message = 'base\n';
  stream += `data ${Buffer.byteLength(message)}\n${message}`;
  for (let index = 0; index < modifiedArg; index += 1) {
    const content = `original ${index}\n`;
    stream += `M 100644 inline data/file-${index}.txt\n`;
    stream += `data ${Buffer.byteLength(content)}\n${content}`;
  }
  git(['fast-import', '--quiet'], { input: stream, maxBuffer: 256 * 1024 * 1024 });
  git(['reset', '--hard', 'main']);
  for (let index = 0; index < modifiedArg; index += 1) {
    writeFileSync(join(target, `data/file-${index}.txt`), `modified ${index}\n`);
  }
}

if (untrackedArg > 0) {
  // 一个已跟踪文件 + N 个未跟踪文件：量的是"未跟踪扫描"这条最慢的路径
  writeFileSync(join(target, 'tracked.txt'), 'tracked\n');
  git(['add', 'tracked.txt']);
  git(['commit', '-m', 'base', '--quiet']);
  mkdirSync(join(target, 'scratch'), { recursive: true });
  for (let index = 0; index < untrackedArg; index += 1) {
    writeFileSync(join(target, `scratch/file-${index}.txt`), `scratch ${index}\n`);
  }
}

const count = (() => {
  try {
    return Number(git(['rev-list', '--count', 'HEAD'], { encoding: 'utf8' }).trim());
  } catch {
    return 0;
  }
})();

process.stdout.write(
  JSON.stringify(
    { path: target, commits: count, branches: branchesArg, untracked: untrackedArg },
    null,
    0,
  ) + '\n',
);
