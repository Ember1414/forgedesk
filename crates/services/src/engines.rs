//! 引擎组合根（实现已下沉到 `forgedesk_git_engine::engines`，T1.9）。
//!
//! # 为什么只剩一个 re-export
//!
//! 快照管理器（`crates/snapshot`）也要同时使用读 / 写两个引擎，而依赖方向是
//! `services → snapshot`，所以组合根必须放在 git-engine。保留这个模块与
//! re-export 是为了让既有的调用方（`forgedesk_services::GitEngines`）不必改动——
//! 一次搬迁只该改变"定义在哪"，不应该顺带要求所有使用点跟着改。

pub use forgedesk_git_engine::engines::GitEngines;
