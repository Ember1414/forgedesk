// 生产构建下隐藏 Windows 控制台窗口；调试构建保留控制台以便看日志。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![forbid(unsafe_code)]

//! ForgeDesk 桌面应用入口（Tauri 宿主）。
//!
//! 本 crate 刻意保持**极薄**：只负责初始化日志、注册命令、启动窗口。
//! 所有业务逻辑都在 `crates/` 下的分层 crate 中（见 AGENTS.md §6）。

use forgedesk_diagnostics::SanitizedFormat;
use tracing_subscriber::EnvFilter;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_tracing();

    let builder = tauri::Builder::default();

    // 命令注册按构建类型分流：演示命令只在开发构建里存在。
    // 这样"用于验证错误链路的入口"不会随正式产物分发给用户——
    // 一个能让应用主动报错的命令没有任何理由存在于发布版里。
    #[cfg(debug_assertions)]
    let builder = builder.invoke_handler(tauri::generate_handler![
        forgedesk_commands::app_version,
        forgedesk_commands::debug_throw_error,
    ]);

    #[cfg(not(debug_assertions))]
    let builder = builder.invoke_handler(tauri::generate_handler![forgedesk_commands::app_version]);

    builder.run(tauri::generate_context!())?;

    Ok(())
}

/// 初始化结构化日志。
///
/// 两点约定：
///
/// 1. **所有日志都经过脱敏**（`SanitizedFormat`，红线 R8）。
///    做成格式化层而不是"每条日志手动调用"，是为了让漏写一处不会造成泄漏。
/// 2. M0 阶段只输出到控制台；文件日志与轮转在 T0.8 接入
///    （届时改为 `tracing-appender` 写入应用日志目录，脱敏层保持不变）。
fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .event_format(SanitizedFormat)
        .init();
}
