#!/usr/bin/env node
/**
 * 合规自动化检查（M0 / T0.12），覆盖 docs/PLAN.md §9.5 红线中**可机器判定**的部分。
 *
 *   1. 名称检查     → R4（产品名/包名不含 Git/GitHub）
 *   2. 免责声明检查 → §9.5（README 必须声明独立性与无关联）
 *   3. 图标检查     → R2（不得使用官方 Logo/Octocat 及其变体）
 *   4. 依赖许可检查 → AGENTS §8（禁止 GPL/AGPL，libgit2 链接例外除外）
 *   5. AI 依赖检查  → R1（产品不含任何 AI/ML 推理能力）
 *   6. 竞品视觉提醒 → R3（无法自动判定，输出人工流程提醒）
 *
 * 设计取舍：
 * - 许可审计自己实现（cargo metadata + pnpm licenses list），不引入 cargo-deny：
 *   本地跑 `pnpm compliance` 不应要求每个贡献者先装 Rust 工具；
 *   cargo-deny（bans/sources/advisories）属于公开阶段的供应链加固，届时再加。
 * - LICENSE-AUDIT.md 重新生成后与已提交版本比对，不一致即失败——
 *   与 format:check 同一套"生成物必须提交"的纪律，防止审计文件悄悄过期。
 * - 每条失败都必须可操作：说明红线编号与修法。
 *
 * 用法：pnpm compliance（或 node scripts/compliance/check.mjs [--list]）
 */
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { existsSync, readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');

/** Windows 上 pnpm 是 .cmd 垫片：Node 24 出于安全（CVE-2024-27980）禁止
 *  无 shell 直接 spawn .cmd（EINVAL），必须走 shell:true。两个连带处理：
 *  1. shell 会回显命令行 → JSON 输出统一在 runJson 里从第一个 { 开始截取；
 *  2. shell + args 数组触发 DEP0190 弃用警告 → 本脚本是一次性 CLI，
 *     且参数全是写死的常量（无用户输入、无注入面），移除 warning 监听消除噪音。
 *  （cargo 是真正的 .exe，不需要 shell，也就没有这两个问题。） */
function pnpmCommand() {
  return process.platform === 'win32' ? 'pnpm.cmd' : 'pnpm';
}

function run(command, args) {
  const isCmdShim = process.platform === 'win32' && command.toLowerCase().endsWith('.cmd');
  if (isCmdShim) {
    process.removeAllListeners('warning');
  }
  return execFileSync(command, args, {
    cwd: repoRoot,
    encoding: 'utf8',
    maxBuffer: 64 * 1024 * 1024,
    ...(isCmdShim ? { shell: true } : {}),
  });
}

/** pnpm 走 shell 时输出里带命令回显行；JSON 输出统一从第一个 { 开始解析。 */
function runJson(command, args) {
  const output = run(command, args);
  const start = output.indexOf('{');
  if (start === -1) {
    throw new Error(`命令输出里没有 JSON：${command} ${args.join(' ')}`);
  }
  return JSON.parse(output.slice(start));
}

const readText = (repoPath) => readFileSync(join(repoRoot, repoPath), 'utf8');
const readJson = (repoPath) => JSON.parse(readText(repoPath));
const exists = (repoPath) => existsSync(join(repoRoot, repoPath));

/** 递归列出某仓库相对目录下的所有文件（仓库相对路径）。 */
function walk(repoPath) {
  const absolute = join(repoRoot, repoPath);
  if (!existsSync(absolute)) {
    return [];
  }
  const files = [];
  for (const entry of readdirSync(absolute)) {
    const path = join(absolute, entry);
    if (statSync(path).isDirectory()) {
      files.push(...walk(`${repoPath}/${entry}`));
    } else {
      files.push(`${repoPath}/${entry}`);
    }
  }
  return files;
}

/** GPL/AGPL 拒绝清单；负向后行断言排除 LGPL 里的 "GPL" 子串。 */
const DENIED_LICENSE_PATTERN = /(?<![A-Za-z])(?:GPL|AGPL)-\d/i;
const LINKING_EXCEPTIONS = new Set(['libgit2', 'libgit2-sys', 'git2']);

/** R1：已知 AI/ML 运行时与 SDK 的名称特征（依赖名与源码全文都扫）。 */
const AI_KEYWORDS =
  /\b(openai|anthropic|claude|ollama|llama|transformers|onnxruntime|tensorflow|pytorch|torch|gpt4all|whisper|huggingface|stable-diffusion|vllm|langchain)\b/i;

const results = [];
/** notes 是给人看的信息（不参与判定），details 才决定成败——
 *  把"允许但请人工确认"这类提示与真正的违规混在一起，会让检查永远红着，
 *  红到没人再看它（真实教训：LGPL 提示一进来，许可证检查就没法通过了）。 */
function record(check, pass, details = [], notes = []) {
  results.push({ check, pass, details, notes });
}

// ---------------------------------------------------------- 1. 名称检查（R4）

function checkNames() {
  const details = [];
  const packageJson = readJson('package.json');
  const tauri = readJson('src-tauri/tauri.conf.json');

  /**
   * 标识符允许的例外：`io.github.<用户名>.<应用名>` 是 GitHub 托管开源项目的
   * 标准 reverse-DNS 写法，这里的 "github" 指托管平台而不是我们的产品
   * （标识符由 ADR-004 定稿）。除这段前缀外，任何位置出现 git/github 都算失败。
   */
  const identifier = tauri.identifier.replace(/^io\.github\./, '');

  const fields = [
    { label: 'package.json name', value: packageJson.name },
    { label: 'tauri.conf.json productName', value: tauri.productName },
    { label: 'tauri.conf.json identifier', value: identifier },
    ...tauri.app.windows.map((window) => ({
      label: `窗口标题（${window.label}）`,
      value: window.title,
    })),
  ];

  const h1 = /^#[ \t]+(.+)$/m.exec(readText('README.md'));
  if (h1 === null) {
    details.push('README.md 缺少一级标题。');
  } else {
    fields.push({ label: 'README.md 标题', value: h1[1].trim() });
  }

  for (const { label, value } of fields) {
    if (/\b(git|github)\b/i.test(String(value))) {
      details.push(`${label} 含 "git/github"：${value}`);
    }
  }

  record('名称检查（R4）', details.length === 0, details);
}

// ---------------------------------------------------------- 2. 免责声明检查

function checkDisclaimer() {
  const details = [];
  const keywords = ['not affiliated', 'Software Freedom Conservancy', 'GitHub, Inc.', 'Tauri'];

  const targets = ['README.md'];
  if (exists('docs/PRIVACY.md')) {
    targets.push('docs/PRIVACY.md');
  }

  for (const target of targets) {
    const text = readText(target);
    for (const keyword of keywords) {
      if (!text.includes(keyword)) {
        details.push(`${target} 缺少关键词 "${keyword}"（模板见 docs/PLAN.md §9.5）。`);
      }
    }
  }

  record('免责声明检查（§9.5）', details.length === 0, details);
}

// ---------------------------------------------------------- 3. 图标检查（R2）

function checkIcons() {
  const details = [];
  const known = readJson('scripts/compliance/known-logos.json');
  const knownHashes = new Set(known.knownSha256 ?? []);
  const forbiddenTokens = known.forbiddenFilenameTokens ?? [];

  if (!exists('docs/brand/icon-source.svg')) {
    details.push('缺少 docs/brand/icon-source.svg（原创矢量真源，R2 的证据链起点）。');
  } else {
    // 先剥离注释再扫描：SVG 头部的设计说明里写着"未使用 Octocat 及任何动物形象"——
    // 这是原创性**证据**， naive 的全文匹配会把它当成品牌使用（真实教训）。
    const svg = readText('docs/brand/icon-source.svg').replace(/<!--[\s\S]*?-->/g, '');
    if (/<image/i.test(svg)) {
      details.push('icon-source.svg 内嵌位图（<image> 标签）——图标必须可追溯到矢量源。');
    }
    if (!/viewBox=/i.test(svg)) {
      details.push('icon-source.svg 缺少 viewBox（可能不是规范的矢量文件）。');
    }
    if (/octocat/i.test(svg)) {
      details.push('icon-source.svg 出现 octocat 字样。');
    }
  }

  const iconFiles = walk('src-tauri/icons');
  if (iconFiles.length === 0) {
    details.push('src-tauri/icons/ 为空：先运行 pnpm tauri icon docs/brand/icon-1024.png。');
  }

  for (const path of iconFiles) {
    const lowered = path.toLowerCase();
    for (const token of forbiddenTokens) {
      if (lowered.includes(token)) {
        details.push(`图标文件名命中受限品牌词 "${token}"：${path}`);
      }
    }

    const hash = createHash('sha256')
      .update(readFileSync(join(repoRoot, path)))
      .digest('hex');
    if (knownHashes.has(hash)) {
      details.push(`图标与已知官方 Logo 哈希一致：${path}`);
    }

    // 尺寸健全性：正常导出的图标不会小到几百字节；占位/损坏文件在这里暴露
    const size = statSync(join(repoRoot, path)).size;
    if (/\.(png|ico|icns)$/.test(lowered) && size < 512) {
      details.push(`图标文件过小（${size} 字节），疑似占位或损坏：${path}`);
    }
  }

  // bundle.icon 引用的文件必须真实存在，否则要到打 tag 时打包器才失败，太晚了
  for (const referenced of readJson('src-tauri/tauri.conf.json').bundle.icon ?? []) {
    if (!exists(`src-tauri/${referenced}`)) {
      details.push(`tauri.conf.json bundle.icon 引用的文件不存在：${referenced}`);
    }
  }

  record('图标检查（R2）', details.length === 0, details);
}

// ---------------------------------------------------------- 4. 依赖许可检查

function rustPackages() {
  // 刻意**不带** --no-deps：那个开关让 metadata 只返回 workspace 成员，
  // 过滤掉 forgedesk-* 之后 Rust 侧只剩 1 个包——约 400 个传递依赖完全没被审计，
  // GPL 检查形同虚设（OPS-6 收口期间发现的真实缺陷）。
  // --locked 保证只解析锁文件内的版本；compliance.yml 的 runner 首次会拉取索引（有网络）。
  const metadata = JSON.parse(
    execFileSync(cargoCommand(), ['metadata', '--locked', '--format-version', '1'], {
      cwd: repoRoot,
      encoding: 'utf8',
      maxBuffer: 64 * 1024 * 1024,
    }),
  );
  return metadata.packages.filter((pkg) => !pkg.name.startsWith('forgedesk-'));
}

/** 定位 cargo：Windows 上 rustup 默认装在 %USERPROFILE%\.cargo\bin，
 *  但贡献者的终端不一定都把它加进 PATH；找不到时显式回退一次，而不是让检查失败。 */
function cargoCommand() {
  try {
    execFileSync('cargo', ['--version'], { stdio: 'ignore' });
    return 'cargo';
  } catch {
    const fallback = join(
      process.env.USERPROFILE ?? process.env.HOME ?? '',
      '.cargo',
      'bin',
      'cargo.exe',
    );
    if (existsSync(fallback)) {
      return fallback;
    }
    return 'cargo'; // 都找不到时让它自然失败，报错信息里会说明原因
  }
}

/**
 * 平台专属二进制包（npm optionalDependencies 的产物）：@img/sharp-win32-x64、
 * @esbuild/linux-x64、@rollup/rollup-darwin-arm64 这一类。
 *
 * 为什么必须从审计里剔除：它们随所在平台变化，而审计文件要与已提交版本逐字节比对——
 * 不剔除的话，Windows 上生成的审计在 Linux CI 上永远"已过期"（真实事故，OPS-6 收口当周）。
 * 这样做的合理性：二进制包与父包（sharp / esbuild / rollup）**同版本发布**，
 * 许可审计以父包为准即可覆盖；其中 sharp 的 libvips 预编译库为 LGPL-3.0，
 * 随应用分发的合规处理在 M7/M8 的 NOTICE 工作中落实（不依赖本审计文件）。
 */
const PLATFORM_PACKAGE_PATTERN =
  /(^@[^/]+\/[^/]*|(?:^|\/))[^/]*\b(win32|linux|darwin|android|freebsd|sunos)-(x64|arm64|ia32|armv7|arm|universal|x86|msvc|gnu|musl)/i;

function nodePackages() {
  const byLicense = runJson(pnpmCommand(), ['licenses', 'list', '--json']);
  const packages = [];
  for (const [license, entries] of Object.entries(byLicense)) {
    for (const entry of entries) {
      if (PLATFORM_PACKAGE_PATTERN.test(entry.name)) {
        continue;
      }
      packages.push({ name: entry.name, version: entry.versions?.[0] ?? '', license });
    }
  }
  return packages;
}

function licenseViolations(packages) {
  const denied = [];
  const unknown = [];
  for (const pkg of packages) {
    if (LINKING_EXCEPTIONS.has(pkg.name)) {
      continue;
    }
    if (typeof pkg.license !== 'string' || pkg.license.trim() === '') {
      unknown.push(pkg);
      continue;
    }
    if (DENIED_LICENSE_PATTERN.test(pkg.license)) {
      denied.push(pkg);
    }
  }
  return { denied, unknown };
}

function renderAudit(rust, node, rustViolations, nodeViolations) {
  const histogram = new Map();
  for (const pkg of [...rust, ...node]) {
    const license = typeof pkg.license === 'string' ? pkg.license : '（未知）';
    histogram.set(license, (histogram.get(license) ?? 0) + 1);
  }
  const noViolations = rustViolations.denied.length === 0 && nodeViolations.denied.length === 0;
  const noUnknown = rustViolations.unknown.length === 0 && nodeViolations.unknown.length === 0;

  return [
    '# 许可证审计（自动生成，请勿手改）',
    '',
    '> 由 `pnpm compliance` 生成；与仓库内版本不一致时 CI 会失败。重新生成：`pnpm compliance`。',
    '> 内容刻意**不含日期与平台专属二进制包**（@img/sharp-*、@esbuild/* 等，与父包同版本，',
    '> 以父包审计为准）——否则本文件会随生成平台与日期漂移，Linux CI 永远对不上（真实教训）。',
    '',
    `## 汇总（Rust 依赖 ${rust.length} 个，npm 依赖 ${node.length} 个）`,
    '',
    '| 许可证 | 数量 |',
    '| --- | --- |',
    ...[...histogram.entries()].sort().map(([license, count]) => `| ${license} | ${count} |`),
    '',
    '## GPL/AGPL 违规（必须为空，libgit2 链接例外除外）',
    '',
    noViolations
      ? '无。'
      : [...rustViolations.denied, ...nodeViolations.denied]
          .map((pkg) => `- ${pkg.name}（${pkg.license}）`)
          .join('\n'),
    '',
    '## 未知许可证的依赖（人工确认）',
    '',
    noUnknown
      ? '无。'
      : [...rustViolations.unknown, ...nodeViolations.unknown]
          .map((pkg) => `- ${pkg.name}`)
          .join('\n'),
    '',
  ].join('\n');
}

function checkLicenses() {
  const details = [];
  let rust = [];
  let node = [];
  try {
    rust = rustPackages();
  } catch (error) {
    details.push(`无法获取 Rust 依赖许可（cargo metadata 失败）：${error.message.split('\n')[0]}`);
  }
  try {
    node = nodePackages();
  } catch (error) {
    details.push(
      `无法获取 npm 依赖许可（pnpm licenses list 失败）：${error.message.split('\n')[0]}`,
    );
  }

  const rustViolations = licenseViolations(rust);
  const nodeViolations = licenseViolations(node);

  for (const pkg of [...rustViolations.denied, ...nodeViolations.denied]) {
    details.push(
      `${pkg.name}（${pkg.license}）使用 GPL/AGPL——请替换（AGENTS.md §8；libgit2 链接例外除外）。`,
    );
  }

  // LGPL 不在禁令内（AGENTS §8 只禁 GPL/AGPL），但单独列出供人复核链接方式
  const notes = [];
  const lgpl = [...rust, ...node].filter((pkg) => /LGPL/i.test(pkg.license ?? ''));
  if (lgpl.length > 0) {
    notes.push(
      `[信息] 使用 LGPL 的依赖（允许，请人工确认链接方式）：${[...new Set(lgpl.map((pkg) => pkg.name))].join('、')}`,
    );
  }

  const audit = renderAudit(rust, node, rustViolations, nodeViolations);
  const auditPath = 'docs/LICENSE-AUDIT.md';

  // 与已提交版本比对；不一致时**就地更新文件**并失败——
  // 贡献者跑一次就能拿到新审计，提交它即可让检查转绿（与 format --write 的体验对齐）
  const committed = exists(auditPath) ? readText(auditPath) : undefined;
  if (committed === undefined) {
    details.push(`${auditPath} 不存在——已生成本次审计，请检查后提交。`);
    writeFileSync(join(repoRoot, 'docs', 'LICENSE-AUDIT.md'), audit, 'utf8');
  } else if (committed !== audit) {
    details.push(`${auditPath} 已过期（依赖或版本变化）——已重新生成，请检查差异后提交。`);
    writeFileSync(join(repoRoot, 'docs', 'LICENSE-AUDIT.md'), audit, 'utf8');
  }

  record('依赖许可检查（AGENTS §8）', details.length === 0, details, notes);
}

// ---------------------------------------------------------- 5. AI 依赖检查（R1）

function checkAiDependencies() {
  const details = [];

  // 5.1 声明面：package.json 的依赖名
  const packageJson = readJson('package.json');
  for (const name of [
    ...Object.keys(packageJson.dependencies ?? {}),
    ...Object.keys(packageJson.devDependencies ?? {}),
  ]) {
    if (AI_KEYWORDS.test(name)) {
      details.push(`package.json 依赖命中 AI 关键词：${name}（红线 R1）。`);
    }
  }

  // 5.2 声明面：所有 Cargo.toml 的依赖键
  const manifests = [
    'Cargo.toml',
    'src-tauri/Cargo.toml',
    ...walk('crates').filter((path) => path.endsWith('Cargo.toml')),
  ];
  for (const manifest of manifests) {
    if (!exists(manifest)) {
      continue;
    }
    for (const line of readText(manifest).split(/\r?\n/)) {
      const dependency = /^\s*([A-Za-z0-9_-]+)\s*=/.exec(line);
      if (dependency !== null && AI_KEYWORDS.test(dependency[1])) {
        details.push(`${manifest} 依赖命中 AI 关键词：${dependency[1]}（红线 R1）。`);
      }
    }
  }

  // 5.3 传递面：Cargo.lock 锁定的全部包名
  if (exists('Cargo.lock')) {
    for (const match of readText('Cargo.lock').matchAll(/^name = "(.+)"$/gm)) {
      if (AI_KEYWORDS.test(match[1])) {
        details.push(`Cargo.lock 中的包命中 AI 关键词：${match[1]}（红线 R1）。`);
      }
    }
  }

  // 5.4 源码面：业务代码全文（依赖目录不在扫描范围）
  const sourceFiles = [
    ...walk('src').filter((path) => /\.(ts|tsx)$/.test(path)),
    ...walk('crates').filter((path) => path.endsWith('.rs')),
    ...walk('src-tauri/src'),
  ];
  for (const path of sourceFiles) {
    const lines = readText(path).split(/\r?\n/);
    lines.forEach((line, index) => {
      if (AI_KEYWORDS.test(line)) {
        details.push(
          `${path}:${index + 1} 源码命中 AI 关键词（红线 R1）：${line.trim().slice(0, 80)}`,
        );
      }
    });
  }

  record('AI 依赖检查（R1）', details.length === 0, details);
}

// ---------------------------------------------------------- 6. 竞品视觉提醒（R3）

function checkCompetitorReview() {
  const details = [];
  if (!exists('.github/COMPETITOR-REVIEW.md')) {
    details.push('缺少 .github/COMPETITOR-REVIEW.md（R3 的人工评审流程说明）。');
  }
  record(
    '竞品视觉检查（R3，人工）',
    details.length === 0,
    details.length > 0
      ? details
      : [
          '提醒：视觉原创性无法自动判定。影响主界面布局的 PR 请按 .github/COMPETITOR-REVIEW.md 完成人工并排评审并记录。',
        ],
  );
}

// ---------------------------------------------------------- 主流程

function main() {
  if (process.argv.includes('--list')) {
    console.log('合规检查项：1)名称 2)免责声明 3)图标 4)依赖许可 5)AI依赖 6)竞品视觉提醒');
    return;
  }

  checkNames();
  checkDisclaimer();
  checkIcons();
  checkLicenses();
  checkAiDependencies();
  checkCompetitorReview();

  let failed = 0;
  for (const { check, pass, details, notes } of results) {
    console.log(`[${pass ? 'PASS' : 'FAIL'}] ${check}`);
    for (const detail of details) {
      console.log(`       - ${detail}`);
    }
    for (const note of notes) {
      console.log(`       * ${note}`);
    }
    if (!pass) {
      failed += 1;
    }
  }

  // 审计文件在 checkLicenses 内部生成/比对（与 format:check 同一套纪律）

  if (failed > 0) {
    console.error('');
    console.error(`合规检查失败：${failed} 项未通过。`);
    process.exit(1);
  }
  console.log('');
  console.log('合规检查全部通过。');
}

main();
