//! 凭据层的错误类型与到 [`AppError`] 的映射。
//!
//! # 为什么不直接返回 AppError
//!
//! 这一层需要区分几件上层不必关心、但对**回退决策**很关键的事：
//! "没存过这条凭据"（正常，调用方应回退到别的后端或提示登录）、"系统凭据库不可用"
//! （要提示装 Secret Service 或改用加密文件）、"口令不对"（重试即可）。
//! 把它们压成 IPC 错误码会丢掉这些差别。映射只发生一次：命令层调 [`CredentialsError::to_app_error`]。
//!
//! # 红线 R8
//!
//! 本模块的所有 Display / message / detail / hint **都不含密文或明文**：
//! 出错信息里最多出现 keyring 的 account 名（`<provider>:<host>:<login>`），
//! 而那是用户自己的账号标识，不是秘密。

use forgedesk_domain::{AppError, ErrorCode};

/// 凭据层错误。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CredentialsError {
    /// 没有为该引用存过凭据（不是故障）。
    #[error("no credential is stored for {0}")]
    NotFound(String),

    /// 系统凭据库不可用（Linux 上没有 Secret Service、macOS 拒绝访问等）。
    #[error("the system credential store is unavailable: {0}")]
    KeyringUnavailable(String),

    /// 加密保险库打不开：口令错误，或文件被改动过（AES-GCM 无法区分这两者）。
    #[error(
        "the encrypted vault could not be unlocked: wrong passphrase or the file was modified"
    )]
    VaultLocked,

    /// 加密保险库文件还不存在。
    #[error("the encrypted vault does not exist: {0}")]
    VaultMissing(String),

    /// 加密保险库的结构不可解析（magic / 版本 / 参数非法）。
    #[error("the encrypted vault is malformed: {0}")]
    VaultMalformed(String),

    /// 本地 IO 失败（路径已脱敏，不含口令）。
    #[error("credential storage IO failed: {0}")]
    Io(String),

    /// 输入不合法（空的 host/login、含分隔符的 provider 等）。
    #[error("invalid credential reference: {0}")]
    Invalid(String),
}

impl CredentialsError {
    /// 对应的稳定错误码（前端据此走 i18n）。
    pub fn code(&self) -> ErrorCode {
        match self {
            // "没存过"不是故障，但 IPC 上必须给出一个码：用 NOT_FOUND 让前端提示"还没有保存凭据"
            Self::NotFound(_) => ErrorCode::NotFound,
            Self::KeyringUnavailable(_) => ErrorCode::KeyringUnavailable,
            // 口令错 / 保险库损坏：本地数据打不开，用户能做的只有重新输入或重建
            Self::VaultLocked | Self::VaultMalformed(_) => ErrorCode::Storage,
            Self::VaultMissing(_) => ErrorCode::NotFound,
            Self::Io(_) => ErrorCode::Storage,
            Self::Invalid(_) => ErrorCode::Validation,
        }
    }

    /// 转成 IPC 错误。
    ///
    /// `hint` 里**只放数据**（路径、account 名、平台原因），不放建议性文字：
    /// 建议由前端按 `code` 走 i18n，否则英文界面里会冒出中文
    /// （见 docs/CODING_STYLE.md §2.1「hint 只放数据」）。
    pub fn to_app_error(&self) -> AppError {
        let error = AppError::new(self.code(), self.to_string());
        match self {
            Self::NotFound(account) | Self::Invalid(account) => error.with_hint(account.clone()),
            Self::KeyringUnavailable(reason) => error.with_hint(reason.clone()),
            Self::VaultMissing(path) | Self::VaultMalformed(path) => error.with_hint(path.clone()),
            Self::Io(detail) => error.with_detail(detail.clone()),
            Self::VaultLocked => error,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_credential_is_reported_as_not_found_with_the_account_as_data() {
        let error =
            CredentialsError::NotFound("github:github.com:octocat".to_owned()).to_app_error();

        assert_eq!(error.code, ErrorCode::NotFound);
        assert_eq!(error.hint.as_deref(), Some("github:github.com:octocat"));
        assert!(!error.retryable);
    }

    #[test]
    fn an_unavailable_keyring_maps_to_its_own_code_so_the_ui_can_suggest_the_fallback() {
        let error = CredentialsError::KeyringUnavailable("dbus: no such service".to_owned());
        let app = error.to_app_error();

        assert_eq!(app.code, ErrorCode::KeyringUnavailable);
        assert_eq!(app.hint.as_deref(), Some("dbus: no such service"));
        // 不可用是环境问题，重试没用：用户必须先装 Secret Service 或改用回退方案
        assert!(!app.retryable);
    }

    #[test]
    fn a_locked_vault_is_local_storage_failure_not_an_authentication_problem() {
        // 口令错≠凭据失效：把它报成 AUTH_* 会让界面提示"重新登录"，
        // 用户会去改远端账号，而真正该做的是重新输入保险库口令
        assert_eq!(CredentialsError::VaultLocked.code(), ErrorCode::Storage);
        assert_eq!(
            CredentialsError::VaultMissing("C:/v".to_owned()).code(),
            ErrorCode::NotFound
        );
    }

    #[test]
    fn error_messages_never_contain_a_secret_placeholder() {
        // 反向断言：错误文案里不允许出现我们约定用来标记密文的字样，
        // 避免以后有人图省事把 secret 拼进 message
        let texts = [
            CredentialsError::NotFound("a".to_owned()).to_string(),
            CredentialsError::KeyringUnavailable("b".to_owned()).to_string(),
            CredentialsError::VaultLocked.to_string(),
            CredentialsError::Io("c".to_owned()).to_string(),
        ];
        for text in texts {
            assert!(!text.contains("secret="), "{text}");
            assert!(!text.contains("password="), "{text}");
        }
    }
}
