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
    /// 继续操作（continue）前仍有未解决的冲突文件。
    ///
    /// 为什么与 [`Self::GitConflict`] 分开：`GIT_CONFLICT` 说的是"仓库里有冲突"
    /// （查询/探测时），`CONFLICT_UNRESOLVED` 说的是"你想继续，但这些文件还没解决"
    /// —— 它携带未解决文件清单（`hint`），界面要指着文件名让用户逐个处理。
    ConflictUnresolved,
    /// 需要认证：未登录、未提供凭据或凭据不足。
    AuthRequired,
    /// 凭据已过期或被撤销，需要重新登录。
    AuthExpired,
    /// SSH 主机密钥未被信任（`Host key verification failed`）。
    ///
    /// 为什么与 [`Self::AuthRequired`] 分开：这两件事的**修复动作完全不同**——
    /// 前者要去核对/信任主机指纹（可能正是中间人攻击的信号，必须让用户自己确认），
    /// 后者是"你还没登录"。混在一个码里，界面只能给出"重新登录"，
    /// 而用户照着做一百遍也不会通过主机密钥校验。
    SshHostKeyUnverified,
    /// SSH 公钥被服务端拒绝（`Permission denied (publickey)`）。
    ///
    /// 与 [`Self::AuthRequired`]（HTTPS 缺凭据）分开：这里的排查方向是
    /// "公钥有没有加到服务端、agent 里有没有加载、是不是用错了 key"。
    SshKeyRejected,
    /// 服务端 TLS 证书校验失败（`SSL certificate problem`）。
    ///
    /// 单独成码的原因：这与"网络不通"的排查方向相反——连接是通的，
    /// 是证书链/自签名/企业中间人代理的问题。归到 NETWORK 会让用户去查网络。
    TlsCertificateRejected,
    /// 代理不可用或拒绝连接（`Proxy CONNECT aborted`、HTTP 407）。
    ///
    /// 单独成码的原因：代理失败时"网络是通的"（能连上代理），
    /// 用户需要去检查代理配置，而不是查自己的网络。
    ProxyFailed,
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
    /// push 被远端以非快进拒绝（本地落后；只有先拉取或 force-with-lease 两条路）。
    PushRejected,
    /// 没有可提交的内容（索引为空，或索引与 HEAD 相同）。
    ///
    /// 为什么单独一个码：这是用户**站在提交按钮前**最常遇到的情况，
    /// 界面要把它变成一句可操作的话（"先暂存一些改动"）。归到通用的 `VALIDATION`
    /// 会让用户跑去检查自己写的提交信息——而问题根本不在那里。
    EmptyCommit,
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
    ///
    /// **新增错误码必须同时加进这里**：漏掉不会有编译错误，
    /// 但 `ErrorCode::parse` 会认不出它（T2.5 加 `PUSH_REJECTED` 时就漏过一次），
    /// 而前端的 i18n 覆盖检查会因此少要求一条文案。
    /// [`tests::all_lists_every_variant_exactly_once`] 用穷举 match + 数量断言盯着这件事。
    pub const ALL: &'static [Self] = &[
        Self::PathNotRepo,
        Self::GitConflict,
        Self::ConflictUnresolved,
        Self::AuthRequired,
        Self::AuthExpired,
        Self::SshHostKeyUnverified,
        Self::SshKeyRejected,
        Self::TlsCertificateRejected,
        Self::ProxyFailed,
        Self::PermissionDenied,
        Self::NotFound,
        Self::Validation,
        Self::Network,
        Self::RateLimited,
        Self::PatchApplyFailed,
        Self::PlanStale,
        Self::HookRejected,
        Self::PushRejected,
        Self::EmptyCommit,
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
            Self::ConflictUnresolved => "CONFLICT_UNRESOLVED",
            Self::AuthRequired => "AUTH_REQUIRED",
            Self::AuthExpired => "AUTH_EXPIRED",
            Self::SshHostKeyUnverified => "SSH_HOST_KEY_UNVERIFIED",
            Self::SshKeyRejected => "SSH_KEY_REJECTED",
            Self::TlsCertificateRejected => "TLS_CERTIFICATE_REJECTED",
            Self::ProxyFailed => "PROXY_FAILED",
            Self::PermissionDenied => "PERMISSION_DENIED",
            Self::NotFound => "NOT_FOUND",
            Self::Validation => "VALIDATION",
            Self::Network => "NETWORK",
            Self::RateLimited => "RATE_LIMITED",
            Self::PatchApplyFailed => "PATCH_APPLY_FAILED",
            Self::PlanStale => "PLAN_STALE",
            Self::HookRejected => "HOOK_REJECTED",
            Self::PushRejected => "PUSH_REJECTED",
            Self::EmptyCommit => "EMPTY_COMMIT",
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
            Self::ConflictUnresolved => {
                "the operation cannot continue: some conflicts are still unresolved"
            }
            Self::AuthRequired => "authentication is required",
            Self::AuthExpired => "the stored credential has expired or was revoked",
            Self::SshHostKeyUnverified => {
                "the SSH host key is not trusted (host key verification failed)"
            }
            Self::SshKeyRejected => {
                "the SSH key was rejected by the remote (publickey authentication failed)"
            }
            Self::TlsCertificateRejected => "the server TLS certificate could not be verified",
            Self::ProxyFailed => "the proxy refused or could not complete the connection",
            Self::PermissionDenied => "permission denied",
            Self::NotFound => "the requested resource does not exist",
            Self::Validation => "invalid input",
            Self::Network => "network request failed",
            Self::RateLimited => "the host rate limit was reached",
            Self::PatchApplyFailed => "the patch could not be applied",
            Self::PlanStale => "the plan is stale: the repository changed after it was built",
            Self::HookRejected => "a Git hook rejected the operation",
            Self::PushRejected => {
                "the push was rejected because the remote has work you do not have"
            }
            Self::EmptyCommit => "there is nothing staged to commit",
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
    ///
    /// `PROXY_FAILED` 算可重试：代理进程重启、切换网络后同一个操作**确实可能成功**。
    /// 而 SSH/TLS 三类不可重试——它们是配置/信任问题，原样重试一百次结果一样。
    pub const fn default_retryable(self) -> bool {
        matches!(
            self,
            Self::Network | Self::RateLimited | Self::PlanStale | Self::ProxyFailed
        )
    }

    /// 该错误码默认可以给出的修复动作。
    ///
    /// 为什么放在 domain：`FixAction` 是 IPC 契约的一部分，而"哪类错误有哪几条出路"
    /// 是领域知识（与哪个命令实现它无关）。命令层只需要实现这些动作指向的命令。
    ///
    /// `command` 必须是**真实存在**的 Tauri 命令名（前端会照它 invoke）。
    /// 还没实现的出口——例如 T4.4 的账号登录、T6.8 的 SSH 主机指纹信任——
    /// 宁可不给按钮：一个点了没反应的按钮比没有按钮更让人困惑。
    /// 需要参数的出口（例如"测试连接"要知道测哪个远端）由调用方用
    /// [`FixAction::with_args`] 补齐，这里只给不带参数的骨架。
    #[must_use]
    pub fn default_actions(self) -> Vec<FixAction> {
        match self {
            Self::SshHostKeyUnverified
            | Self::SshKeyRejected
            | Self::TlsCertificateRejected
            | Self::ProxyFailed => vec![FixAction::new(
                "test-connection",
                "errors:actions.testConnection",
                "credential_test_remote",
            )],
            _ => Vec::new(),
        }
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

        // ---- SSH / TLS / 代理（必须先于通用认证与网络规则）----
        //
        // 顺序理由：`Permission denied (publickey)` 里也含 "permission denied"，
        // 而 TLS 证书失败的原文里常带 "unable to access"（看起来像网络问题）。
        // 先认最具体的那一类，才能给用户**方向正确的**排查建议：
        // SSH key、主机指纹、证书、代理这四件事的修法互不相同。
        if has("host key verification failed") || has("remote host identification has changed") {
            return Self::SshHostKeyUnverified;
        }
        if has("permission denied (publickey)")
            || has("no such identity")
            || has("no supported authentication methods")
            || has("publickey authentication failed")
        {
            return Self::SshKeyRejected;
        }
        if has("ssl certificate problem")
            || has("certificate verify failed")
            || has("server certificate verification failed")
            || has("unable to get local issuer certificate")
            || has("self-signed certificate")
            || has("certificate has expired")
        {
            return Self::TlsCertificateRejected;
        }
        if has("proxy connect aborted")
            || has("http code 407")
            || has("could not connect to proxy")
            || has("proxy error")
            || has("received http code 407")
        {
            return Self::ProxyFailed;
        }

        // ---- 认证类（先判断"过期"，再判断"缺少"）----
        if has("token expired")
            || has("expired token")
            || has("credential has expired")
            || has("bad credentials")
            || has("invalid username or password")
        {
            return Self::AuthExpired;
        }
        // 注意：`permission denied (publickey)` 与 `no such identity` 已在上面归入
        // SSH_KEY_REJECTED，这里不再重复（重复会让规则顺序变得难以推理）。
        if has("authentication failed")
            || has("could not read username")
            || has("could not read password")
            || has("terminal prompts disabled")
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
        if has("non-fast-forward") || has("fetch first") || has("behind its remote-tracking branch")
        {
            return Self::PushRejected;
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
                "remote: HTTP Basic: Access denied\nfatal: Invalid username or password",
                ErrorCode::AuthExpired,
            ),
            // ---- T2.7：SSH / TLS / 代理（样本取自真实的 git 输出）----
            (
                "git@github.com: Permission denied (publickey).\nfatal: Could not read from remote repository.",
                ErrorCode::SshKeyRejected,
            ),
            (
                "no such identity: /home/u/.ssh/id_ed25519: No such file or directory",
                ErrorCode::SshKeyRejected,
            ),
            (
                "git@example.com: no supported authentication methods available (server sent: publickey)",
                ErrorCode::SshKeyRejected,
            ),
            (
                "Host key verification failed.\nfatal: Could not read from remote repository.",
                ErrorCode::SshHostKeyUnverified,
            ),
            (
                "@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@\n\
                 @    WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!     @\n\
                 @@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@",
                ErrorCode::SshHostKeyUnverified,
            ),
            (
                "fatal: unable to access 'https://example.com/a.git/': SSL certificate problem: self-signed certificate",
                ErrorCode::TlsCertificateRejected,
            ),
            (
                "fatal: unable to access 'https://example.com/a.git/': SSL certificate problem: unable to get local issuer certificate",
                ErrorCode::TlsCertificateRejected,
            ),
            (
                "fatal: unable to access 'https://example.com/a.git/': server certificate verification failed. CAfile: none CRLfile: none",
                ErrorCode::TlsCertificateRejected,
            ),
            (
                "fatal: unable to access 'https://example.com/a.git/': Received HTTP code 407 from proxy after CONNECT",
                ErrorCode::ProxyFailed,
            ),
            (
                "error: Proxy CONNECT aborted",
                ErrorCode::ProxyFailed,
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

    /// 穷举 match：新增错误码时编译器会强制作者在这里补一行，
    /// 从而逼着他顺手检查 `ALL`（`ALL` 漏项不会有编译错误，这是 T2.5 踩过的坑）。
    fn describe(code: ErrorCode) -> &'static str {
        match code {
            ErrorCode::PathNotRepo => "PATH_NOT_REPO",
            ErrorCode::GitConflict => "GIT_CONFLICT",
            ErrorCode::ConflictUnresolved => "CONFLICT_UNRESOLVED",
            ErrorCode::AuthRequired => "AUTH_REQUIRED",
            ErrorCode::AuthExpired => "AUTH_EXPIRED",
            ErrorCode::SshHostKeyUnverified => "SSH_HOST_KEY_UNVERIFIED",
            ErrorCode::SshKeyRejected => "SSH_KEY_REJECTED",
            ErrorCode::TlsCertificateRejected => "TLS_CERTIFICATE_REJECTED",
            ErrorCode::ProxyFailed => "PROXY_FAILED",
            ErrorCode::PermissionDenied => "PERMISSION_DENIED",
            ErrorCode::NotFound => "NOT_FOUND",
            ErrorCode::Validation => "VALIDATION",
            ErrorCode::Network => "NETWORK",
            ErrorCode::RateLimited => "RATE_LIMITED",
            ErrorCode::PatchApplyFailed => "PATCH_APPLY_FAILED",
            ErrorCode::PlanStale => "PLAN_STALE",
            ErrorCode::HookRejected => "HOOK_REJECTED",
            ErrorCode::PushRejected => "PUSH_REJECTED",
            ErrorCode::EmptyCommit => "EMPTY_COMMIT",
            ErrorCode::RestoreVerifyFailed => "RESTORE_VERIFY_FAILED",
            ErrorCode::KeyringUnavailable => "KEYRING_UNAVAILABLE",
            ErrorCode::Storage => "STORAGE",
            ErrorCode::PtyUnsupported => "PTY_UNSUPPORTED",
            ErrorCode::UnsupportedByEngine => "UNSUPPORTED_BY_ENGINE",
            ErrorCode::Cancelled => "CANCELLED",
            ErrorCode::Internal => "INTERNAL",
        }
    }

    #[test]
    fn all_lists_every_variant_exactly_once() {
        // 26 个变体（新增时必须同时改 ALL 与本断言里的数字）
        assert_eq!(ErrorCode::ALL.len(), 26);

        let mut seen = std::collections::BTreeSet::new();
        for code in ErrorCode::ALL {
            assert_eq!(describe(*code), code.as_str(), "as_str 与穷举表不一致");
            assert!(
                seen.insert(code.as_str()),
                "{} 在 ALL 里重复",
                code.as_str()
            );
            // parse 必须认得出 ALL 里的每一个：认不出意味着 IPC 参数收敛会失败
            assert_eq!(ErrorCode::parse(code.as_str()), Some(*code));
        }
    }

    #[test]
    fn the_ssh_and_tls_codes_offer_a_connection_test_but_no_dead_buttons() {
        for code in [
            ErrorCode::SshHostKeyUnverified,
            ErrorCode::SshKeyRejected,
            ErrorCode::TlsCertificateRejected,
            ErrorCode::ProxyFailed,
        ] {
            let actions = code.default_actions();
            assert_eq!(actions.len(), 1, "{code:?}");
            assert_eq!(actions[0].command, "credential_test_remote");
            assert_eq!(actions[0].label_key, "errors:actions.testConnection");
        }

        // 还没实现的出口（T4.4 重新登录、T6.8 信任主机指纹）不给按钮
        assert!(ErrorCode::AuthRequired.default_actions().is_empty());
        assert!(ErrorCode::SshHostKeyUnverified.default_actions()[0]
            .args
            .is_none());
    }

    #[test]
    fn only_the_proxy_failure_is_retryable_among_the_new_codes() {
        assert!(ErrorCode::ProxyFailed.default_retryable());
        // SSH/TLS 是配置与信任问题：原样重试一百次结果一样
        assert!(!ErrorCode::SshHostKeyUnverified.default_retryable());
        assert!(!ErrorCode::SshKeyRejected.default_retryable());
        assert!(!ErrorCode::TlsCertificateRejected.default_retryable());
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
