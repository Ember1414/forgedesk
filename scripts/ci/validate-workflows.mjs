#!/usr/bin/env node
/**
 * 校验 .github 下的配置 YAML：workflows 的完整校验 + Issue 模板的语法校验。
 *
 * 为什么需要它：工作流里的错误（YAML 语法、缺 timeout、权限过宽）通常要到
 * 推送之后才在 GitHub 上暴露，一次失败就白烧配额；而 CI 的免费额度在私有阶段
 * 是稀缺资源。把这些问题变成**本地可判定**的检查，是"配额保护"的一部分。
 * Issue 模板虽然不跑 CI，但语法错误会让 GitHub 拒绝渲染表单（提交者看到的是报错页），
 * 因此至少要做语法校验。
 *
 * 检查分级：
 *   ERROR —— 直接失败（YAML 非法、缺必需字段、job 缺 timeout-minutes、shell 语法错误）
 *   WARN  —— 通过但打印（权限未声明、并发未声明、action 未固定到 commit SHA）
 *
 * 第四类检查（M7 追加）：**`run:` 里的 bash 脚本做 `bash -n` 语法校验**。
 * 理由：工作流的 shell 胶水是"本地跑不到、要到 runner 上才炸"的部分，而语法错误
 * （少一个 fi、引号不配）会在**发版当天**才暴露，且报错位置往往指向整个文件而不是那一行。
 * `bash -n` 只解析不执行，是零风险的提前暴露手段——本检查加入时，release.yml 里
 * 恰好有一处多余的 `fi` 等着它（正是加它的理由）。
 *
 * 用法：node scripts/ci/validate-workflows.mjs
 */
import { execFileSync } from 'node:child_process';
import { readdirSync, readFileSync, existsSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { parse } from 'yaml';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');
const workflowsDir = join(repoRoot, '.github', 'workflows');

/** commit SHA 固定形式：40 位十六进制 */
const SHA_PATTERN = /^[0-9a-f]{40}$/;

const errors = [];
const warnings = [];

/** 收集一个 job 里所有 `uses:` 的值。 */
function collectUses(job) {
  const steps = Array.isArray(job?.steps) ? job.steps : [];
  return steps
    .map((step) => (typeof step?.uses === 'string' ? step.uses : undefined))
    .filter((value) => value !== undefined);
}

/**
 * 找到一台**真能解析脚本**的 bash，找不到返回 null。
 *
 * 为什么不直接用 `bash`：Windows 的 PATH 上常有 `C:\Windows\System32\bash.exe`，
 * 那是 WSL 的启动器——没装发行版时它连 `--version` 都失败（本机实测如此），
 * 于是"检查被静默跳过"，而跳过会被误读成"校验通过"。因此按优先级探测：
 * `BASH` 环境变量 → Git for Windows 的两处安装路径 → PATH 上的 `bash`，
 * 并对每个候选断言它真是 GNU bash 且能正常退出。
 */
let cachedBash;
function resolveBash() {
  if (cachedBash !== undefined) {
    return cachedBash;
  }
  const candidates = [
    process.env.BASH,
    'C:\\Program Files\\Git\\bin\\bash.exe',
    'C:\\Program Files\\Git\\usr\\bin\\bash.exe',
    'bash',
  ].filter((value) => typeof value === 'string' && value !== '');

  for (const candidate of candidates) {
    try {
      const version = execFileSync(candidate, ['--version'], { encoding: 'utf8' });
      if (version.startsWith('GNU bash')) {
        cachedBash = candidate;
        return cachedBash;
      }
    } catch {
      // 候选不可用（不存在 / WSL 无发行版 / 不是 bash）：试下一个
    }
  }
  cachedBash = null;
  return cachedBash;
}

/**
 * 这个 step 的 `run:` 会被 **bash** 执行吗？
 *
 * GitHub 的默认 shell 取决于 runner：Linux/macOS 是 bash，Windows 是 pwsh。
 * 显式写了 `shell:` 就以它为准（`bash` 及 `bash -e {0}` 之类的写法都算 bash）。
 * 判断错方向的代价是"用错解释器判断语法"，所以宁可漏检（跳过）也不误检。
 */
function usesBash(job, step) {
  const shell = typeof step?.shell === 'string' ? step.shell.trim().toLowerCase() : '';
  if (shell !== '') {
    return shell === 'bash' || shell.startsWith('bash ');
  }
  const runsOn = typeof job['runs-on'] === 'string' ? job['runs-on'].toLowerCase() : '';
  return runsOn !== '' && !runsOn.includes('windows');
}

/** 对一段脚本做 `bash -n`（只解析不执行）；返回错误信息或 null。 */
function bashSyntaxError(script) {
  const bash = resolveBash();
  if (bash === null) {
    return null;
  }
  try {
    execFileSync(bash, ['-n'], { input: script, stdio: ['pipe', 'ignore', 'pipe'] });
    return null;
  } catch (error) {
    const stderr = error.stderr === undefined ? '' : String(error.stderr);
    // `bash -n` 对 stdin 的来源名不固定，这里只取最后一行有效信息
    return stderr.trim().split('\n').slice(-1)[0] ?? error.message;
  }
}

/** 判断 `uses` 是否已固定到 commit SHA。 */
function isPinnedToSha(uses) {
  const at = uses.lastIndexOf('@');
  if (at === -1) {
    return false;
  }
  return SHA_PATTERN.test(uses.slice(at + 1));
}

function validateWorkflow(fileName, text) {
  const label = `.github/workflows/${fileName}`;

  let doc;
  try {
    doc = parse(text);
  } catch (error) {
    errors.push(`${label}: YAML 解析失败 —— ${error.message}`);
    return;
  }

  if (doc === null || typeof doc !== 'object') {
    errors.push(`${label}: 顶层不是一个映射（object）`);
    return;
  }

  if (typeof doc.name !== 'string' || doc.name.trim() === '') {
    errors.push(`${label}: 缺少 name 字段`);
  }
  if (doc.on === undefined) {
    errors.push(`${label}: 缺少 on 触发条件`);
  }
  if (doc.jobs === null || typeof doc.jobs !== 'object' || Array.isArray(doc.jobs)) {
    errors.push(`${label}: 缺少 jobs 或 jobs 不是映射`);
    return;
  }

  if (doc.permissions === undefined) {
    warnings.push(`${label}: 未声明顶层 permissions —— 建议显式声明最小权限（如 contents: read）`);
  }
  if (doc.concurrency === undefined) {
    warnings.push(
      `${label}: 未声明 concurrency —— 建议加 concurrency 以取消同分支旧运行，节省配额`,
    );
  }

  for (const [jobId, job] of Object.entries(doc.jobs)) {
    if (job === null || typeof job !== 'object') {
      errors.push(`${label}: job "${jobId}" 不是映射`);
      continue;
    }

    // 配额保护：没有超时限制的 job 可能一直挂着烧额度
    if (job['timeout-minutes'] === undefined) {
      errors.push(`${label}: job "${jobId}" 缺少 timeout-minutes（防止挂死任务持续消耗配额）`);
    }

    if (job['runs-on'] === undefined) {
      errors.push(`${label}: job "${jobId}" 缺少 runs-on`);
    }

    for (const uses of collectUses(job)) {
      if (!isPinnedToSha(uses)) {
        warnings.push(`${label}: job "${jobId}" 的 action 未固定到 commit SHA —— ${uses}`);
      }
    }

    // `bash -n` 只能解析、不能执行，因此对每个 bash 步骤都做一次是安全的
    if (resolveBash() !== null) {
      const steps = Array.isArray(job.steps) ? job.steps : [];
      steps.forEach((step, index) => {
        const script = typeof step?.run === 'string' ? step.run : undefined;
        if (script === undefined || !usesBash(job, step)) {
          return;
        }
        const syntaxError = bashSyntaxError(script);
        if (syntaxError !== null) {
          errors.push(
            `${label}: job "${jobId}" 第 ${index + 1} 个 step 的 run 脚本 bash 语法错误 —— ${syntaxError}`,
          );
        }
      });
    }
  }
}

// ---------------------------------------------------------------- 执行

if (!existsSync(workflowsDir)) {
  console.error('未找到 .github/workflows 目录。');
  process.exit(1);
}

const files = readdirSync(workflowsDir).filter(
  (name) => name.endsWith('.yml') || name.endsWith('.yaml'),
);

if (files.length === 0) {
  console.error('.github/workflows 下没有任何工作流文件。');
  process.exit(1);
}

for (const fileName of files) {
  validateWorkflow(fileName, readFileSync(join(workflowsDir, fileName), 'utf8'));
}

console.log(`已校验 ${files.length} 个工作流文件：${files.join(', ')}`);
if (resolveBash() === null) {
  // 明确说出来而不是静默跳过：静默跳过会让人以为"shell 检查过了"
  console.log('注意：本机没有 bash，已跳过 run 脚本的语法校验（CI 的 ubuntu runner 上会做）。');
}
console.log('');

// Issue 模板：语法错误会让 GitHub 直接拒绝渲染表单，且本地无感，所以一并校验。
const issueTemplateDir = join(repoRoot, '.github', 'ISSUE_TEMPLATE');
let issueTemplates = 0;
if (existsSync(issueTemplateDir)) {
  for (const fileName of readdirSync(issueTemplateDir)) {
    if (!fileName.endsWith('.yml') && !fileName.endsWith('.yaml')) {
      continue;
    }
    const label = `.github/ISSUE_TEMPLATE/${fileName}`;
    try {
      const doc = parse(readFileSync(join(issueTemplateDir, fileName), 'utf8'));
      // GitHub 要求表单有 name / description / body 三件套；缺了表单会显示为空白
      if (typeof doc?.name !== 'string' || typeof doc?.description !== 'string') {
        errors.push(`${label}: 缺少 name 或 description 字段`);
      }
      if (!Array.isArray(doc?.body)) {
        errors.push(`${label}: 缺少 body 列表`);
      }
      issueTemplates += 1;
    } catch (error) {
      errors.push(`${label}: YAML 解析失败 —— ${error.message}`);
    }
  }
}

if (issueTemplates > 0) {
  console.log(`已校验 ${issueTemplates} 个 Issue 模板的语法与必需字段`);
  console.log('');
}

for (const warning of warnings) {
  console.log(`WARN  ${warning}`);
}
for (const error of errors) {
  console.error(`ERROR ${error}`);
}

console.log('');
if (errors.length > 0) {
  console.error(`工作流校验失败：${errors.length} 个错误，${warnings.length} 个警告。`);
  process.exit(1);
}
console.log(`工作流校验通过（${warnings.length} 个警告，见上）。`);
