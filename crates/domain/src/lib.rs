//! 纯领域逻辑层：领域模型、状态机、错误类型。禁止任何 IO 依赖。
//!
//! 归属里程碑：见 docs/PLAN.md §5.2 的模块划分。
//!
//! # 分层约束（重要）
//!
//! 本 crate 的第三方依赖被刻意限制为 `serde` / `serde_json` / `thiserror` /
//! `time` / `uuid`，**禁止**引入任何 IO 或运行时依赖：
//!
//! - `git2`（libgit2 绑定）
//! - `rusqlite`（本地数据库）
//! - `reqwest` / `tokio`（网络与异步运行时）
//! - 任何直接使用 `std::process` 的代码
//!
//! 这条约束由 `tests/layering.rs` 中的自动化断言保护，不是口头约定。
//! 原因：领域逻辑必须能在无环境、无网络、无文件系统的条件下被单元测试，
//! 这是本项目"可验证性优先"的基础。

#![forbid(unsafe_code)]

pub mod error;

pub use error::{AppError, AppResult, ErrorCode, FixAction};
