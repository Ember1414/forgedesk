/**
 * 钩子输出的轻量结构化（T1.8）。
 *
 * # 为什么只做模式匹配
 *
 * 红线 R1 禁止任何模型推理，因此这里只识别**已知工具的输出形态**
 * （eslint / prettier / tsc / clippy / npm / lint-staged 等），其余原样保留。
 * 用户真正需要的是"哪几行是错误"，而不是一段被重新表述过的总结。
 *
 * # 为什么阶段只能是"推断"
 *
 * git **不会**告诉调用方是哪个钩子失败的：它只把钩子自己的 stdout/stderr
 * 原样透传，退出码也不区分阶段。因此当输出里没有明确的钩子特征时，
 * 任何断言都只是猜测——界面必须写"看起来是…（根据输出推断）"。
 * 猜错的代价很具体：用户会去改一个根本没运行过的东西。
 */

/** 推断出的失败阶段。 */
export type HookPhase = 'code-check' | 'message-check' | 'dependencies' | 'unknown';

/** 一行的类别（用于着色，不影响原始文本）。 */
export type HookLineKind = 'error' | 'warning' | 'info' | 'plain';

/** 结构化后的一行。 */
export interface HookOutputLine {
  readonly text: string;
  readonly kind: HookLineKind;
}

/** 一次钩子输出的分析结果。 */
export interface HookOutputReport {
  readonly phase: HookPhase;
  /** 输出里能认出的工具名（稳定短名，界面可直接显示）。 */
  readonly tools: readonly string[];
  readonly lines: readonly HookOutputLine[];
  readonly errorCount: number;
  readonly warningCount: number;
}

/** 工具特征表：命中即认为这个工具在输出里出现过。 */
const TOOL_PATTERNS: readonly { readonly name: string; readonly pattern: RegExp }[] = [
  { name: 'eslint', pattern: /\beslint\b|no-console|@typescript-eslint/i },
  { name: 'prettier', pattern: /prettier|\[warn\]|\[error\]/i },
  { name: 'typescript', pattern: /\bTS\d{4}\b|\btsc\b/i },
  { name: 'clippy', pattern: /\bclippy\b|error\[E\d{4}\]/i },
  { name: 'lint-staged', pattern: /lint-staged/i },
  { name: 'jest', pattern: /\bjest\b/ },
  { name: 'vitest', pattern: /\bvitest\b|❯/ },
  { name: 'secrets-scan', pattern: /gitleaks|detect-secrets|secret/i },
  { name: 'npm', pattern: /npm ERR!|npm WARN/ },
];

/** 提交信息检查类输出的特征。 */
const MESSAGE_PHASE_PATTERN =
  /commit[- ]?msg|commit message|subject|message may not be empty|\b\d{2}[- ]character\b/i;

/** 依赖安装类输出的特征。 */
const DEPS_PHASE_PATTERN = /npm (ERR|WARN)!|yarn error|pnpm|cannot find module|ENOENT|EACCES/i;

/** 判断一行的类别。 */
export function classifyLine(text: string): HookLineKind {
  const trimmed = text.trim();
  if (trimmed === '') {
    return 'plain';
  }
  // git 自己的 `hint:` 不是失败原因。把它当成 error 会让"哪几行是错误"失去意义
  if (/^hint:/i.test(trimmed)) {
    return 'info';
  }
  if (
    /(^|\s)error(\s|:)/i.test(trimmed) ||
    /^\s*\d+:\d+\s+error\b/i.test(trimmed) ||
    /[✖✗]/.test(trimmed) ||
    /\berrors?\b/i.test(trimmed)
  ) {
    return 'error';
  }
  if (
    /(^|\s)warning(\s|:)/i.test(trimmed) ||
    /\[warn\]/i.test(trimmed) ||
    /^\s*\d+:\d+\s+warning\b/i.test(trimmed) ||
    /[⚠]/.test(trimmed)
  ) {
    return 'warning';
  }
  return 'plain';
}

/** 推断失败阶段。 */
function detectPhase(output: string, tools: readonly string[]): HookPhase {
  // 信息检查优先：它的输出里常常夹着 "test"/"lint" 之类的词（例如
  // "commit-msg: subject must not start with a test name"），而那不代表代码检查跑过
  if (MESSAGE_PHASE_PATTERN.test(output)) {
    return 'message-check';
  }
  if (DEPS_PHASE_PATTERN.test(output)) {
    return 'dependencies';
  }
  if (tools.length > 0) {
    return 'code-check';
  }
  return 'unknown';
}

/** 分析一次钩子输出。 */
export function analyzeHookOutput(output: string): HookOutputReport {
  const lines: HookOutputLine[] = output.split(/\r?\n/).map((text) => ({
    text,
    kind: classifyLine(text),
  }));
  const tools = TOOL_PATTERNS.filter((tool) => tool.pattern.test(output)).map((tool) => tool.name);

  return {
    phase: detectPhase(output, tools),
    tools,
    lines,
    errorCount: lines.filter((line) => line.kind === 'error').length,
    warningCount: lines.filter((line) => line.kind === 'warning').length,
  };
}
