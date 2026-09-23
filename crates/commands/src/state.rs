//! 命令层的应用级共享状态。
//!
//! 由 `src-tauri` 在启动时构建（打开数据库 + 执行迁移 + 解析日志目录）并通过
//! `manage` 注入；命令只借出只读引用。
//!
//! 为什么单独一个模块：T0.7 时它住在 `settings.rs` 里，而 T0.8 的日志命令也要用它。
//! 一旦多个命令族共享同一个状态，"它归谁"就不该由先写它的那个模块决定——
//! 否则每加一个命令族都要去改不相干的文件。

use std::path::PathBuf;
use std::sync::Arc;

use forgedesk_storage::Database;

/// 应用级共享状态。
#[derive(Debug)]
pub struct AppState {
    /// 本地数据库（并发策略见 `forgedesk_storage::Database`）。
    pub database: Arc<Database>,
    /// 日志目录（由宿主用 Tauri 的 `app_log_dir()` 解析后传入）。
    ///
    /// 为什么由宿主传入而不是在这里解析：`platform` crate 刻意不依赖 Tauri，
    /// 这样日志与会话逻辑能在纯 Rust 测试里跑，不必启动桌面运行时。
    pub log_dir: PathBuf,
}
