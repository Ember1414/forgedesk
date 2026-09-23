// 生产构建下隐藏 Windows 控制台窗口；调试构建保留控制台以便看日志。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![forbid(unsafe_code)]

//! ForgeDesk 桌面应用入口（Tauri 宿主）。
//!
//! 本 crate 刻意保持**极薄**：只负责初始化日志、打开数据库并迁移、注册命令、启动窗口。
//! 所有业务逻辑都在 `crates/` 下的分层 crate 中（见 AGENTS.md §6）。

use std::path::Path;
use std::sync::{Arc, Mutex};

use forgedesk_commands::AppState;
use forgedesk_diagnostics::SanitizingMakeWriter;
use forgedesk_platform::session::{detect_previous_session, start_session, SessionMarker};
use forgedesk_platform::{install_panic_hook, non_blocking_writer, LogFlushGuard, LogPolicy};
use forgedesk_storage::{migrate, Database};
use tauri::{Manager, RunEvent};
use tracing::{info, warn};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

/// 数据库文件名（位于应用数据目录）。
const DATABASE_FILE: &str = "forgedesk.db";

/// 应用版本（编译期注入，用于日志、会话标记与 panic 报告）。
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// 日志系统的运行期句柄。
///
/// 两个字段都必须活到进程结束：
/// - `_log_guard`：非阻塞写入的 flush 守卫，drop 时把缓冲写进文件；
/// - `session`：会话标记，正常退出时删除它（不删 = 下次启动判定为异常退出）。
///
/// 用 `Mutex<Option<..>>` 是为了在退出事件里 `take()` 出来显式结束会话——
/// 只在 Drop 里做这件事的话，Tauri 的退出路径与 Rust 的析构顺序会让语义变得不可验证。
struct RuntimeHandles {
    _log_guard: LogFlushGuard,
    session: Mutex<Option<SessionMarker>>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let builder = tauri::Builder::default()
        // 日志、panic hook、数据库都在 setup 中初始化：
        // 因为 `app_log_dir()` / `app_data_dir()` 只有在拿到 App 句柄后才可用。
        // 代价是 Tauri 自身在 setup 之前的那几行日志不会被记录——那些是框架内部
        // 初始化信息，对用户与我们排查应用问题都没有价值。
        .setup(|app| {
            let log_dir = app.path().app_log_dir()?;
            let data_dir = app.path().app_data_dir()?;

            // 顺序很重要：先装日志与 panic hook，再打开数据库。
            // 数据库初始化是最容易在启动期失败的一步，它失败时我们要能看到原因。
            let guard = init_logging(&log_dir)?;
            install_panic_hook(&log_dir, APP_VERSION);

            // 上一次是否异常退出：T0.8 只记录事实，M7/T7.5 会据此提供恢复引导
            if let Some(previous) = detect_previous_session(&log_dir) {
                warn!(
                    marker = %previous.marker_path.display(),
                    pid = previous.info.as_ref().map(|info| info.pid),
                    version = previous.info.as_ref().map(|info| info.version.as_str()),
                    started_at = previous.info.as_ref().map(|info| info.started_at),
                    "检测到上次会话未正常退出"
                );
            }

            let session = start_session(&log_dir, APP_VERSION)?;

            let database_path = data_dir.join(DATABASE_FILE);
            let database = Database::open(&database_path)?;
            let report = migrate(&database)?;
            if !report.applied.is_empty() {
                info!(
                    from = report.from_version,
                    to = report.to_version,
                    applied = report.applied.len(),
                    backup = report.backup_path.map(|path| path.display().to_string()),
                    "数据库已迁移"
                );
            }

            info!(
                version = APP_VERSION,
                log_dir = %log_dir.display(),
                database = %database_path.display(),
                "应用已启动"
            );

            app.manage(AppState {
                database: Arc::new(database),
                log_dir,
            });
            app.manage(RuntimeHandles {
                _log_guard: guard,
                session: Mutex::new(Some(session)),
            });

            Ok(())
        });

    // 命令注册按构建类型分流：演示命令只在开发构建里存在。
    // 这样"用于验证错误链路的入口"不会随正式产物分发给用户——
    // 一个能让应用主动报错的命令没有任何理由存在于发布版里。
    #[cfg(debug_assertions)]
    let builder = builder.invoke_handler(tauri::generate_handler![
        forgedesk_commands::app_version,
        forgedesk_commands::settings_get,
        forgedesk_commands::settings_set,
        forgedesk_commands::settings_all,
        forgedesk_commands::logs_open,
        forgedesk_commands::logs_tail,
        forgedesk_commands::debug_throw_error,
        forgedesk_commands::debug_panic,
    ]);

    #[cfg(not(debug_assertions))]
    let builder = builder.invoke_handler(tauri::generate_handler![
        forgedesk_commands::app_version,
        forgedesk_commands::settings_get,
        forgedesk_commands::settings_set,
        forgedesk_commands::settings_all,
        forgedesk_commands::logs_open,
        forgedesk_commands::logs_tail,
    ]);

    let app = builder.build(tauri::generate_context!())?;

    app.run(|handle, event| {
        if let RunEvent::Exit = event {
            // 正常退出：删除会话标记。留在这里而不是 Drop 里，是因为
            // "正常退出"必须在代码里可见——否则将来有人加了 `std::process::exit`
            // 或提前返回，会话标记会一直残留，用户每次启动都会看到"上次异常退出"。
            if let Some(handles) = handle.try_state::<RuntimeHandles>() {
                if let Ok(mut guard) = handles.session.lock() {
                    if let Some(marker) = guard.take() {
                        if let Err(error) = marker.finish() {
                            warn!(%error, "删除会话标记失败");
                        }
                    }
                }
            }
        }
    });

    Ok(())
}

/// 初始化日志：文件（JSON Lines）+ 控制台（仅调试构建，人类可读）。
///
/// 三点约定：
///
/// 1. **文件始终写**（不只是 release）。T0.8 的 `logs_tail` / `logs_open` 需要真实文件，
///    如果只有 release 才落盘，这两个功能在开发期根本无法验证——而"只在发布版才跑"
///    的代码路径正是最容易坏的那一类。
/// 2. 控制台只在调试构建输出：release 下 Windows 子系统没有控制台，
///    在 macOS/Linux 上从终端启动时多出来的输出对普通用户只是噪音。
/// 3. **所有输出都经过脱敏**（`SanitizingMakeWriter`，红线 R8）。
///    脱敏在写入层完成，因此控制台的自由格式与文件的 JSON 格式共用同一份规则；
///    文件用 JSON 是为了让 `logs_tail` 能解析时间戳与级别并按时间高亮。
fn init_logging(log_dir: &Path) -> Result<LogFlushGuard, Box<dyn std::error::Error>> {
    let (writer, guard, _directory) = non_blocking_writer(log_dir, LogPolicy::default())?;

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let file_layer = tracing_subscriber::fmt::layer()
        .json()
        // 非阻塞写入器 → 按行脱敏 → 文件；顺序不能颠倒（脱敏必须在最靠近落盘的位置）
        .with_writer(SanitizingMakeWriter::new(writer))
        .with_ansi(false)
        .with_target(true);

    #[cfg(debug_assertions)]
    let console_layer = Some(
        tracing_subscriber::fmt::layer()
            .with_writer(SanitizingMakeWriter::new(std::io::stderr))
            .with_target(true),
    );
    #[cfg(not(debug_assertions))]
    let console_layer: Option<tracing_subscriber::fmt::Layer<_>> = None;

    tracing_subscriber::registry()
        .with(filter)
        .with(file_layer)
        .with(console_layer)
        .init();

    Ok(guard)
}
