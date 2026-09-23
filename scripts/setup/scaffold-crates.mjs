#!/usr/bin/env node
/**
 * 生成 Rust workspace 的 crate 骨架（M0 / T0.2）。
 *
 * 为什么用脚本而不是逐个手写：12 个 crate 的 Cargo.toml / lib.rs 高度同构，
 * 把它们固化成可重复执行的生成器，既避免手抄出错，也让"分层结构"这件事本身可审计。
 *
 * 特性：
 *   - 幂等：已存在且非空的文件不会被覆盖（除非传 --force）
 *   - 严格分层：每个 crate 的 Cargo.toml 只声明它所在层允许的依赖
 *   - 自动校验 domain 层未引入 IO 依赖（见 scripts/setup/check-domain-purity.mjs）
 *
 * 用法：
 *   node scripts/setup/scaffold-crates.mjs
 *   node scripts/setup/scaffold-crates.mjs --force   # 覆盖已有文件
 */
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');
const force = process.argv.includes('--force');

/**
 * 每个 crate 的定义。
 * deps: 使用 workspace 继承写法 { workspace = true }，
 *       并把允许的第三方依赖列出来，强制"层只能依赖自己允许的东西"。
 */
const CRATES = [
  {
    dir: 'domain',
    name: 'forgedesk-domain',
    doc: '纯领域逻辑层：领域模型、状态机、错误类型。禁止任何 IO 依赖。',
    // serde_json 是纯数据处理（序列化错误与 DTO），不属于 IO，domain 允许使用；
    // 这条与 crates/domain/tests/layering.rs 的期望集合必须一致。
    deps: ['serde', 'serde_json', 'thiserror', 'time', 'uuid'],
  },
  {
    dir: 'git-engine',
    name: 'forgedesk-git-engine',
    doc: 'Git 引擎抽象层：GitEngine trait 与 CLI / libgit2 双实现。',
    deps: ['serde', 'thiserror', 'tracing', 'tokio', 'tokio-util'],
    internal: ['forgedesk-domain'],
  },
  {
    dir: 'storage',
    name: 'forgedesk-storage',
    doc: '本地存储层：SQLite 仓储、迁移与查询。',
    // rusqlite：本地 SQLite（bundled，避免依赖系统库）；serde_json：设置值以 JSON 字符串存储
    deps: ['rusqlite', 'serde', 'serde_json', 'thiserror', 'tracing'],
    internal: ['forgedesk-domain'],
  },
  {
    dir: 'provider',
    name: 'forgedesk-provider',
    doc: '托管平台适配层：HostProvider trait 与 GitHub / GitLab / Gitea 实现。',
    deps: ['serde', 'thiserror', 'tracing', 'tokio'],
    internal: ['forgedesk-domain'],
  },
  {
    dir: 'snapshot',
    name: 'forgedesk-snapshot',
    doc: '操作快照与回滚：破坏性操作前的状态打点与一致性校验。',
    deps: ['serde', 'thiserror', 'tracing'],
    internal: ['forgedesk-domain', 'forgedesk-git-engine'],
  },
  {
    dir: 'diagnostics',
    name: 'forgedesk-diagnostics',
    doc: '错误诊断引擎：把 git 与网络的原始错误映射为人话原因与可执行的修复动作。',
    // tracing / tracing-subscriber：脱敏层挂在日志格式化层上（T0.6），
    // 文件日志用 JSON 渲染器所以需要 json feature（见根 Cargo.toml）
    deps: ['serde', 'thiserror', 'tracing', 'tracing-subscriber'],
    // 仅测试：断言脱敏后的 JSON 日志行仍然合法
    devDeps: ['serde_json'],
    internal: ['forgedesk-domain'],
  },
  {
    dir: 'credentials',
    name: 'forgedesk-credentials',
    doc: '凭据管理：基于系统 keyring 的安全存储（Windows Credential Manager / macOS Keychain / libsecret）。',
    deps: ['serde', 'thiserror', 'secrecy'],
    internal: ['forgedesk-domain'],
  },
  {
    dir: 'jobs',
    name: 'forgedesk-jobs',
    doc: '长任务系统：任务注册、进度广播与取消令牌。',
    deps: ['serde', 'thiserror', 'tracing', 'tokio', 'tokio-util', 'uuid'],
    internal: ['forgedesk-domain'],
  },
  {
    dir: 'platform',
    name: 'forgedesk-platform',
    doc: '平台适配层：凭据库、shell 解析、路径规范化、日志文件、文件监听、系统通知、系统集成。',
    // tracing-subscriber / tracing-appender / time：日志落盘、轮转与末尾读取（T0.8）
    // forgedesk-diagnostics：所有出境的日志与日志行必须脱敏（红线 R8）
    deps: [
      'serde',
      'serde_json',
      'thiserror',
      'time',
      'tracing',
      'tracing-appender',
      'tracing-subscriber',
      'tokio',
    ],
    internal: ['forgedesk-diagnostics', 'forgedesk-domain'],
  },
  {
    dir: 'services',
    name: 'forgedesk-services',
    doc: '用例编排层：把领域逻辑与基础设施组合成完整业务动作（打开仓库、提交、同步、冲突解决…）。',
    deps: ['serde', 'thiserror', 'tracing', 'tokio', 'tokio-util', 'uuid'],
    internal: [
      'forgedesk-domain',
      'forgedesk-git-engine',
      'forgedesk-storage',
      'forgedesk-provider',
      'forgedesk-snapshot',
      'forgedesk-diagnostics',
      'forgedesk-credentials',
      'forgedesk-jobs',
      'forgedesk-platform',
    ],
  },
  {
    dir: 'commands',
    name: 'forgedesk-commands',
    doc: 'Tauri IPC 命令层：参数校验、能力等级校验、审计与 DTO 转换。',
    // anyhow：统一错误转换层的输入类型（把 anyhow/thiserror 错误映射为 AppError）
    // tauri：命令宏与运行时
    deps: ['anyhow', 'serde', 'serde_json', 'tauri', 'thiserror', 'tracing'],
    internal: [
      'forgedesk-domain',
      'forgedesk-diagnostics',
      'forgedesk-platform',
      'forgedesk-services',
      'forgedesk-storage',
    ],
  },
  {
    dir: 'plugin-host',
    name: 'forgedesk-plugin-host',
    doc: '插件宿主：清单解析、权限校验、WASI 沙箱运行与扩展点注册。',
    deps: ['serde', 'serde_json', 'thiserror', 'tracing'],
    internal: ['forgedesk-domain'],
  },
];

const created = [];
const skipped = [];

function writeIfAllowed(path, content) {
  if (existsSync(path) && !force) {
    const existing = readFileSync(path, 'utf8');
    if (existing.trim().length > 0) {
      skipped.push(path);
      return;
    }
  }
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, content, 'utf8');
  created.push(path);
}

function cargoToml(crate) {
  const lines = [];
  lines.push('# 本文件由 scripts/setup/scaffold-crates.mjs 生成。');
  lines.push('# 修改结构请改生成器后重新执行，避免手工编辑与生成结果不一致。');
  lines.push('');
  lines.push('[package]');
  lines.push(`name = "${crate.name}"`);
  lines.push('version.workspace = true');
  lines.push('edition.workspace = true');
  lines.push('rust-version.workspace = true');
  lines.push('license.workspace = true');
  lines.push('repository.workspace = true');
  lines.push('authors.workspace = true');
  lines.push('');
  lines.push('[dependencies]');
  for (const dep of crate.internal ?? []) {
    lines.push(`${dep}.workspace = true`);
  }
  for (const dep of crate.deps ?? []) {
    lines.push(`${dep}.workspace = true`);
  }
  if ((crate.devDeps ?? []).length > 0) {
    lines.push('');
    lines.push('[dev-dependencies]');
    lines.push('# 仅测试使用，不进入生产依赖图');
    for (const dep of crate.devDeps) {
      lines.push(`${dep}.workspace = true`);
    }
  }
  lines.push('');
  lines.push('[lints]');
  lines.push('workspace = true');
  lines.push('');
  return lines.join('\n');
}

function libRs(crate) {
  return [
    `//! ${crate.doc}`,
    '//!',
    '//! 归属里程碑：见 docs/PLAN.md 的模块划分（§5.2）与对应任务。',
    '//! 本 crate 尚未实现具体逻辑，仅在 M0/T0.2 阶段建立分层骨架。',
    '',
    '#![forbid(unsafe_code)]',
    '',
    '/// crate 名称，用于日志与诊断中标识来源。',
    'pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");',
    '',
  ].join('\n');
}

for (const crate of CRATES) {
  const crateDir = join(repoRoot, 'crates', crate.dir);
  writeIfAllowed(join(crateDir, 'Cargo.toml'), cargoToml(crate));
  writeIfAllowed(join(crateDir, 'src', 'lib.rs'), libRs(crate));
}

console.log(`已创建 ${created.length} 个文件，跳过 ${skipped.length} 个已存在文件。`);
if (skipped.length > 0) {
  console.log('跳过的文件（已存在且非空；如需覆盖请加 --force）：');
  for (const path of skipped) {
    console.log(`  ${path.replace(repoRoot, '.')}`);
  }
}
