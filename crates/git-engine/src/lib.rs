//! Git 引擎抽象层：`GitEngine` trait 与 CLI / libgit2 双实现。
//!
//! 归属里程碑：见 docs/PLAN.md §5.6（引擎抽象）与 §5.2（模块划分）。
//!
//! 模块分层（自下而上）：
//!
//! - [`process`]：`GitProcess` 安全执行器（参数数组、固定环境、超时、取消、
//!   字节级捕获、脱敏日志）。所有 CLI 调用必须经它，不允许在别处直接
//!   `Command::new("git")`——见该模块头的理由。
//! - [`parsers`]：git 机器可读输出的解析器（porcelain v2 / numstat / log / ls-files）。
//!   它们不做 IO，因此可以用固定样本完整覆盖边界（`tests/fixtures/`）。
//! - [`engine`]：`GitEngine` trait 与两套实现，以及参数构造、进度解析与同步桥。
//! - [`probe`]：对 `.git` 目录与工作区的只读文件系统探测（如"是否使用 LFS"），
//!   用于补上 git 命令行没有便宜问法的那些事实。
//!
//! 谁来做哪一半由 `services` 层决定：读走 libgit2（无进程开销、可高频调用），
//! 写走系统 git CLI（完整复刻用户环境：hooks、attributes、签名、filter）。

#![forbid(unsafe_code)]

pub mod engine;
pub mod engines;
pub mod parsers;
pub mod probe;
pub mod process;

/// crate 名称，用于日志与诊断中标识来源。
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");
