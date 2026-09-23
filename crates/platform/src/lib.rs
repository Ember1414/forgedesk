//! 平台适配层：凭据库、shell 解析、路径规范化、文件监听、系统通知、系统集成。
//!
//! 归属里程碑：见 docs/PLAN.md 的模块划分（§5.2）与对应任务。
//! 本 crate 尚未实现具体逻辑，仅在 M0/T0.2 阶段建立分层骨架。

#![forbid(unsafe_code)]

/// crate 名称，用于日志与诊断中标识来源。
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");
