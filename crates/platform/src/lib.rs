//! 平台适配层：日志文件、会话标记、shell 解析、路径规范化、系统集成。
//!
//! 归属里程碑：M0 / T0.8（日志落盘、轮转、panic 留档、会话标记）。
//!
//! # 边界
//!
//! 本 crate 处理"与操作系统打交道"的部分，不含业务逻辑：
//!
//! - [`logging`]：日志文件的轮转策略（按天 + 按大小）、保留期清理、末尾读取。
//!   **脱敏**由 `forgedesk-diagnostics` 提供，这里只管文件与格式。
//! - [`panic`]：崩溃现场留档（独立文件，不依赖轮转与日志系统可用性）。
//! - [`session`]：会话标记（判断上次是否异常退出）——T0.8 只给原语，
//!   策略（pid 存活判断、安全模式）属于 M7/T7.5。
//! - [`shell`]：在系统文件管理器中打开目录。
//!
//! # 与其它层的关系
//!
//! 路径由宿主（`src-tauri` 使用 Tauri 的 `app_log_dir()`）解析后传入，
//! 本 crate **不依赖 Tauri**：这样日志与会话逻辑可以在纯 Rust 测试里跑，
//! 不必启动桌面运行时。

#![forbid(unsafe_code)]

pub mod logging;
pub mod panic;
pub mod session;
pub mod shell;

pub use logging::{non_blocking_writer, tail, LogFlushGuard, LogLine, LogPolicy, RotatingWriter};
pub use panic::{install_panic_hook, write_panic_report, PanicReport};
pub use session::{start_session, PreviousSession, SessionInfo, SessionMarker};
pub use shell::open_in_file_manager;

/// crate 名称，用于日志与诊断中标识来源。
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");
