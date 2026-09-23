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
    /// 本地数据存储失败（SQLite 打开、迁移、读写）。
    ///
    /// 为什么单独一个码：本地存储失败是**用户可自救**的一类问题
    /// （磁盘满、数据目录只读、数据库文件被别的进程占用），
    /// 给一句"检查磁盘空间与数据目录权限"比笼统的"内部错误"有用得多。
    Storage,
    /// 当前平台不支持伪终端（PTY）。
    PtyUnsupported,
    /// 当前 Git 引擎不支持该操作。
    UnsupportedByEngine,
    /// 操作被用户取消（或随应用退出中止）。
    ///
    /// 为什么不是 `Internal`：取消是**用户主动的、预期内的**结果，
    /// 界面应当安静地显示"已取消"，而不是弹一个"内部错误"的红色提示。
    /// 把两者混在一起会让用户以为自己点坏了什么。
    Cancelled,
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
        Self::Storage,
        Self::PtyUnsupported,
        Self::UnsupportedByEngine,
        Self::Cancelled,
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
            Self::Storage => "STORAGE",
            Self::PtyUnsupported => "PTY_UNSUPPORTED",
            Self::UnsupportedByEngine => "UNSUPPORTED_BY_ENGINE",
            Self::Cancelled => "CANCELLED",
            Self::Internal => "INTERNAL",
        }
    }

    /// 前端 i18n 标题 key，形如 `errors.PATH_NOT_REPO.title`。
    pub fn i18n_key(self) -> String {
        format!("errors.{}.title", self.as_str())
    }

    /// 前端 i18n 建议（hint）key，形如 `errors.PATH_NOT_REPO.hint`。
    ///
    /// 为什么需要它：不是每个错误都能在现场给出针对性建议（例如"网络不可达"），
    /// 但**每个错误都必须给用户一句"接下来能做什么"**，否则用户只能来问人。
    /// 因此兜底文案放在 i18n 里，由前端在 `AppError.hint` 缺省时使用。
    pub fn hint_i18n_key(self) -> String {
        format!("errors.{}.hint", self.as_str())
    }

    /// 默认的开发者可读描述（英文）。
    ///
    /// 用途：后端在不知道更具体原因时的兜底 message。用户可见文案不取这里，
    /// 而是由前端按 `code` 走 i18n —— 保证同一个错误码在中英文下都有人话描述。
    pub const fn default_message(self) -> &'static str {
        match self {
            Self::PathNotRepo => "the path is not a Git repository",
            Self::GitConflict => "the repository has unresolved conflicts",
            Self::AuthRequired => "authentication is required",
            Self::AuthExpired => "the stored credential has expired or was revoked",
            Self::PermissionDenied => "permission denied",
            Self::NotFound => "the requested resource does not exist",
            Self::Validation => "invalid input",
            Self::Network => "network request failed",
            Self::RateLimited => "the host rate limit was reached",
            Self::PatchApplyFailed => "the patch could not be applied",
            Self::PlanStale => "the plan is stale: the repository changed after it was built",
            Self::HookRejected => "a Git hook rejected the operation",
            Self::RestoreVerifyFailed => "the restored state failed verification",
            Self::KeyringUnavailable => "the system credential store is unavailable",
            Self::Storage => "local data storage failed",
            Self::PtyUnsupported => "pseudo-terminal is not supported on this platform",
            Self::UnsupportedByEngine => "the current Git engine does not support this operation",
            Self::Cancelled => "the operation was cancelled",
            Self::Internal => "an internal error occurred",
        }
    }

    /// 默认是否可重试。调用方可覆盖该判断。
    pub const fn default_retryable(self) -> bool {
        matches!(self, Self::Network | Self::RateLimited | Self::PlanStale)
    }

    /// 从字符串解析错误码（大小写不敏感，与 [`Self::as_str`] 一致）。
    ///
    /// 用途：IPC 参数里传递错误码（如演示命令、错误注入测试）时，
    /// 必须把外部输入收敛到已知枚举，而不是把字符串当错误码直接回显给用户。
    pub fn parse(value: &str) -> Option<Self> {
        let needle = value.trim();
        Self::ALL
            .iter()
            .copied()
            .find(|code| code.as_str().eq_ignore_ascii_case(needle))
    }

    /// 从原始错误文本（git stderr、网络响应等）推断错误码。
    ///
    /// 为什么放在 domain：这是**纯字符串逻辑**，没有 IO，正是领域层该做的事；
    /// 放在命令层会导致它无法被单测覆盖（命令层依赖 Tauri 运行时）。
    ///
    /// 匹配顺序很重要，例如 `token expired` 必须先于 `token`，
    /// `permission denied (publickey)` 必须是认证问题而不是文件权限问题。
    /// 无法判定时返回 [`ErrorCode::Internal`]，绝不上抛——错误分类失败不应再产生错误。
    pub fn classify(raw: &str) -> Self {
        let text = raw.to_ascii_lowercase();
        let has = |needle: &str| text.contains(needle);

        // 说明：这里刻意**不**匹配裸的数字状态码（401/403/404/429）。
        // git 的输出里到处是提交哈希，"404" 出现在某段 SHA 里会把普通错误误判成"资源不存在"，
        // 而错误码一旦被误判，用户看到的就是完全不相干的修复建议。只认带上下文的文字。

        // ---- 认证类（先判断"过期"，再判断"缺少"）----
        if has("token expired")
            || has("expired token")
            || has("credential has expired")
            || has("bad credentials")
            || has("invalid username or password")
        {
            return Self::AuthExpired;
        }
        if has("authentication failed")
            || has("could not read username")
            || has("could not read password")
            || has("permission denied (publickey)")
            || has("terminal prompts disabled")
            || has("no such identity")
            || has("authentication required")
        {
            return Self::AuthRequired;
        }

        // ---- 网络与限流 ----
        if has("rate limit") || has("too many requests") || has("secondary rate") {
            return Self::RateLimited;
        }
        if has("could not resolve host")
            || has("connection refused")
            || has("connection timed out")
            || has("operation timed out")
            || has("network is unreachable")
            || has("tls handshake")
            || has("failed to connect")
        {
            return Self::Network;
        }

        // ---- 仓库状态类 ----
        if has("not a git repository") {
            return Self::PathNotRepo;
        }
        if has("unmerged paths")
            || has("fix conflicts")
            || has("merge conflict")
            || has("conflict (content)")
            || has("automatic merge failed")
        {
            return Self::GitConflict;
        }
        if has("patch does not apply") || has("patch failed") || has("corrupt patch") {
            return Self::PatchApplyFailed;
        }
        if has("hook declined")
            || has("pre-receive hook")
            || has("pre-commit hook")
            || has("commit-msg hook")
            || has("rejected by hook")
        {
            return Self::HookRejected;
        }
        if has("would be overwritten")
            || has("index.lock")
            || has("stale")
            || has("has changed since")
        {
            return Self::PlanStale;
        }
        if has("keyring") || has("secret service") || has("no such interface") {
            return Self::KeyringUnavailable;
        }
        if has("pty") || has("conpty") {
            return Self::PtyUnsupported;
        }
        if has("verify failed") || has("restore failed") {
            return Self::RestoreVerifyFailed;
        }

        // ---- 资源与权限 ----
        if has("permission denied") || has("access is denied") {
            return Self::PermissionDenied;
        }
        if has("not found")
            || has("unknown revision")
            || has("does not exist")
            || has("no such file")
        {
            return Self::NotFound;
        }
        if has("not supported") || has("unsupported") || has("unknown option") {
            return Self::UnsupportedByEngine;
        }
        if has("invalid")
            || has("unexpected argument")
            || has("usage:")
            || has("malformed")
            || has("validation")
        {
            return Self::Validation;
        }

        Self::Internal
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

    /// 用错误码的默认描述创建错误。
    ///
    /// 用途：错误分类器（[`ErrorCode::classify`]）只判断出"是哪一类错"时，
    /// 交给本构造函数补上兜底描述，避免出现空 message。
    pub fn from_code(code: ErrorCode) -> Self {
        Self::new(code, code.default_message())
    }

    /// 该错误在界面上应展示的标题 i18n key。
    pub fn i18n_title_key(&self) -> String {
        self.code.i18n_key()
    }

    /// 当后端未给出针对性建议时，前端应使用的兜底建议 i18n key。
    ///
    /// 返回 `None` 表示后端已经给了具体建议（`hint`），前端应优先展示它。
    pub fn fallback_hint_i18n_key(&self) -> Option<String> {
        if self.hint.is_none() {
            Some(self.code.hint_i18n_key())
        } else {
            None
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

    #[test]
    fn every_code_has_a_default_message_and_i18n_keys() {
        for code in ErrorCode::ALL {
            assert!(
                !code.default_message().is_empty(),
                "{code:?} 缺少默认描述，前端将显示空标题"
            );
            assert!(code.i18n_key().starts_with("errors."));
            assert!(code.hint_i18n_key().ends_with(".hint"));
        }
    }

    #[test]
    fn from_code_fills_default_message() {
        let error = AppError::from_code(ErrorCode::Network);
        assert_eq!(error.message, ErrorCode::Network.default_message());
        assert!(error.retryable, "网络错误默认可重试");
    }

    #[test]
    fn fallback_hint_key_only_when_backend_gave_no_hint() {
        let without = AppError::from_code(ErrorCode::GitConflict);
        assert_eq!(
            without.fallback_hint_i18n_key().as_deref(),
            Some("errors.GIT_CONFLICT.hint")
        );

        let with = AppError::from_code(ErrorCode::GitConflict).with_hint("先解决冲突文件");
        assert_eq!(with.fallback_hint_i18n_key(), None);
    }

    /// 分类器是"把 stderr 变成人话"的第一步，误判的代价是用户看到毫不相干的建议，
    /// 因此每种典型错误都必须有断言。
    #[test]
    fn classify_maps_typical_failures() {
        let cases = [
            (
                "fatal: not a git repository (or any of the parent directories)",
                ErrorCode::PathNotRepo,
            ),
            (
                "error: Unmerged paths:\n  fix conflicts and then commit the result",
                ErrorCode::GitConflict,
            ),
            (
                "fatal: Authentication failed for 'https://example.com/a/b.git/'",
                ErrorCode::AuthRequired,
            ),
            (
                "fatal: could not read Username for 'https://example.com'",
                ErrorCode::AuthRequired,
            ),
            (
                "git@example.com: Permission denied (publickey).",
                ErrorCode::AuthRequired,
            ),
            (
                "remote: HTTP Basic: Access denied\nfatal: Invalid username or password",
                ErrorCode::AuthExpired,
            ),
            (
                "fatal: could not resolve host: example.com",
                ErrorCode::Network,
            ),
            (
                "API rate limit exceeded for installation",
                ErrorCode::RateLimited,
            ),
            ("error: patch does not apply", ErrorCode::PatchApplyFailed),
            (
                "remote: error: hook declined to update refs/heads/main",
                ErrorCode::HookRejected,
            ),
            (
                "error: Your local changes would be overwritten by merge",
                ErrorCode::PlanStale,
            ),
            (
                "fatal: Unable to create '/repo/.git/index.lock': File exists",
                ErrorCode::PlanStale,
            ),
            (
                "error: unable to create keyring entry",
                ErrorCode::KeyringUnavailable,
            ),
            (
                "fatal: unknown revision or path not in the working tree",
                ErrorCode::NotFound,
            ),
            (
                "error: unknown option `--frobnicate'",
                ErrorCode::UnsupportedByEngine,
            ),
            (
                "fatal: invalid reference: refs/heads/",
                ErrorCode::Validation,
            ),
            ("fatal: something entirely unexpected", ErrorCode::Internal),
        ];

        for (raw, expected) in cases {
            assert_eq!(ErrorCode::classify(raw), expected, "分类错误：{raw}");
        }
    }

    /// 提交哈希里出现 "404" 之类的数字时不能被误判（git 输出里到处是 SHA）。
    #[test]
    fn classify_ignores_bare_numeric_status_codes() {
        let raw = "error: could not detach HEAD at 5f4a4042b1d0e9c8";
        assert_eq!(ErrorCode::classify(raw), ErrorCode::Internal);
    }

    #[test]
    fn parse_accepts_as_str_output_and_rejects_unknown() {
        for code in ErrorCode::ALL {
            assert_eq!(ErrorCode::parse(code.as_str()), Some(*code));
        }
        assert_eq!(
            ErrorCode::parse(" git_conflict "),
            Some(ErrorCode::GitConflict)
        );
        assert_eq!(ErrorCode::parse("NOT_A_CODE"), None);
    }
}
