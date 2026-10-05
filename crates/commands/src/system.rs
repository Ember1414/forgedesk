//! 系统与运行时相关命令。
//!
//! 为什么命令定义在**子模块**而不是 crate 根 `lib.rs`：
//! `#[tauri::command]` 会为每个命令生成一个 `#[macro_export]` 的宏
//! （形如 `__tauri_command_name_<fn>`），它落在 **crate 根**的宏命名空间；
//! 若命令函数本身也定义在 crate 根，宏生成的 `pub use __tauri_command_name_<fn>;`
//! 会与前者同名，触发 E0255（"must be defined only once in the macro namespace"）。
//! 因此本项目约定：**所有命令一律定义在子模块中，由 `lib.rs` 重导出**。

use forgedesk_domain::AppResult;
use serde::{Deserialize, Serialize};

/// 应用版本与构建信息。
///
/// 序列化为 camelCase，与前端 TypeScript DTO 一一对应
/// （见 `src/lib/ipc/index.ts` 的 `AppVersion`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppVersion {
    /// 语义化版本号。
    pub version: String,
    /// 构建时的 git 提交短哈希；不在 git 仓库中构建时为 `"unknown"`。
    pub git_sha: String,
    /// 目标平台标识，形如 `x86_64-windows`。
    pub target: String,
    /// 构建类型：`"debug"` 或 `"release"`。
    pub profile: String,
}

/// 返回应用版本与构建信息。
///
/// 能力等级：`ReadOnly`（无参数、无副作用）。
///
/// 用途：验证前后端 IPC 通路是否打通；同时让用户在提 Issue 时能一键提供
/// 版本 + 提交号 + 平台信息。
#[tauri::command]
pub fn app_version() -> AppResult<AppVersion> {
    Ok(AppVersion {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        git_sha: option_env!("FORGEDESK_GIT_SHA")
            .unwrap_or("unknown")
            .to_owned(),
        target: format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS),
        profile: if cfg!(debug_assertions) {
            "debug".to_owned()
        } else {
            "release".to_owned()
        },
    })
}

/// 前端上报一条未捕获错误。
///
/// 能力等级：`Mutating`（写日志文件；不碰仓库、不碰用户数据，因此界面不需要确认）。
///
/// # 为什么需要这个命令
///
/// 界面里的未捕获错误在开发与 E2E 下收进 `window.__errs`（PLAN §10 的 DoD），
/// 但**生产环境里那个数组没人看**：用户看到的只是"某处坏了一下"，
/// 而我们的日志里一行都没有。把错误写进本地日志，是让"偶发的一次白屏"
/// 变成可排查证据的唯一办法。
///
/// # 纪律
///
/// - **截断而不是拒绝**：报错本身就说明出了意外，再因为"消息太长"丢掉它
///   等于把仅有的线索扔了。消息截 2KB、调用栈截 8KB；
/// - **脱敏交给写入层**：这里的字符串原样进 `tracing`，由
///   `SanitizingMakeWriter` 统一抹掉令牌（红线 R8）。在这里再脱一遍会多出
///   第二套规则，而两套规则迟早不一致；
/// - **不进审计**：审计记的是"用户对仓库做了什么"，记一条"界面崩了一下"
///   只会稀释它。
#[tauri::command]
pub fn log_frontend_error(message: String, stack: Option<String>) -> AppResult<()> {
    let (message, message_truncated) = truncate(&message, MESSAGE_LIMIT);
    let stack = stack.map(|value| truncate(&value, STACK_LIMIT));

    tracing::error!(
        target: "forgedesk::frontend",
        message = %message,
        message_truncated,
        stack = %stack.as_ref().map(|(text, _)| text.as_str()).unwrap_or(""),
        stack_truncated = stack.as_ref().is_some_and(|(_, truncated)| *truncated),
        "前端未捕获错误"
    );

    Ok(())
}

/// 用系统默认浏览器打开 http(s) 链接（终端链接识别、文档与反馈入口）。
///
/// 能力等级：`Network`（把一个 URL 交给系统默认处理程序；只接受
/// http/https 且无空白/控制字符，其余一律 `VALIDATION`——
/// 见 `forgedesk_platform::shell::open_url` 的安全说明）。
#[tauri::command]
pub fn system_open_url(url: String) -> AppResult<()> {
    forgedesk_platform::open_url(&url)
}

/// 单条错误消息的上限（2KB）。
pub const MESSAGE_LIMIT: usize = 2 * 1024;
/// 调用栈的上限（8KB）。
pub const STACK_LIMIT: usize = 8 * 1024;

/// 按**字符**边界截断（字节截断会把多字节字符切一半，日志里出现乱码）。
///
/// 返回截断后的文本与"是否发生过截断"：后者要记进日志，否则读日志的人
/// 会把一段半截栈当成完整栈，然后在缺失的帧上做出错误结论。
#[must_use]
pub fn truncate(text: &str, limit: usize) -> (String, bool) {
    if text.len() <= limit {
        return (text.to_owned(), false);
    }
    let mut end = limit;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    (text[..end].to_owned(), true)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{app_version, truncate, MESSAGE_LIMIT};

    #[test]
    fn app_version_reports_expected_fields() {
        let info = app_version().expect("app_version 不应失败");

        assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
        assert!(!info.target.is_empty(), "target 不应为空");
        assert!(
            info.target.contains('-'),
            "target 应为 <arch>-<os> 形式，实际：{}",
            info.target
        );
        assert!(
            info.profile == "debug" || info.profile == "release",
            "profile 只能是 debug 或 release，实际：{}",
            info.profile
        );
    }

    #[test]
    fn a_short_error_passes_through_untouched() {
        let (text, truncated) = truncate("boom", MESSAGE_LIMIT);
        assert_eq!(text, "boom");
        assert!(!truncated);
    }

    #[test]
    fn a_long_error_is_truncated_and_flagged() {
        let (text, truncated) = truncate(&"x".repeat(MESSAGE_LIMIT + 10), MESSAGE_LIMIT);
        assert_eq!(text.len(), MESSAGE_LIMIT);
        assert!(truncated, "必须标记已截断：半截文本不能当成完整文本");
    }

    #[test]
    fn truncation_never_splits_a_character() {
        // 一个汉字 3 字节：按字节截断会把最后一个字切一半，日志里就是乱码
        let text = "错".repeat(MESSAGE_LIMIT);
        let (kept, truncated) = truncate(&text, MESSAGE_LIMIT);
        assert!(truncated);
        assert_eq!(kept.len() % 3, 0, "必须停在字符边界上");
        assert!(kept.chars().all(|c| c == '错'));
    }

    #[test]
    fn app_version_serializes_to_camel_case() {
        let info = app_version().expect("app_version 不应失败");
        let json = serde_json::to_string(&info).unwrap();

        assert!(json.contains("\"gitSha\""), "应输出 camelCase 字段：{json}");
        assert!(
            !json.contains("git_sha"),
            "不应出现 snake_case 字段：{json}"
        );
    }
}
