//! 统一错误转换层：把任意来源的错误映射为跨 IPC 边界的 [`AppError`]。
//!
//! # 为什么需要它
//!
//! 每个命令都可能失败，而失败的形式五花八门：`anyhow::Error`（来自 infra crate）、
//! `thiserror` 领域错误、字符串化的 git stderr、甚至 Tauri 自身抛出的错误。
//! 如果每个命令各自 `map_err`，会出现三类长期问题：
//!
//! 1. **同一故障多种错误码**：用户看到的修复建议取决于谁写的那个命令；
//! 2. **detail 忘记脱敏**：把 git stderr 直接塞进 `detail` 就可能带出远端 URL 里的凭据（红线 R8）；
//! 3. **错误码语义漂移**：新人在某处写 `Internal` 了事，前端再也给不出有效建议。
//!
//! 因此本模块是**唯一**的错误转换入口：分类交给领域层的纯函数
//! [`forgedesk_domain::ErrorCode::classify`]，脱敏交给
//! [`forgedesk_diagnostics::sanitize_log`]，这里只做"把两者串起来"。
//!
//! # 能力等级与审计
//!
//! 本层只负责"把错误说清楚"，不负责"记录发生了什么"——审计由 `AuditLog` 负责
//! （AGENTS.md §6 的单一写入口约束），错误路径同样要审计，见 M1 的 T1.11。

use forgedesk_diagnostics::sanitize_log;
use forgedesk_domain::{AppError, ErrorCode};

/// 错误链的最大展示深度。
///
/// 为什么要限制：`anyhow` 的链在深层嵌套里可能很长，而 UI 的"详情"区域要能一屏看完；
/// 更重要的是，链越深越可能包含与本次失败无关的历史上下文。
const MAX_CHAIN_DEPTH: usize = 8;

/// 把 `anyhow::Error` 转换为 `AppError`。
///
/// 转换规则：
/// - 错误码：用整条错误链的文本调用 [`ErrorCode::classify`]，先出现的判定规则优先；
/// - `message`：链顶（最贴近用户可见原因的那层）的描述；
/// - `detail`：整条链的拼接结果，**经过脱敏**；
/// - `retryable`：由错误码的默认值决定（例如 `NETWORK` 可重试，`VALIDATION` 不可）。
pub fn to_app_error(error: &anyhow::Error) -> AppError {
    let chain = collect_chain(error);
    let code = ErrorCode::classify(&chain);

    let message = error
        .to_string()
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .to_owned();
    let message = if message.is_empty() {
        code.default_message().to_owned()
    } else {
        message
    };

    let mut app_error = AppError::new(code, message);

    // detail 只在"确实有额外信息"时附加：与 message 相同的内容重复展示没有价值
    let detail = sanitize_log(&chain);
    if !detail.is_empty() && detail != app_error.message {
        app_error = app_error.with_detail(detail);
    }

    app_error
}

/// 把错误链拍平成一段文本（用于分类与展示）。
fn collect_chain(error: &anyhow::Error) -> String {
    let mut parts: Vec<String> = Vec::new();
    for (index, cause) in error.chain().enumerate() {
        if index >= MAX_CHAIN_DEPTH {
            parts.push("…（错误链已截断）".to_owned());
            break;
        }
        let text = cause.to_string();
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            parts.push(trimmed.to_owned());
        }
    }
    parts.join("\n")
}

/// 已把错误处理成"可展示形态"的操作结果。
///
/// 约定：命令层内部允许用 `anyhow::Result`（方便 `?`），但**离开命令层之前**
/// 必须经过 [`to_app_error`]。为了让这条约定在类型上成立，通常的写法是：
///
/// ```ignore
/// #[tauri::command]
/// pub fn some_action() -> AppResult<Dto> {
///     let result: anyhow::Result<Dto> = inner();
///     result.map_err(|error| to_app_error(&error))
/// }
/// ```
pub type Fallible<T> = anyhow::Result<T>;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use anyhow::anyhow;
    use forgedesk_domain::ErrorCode;

    use super::to_app_error;

    #[test]
    fn classifies_from_the_whole_chain() {
        // 顶层是泛泛的"命令失败"，真正的原因在 cause 里：
        // 只看顶层会退化成 INTERNAL，用户拿不到有用的建议
        let error =
            anyhow::anyhow!("fetch failed").context("fatal: could not resolve host: example.com");
        let mapped = to_app_error(&error);

        assert_eq!(mapped.code, ErrorCode::Network);
        assert!(mapped.retryable, "网络错误应可重试");
    }

    #[test]
    fn keeps_top_level_message_for_display() {
        let error = anyhow!("git apply 失败").context("staging failed");
        let mapped = to_app_error(&error);

        assert_eq!(mapped.message, "staging failed");
    }

    #[test]
    fn falls_back_to_default_message_when_empty() {
        let error = anyhow!("");
        let mapped = to_app_error(&error);

        assert_eq!(mapped.code, ErrorCode::Internal);
        assert_eq!(mapped.message, ErrorCode::Internal.default_message());
    }

    #[test]
    fn detail_is_sanitized_and_omitted_when_redundant() {
        let secret = "ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
        let error = anyhow!("clone failed").context(format!("remote: {secret} rejected"));
        let mapped = to_app_error(&error);

        let detail = mapped.detail.expect("应保留错误链作为详情");
        assert!(!detail.contains(secret), "detail 未脱敏：{detail}");
        assert!(
            detail.contains("ghp_«redacted»"),
            "应保留可辨识前缀：{detail}"
        );

        // 与 message 完全相同的链不再重复作为 detail
        let single = anyhow!("only one layer");
        assert_eq!(to_app_error(&single).detail, None);
    }

    #[test]
    fn truncates_very_deep_chains() {
        let mut error = anyhow!("root cause: permission denied");
        for index in 0..20 {
            error = error.context(format!("layer {index}"));
        }
        let mapped = to_app_error(&error);

        let detail = mapped.detail.expect("应有详情");
        assert!(detail.contains("错误链已截断"), "深链应被截断：{detail}");
        assert!(
            detail.lines().count() <= super::MAX_CHAIN_DEPTH + 1,
            "截断后行数仍然过多：{}",
            detail.lines().count()
        );
    }

    #[test]
    fn maps_common_git_failures_to_actionable_codes() {
        let cases = [
            (
                "fatal: not a git repository (or any parent up to mount point)",
                ErrorCode::PathNotRepo,
            ),
            (
                "error: Unmerged paths: fix conflicts",
                ErrorCode::GitConflict,
            ),
            (
                "fatal: Authentication failed for 'https://example.com/a.git/'",
                ErrorCode::AuthRequired,
            ),
            ("error: patch does not apply", ErrorCode::PatchApplyFailed),
        ];

        for (raw, expected) in cases {
            let mapped = to_app_error(&anyhow!("{raw}"));
            assert_eq!(mapped.code, expected, "分类错误：{raw}");
            assert!(!mapped.message.is_empty());
        }
    }
}
