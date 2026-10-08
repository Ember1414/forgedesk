//! 启动恢复与安全模式（M7 / T7.5）。
//!
//! | 命令 | 能力等级 | 说明 |
//! | --- | --- | --- |
//! | [`app_startup_report`] | `ReadOnly` | 报告上次是否异常退出、当前是否安全模式 |
//! | [`app_restart`] | `Mutating`（写标记文件 + 重启进程） | 重启应用，可选择以安全模式重启 |
//!
//! # 为什么"安全模式"是一次性标记而不是持久设置
//!
//! 它只在"崩溃后恢复"这一条路径上有意义。做成持久开关的话，用户为了排查点一次，
//! 之后忘了关，插件会一直被禁用——那比崩溃本身更糟。因此 [`app_restart`] 写入的
//! 标记在下次启动时**被消费（读取即删除）**，只影响那一次启动。
//!
//! # 为什么重启前必须结束会话标记
//!
//! 会话标记的存在 = "上次没有正常退出"（见 `forgedesk_platform::session`）。
//! 主动重启如果不删它，下一次启动会把"我们自己的重启"读成"上次崩溃"，
//! 于是每次重启都弹一次恢复提示——**假警报会淹没真正的崩溃**。因此
//! [`app_restart`] 先经 [`SessionEnder`] 结束当前会话，再重启。

use forgedesk_domain::AppResult;
use forgedesk_platform::session::{clear_safe_mode, request_safe_mode, PreviousSession};
use serde::Serialize;
use tauri::State;

use crate::state::AppState;

/// 上一次异常退出的会话信息（对应残留会话标记的内容）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LastExitDto {
    /// 崩溃进程的 pid（标记无法解析时为 None）。
    pub pid: Option<u32>,
    /// 崩溃时的应用版本。
    pub version: Option<String>,
    /// 崩溃会话的开始时间（Unix 毫秒）。
    pub started_at_ms: Option<i64>,
    /// 标记文件的修改时间（Unix 毫秒），可近似看作崩溃发生的时间。
    pub detected_at_ms: Option<i64>,
}

/// 启动恢复报告（IPC 返回形状）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupReportDto {
    /// 上次是否异常退出。
    pub abnormal_exit: bool,
    /// 上次异常退出的细节（没有时为 None）。
    pub last_exit: Option<LastExitDto>,
    /// 本次是否为安全模式启动。
    pub safe_mode: bool,
    /// 日志目录（界面上的"查看日志"入口用它）。
    pub log_dir: String,
}

/// 启动时收集的恢复信息（宿主在装配 [`AppState`] 时计算一次）。
#[derive(Debug, Clone)]
pub struct StartupReport {
    /// 上次是否异常退出。
    pub abnormal_exit: bool,
    /// 上次异常退出的细节。
    pub last_exit: Option<LastExitDto>,
    /// 本次是否为安全模式启动。
    pub safe_mode: bool,
}

impl StartupReport {
    /// 由"残留会话标记 + 本次是否安全模式"构造报告。
    pub fn from_previous(previous: Option<PreviousSession>, safe_mode: bool) -> Self {
        let last_exit = previous.as_ref().map(|session| LastExitDto {
            pid: session.info.as_ref().map(|info| info.pid),
            version: session.info.as_ref().map(|info| info.version.clone()),
            started_at_ms: session.info.as_ref().map(|info| info.started_at),
            detected_at_ms: session.modified_at,
        });
        Self {
            abnormal_exit: previous.is_some(),
            last_exit,
            safe_mode,
        }
    }
}

/// 结束当前会话（删除会话标记）的能力，由宿主注入。
///
/// 为什么不直接在命令里删文件：会话标记的生命周期由宿主持有
/// （见 `src-tauri` 的 `RuntimeHandles`），标记对象只能被 `take()` 一次；
/// 命令层持有路径去删会让"退出时也删一次"变成两条互不知情的路径。
pub struct SessionEnder(Box<dyn Fn() -> AppResult<()> + Send + Sync>);

impl SessionEnder {
    /// 用宿主提供的实现构造。
    pub fn new<F>(end: F) -> Self
    where
        F: Fn() -> AppResult<()> + Send + Sync + 'static,
    {
        Self(Box::new(end))
    }

    /// 结束会话（幂等：标记已被取走时不再动作）。
    pub fn end(&self) -> AppResult<()> {
        (self.0)()
    }
}

impl std::fmt::Debug for SessionEnder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SessionEnder")
    }
}

/// 启动恢复报告。能力等级：`ReadOnly`。
///
/// 界面在启动时调用一次：若 `abnormalExit` 为真，展示"上次异常退出"提示，
/// 并让用户选择「以安全模式重启」或「继续」；`safeMode` 为真时界面显示常驻标识。
#[tauri::command(async)]
pub fn app_startup_report(state: State<'_, AppState>) -> AppResult<StartupReportDto> {
    Ok(StartupReportDto {
        abnormal_exit: state.startup.abnormal_exit,
        last_exit: state.startup.last_exit.clone(),
        safe_mode: state.startup.safe_mode,
        log_dir: state.log_dir.display().to_string(),
    })
}

/// 重启应用，可选择以安全模式重启。能力等级：`Mutating`（写标记文件并重启进程）。
///
/// - `safeMode = true`：写入一次性标记 → 新进程不加载任何插件、禁用终端；
/// - `safeMode = false`：清除标记 → 正常启动（用于"退出安全模式"）。
///
/// 重启前先结束会话标记，否则新进程会把这次主动重启误判为崩溃。
#[tauri::command]
pub fn app_restart(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    safe_mode: bool,
) -> AppResult<()> {
    if safe_mode {
        request_safe_mode(&state.log_dir)?;
    } else {
        clear_safe_mode(&state.log_dir)?;
    }
    state.end_session.end()?;
    app.restart();
    #[allow(unreachable_code)]
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::StartupReport;
    use forgedesk_platform::session::{PreviousSession, SessionInfo};

    #[test]
    fn a_fresh_start_has_no_abnormal_exit() {
        let report = StartupReport::from_previous(None, false);
        assert!(!report.abnormal_exit);
        assert!(report.last_exit.is_none());
        assert!(!report.safe_mode);
    }

    #[test]
    fn a_leftover_marker_becomes_the_last_exit_details() {
        let previous = PreviousSession {
            info: Some(SessionInfo {
                pid: 4242,
                version: "0.7.0".to_owned(),
                started_at: 1_700_000_000_000,
            }),
            marker_path: std::path::PathBuf::from("/tmp/session.lock"),
            modified_at: Some(1_700_000_005_000),
        };

        let report = StartupReport::from_previous(Some(previous), true);
        assert!(report.abnormal_exit);
        assert!(report.safe_mode);
        let last = report.last_exit.expect("应带上次退出细节");
        assert_eq!(last.pid, Some(4242));
        assert_eq!(last.version.as_deref(), Some("0.7.0"));
        assert_eq!(last.started_at_ms, Some(1_700_000_000_000));
        assert_eq!(last.detected_at_ms, Some(1_700_000_005_000));
    }

    /// 标记内容解析失败（文件被改坏）时仍是异常退出——信息可以缺，事实不能丢。
    #[test]
    fn a_corrupted_marker_still_reports_an_abnormal_exit_without_details() {
        let previous = PreviousSession {
            info: None,
            marker_path: std::path::PathBuf::from("/tmp/session.lock"),
            modified_at: None,
        };

        let report = StartupReport::from_previous(Some(previous), false);
        assert!(report.abnormal_exit);
        let last = report.last_exit.expect("即使内容缺失也应给出条目");
        assert_eq!(last.pid, None);
        assert_eq!(last.version, None);
    }
}
