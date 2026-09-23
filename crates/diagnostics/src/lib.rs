//! 错误诊断引擎：把 git / 网络的原始错误映射为人话原因与可执行修复动作。
//!
//! 归属里程碑：见 docs/PLAN.md 的模块划分（§5.2）。
//!
//! M0（T0.6）先落地其中最基础、且**必须最早**具备的能力——脱敏：
//!
//! - [`sanitize`]：日志与错误详情的脱敏规则（红线 R8 的实现）。
//! - [`tracing_format::SanitizedFormat`]：把脱敏接到 tracing 的输出层，
//!   使"任何日志出境前都会被过筛"成为结构性保证，而不是靠自觉。
//!
//! 诊断规则引擎（Regex + 结构化规则 + 动作绑定）落在 T5.5；本 crate 目前不含 IO。

#![forbid(unsafe_code)]

pub mod sanitize;
pub mod tracing_format;

pub use sanitize::{sanitize_log, REDACTED};
pub use tracing_format::SanitizedFormat;

/// crate 名称，用于日志与诊断中标识来源。
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");
