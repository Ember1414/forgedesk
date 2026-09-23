//! Git 引擎抽象层：`GitEngine` trait 与 CLI / libgit2 双实现。
//!
//! 归属里程碑：见 docs/PLAN.md §5.6（引擎抽象）与 §5.2（模块划分）。
//!
//! M1/T1.1 先落地两个**纯能力**模块，它们是后续所有 git 操作的公共底座：
//!
//! - [`process`]：`GitProcess` 安全执行器（参数数组、固定环境、超时、取消、
//!   字节级捕获、脱敏日志）。所有 CLI 调用必须经它，不允许在别处直接
//!   `Command::new("git")`——见该模块头的理由。
//! - [`parsers`]：git 机器可读输出的解析器（porcelain v2 / numdiff / log / ls-files）。
//!   它们不做 IO，因此可以用固定样本完整覆盖边界（`tests/fixtures/`）。
//!
//! `GitEngine` trait 与双实现（`CliGitEngine` / `Libgit2Engine`）落在 T1.2。

#![forbid(unsafe_code)]

pub mod parsers;
pub mod process;

/// crate 名称，用于日志与诊断中标识来源。
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");
