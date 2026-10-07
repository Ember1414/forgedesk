//! 会话标记：判断"上次是否正常退出"。
//!
//! T0.8 只提供**原语**：写标记、正常退出时删除、启动时检测残留。
//! 具体的策略（pid 存活判断、连续三次异常退出进入安全模式、崩溃报告生成）
//! 属于 M7 的 T7.5；把原语和策略分开，是为了让 T0.8 不提前决定 M7 的策略，
//! 同时让 M7 有可靠的依据可用。
//!
//! 为什么用"标记文件存在与否"而不是读某个数据库字段：崩溃可能发生在数据库可用之前
//! （甚至连日志目录都是第一次创建），标记文件是最小依赖的取证方式。
//!
//! 标记内容为 JSON，包含 pid、版本与启动时间；M7 需要判断"残留的 pid 是否还活着"
//! 以及"崩溃发生在哪个版本"。

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use forgedesk_domain::{AppError, AppResult, ErrorCode};
use serde::{Deserialize, Serialize};

/// 会话标记文件名。
pub const SESSION_MARKER_FILE: &str = "session.lock";

/// 标记文件内容。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    /// 进程 id（M7 用它判断这个进程是否还活着）。
    pub pid: u32,
    /// 应用版本。
    pub version: String,
    /// 启动时间（Unix 毫秒）。
    pub started_at: i64,
}

/// 上一次会话留下的标记（说明上次没有正常退出）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviousSession {
    /// 标记内容（解析失败时为 `None`，但文件存在本身就说明异常退出）。
    pub info: Option<SessionInfo>,
    /// 标记文件路径。
    pub marker_path: PathBuf,
    /// 标记文件的修改时间（Unix 毫秒）。
    pub modified_at: Option<i64>,
}

/// 当前会话的标记句柄；调用 [`SessionMarker::finish`] 表示正常退出。
#[derive(Debug)]
pub struct SessionMarker {
    path: PathBuf,
}

impl SessionMarker {
    /// 标记文件路径。
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 正常退出：删除标记。
    ///
    /// 只暴露这一条"结束"路径（而不是实现 Drop）：如果靠 Drop，任何提前返回、
    /// `std::process::exit` 或 abort 都会让语义变得含糊；显式调用让"正常退出"这件事
    /// 在代码里可见，也让每个调用点都必须想清楚自己是不是真的走到了退出。
    pub fn finish(self) -> AppResult<()> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            // 已经不存在（例如被外部清理）视同成功
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(AppError::new(
                ErrorCode::Storage,
                "could not remove the session marker",
            )
            .with_detail(format!("{}: {error}", self.path.display()))),
        }
    }
}

/// 当前时间（Unix 毫秒）。
fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or_default()
}

/// 开始一个会话：写入标记文件。
pub fn start_session(directory: impl AsRef<Path>, version: &str) -> AppResult<SessionMarker> {
    let directory = directory.as_ref();
    std::fs::create_dir_all(directory).map_err(|error| {
        AppError::new(ErrorCode::Storage, "could not create the log directory")
            .with_detail(format!("{}: {error}", directory.display()))
    })?;

    let path = directory.join(SESSION_MARKER_FILE);
    let info = SessionInfo {
        pid: std::process::id(),
        version: version.to_owned(),
        started_at: now_millis(),
    };

    let payload = serde_json::to_string_pretty(&info).map_err(|error| {
        AppError::new(ErrorCode::Storage, "could not serialize the session info")
            .with_detail(error.to_string())
    })?;

    std::fs::write(&path, payload).map_err(|error| {
        AppError::new(ErrorCode::Storage, "could not write the session marker")
            .with_detail(format!("{}: {error}", path.display()))
    })?;

    Ok(SessionMarker { path })
}

/// 检测上次是否异常退出（存在残留标记即为异常）。
///
/// 不在这里判断"pid 是否还活着"：那是平台相关的探测（Windows 需要系统 API），
/// 且属于 M7 的策略范畴；T0.8 只如实报告"有一个残留标记"。
pub fn detect_previous_session(directory: impl AsRef<Path>) -> Option<PreviousSession> {
    let marker_path = directory.as_ref().join(SESSION_MARKER_FILE);
    let metadata = std::fs::metadata(&marker_path).ok()?;

    let modified_at = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX));

    let info = std::fs::read_to_string(&marker_path)
        .ok()
        .and_then(|content| serde_json::from_str::<SessionInfo>(&content).ok());

    Some(PreviousSession {
        info,
        marker_path,
        modified_at,
    })
}

/// 安全模式请求标记文件名（T7.5）。
///
/// 语义是**一次性**的：写标记 = "请下次启动进入安全模式"；启动时消费（读取并删除）
/// 它。用一次性语义而不是持久设置，是因为安全模式应当只影响**这一次**恢复启动：
/// 用户重启回正常模式时不必再去关掉一个开关（也避免忘记关导致插件长期被禁）。
pub const SAFE_MODE_FLAG_FILE: &str = "safe-mode.flag";

/// 请求下次启动进入安全模式（写标记）。
///
/// 在"崩溃恢复"对话框里点"以安全模式重启"时调用；随后宿主重启应用，
/// 新进程在启动时消费该标记。
pub fn request_safe_mode(directory: impl AsRef<Path>) -> AppResult<()> {
    let directory = directory.as_ref();
    std::fs::create_dir_all(directory).map_err(|error| {
        AppError::new(ErrorCode::Storage, "could not create the log directory")
            .with_detail(format!("{}: {error}", directory.display()))
    })?;
    let path = directory.join(SAFE_MODE_FLAG_FILE);
    std::fs::write(&path, b"safe-mode").map_err(|error| {
        AppError::new(ErrorCode::Storage, "could not write the safe-mode flag")
            .with_detail(format!("{}: {error}", path.display()))
    })
}

/// 清除安全模式请求（幂等：标记不存在也算成功）。
pub fn clear_safe_mode(directory: impl AsRef<Path>) -> AppResult<()> {
    let path = directory.as_ref().join(SAFE_MODE_FLAG_FILE);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(
            AppError::new(ErrorCode::Storage, "could not remove the safe-mode flag")
                .with_detail(format!("{}: {error}", path.display())),
        ),
    }
}

/// 消费安全模式标记：存在则删除并返回 `true`。
///
/// 读失败（权限等）按"没有请求"处理——安全模式是**降级**手段，
/// 拿不到标记时正常启动，不能因为一个标记读不出来就让应用起不来。
pub fn take_safe_mode_request(directory: impl AsRef<Path>) -> bool {
    let path = directory.as_ref().join(SAFE_MODE_FLAG_FILE);
    match std::fs::remove_file(&path) {
        Ok(()) => true,
        Err(_) => false,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{detect_previous_session, start_session, SESSION_MARKER_FILE};

    fn unique_dir(label: &str) -> std::path::PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let suffix = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut path = std::env::temp_dir();
        path.push(format!(
            "forgedesk-session-{label}-{}-{suffix}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        path
    }

    #[test]
    fn fresh_directory_has_no_previous_session() {
        let dir = unique_dir("fresh");
        assert!(detect_previous_session(&dir).is_none());
    }

    #[test]
    fn clean_exit_leaves_no_marker() {
        let dir = unique_dir("clean");

        let marker = start_session(&dir, "0.0.1").expect("应能开始会话");
        assert!(marker.path().exists(), "会话开始时应有标记文件");

        let detected = detect_previous_session(&dir).expect("标记存在时应被检测到");
        assert_eq!(
            detected.info.as_ref().map(|info| info.pid),
            Some(std::process::id())
        );
        assert_eq!(
            detected.info.as_ref().map(|info| info.version.as_str()),
            Some("0.0.1")
        );

        marker.finish().expect("正常退出应删除标记");
        assert!(!dir.join(SESSION_MARKER_FILE).exists());
        assert!(
            detect_previous_session(&dir).is_none(),
            "正常退出后不应再判定为异常退出"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// 崩溃场景：标记没有被删除，下次启动必须能发现它。
    #[test]
    fn leftover_marker_is_reported_as_previous_session() {
        let dir = unique_dir("crash");

        // 模拟"启动后崩溃"：写入标记但不调用 finish
        let marker = start_session(&dir, "0.0.1").expect("应能开始会话");
        let marker_path = marker.path().to_path_buf();
        drop(marker); // 进程"消失"

        let detected = detect_previous_session(&dir).expect("残留标记必须被检测到");
        assert_eq!(detected.marker_path, marker_path);
        assert!(detected.modified_at.is_some(), "应带上标记文件的修改时间");
        assert!(detected.info.is_some(), "内容应可解析");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// 标记文件被手工改坏时，文件存在本身就是"异常退出"的证据，
    /// 解析失败不能让检测整体失效（否则崩溃证据就丢了）。
    #[test]
    fn corrupted_marker_still_counts_as_unclean_exit() {
        let dir = unique_dir("corrupt");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(SESSION_MARKER_FILE), b"{ not json").unwrap();

        let detected = detect_previous_session(&dir).expect("文件存在即应被检测到");
        assert!(detected.info.is_none(), "内容解析失败时 info 为 None");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn finish_is_idempotent_when_marker_already_gone() {
        let dir = unique_dir("gone");
        let marker = start_session(&dir, "0.0.1").unwrap();
        std::fs::remove_file(marker.path()).unwrap();

        // 已被外部清理时，finish 不应报错（否则退出路径会多出一次假失败）
        marker.finish().expect("重复删除应视同成功");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// 安全模式标记是一次性的：消费一次后即消失，重启回正常模式不需要额外操作。
    #[test]
    fn safe_mode_request_is_consumed_exactly_once() {
        let dir = unique_dir("safe-mode");
        std::fs::create_dir_all(&dir).unwrap();

        assert!(
            !super::take_safe_mode_request(&dir),
            "没有请求时不应进入安全模式"
        );

        super::request_safe_mode(&dir).expect("应能写入安全模式请求");
        assert!(super::take_safe_mode_request(&dir), "写入后应被消费为真");
        assert!(
            !super::take_safe_mode_request(&dir),
            "一次性语义：第二次消费必须为假"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// 清除是幂等的：没有标记时也不报错（退出安全模式会无条件调用它）。
    #[test]
    fn clearing_a_missing_safe_mode_flag_is_not_an_error() {
        let dir = unique_dir("safe-mode-clear");
        std::fs::create_dir_all(&dir).unwrap();

        super::clear_safe_mode(&dir).expect("清除不存在的标记不应报错");
        super::request_safe_mode(&dir).unwrap();
        super::clear_safe_mode(&dir).expect("清除已存在的标记不应报错");
        assert!(
            !super::take_safe_mode_request(&dir),
            "清除后不应再进入安全模式"
        );

        std::fs::remove_dir_all(&dir).ok();
    }
}
