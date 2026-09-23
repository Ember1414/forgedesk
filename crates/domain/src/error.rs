//! 统一错误模型。
//!
//! 设计要点（对应 docs/PLAN.md §5.5）：
//!
//! - `ErrorCode` 是**前后端契约**：只允许新增，不允许重命名或删除。
//!   前端据此做 i18n 映射与分支处理，因此序列化形式必须是稳定的字符串。
//! - `AppError` 携带 `hint` 与 `actions`，让 UI 能直接把"错误"变成"可执行的修复入口"，
//!   而不是丢给用户一段 stderr。
//! - `detail` 只用于展示原始信息（如 git stderr），**必须经过脱敏**后才能填入
//!   （脱敏实现见 T0.6 的 `sanitize_log`）。本模块不负责脱敏，但所有构造函数都
//!   在文档中标注了该约束。

use serde::{Deserialize, Serialize};

/// 稳定错误码。
///
/// 序列化形式为 `SCREAMING_SNAKE_CASE`，例如 `PathNotRepo` → `"PATH_NOT_REPO"`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    /// 目标路径不是 Git 仓库（也不在其子目录中）。
    PathNotRepo,
    /// 存在未解决的合并 / rebase / cherry-pick 冲突。
    GitConflict,
    /// 需要认证：未登录、未提供凭据或凭据不足。
    AuthRequired,
    /// 凭据已过期或被撤销，需要重新登录。
    AuthExpired,
    /// 权限不足（缺少必要的作用域或文件系统权限）。
    PermissionDenied,
    /// 资源不存在（仓库、分支、提交、PR 等）。
    NotFound,
    /// 参数校验失败。
    Validation,
    /// 网络错误（不可达、超时、代理失败）。
    Network,
    /// 触发托管平台的 API 速率限制。
    RateLimited,
    /// 补丁应用失败（行级 / 块级暂存场景）。
    PatchApplyFailed,
    /// 操作计划已过期，或仓库在计划生成后被外部修改。
    PlanStale,
    /// Git 钩子拒绝了本次操作。
    HookRejected,
    /// 快照回滚后的状态校验未通过。
    RestoreVerifyFailed,
    /// 系统凭据库不可用（例如 Linux 上缺少 Secret Service）。
    KeyringUnavailable,
    /// 当前平台不支持伪终端（PTY）。
    PtyUnsupported,
    /// 当前 Git 引擎不支持该操作。
    UnsupportedByEngine,
    /// 未归类的内部错误。
    Internal,
}

impl ErrorCode {
    /// 全部错误码，用于遍历（例如校验 i18n 覆盖率）。
    pub const ALL: &'static [Self] = &[
        Self::PathNotRepo,
        Self::GitConflict,
        Self::AuthRequired,
        Self::AuthExpired,
        Self::PermissionDenied,
        Self::NotFound,
        Self::Validation,
        Self::Network,
        Self::RateLimited,
        Self::PatchApplyFailed,
        Self::PlanStale,
        Self::HookRejected,
        Self::RestoreVerifyFailed,
        Self::KeyringUnavailable,
        Self::PtyUnsupported,
        Self::UnsupportedByEngine,
        Self::Internal,
    ];

    /// 稳定的字符串形式（与 serde 序列化结果一致）。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PathNotRepo => "PATH_NOT_REPO",
            Self::GitConflict => "GIT_CONFLICT",
            Self::AuthRequired => "AUTH_REQUIRED",
            Self::AuthExpired => "AUTH_EXPIRED",
            Self::PermissionDenied => "PERMISSION_DENIED",
            Self::NotFound => "NOT_FOUND",
            Self::Validation => "VALIDATION",
            Self::Network => "NETWORK",
            Self::RateLimited => "RATE_LIMITED",
            Self::PatchApplyFailed => "PATCH_APPLY_FAILED",
            Self::PlanStale => "PLAN_STALE",
            Self::HookRejected => "HOOK_REJECTED",
            Self::RestoreVerifyFailed => "RESTORE_VERIFY_FAILED",
            Self::KeyringUnavailable => "KEYRING_UNAVAILABLE",
            Self::PtyUnsupported => "PTY_UNSUPPORTED",
            Self::UnsupportedByEngine => "UNSUPPORTED_BY_ENGINE",
            Self::Internal => "INTERNAL",
        }
    }

    /// 前端 i18n 标题 key，形如 `errors.PATH_NOT_REPO.title`。
    pub fn i18n_key(self) -> String {
        format!("errors.{}.title", self.as_str())
    }

    /// 默认是否可重试。调用方可覆盖该判断。
    pub const fn default_retryable(self) -> bool {
        matches!(self, Self::Network | Self::RateLimited | Self::PlanStale)
    }
}

/// 可由用户点击执行的修复动作。
///
/// 前端据此渲染按钮，点击后调用 `command` 指向的 Tauri 命令。
/// 注意：`command` 为 `Mutating` 或 `Dangerous` 能力的动作，前端**必须**先走
/// 统一的危险操作对话框（计划预览 + 快照），不得直接执行。
///
/// 序列化约定：与前端 DTO 一致使用 camelCase（`label_key` → `labelKey`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FixAction {
    /// 动作标识，前端用于埋点与去重。
    pub id: String,
    /// 按钮文案的 i18n key。
    pub label_key: String,
    /// 要调用的 Tauri 命令名。
    pub command: String,
    /// 调用参数；缺省表示无参。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<serde_json::Value>,
}

impl FixAction {
    /// 创建一个无参修复动作。
    pub fn new(
        id: impl Into<String>,
        label_key: impl Into<String>,
        command: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            label_key: label_key.into(),
            command: command.into(),
            args: None,
        }
    }

    /// 附加调用参数。
    #[must_use]
    pub fn with_args(mut self, args: serde_json::Value) -> Self {
        self.args = Some(args);
        self
    }
}

/// 应用统一错误类型。
///
/// 所有跨越 IPC 边界的错误都必须是本类型，便于前端用同一套逻辑展示。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppError {
    /// 稳定错误码。
    pub code: ErrorCode,
    /// 开发者可读的英文描述（用户可见文案由前端按 `code` 做 i18n）。
    pub message: String,
    /// 原始细节（如 git stderr）。**填入前必须脱敏，禁止包含 token / 密码。**
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// 面向用户的一句话建议。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// 可点击的修复动作。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<FixAction>,
    /// 是否可重试。
    pub retryable: bool,
}

impl AppError {
    /// 用错误码与描述创建错误，`retryable` 取该错误码的默认值。
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            detail: None,
            hint: None,
            actions: Vec::new(),
            retryable: code.default_retryable(),
        }
    }

    /// 附加原始细节。调用方必须保证已脱敏。
    #[must_use]
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// 附加用户建议。
    #[must_use]
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// 追加一个修复动作。
    #[must_use]
    pub fn with_action(mut self, action: FixAction) -> Self {
        self.actions.push(action);
        self
    }

    /// 覆盖默认可重试判断。
    #[must_use]
    pub const fn with_retryable(mut self, retryable: bool) -> Self {
        self.retryable = retryable;
        self
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for AppError {}

/// 领域层与业务层的统一结果类型。
pub type AppResult<T> = Result<T, AppError>;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{AppError, ErrorCode, FixAction};

    #[test]
    fn error_code_serializes_to_screaming_snake_case() {
        let cases = [
            (ErrorCode::PathNotRepo, "\"PATH_NOT_REPO\""),
            (ErrorCode::RateLimited, "\"RATE_LIMITED\""),
            (ErrorCode::UnsupportedByEngine, "\"UNSUPPORTED_BY_ENGINE\""),
        ];
        for (code, expected) in cases {
            assert_eq!(serde_json::to_string(&code).unwrap(), expected);
        }
    }

    #[test]
    fn as_str_matches_serde_output_for_all_codes() {
        for code in ErrorCode::ALL {
            let serialized = serde_json::to_string(code).unwrap();
            assert_eq!(
                serialized,
                format!("\"{}\"", code.as_str()),
                "as_str 与 serde 序列化不一致：{code:?}"
            );
        }
    }

    #[test]
    fn all_codes_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for code in ErrorCode::ALL {
            assert!(seen.insert(code.as_str()), "错误码重复：{}", code.as_str());
        }
        assert_eq!(seen.len(), ErrorCode::ALL.len());
    }

    #[test]
    fn i18n_key_is_prefixed() {
        assert_eq!(
            ErrorCode::GitConflict.i18n_key(),
            "errors.GIT_CONFLICT.title"
        );
    }

    #[test]
    fn error_deserializes_from_frontend_shape() {
        let json = r#"{
            "code": "PATCH_APPLY_FAILED",
            "message": "git apply failed",
            "detail": "error: patch does not apply",
            "hint": "请刷新状态后重试",
            "actions": [{"id": "refresh", "labelKey": "errors.refresh", "command": "git_status"}],
            "retryable": true
        }"#;
        let error: AppError = serde_json::from_str(json).unwrap();
        assert_eq!(error.code, ErrorCode::PatchApplyFailed);
        assert_eq!(error.actions.len(), 1);
        assert_eq!(error.actions[0].command, "git_status");
        assert_eq!(error.actions[0].args, None);
        assert!(error.retryable);
    }

    #[test]
    fn optional_fields_are_omitted_when_empty() {
        let error = AppError::new(ErrorCode::Internal, "boom");
        let json = serde_json::to_string(&error).unwrap();
        assert!(!json.contains("detail"));
        assert!(!json.contains("hint"));
        assert!(!json.contains("actions"));
        assert!(json.contains("\"retryable\":false"));
    }

    #[test]
    fn builder_sets_retryable_default_from_code() {
        assert!(AppError::new(ErrorCode::Network, "offline").retryable);
        assert!(!AppError::new(ErrorCode::Validation, "bad input").retryable);
        assert!(
            !AppError::new(ErrorCode::Network, "x")
                .with_retryable(false)
                .retryable
        );
    }

    #[test]
    fn fix_action_serializes_camel_case_keys() {
        let action = FixAction::new("refresh", "errors.refresh", "git_status")
            .with_args(serde_json::json!({ "repoId": 1 }));
        let json = serde_json::to_string(&action).unwrap();
        assert!(json.contains("\"labelKey\""));
        assert!(!json.contains("label_key"));
        assert!(json.contains("\"repoId\":1"));
    }

    #[test]
    fn display_includes_code() {
        let error = AppError::new(ErrorCode::PlanStale, "plan expired");
        assert_eq!(error.to_string(), "[PLAN_STALE] plan expired");
    }
}
