#!/usr/bin/env node
/**
 * 校验「仓库自身的一致性」——主要针对那些在本地永远正常、只在 CI 上炸掉的问题。
 *
 * 为什么需要它（两起真实事故，同根）：
 *
 *   事故一：.gitignore 里为了拦截密钥目录，写了一条"任意层级的 credentials 目录"
 *   通配规则。它同时命中了源码目录 `crates/credentials/`，于是该 crate 从未被提交。
 *   本地开发一切正常（文件就在磁盘上），CI 一 checkout 就少了这个目录，
 *   cargo 立刻失败，报错却指向别处：
 *     failed to load manifest for workspace member `.../src-tauri`
 *   （因为 src-tauri → commands → services → credentials 这条依赖链断在了末端，
 *     cargo 只会报它正在加载的那个成员）。
 *
 *   事故二：未锚定的 `logs/` 规则命中了前端源码目录 `src/features/logs/`，
 *   三个日志查看组件从未提交。本地 typecheck 全绿，CI 直接报
 *     TS2307: Cannot find module '@/features/logs/LogViewerDialog'
 *   —— 和事故一完全是同一类失败：本地有、仓库里没有。
 *
 * 结论：凡是「本地有 / 仓库里没有」的文件或目录，都会变成这类难以定位的失败。
 * 因此本脚本做两件事：
 *   1. workspace 成员必须存在于磁盘、必须被 git 跟踪、必须不被 .gitignore 忽略；
 *   2. **源码树（src/ crates/ scripts/ docs/ .github/）内不允许存在任何被忽略的文件** ——
 *      这一条把"任何 .gitignore 规则吞掉源码"都变成推送前的错误，不再依赖记住某个具体目录名。
 *
 * 检查分级：
 *   ERROR —— 退出码 1（缺失目录、被忽略、未被跟踪、成员清单与磁盘不一致、源码树被忽略）
 *
 * 用法：node scripts/ci/validate-repo.mjs
 */
import { execFileSync } from 'node:child_process';
import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');
const errors = [];

/** 执行 git 命令并取回 stdout；退出码非 0 时返回 undefined 而不抛异常（stderr 静默）。 */
function tryGit(args) {
  try {
    return execFileSync('git', args, {
      cwd: repoRoot,
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'ignore'],
    }).trim();
  } catch {
    return undefined;
  }
}

/**
 * 判断路径是否真的被 .gitignore 忽略。
 *
 * 用 `--quiet --no-index` 而不是解析 `-v` 的输出：
 *   · `--quiet` 直接以退出码作答（0 = 忽略，1 = 不忽略）；
 *   · `--no-index` 让判断只看规则、不看索引状态，否则「已跟踪的文件」会被规则外的因素干扰；
 *   · `-v` 的输出会同时打印否定规则（以 ! 开头，表示"重新纳入跟踪"），
 *     靠文本判断容易把"被例外救回来"误判成"被忽略"。
 */
function isIgnored(repoPath) {
  try {
    execFileSync('git', ['check-ignore', '--quiet', '--no-index', '--', repoPath], {
      cwd: repoRoot,
      stdio: 'ignore',
    });
    return true;
  } catch {
    return false;
  }
}

/** 命中某条 .gitignore 规则时的可读描述（仅用于报错信息）。 */
function ignoredByRule(repoPath) {
  return tryGit(['check-ignore', '-v', '--no-index', '--', repoPath]) ?? '';
}

/** 判断路径是否已被 git 跟踪。 */
function isTracked(repoPath) {
  return tryGit(['ls-files', '--error-unmatch', '--', repoPath]) !== undefined;
}

// ---------------------------------------------------------------- 解析 Cargo 清单

const cargoTomlPath = join(repoRoot, 'Cargo.toml');
if (!existsSync(cargoTomlPath)) {
  console.error('未找到根 Cargo.toml。');
  process.exit(1);
}
const cargoToml = readFileSync(cargoTomlPath, 'utf8');

/** 取出 `members = [ ... ]` 中的条目（workspace 成员清单）。 */
function parseWorkspaceMembers(text) {
  const match = /^[ \t]*members[ \t]*=[ \t]*\[([^\]]*)\]/m.exec(text);
  if (match === null) {
    return undefined;
  }
  return [...match[1].matchAll(/"([^"]+)"/g)].map((item) => item[1]);
}

/** 取出 `[workspace.dependencies]` 中所有 `path = "..."` 的内部 crate 路径。 */
function parseWorkspacePathDeps(text) {
  const tableStart = text.indexOf('[workspace.dependencies]');
  if (tableStart === -1) {
    return [];
  }
  const rest = text.slice(tableStart + '[workspace.dependencies]'.length);
  const nextTable = rest.search(/^\[/m);
  const table = nextTable === -1 ? rest : rest.slice(0, nextTable);
  return [...table.matchAll(/path[ \t]*=[ \t]*"([^"]+)"/g)].map((item) => item[1]);
}

const members = parseWorkspaceMembers(cargoToml);
if (members === undefined) {
  errors.push('Cargo.toml: 未找到 [workspace] 的 members 清单，无法校验。');
}

// ---------------------------------------------------------------- 检查 workspace 成员

if (members !== undefined) {
  for (const member of members) {
    const manifest = `${member}/Cargo.toml`;

    if (!existsSync(join(repoRoot, member))) {
      errors.push(
        `Cargo.toml: workspace 成员 "${member}" 在磁盘上不存在（CI 上 cargo 会直接失败）。`,
      );
      continue;
    }
    if (!existsSync(join(repoRoot, manifest))) {
      errors.push(`Cargo.toml: workspace 成员 "${member}" 缺少 Cargo.toml。`);
      continue;
    }

    if (isIgnored(manifest)) {
      errors.push(
        `Cargo.toml: workspace 成员 "${member}" 被 .gitignore 忽略 —— 该目录不会进入仓库，` +
          `CI 上 cargo 将报 "failed to load manifest for workspace member"。` +
          `命中规则：${ignoredByRule(manifest)}`,
      );
      continue;
    }

    if (!isTracked(manifest)) {
      errors.push(
        `Cargo.toml: workspace 成员 "${member}" 未被 git 跟踪 —— 请先 git add 并提交，` +
          `否则 CI 上 cargo 会因缺少该成员而失败。`,
      );
    }
  }
}

// ---------------------------------------------------------------- 检查内部 path 依赖

for (const depPath of parseWorkspacePathDeps(cargoToml)) {
  const manifest = `${depPath}/Cargo.toml`;

  if (!existsSync(join(repoRoot, manifest))) {
    errors.push(`Cargo.toml: [workspace.dependencies] 引用的路径 "${depPath}" 缺少 Cargo.toml。`);
    continue;
  }
  if (isIgnored(manifest)) {
    errors.push(
      `Cargo.toml: [workspace.dependencies] 引用的路径 "${depPath}" 被 .gitignore 忽略。` +
        `命中规则：${ignoredByRule(manifest)}`,
    );
    continue;
  }
  if (!isTracked(manifest)) {
    errors.push(`Cargo.toml: [workspace.dependencies] 引用的路径 "${depPath}" 未被 git 跟踪。`);
  }
}

// ---------------------------------------------------------------- 检查磁盘与成员清单是否同步

const cratesDir = join(repoRoot, 'crates');
if (existsSync(cratesDir)) {
  const onDisk = readdirSync(cratesDir).filter((entry) =>
    statSync(join(cratesDir, entry)).isDirectory(),
  );

  for (const entry of onDisk) {
    const relative = `crates/${entry}`;
    if (!existsSync(join(cratesDir, entry, 'Cargo.toml'))) {
      errors.push(`crates/${entry}: 目录存在但没有 Cargo.toml，不属于合法 crate。`);
      continue;
    }
    if (members !== undefined && !members.includes(relative)) {
      errors.push(`crates/${entry}: 磁盘上存在该 crate，但没有登记到根 Cargo.toml 的 members。`);
    }
  }

  for (const member of members ?? []) {
    if (member.startsWith('crates/') && !onDisk.includes(member.slice('crates/'.length))) {
      errors.push(`Cargo.toml: members 里的 "${member}" 在 crates/ 下不存在。`);
    }
  }
}

// ---------------------------------------------------------------- 检查源码树内是否有被忽略的文件

/**
 * 受保护的源码树（顶层目录）。
 *
 * 判断依据：这些目录里的文件**只可能属于仓库内容**；运行时输出（dist、target、
 * coverage、日志）要么不会出现在这里，要么本来就该锚定到仓库根。
 * 出现在这份清单之外的被忽略文件（node_modules、target…）是正常情况。
 */
const PROTECTED_SOURCE_ROOTS = ['src', 'crates', 'scripts', 'docs', '.github'];

/**
 * 列出受保护源码树内「被 .gitignore 忽略且未被跟踪」的文件。
 *
 * 两个实现细节都有真实教训：
 *
 * 1. **把 pathspec 限定在受保护目录内**（`-- src crates …`）：
 *    不限定的话 git 会列出全部 node_modules/target（MB 级），
 *    一旦超过 execFileSync 默认 1MB 缓冲就抛 ENOBUFS——而异常被 tryGit 静默吞掉后
 *    本检查会**返回空数组并报"通过"**，恰好在该生效的时候失效。
 *    限定 pathspec 后输出只有几十行，既快又稳。
 * 2. **失败必须响亮**：这里故意不用 tryGit（它吞异常返回 undefined）。
 *    一致性门禁绝不能 fail-open——git 挂了就该让脚本失败，而不是装作没问题。
 */
function listIgnoredFiles() {
  const output = execFileSync(
    'git',
    [
      'ls-files',
      '--others',
      '--ignored',
      '--exclude-standard',
      '-z',
      '--',
      ...PROTECTED_SOURCE_ROOTS,
    ],
    {
      cwd: repoRoot,
      encoding: 'utf8',
      maxBuffer: 64 * 1024 * 1024,
      stdio: ['ignore', 'pipe', 'ignore'],
    },
  );
  // -z 用 NUL 分隔，文件名不可能包含 NUL，因此按 NUL 切分是安全的
  return output.split('\0').filter(Boolean);
}

const ignoredInSourceTrees = listIgnoredFiles().filter((path) =>
  PROTECTED_SOURCE_ROOTS.includes(path.split('/')[0]),
);

for (const path of ignoredInSourceTrees) {
  errors.push(
    `源码树内的文件被 .gitignore 忽略，它永远不会进入仓库（本地却存在，CI 会因缺少它而失败）：${path}` +
      ` —— 命中规则：${ignoredByRule(path)}。` +
      `修复方式：把该规则锚定到仓库根（加前导 /），或为该目录添加例外（!规则）。`,
  );
}

// ---------------------------------------------------------------- 输出

for (const error of errors) {
  console.error(`ERROR ${error}`);
}

if (errors.length > 0) {
  console.error('');
  console.error(`仓库一致性校验失败：${errors.length} 个错误。`);
  process.exit(1);
}

const memberCount = members?.length ?? 0;
console.log(
  `仓库一致性校验通过：${memberCount} 个 workspace 成员全部存在、未被 .gitignore 忽略、已被 git 跟踪；` +
    `源码树（${PROTECTED_SOURCE_ROOTS.join(' ')}）内没有被忽略的文件。`,
);
