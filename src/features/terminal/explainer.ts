/**
 * git 命令解释器（T5.4）：本地知识库匹配。
 *
 * # 数据与红线
 *
 * 知识库是 `assets/git-explains.yaml`（构建期以 `?raw` 打包进产物），
 * 运行期**零网络调用**——这由 explainer.test.ts 的 grep 断言盯着，
 * 也是红线 R1（无 AI 推理：解释来自本地规则，不是任何模型）。
 *
 * # 匹配规则（与后端 danger 识别器同一套"轻量词法"）
 *
 * 1. 找到 git 程序 token（`git` / `git.exe` / 带路径）；
 * 2. 跳过全局选项（`-C <path>`、`-c <k=v>` 等吃参数的选项）；
 * 3. 第一个非选项 token = 子命令，在知识库里查条目；
 * 4. 其后的参数按序匹配该条目的 subcommands（子命令名或关键参数）。
 * 5. 匹配不到返回 null——宁可少解释，不可乱解释。
 */
import { parse } from 'yaml';

import rawKnowledgeBase from '../../../assets/git-explains.yaml?raw';

/** 风险等级（与知识库 YAML 的 risk 字段一致）。 */
export type ExplainRisk = 'safe' | 'caution' | 'dangerous';

/** 一条解释条目（命令级或子命令/参数级）。 */
export interface ExplainEntry {
  /** 子命令名（命令级条目为 null）。 */
  readonly name: string | null;
  readonly risk: ExplainRisk;
  readonly summary: string;
  /** 官方文档链接（经系统浏览器打开，前端会再做协议白名单校验）。 */
  readonly docsUrl?: string;
  /** 典型用法（"插入到当前行"按钮的内容）。 */
  readonly example?: string;
  /** 等价图形入口的仓库内路由段（如 `/history`；缺省 = 无对应入口）。 */
  readonly equivalentUiRoute?: string;
}

/** 一次解释的结果。 */
export interface GitExplain {
  /** 命中的 git 子命令名（如 `reset`）。 */
  readonly commandName: string;
  /** 命令级条目。 */
  readonly command: ExplainEntry;
  /** 命中的子命令 / 关键参数条目（无匹配时为 null）。 */
  readonly sub: ExplainEntry | null;
}

interface RawSubEntry {
  readonly name?: string;
  readonly risk?: string;
  readonly summary?: string;
  readonly docs_url?: string;
  readonly example?: string;
  readonly equivalent_ui_route?: string;
}

interface RawCommandEntry {
  readonly command?: string;
  readonly category?: string;
  readonly risk?: string;
  readonly summary?: string;
  readonly docs_url?: string;
  readonly example?: string;
  readonly equivalent_ui_route?: string;
  readonly subcommands?: readonly RawSubEntry[];
}

interface RawKnowledgeBase {
  readonly version?: number;
  readonly language?: string;
  readonly commands?: readonly RawCommandEntry[];
}

/** 解析结果缓存（模块级：知识库构建期定死，进程内只解析一次）。 */
const parsed = parse(rawKnowledgeBase) as RawKnowledgeBase;

/** 分类清单（字典页的筛选项；保持 YAML 中出现顺序）。 */
export const EXPLAIN_CATEGORIES: readonly string[] = [
  ...new Set((parsed.commands ?? []).map((entry) => entry.category ?? '')),
].filter((category) => category !== '');

function toEntry(raw: RawCommandEntry): ExplainEntry {
  return {
    name: null,
    risk: (raw.risk as ExplainRisk) ?? 'safe',
    summary: raw.summary ?? '',
    ...(raw.docs_url === undefined ? {} : { docsUrl: raw.docs_url }),
    ...(raw.example === undefined ? {} : { example: raw.example }),
    ...(raw.equivalent_ui_route === undefined
      ? {}
      : { equivalentUiRoute: raw.equivalent_ui_route }),
  };
}

function toSubEntry(raw: RawSubEntry): ExplainEntry {
  return {
    name: raw.name ?? '',
    risk: (raw.risk as ExplainRisk) ?? 'safe',
    summary: raw.summary ?? '',
    ...(raw.docs_url === undefined ? {} : { docsUrl: raw.docs_url }),
    ...(raw.example === undefined ? {} : { example: raw.example }),
    ...(raw.equivalent_ui_route === undefined
      ? {}
      : { equivalentUiRoute: raw.equivalent_ui_route }),
  };
}

/** 知识库全部命令条目（字典页数据源）。 */
export const EXPLAIN_COMMANDS: readonly ExplainEntry[] = (parsed.commands ?? []).map(toEntry);

const COMMAND_INDEX = new Map<string, { raw: RawCommandEntry; entry: ExplainEntry }>();
for (const raw of parsed.commands ?? []) {
  if (raw.command !== undefined) {
    COMMAND_INDEX.set(raw.command, { raw, entry: toEntry(raw) });
  }
}

/** 分词：空白分隔，成对引号内的空白保留（与后端 danger 识别器同一口径）。 */
export function tokenizeCommand(line: string): string[] {
  const tokens: string[] = [];
  let current = '';
  let inSingle = false;
  let inDouble = false;
  for (const char of line.trim()) {
    if (char === "'" && !inDouble) {
      inSingle = !inSingle;
    } else if (char === '"' && !inSingle) {
      inDouble = !inDouble;
    } else if (char === ' ' && !inSingle && !inDouble) {
      if (current !== '') {
        tokens.push(current);
        current = '';
      }
    } else {
      current += char;
    }
  }
  if (current !== '') {
    tokens.push(current);
  }
  return tokens;
}

function isGitProgram(token: string): boolean {
  const file = token.split(/[\\/]/).pop() ?? token;
  const lowered = file.toLowerCase();
  return lowered === 'git' || lowered === 'git.exe';
}

/** 吃一个参数的全局选项（`-C <path>` / `-c <k=v>` 等）。 */
const GLOBAL_FLAGS_WITH_VALUE = new Set([
  '-C',
  '--exec-path',
  '--namespace',
  '-c',
  '--git-dir',
  '--work-tree',
]);

/** 把一行键入解释成知识库条目；不是 git 命令或没有条目时返回 null。 */
export function explainGitCommand(line: string): GitExplain | null {
  const tokens = tokenizeCommand(line);
  let index = 0;
  while (index < tokens.length) {
    const token: string | undefined = tokens[index];
    if (token === undefined) {
      break;
    }
    if (isGitProgram(token)) {
      index += 1;
      break;
    }
    if (token.startsWith('-')) {
      index += 1;
      continue;
    }
    return null;
  }
  if (index >= tokens.length) {
    return null;
  }

  // 跳过全局选项
  while (index < tokens.length) {
    const token: string | undefined = tokens[index];
    if (token === undefined || !token.startsWith('-')) {
      break;
    }
    if (GLOBAL_FLAGS_WITH_VALUE.has(token)) {
      index += 2;
      continue;
    }
    index += 1;
  }

  const subcommand = tokens[index];
  if (subcommand === undefined || subcommand.startsWith('-')) {
    return null;
  }
  const subcommandName: string = subcommand;
  const hit = COMMAND_INDEX.get(subcommandName);
  if (!hit) {
    return null;
  }
  index += 1;

  // 子命令 / 关键参数：按出现顺序，找到第一个命中的条目
  const rawSubs = hit.raw.subcommands ?? [];
  let sub: ExplainEntry | null = null;
  for (const token of tokens.slice(index)) {
    const matched = rawSubs.find((raw) => raw.name === token);
    if (matched) {
      sub = toSubEntry(matched);
      break;
    }
  }

  return { commandName: subcommandName, command: hit.entry, sub };
}

/** 字典页条目：命令级条目 + 其全部子级。 */
export interface DictionaryEntry {
  readonly category: string;
  readonly command: ExplainEntry;
  /** 子命令 / 关键参数条目（按知识库顺序）。 */
  readonly subs: readonly ExplainEntry[];
}

/** 字典页数据源（知识库顺序）。 */
export const EXPLAIN_DICTIONARY: readonly DictionaryEntry[] = (parsed.commands ?? []).map(
  (raw) => ({
    category: raw.category ?? '',
    command: toEntry(raw),
    subs: (raw.subcommands ?? []).map(toSubEntry),
  }),
);

/** 知识库条目总数（字典页与测试用）。 */
export const EXPLAIN_ENTRY_COUNT: number =
  EXPLAIN_COMMANDS.length +
  (parsed.commands ?? []).reduce((sum, raw) => sum + (raw.subcommands?.length ?? 0), 0);
