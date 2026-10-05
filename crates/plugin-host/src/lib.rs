//! 插件宿主：清单解析、权限校验、WASI 沙箱运行与扩展点注册（M6 / T6.1 起）。
//!
//! 分层位置：`plugin-host` 是 IO 边界 crate（与 `git-engine`、`storage` 同级），
//! 清单与权限模型是纯数据（可单测）；wasm 引擎接入层在 T6.1 引擎选型审批
//! （wasmtime 体积影响）落地后于 `runtime` 模块实现，见 `docs/PLAN.md` §M6 风险表。

#![forbid(unsafe_code)]

pub mod engine_wasmi;
pub mod host;
pub mod manager;
pub mod manifest;
pub mod panel_dsl;
pub mod permission;
pub mod runtime;

/// crate 名称，用于日志与诊断中标识来源。
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");
