//! 用例编排层：把领域逻辑与基础设施组合成完整业务动作（打开仓库、提交、同步、冲突解决…）。
//!
//! 归属里程碑：见 docs/PLAN.md 的模块划分（§5.2）与对应任务。
//! 本 crate 尚未实现具体逻辑，仅在 M0/T0.2 阶段建立分层骨架。

#![forbid(unsafe_code)]

/// crate 名称，用于日志与诊断中标识来源。
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");
