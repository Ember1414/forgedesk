//! 凭据的标识模型：类型、引用（provider/host/login）与 keyring 的 account 命名。
//!
//! # account 的格式为什么必须固定
//!
//! 系统凭据库的条目名一旦变了（比如从 `github.com/octocat` 改成
//! `github:github.com:octocat`），**用户已经保存的凭据会全部变成孤儿**：能查到、
//! 但读不出来，表现为"明明登录过却一直要求重新登录"。因此：
//!
//! - 这里是唯一拼装 account 的地方；
//! - [`CredentialRef::parse`] 必须能反向解析（`list()` 要把 account 还原成引用）；
//! - 因此各段**禁止**包含分隔符 `:`，由 [`CredentialRef::validate`] 强制。

use serde::{Deserialize, Serialize};

use crate::error::CredentialsError;

/// keyring 的 service 名（三平台统一）。
///
/// 固定字面量而不是用 crate 名或产品名变量：它决定了用户在系统凭据管理器里
/// 看到的条目分组，改动等同于"让所有人重新登录一次"。
pub const SERVICE_NAME: &str = "org.forgedesk.app";

/// account 各段的分隔符。
const SEPARATOR: char = ':';

/// 凭据类型。
///
/// 保存类型而不是只存字符串，是为了在界面上说清"这是个人访问令牌还是密码"——
/// 用户排查登录问题时第一句要问的就是这个。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CredentialKind {
    /// 个人访问令牌（GitHub PAT、GitLab token…）。
    Pat,
    /// OAuth 令牌（T4.x 的设备码流程会用到）。
    Oauth,
    /// 用户名 + 密码（自建 git 服务、企业内网）。
    Password,
}

impl CredentialKind {
    /// 稳定字符串形式（写进索引文件、日志与诊断）。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pat => "pat",
            Self::Oauth => "oauth",
            Self::Password => "password",
        }
    }

    /// 从稳定字符串解析（索引文件可能被别的版本写过，未知值必须能安全降级）。
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "pat" => Some(Self::Pat),
            "oauth" => Some(Self::Oauth),
            "password" => Some(Self::Password),
            _ => None,
        }
    }
}

/// 一条凭据的引用：**不含**密文本体。
///
/// 可以安全地进日志、进审计、进错误详情（红线 R8）。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialRef {
    /// 提供方标识（`github` / `gitlab` / `generic`）。
    pub provider: String,
    /// 主机名（`github.com`；自建服务可以是 `git.internal:8443`——端口会带 `:`，
    /// 因此校验时只禁止 provider 与 login 含分隔符，host 允许）。
    pub host: String,
    /// 账号标识（用户名或登录名）。
    pub login: String,
}

impl CredentialRef {
    /// 构造并校验。
    pub fn new(
        provider: impl Into<String>,
        host: impl Into<String>,
        login: impl Into<String>,
    ) -> Result<Self, CredentialsError> {
        let reference = Self {
            provider: provider.into(),
            host: host.into(),
            login: login.into(),
        };
        reference.validate()?;
        Ok(reference)
    }

    /// 校验：非空、且 provider / login 不含分隔符。
    ///
    /// host 允许含 `:`（`host:port`）。代价是解析时 host 与 login 的边界不能靠
    /// `split(':')` 推断，见 [`Self::parse`] 的实现说明。
    pub fn validate(&self) -> Result<(), CredentialsError> {
        if self.provider.trim().is_empty() {
            return Err(CredentialsError::Invalid(
                "provider must not be empty".to_owned(),
            ));
        }
        if self.host.trim().is_empty() {
            return Err(CredentialsError::Invalid(
                "host must not be empty".to_owned(),
            ));
        }
        if self.login.trim().is_empty() {
            return Err(CredentialsError::Invalid(
                "login must not be empty".to_owned(),
            ));
        }
        if self.provider.contains(SEPARATOR) {
            return Err(CredentialsError::Invalid(
                "provider must not contain ':'".to_owned(),
            ));
        }
        if self.login.contains(SEPARATOR) {
            return Err(CredentialsError::Invalid(
                "login must not contain ':'".to_owned(),
            ));
        }
        if self.login.contains('\n') || self.provider.contains('\n') || self.host.contains('\n') {
            return Err(CredentialsError::Invalid(
                "credential reference must not contain newlines".to_owned(),
            ));
        }
        Ok(())
    }

    /// keyring 的 account 名：`<provider>:<host>:<login>`。
    pub fn account(&self) -> String {
        format!(
            "{}{SEPARATOR}{}{SEPARATOR}{}",
            self.provider, self.host, self.login
        )
    }

    /// 从 account 名反向解析。
    ///
    /// 分割方式：**第一个**分隔符切 provider，**最后一个**分隔符切 login，
    /// 中间全部属于 host。这样 `host:port` 与带点的域名都能原样还原；
    /// provider / login 被校验为不含分隔符，因此不会歧义。
    pub fn parse(account: &str) -> Option<Self> {
        let first = account.find(SEPARATOR)?;
        let last = account.rfind(SEPARATOR)?;
        if first == last {
            // 只有一段分隔符：缺 host 或 login，不是我们写出去的格式
            return None;
        }
        let reference = Self {
            provider: account[..first].to_owned(),
            host: account[first + 1..last].to_owned(),
            login: account[last + 1..].to_owned(),
        };
        reference.validate().ok()?;
        Some(reference)
    }

    /// 展示用字符串（错误提示、审计）；与 account 相同，便于用户去系统凭据管理器里核对。
    pub fn display(&self) -> String {
        self.account()
    }
}

/// 一条凭据的元数据（**不含**密文），用于 `list()` 与设置页展示。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialMeta {
    /// 凭据引用。
    pub key: CredentialRef,
    /// 凭据类型。
    pub kind: CredentialKind,
    /// 写入时间（Unix 毫秒）；界面按它排序与显示"何时保存的"。
    pub created_at_ms: i64,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn github() -> CredentialRef {
        CredentialRef::new("github", "github.com", "octocat").expect("valid reference")
    }

    #[test]
    fn the_account_name_is_the_documented_provider_host_login_triple() {
        assert_eq!(github().account(), "github:github.com:octocat");
        assert_eq!(SERVICE_NAME, "org.forgedesk.app");
    }

    #[test]
    fn parsing_an_account_name_round_trips_including_hosts_with_ports() {
        for account in [
            "github:github.com:octocat",
            "generic:git.internal:8443:alice",
            "gitlab:gitlab.example.com:team/lead",
        ] {
            let parsed = CredentialRef::parse(account).expect("parsable");
            assert_eq!(parsed.account(), account, "{account}");
        }
    }

    #[test]
    fn a_host_with_a_port_keeps_the_port_in_the_host_segment() {
        let reference = CredentialRef::parse("generic:git.internal:8443:alice").expect("parsable");

        assert_eq!(reference.host, "git.internal:8443");
        assert_eq!(reference.login, "alice");
    }

    #[test]
    fn malformed_accounts_are_rejected_instead_of_guessed() {
        // 少一段（缺 login）→ 不能猜，宁可返回 None 让索引里那条被忽略
        assert!(CredentialRef::parse("github:github.com").is_none());
        assert!(CredentialRef::parse("github").is_none());
        assert!(CredentialRef::parse("github::octocat").is_none());
        assert!(CredentialRef::parse(":github.com:octocat").is_none());
    }

    #[test]
    fn references_that_would_break_the_account_format_are_rejected_up_front() {
        // provider/login 含分隔符会让 account 无法反解 —— 必须在写入前拦住
        assert!(CredentialRef::new("git:hub", "github.com", "octocat").is_err());
        assert!(CredentialRef::new("github", "github.com", "octo:cat").is_err());
        assert!(CredentialRef::new("github", "github.com", "  ").is_err());
        assert!(CredentialRef::new("", "github.com", "octocat").is_err());
        assert!(CredentialRef::new("github", "github.com", "octo\ncat").is_err());
    }

    #[test]
    fn credential_kinds_round_trip_through_their_stable_strings() {
        for kind in [
            CredentialKind::Pat,
            CredentialKind::Oauth,
            CredentialKind::Password,
        ] {
            assert_eq!(CredentialKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(CredentialKind::parse("bearer"), None);
        assert_eq!(CredentialKind::Pat.as_str(), "pat");
    }

    #[test]
    fn credential_metadata_serialises_its_reference_as_camel_case() {
        let meta = CredentialMeta {
            key: github(),
            kind: CredentialKind::Pat,
            created_at_ms: 1_700_000_000_000,
        };
        let json = serde_json::to_value(&meta).expect("serialisable");

        assert_eq!(json["kind"], "pat");
        assert_eq!(json["createdAtMs"], 1_700_000_000_000_i64);
        assert_eq!(json["key"]["login"], "octocat");
    }
}
