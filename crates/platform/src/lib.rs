//! 平台适配层：日志文件、会话标记、shell 解析、路径规范化、系统集成、文件监听。
//!
//! 归属里程碑：M0 / T0.8（日志落盘、轮转、panic 留档、会话标记）；
//! M1 / T1.10（仓库文件监听）；M6 / T6.9（路径规范化、shell 解析、系统集成、
//! 通知接口、环境能力检测）。
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
//! - [`shell_resolver`]：列出可用 shell 与默认建议（终端页 T5.2 消费）。
//! - [`path_normalizer`]：跨平台路径比较（大小写 / 长路径前缀 / NFC）。
//! - [`system_integration`]：定位文件、默认应用打开、开机自启。
//! - [`notify`]：桌面通知接口（真实 toast 随 T7 落地，当前日志兜底）。
//! - [`platform_checks`]：无图形环境与 Linux inotify 上限检测。
//! - [`watcher`]：仓库文件监听（过滤噪音 → 去抖动 → 溢出保护），把操作系统的
//!   原始事件收敛成"界面该刷新什么"。事件怎么送到前端由调用方决定。
//!
//! # 与其它层的关系
//!
//! 路径由宿主（`src-tauri` 使用 Tauri 的 `app_log_dir()`）解析后传入，
//! 本 crate **不依赖 Tauri**：这样日志、会话与监听逻辑可以在纯 Rust 测试里跑，
//! 不必启动桌面运行时。

#![forbid(unsafe_code)]

pub mod logging;
pub mod notify;
pub mod panic;
pub mod path_normalizer;
pub mod platform_checks;
pub mod session;
pub mod shell;
pub mod shell_resolver;
pub mod system_integration;
pub mod watcher;

pub use logging::{non_blocking_writer, tail, LogFlushGuard, LogLine, LogPolicy, RotatingWriter};
pub use notify::{notifier_for_current_platform, Notification, NotificationKind, Notifier};
pub use panic::{install_panic_hook, write_panic_report, PanicReport};
pub use path_normalizer::{PathNormalizer, EXTENDED_PATH_THRESHOLD};
pub use platform_checks::{
    ensure_inotify_capacity, inotify_limits, is_headless, is_headless_env, InotifyLimits,
};
pub use session::{start_session, PreviousSession, SessionInfo, SessionMarker};
pub use shell::{open_in_file_manager, open_url};
pub use shell_resolver::{
    path_search_dirs, pick_default, probe_shells, ShellInfo, ShellKind, ShellResolver,
    SystemShellResolver,
};
pub use system_integration::{
    autostart_for_current_platform, open_file_with_default, reveal_in_file_manager, Autostart,
    UnsupportedAutostart,
};
pub use watcher::{
    FileWatcher, NotifyFileWatcher, WatchCallback, WatchError, WatchEvent, WatchKind, WatchOptions,
    WatcherHandle, DEFAULT_DEBOUNCE, DEFAULT_DEBOUNCE_MS, DEFAULT_MAX_PATHS,
};

/// crate 名称，用于日志与诊断中标识来源。
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");
