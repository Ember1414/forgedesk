//! 环境能力检测（T6.9）：无图形环境、Linux inotify 上限。
//!
//! 这两个检测的共同点：它们回答"这个环境能不能/能撑多久"，答案要变成
//! **给用户的明确提示**而不是等操作失败后甩一个底层错误码。

use forgedesk_domain::{AppError, AppResult, ErrorCode};

/// 在给定环境变量访问器下判断是否"无图形环境"（纯函数，可测）。
///
/// 只有 Linux 需要检测：无 DISPLAY 且无 WAYLAND_DISPLAY 时，任何依赖 GUI 的
/// 能力（窗口、系统托盘、dbus 通知）都会直接失败。macOS/Windows 恒有图形栈。
pub fn is_headless_env(get: impl Fn(&str) -> Option<String>) -> bool {
    if !cfg!(target_os = "linux") {
        return false;
    }
    let display = get("DISPLAY").unwrap_or_default();
    let wayland = get("WAYLAND_DISPLAY").unwrap_or_default();
    display.trim().is_empty() && wayland.trim().is_empty()
}

/// 用真实环境判断是否无图形环境。
pub fn is_headless() -> bool {
    is_headless_env(|name| std::env::var(name).ok())
}

/// 无图形环境下的统一错误（调用方在启动早期检查并给出指引，而不是崩溃）。
pub fn headless_error() -> AppError {
    AppError::new(ErrorCode::Internal, "no graphical environment detected")
        .with_detail("DISPLAY and WAYLAND_DISPLAY are both unset".to_owned())
}

/// Linux inotify 内核上限（/proc/sys/fs/inotify 的两个关键值）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InotifyLimits {
    /// 单用户可监听的文件/目录总数（ watches）。
    pub max_user_watches: u64,
    /// 单用户可创建的 inotify 实例数。
    pub max_user_instances: u64,
}

/// 解析 /proc 风格的单值文件内容（首行整数）。
pub fn parse_proc_limit(text: &str) -> Option<u64> {
    text.split_whitespace().next()?.parse::<u64>().ok()
}

/// 读取 inotify 上限；仅 Linux 有意义，其它平台返回 None。
pub fn inotify_limits() -> Option<InotifyLimits> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let watches = std::fs::read_to_string("/proc/sys/fs/inotify/max_user_watches").ok()?;
    let instances = std::fs::read_to_string("/proc/sys/fs/inotify/max_user_instances").ok()?;
    Some(InotifyLimits {
        max_user_watches: parse_proc_limit(&watches)?,
        max_user_instances: parse_proc_limit(&instances)?,
    })
}

/// 校验给定上限是否够用（大仓库会一次监听数万个路径）。
///
/// `watches_needed` 由调用方估算（仓库内目录数 × 2 是常用近似）。
pub fn ensure_inotify_capacity(
    limits: InotifyLimits,
    instances_needed: u64,
    watches_needed: u64,
) -> AppResult<()> {
    if limits.max_user_instances < instances_needed {
        return Err(AppError::new(
            ErrorCode::Internal,
            "the inotify instance limit is too low for repository watching",
        )
        .with_detail(format!(
            "max_user_instances={} needed={instances_needed}",
            limits.max_user_instances
        ))
        .with_hint("sysctl fs.inotify.max_user_instances"));
    }
    if limits.max_user_watches < watches_needed {
        return Err(AppError::new(
            ErrorCode::Internal,
            "the inotify watch limit is too low for repository watching",
        )
        .with_detail(format!(
            "max_user_watches={} needed={watches_needed}",
            limits.max_user_watches
        ))
        .with_hint("sysctl fs.inotify.max_user_watches"));
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn headless_detection_requires_both_display_vars_to_be_empty() {
        let headless = is_headless_env(|_| None);
        // 在非 Linux 平台永远不是 headless（cfg 决定），因此分平台断言
        if cfg!(target_os = "linux") {
            assert!(headless);
        } else {
            assert!(!headless);
        }

        let with_wayland =
            is_headless_env(|name| (name == "WAYLAND_DISPLAY").then(|| "wayland-0".to_owned()));
        let with_x11 = is_headless_env(|name| (name == "DISPLAY").then(|| ":0".to_owned()));
        // 非 Linux 下这两个访问器也没机会返回 true
        if cfg!(target_os = "linux") {
            assert!(!with_wayland);
            assert!(!with_x11);
        }
    }

    #[test]
    fn proc_limit_parsing_takes_the_first_token_and_ignores_junk() {
        assert_eq!(parse_proc_limit("1048576\n"), Some(1_048_576));
        assert_eq!(parse_proc_limit("  128 "), Some(128));
        assert_eq!(parse_proc_limit("not-a-number"), None);
        assert_eq!(parse_proc_limit(""), None);
    }

    #[test]
    fn capacity_check_reports_which_limit_is_exhausted() {
        let limits = InotifyLimits {
            max_user_watches: 8192,
            max_user_instances: 128,
        };
        assert!(ensure_inotify_capacity(limits, 1, 4096).is_ok());

        let error = ensure_inotify_capacity(limits, 1, 999_999).unwrap_err();
        assert!(error.to_string().contains("watch limit"));
        assert!(error
            .hint
            .as_deref()
            .unwrap_or_default()
            .contains("max_user_watches"));

        let error = ensure_inotify_capacity(limits, 512, 4096).unwrap_err();
        assert!(error.to_string().contains("instance limit"));
    }
}
