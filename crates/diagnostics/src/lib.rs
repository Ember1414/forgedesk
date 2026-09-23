//! 错误诊断引擎：把 git / 网络的原始错误映射为人话原因与可执行修复动作。
//!
//! 归属里程碑：见 docs/PLAN.md 的模块划分（§5.2）。
//!
//! M0（T0.6 / T0.8）先落地其中最基础、且**必须最早**具备的能力——脱敏：
//!
//! - [`sanitize`]：日志与错误详情的脱敏规则（红线 R8 的实现）。
//! - [`sanitizing_writer`]：把脱敏接到日志的**写入层**，使任何出境文本
//!   （控制台、文件、将来的崩溃报告）都会被过筛——这是结构性保证，而不是靠自觉。
//!   放在写入层而不是事件格式化层的原因见该模块头（JSON 格式化器无法被包装）。
//!
//! 诊断规则引擎（Regex + 结构化规则 + 动作绑定）落在 T5.5；本 crate 目前不含 IO。

#![forbid(unsafe_code)]

pub mod sanitize;
pub mod sanitizing_writer;

pub use sanitize::{sanitize_log, REDACTED};
pub use sanitizing_writer::{SanitizingMakeWriter, SanitizingWriter};

/// crate 名称，用于日志与诊断中标识来源。
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");
