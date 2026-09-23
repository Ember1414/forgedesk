// 生产构建下隐藏 Windows 控制台窗口；调试构建保留控制台以便看日志。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![forbid(unsafe_code)]

//! ForgeDesk 桌面应用入口（Tauri 宿主）。
//!
//! 本 crate 刻意保持**极薄**：只负责初始化日志、注册命令、启动窗口。
//! 所有业务逻辑都在 `crates/` 下的分层 crate 中（见 AGENTS.md §6）。

use tracing_subscriber::EnvFilter;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_tracing();

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![forgedesk_commands::app_version])
        .run(tauri::generate_context!())?;

    Ok(())
}

/// 初始化结构化日志。
///
/// M0 阶段只输出到控制台；文件日志与日志轮转在 T0.8 接入
/// （届时改为 `tracing-appender` 写入应用日志目录，并统一应用日志脱敏层）。
fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .init();
}
