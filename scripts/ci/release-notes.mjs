#!/usr/bin/env node
/**
 * 由 Conventional Commits 生成分类的 Release Notes（M7 / T7.3）。
 *
 * # 为什么自己生成而不是用 GitHub 的自动生成
 *
 * GitHub 的 "Generate release notes" 依赖 PR 标题与标签，而本仓库的提交历史
 * （尤其是 M0–M7 的代理提交）是**直接提交到 main 的**，没有 PR 可依赖。
 * 提交标题本身已经遵守 Conventional Commits（`docs/PLAN.md` §11.4），
 * 因此用 `git log` 分类是这里唯一可靠的信息源。
 *
 * # 分类规则
 *
 * - 破坏性变更（`type!:` 或标题含 `BREAKING CHANGE`）：单独置顶，**最醒目**；
 * - 其余按 `<type>` 落桶，桶的顺序固定（功能 → 修复 → 性能 → 重构 → 文档 → 测试 → 构建/杂项）；
 * - 认不出的标题（无 `<type>:` 前缀）落到"其他"，**不丢弃**——发布说明宁可多一条
 *   也不该把一次真实改动藏起来。
 *
 * # 确定性
 *
 * 同一段范围重复运行输出一致（按 `git log` 的既有顺序，不再排序）：
 * 发布说明要能重生成并对比，否则"说明变了"这件事无法审计。
 *
 * 用法：
 *   node scripts/ci/release-notes.mjs --version 0.7.1 [--from <rev>] [--to <rev>] [--out <文件>]
 *
 * 不传 `--from` 时使用**从第一个提交到 `--to`** 的全量历史（本仓库目前没有 tag，
 * 首次发布就属于这种情况）；正式发版后由流水线传入上一个 tag。
 */
import { execFileSync } from 'node:child_process';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { writeFileSync } from 'node:fs';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');

function parseArgs(argv) {
  const options = { version: undefined, from: undefined, to: 'HEAD', out: undefined };
  for (let index = 0; index < argv.length; index += 1) {
    const key = argv[index];
    const next = argv[index + 1];
    switch (key) {
      case '--version':
        options.version = next;
        index += 1;
        break;
      case '--from':
        options.from = next;
        index += 1;
        break;
      case '--to':
        options.to = next;
        index += 1;
        break;
      case '--out':
        options.out = next;
        index += 1;
        break;
      default:
        console.error(`未知参数：${key}`);
        process.exit(2);
    }
  }
  if (options.version === undefined) {
    console.error('缺少 --version <版本号>（用于标题）。');
    process.exit(2);
  }
  return options;
}

/** 分类桶：顺序即输出顺序。`label` 同时用作 Markdown 小节标题。 */
const SECTIONS = [
  { key: 'feat', label: '新功能', types: ['feat'] },
  { key: 'fix', label: '修复', types: ['fix', 'hotfix'] },
  { key: 'perf', label: '性能', types: ['perf'] },
  { key: 'refactor', label: '重构', types: ['refactor', 'style'] },
  { key: 'docs', label: '文档', types: ['docs'] },
  { key: 'test', label: '测试', types: ['test'] },
  { key: 'build', label: '构建与杂项', types: ['build', 'ci', 'chore', 'revert'] },
];

const CONVENTIONAL = /^(?<type>[a-z]+)(?:\((?<scope>[^)]*)\))?(?<breaking>!)?:\s*(?<subject>.+)$/;

/** 把一条提交标题分类；返回 `{ breaking, sectionKey, text }`。 */
function classify(subject) {
  const match = CONVENTIONAL.exec(subject);
  if (match === null || match.groups === undefined) {
    // 无前缀（例如 M0 阶段的 "bootstrap ForgeDesk skeleton"）：如实归入"其他"
    return { breaking: false, sectionKey: 'other', text: subject };
  }

  const { type, scope, breaking, subject: body } = match.groups;
  const normalizedType = (type ?? '').toLowerCase();
  const section = SECTIONS.find((candidate) => candidate.types.includes(normalizedType));
  const scopePrefix = scope === undefined || scope === '' ? '' : `**${scope}**: `;
  return {
    breaking: breaking === '!',
    sectionKey: section?.key ?? 'other',
    text: `${scopePrefix}${body ?? ''}`,
  };
}

const options = parseArgs(process.argv.slice(2));

const range = options.from === undefined ? options.to : `${options.from}..${options.to}`;
// `%s` 是提交标题首行；合并提交单独标注（它们的标题通常无类型前缀，落到"其他"）
const raw = execFileSync(
  'git',
  ['log', range, '--no-merges', '--pretty=format:%s', '--date=short'],
  { cwd: repoRoot, encoding: 'utf8', maxBuffer: 32 * 1024 * 1024 },
);

const subjects = raw
  .split('\n')
  .map((line) => line.trim())
  .filter((line) => line !== '');

const buckets = new Map([...SECTIONS.map((section) => [section.key, []]), ['other', []]]);
const breaking = [];

for (const subject of subjects) {
  const { breaking: isBreaking, sectionKey, text } = classify(subject);
  buckets.get(sectionKey)?.push(`- ${text}`);
  if (isBreaking) {
    breaking.push(`- ${text}`);
  }
}

const lines = [];
lines.push(`## ForgeDesk ${options.version}`);
lines.push('');

if (breaking.length > 0) {
  lines.push('### ⚠️ 破坏性变更');
  lines.push('');
  lines.push(...breaking);
  lines.push('');
}

let empty = true;
for (const section of SECTIONS) {
  const items = buckets.get(section.key) ?? [];
  if (items.length === 0) {
    continue;
  }
  empty = false;
  lines.push(`### ${section.label}`);
  lines.push('');
  lines.push(...items);
  lines.push('');
}

const other = buckets.get('other') ?? [];
if (other.length > 0) {
  empty = false;
  lines.push('### 其他');
  lines.push('');
  lines.push(...other);
  lines.push('');
}

if (empty) {
  console.error(`范围内没有提交（range=${range}），请检查 --from / --to。`);
  process.exit(1);
}

lines.push(
  `_由 \`scripts/ci/release-notes.mjs\` 从 Conventional Commits 生成（范围 ${range}，共 ${subjects.length} 条）。_`,
);
lines.push('');

const content = lines.join('\n');
if (options.out === undefined) {
  process.stdout.write(content);
} else {
  writeFileSync(options.out, content, 'utf8');
  console.error(`已写入 ${options.out}（${subjects.length} 条提交）`);
}
