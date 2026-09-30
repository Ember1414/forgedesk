//! OAuth Device Flow 与 PAT 校验的数据类型。
//!
//! # 为什么类型在这里、实现在 `github.rs`
//!
//! 这些类型是 `AuthFlow` trait 签名的一部分（services 层要通过 trait
//! 使用它们），与"哪个 provider 怎么发 HTTP"无关。单独成模块让
//! trait 的方法签名稳定，实现细节留在各 provider 文件里。
//!
//! # 交互流程（GitHub Device Flow，RFC 8628）
//!
//! ```text
//! start_device_flow  →  DeviceFlowStart { user_code, verification_uri, … }
//!   （用户在浏览器输码）
//! poll_device_flow   →  Pending | SlowDown | Authorized | Expired | Denied
//! ```
//!
//! `device_code` 是秘密（拿着它才能换令牌），用 `SecretString` 包裹，
//! `Debug` 输出自动脱敏；`user_code` 是给用户看的，不是秘密。

use secrecy::SecretString;
use serde::{Deserialize, Serialize};

/// Device Flow 的启动结果。
///
/// 刻意**不实现** Serialize/Deserialize：`device_code` 是秘密，
/// 这个值只活在 Rust 侧的轮询循环里（红线 R8）；UI 只拿
/// `user_code` / `verification_uri`（由 services 层拆出非秘密字段）。
/// 编译期就堵死"顺手把它发给前端"的可能。
#[derive(Debug, Clone)]
pub struct DeviceFlowStart {
    /// 轮询时回传的设备码（秘密，禁止展示给用户/写日志）。
    pub device_code: SecretString,
    /// 用户要输入的码（如 `WDJB-MJTK`），UI 大字展示 + 一键复制。
    pub user_code: String,
    /// 用户要去输码的页面（`https://github.com/login/device`）。
    pub verification_uri: String,
    /// 携带 user_code 的直链（存在时 UI 可"打开即填"）。
    pub verification_uri_complete: Option<String>,
    /// 整个流程的过期秒数（超时后必须重新 start）。
    pub expires_in_secs: u64,
    /// 轮询的最小间隔秒数（GitHub 默认 5s；收到 slow_down 后 +5s）。
    pub interval_secs: u64,
}

/// 一次轮询的结果。
///
/// 与 [`DeviceFlowStart`] 同理：`token` 是秘密，本枚举不可序列化。
#[derive(Debug, Clone)]
pub enum DeviceFlowPoll {
    /// 用户已授权：令牌到手（`scope` 是实际授予的作用域，可能与请求不同）。
    Authorized {
        /// 访问令牌（秘密；下一步交给 services 层写入凭据库）。
        token: SecretString,
        /// 实际授予的作用域（空格分隔），可能少于请求的作用域。
        scope: Option<String>,
    },
    /// 用户尚未输码：继续按 interval 轮询。
    Pending,
    /// 轮询过快：调用方把间隔 +5s 后继续。
    SlowDown,
    /// 设备码已过期：整个流程重新开始。
    Expired,
    /// 用户拒绝了授权：不要重试，直接结束向导。
    Denied,
}

/// PAT / OAuth 令牌校验通过后的账号信息（来自 `/user` 与响应头）。
///
/// 不含秘密，可以（也需要）跨 IPC 发给账号 UI。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifiedAccount {
    /// 登录名（`octocat`），账号在凭据库里的自然键。
    pub login: String,
    /// 头像地址（可能缺失）。
    pub avatar_url: Option<String>,
    /// 令牌实际拥有的作用域（`x-oauth-scopes` 头，逗号分隔；细粒度 PAT 为空）。
    pub scopes: Vec<String>,
}

/// Device Flow 默认请求的作用域。
///
/// M4 交付物（PR / Issue / Actions / 仓库管理）所需的并集；
/// 具体调用可传更小的集合——GitHub 会把实际授予的作用域回传，
/// 账号 UI 据此提示"权限不足"而不是默默失败。
pub const DEFAULT_SCOPES: &str = "repo read:org workflow";

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{DeviceFlowPoll, DeviceFlowStart};
    use secrecy::ExposeSecret;

    /// 红线 R8：device_code 与换取到的令牌不得出现在 Debug 输出里。
    #[test]
    fn debug_output_never_contains_secrets() {
        let start = DeviceFlowStart {
            device_code: secrecy::SecretString::from("super-secret-device-code"),
            user_code: "WDJB-MJTK".to_owned(),
            verification_uri: "https://github.com/login/device".to_owned(),
            verification_uri_complete: None,
            expires_in_secs: 900,
            interval_secs: 5,
        };
        let dumped = format!("{start:?}");
        assert!(!dumped.contains("super-secret-device-code"), "{dumped}");
        assert!(dumped.contains("WDJB-MJTK"), "user_code 不是秘密，要可见");

        let poll = DeviceFlowPoll::Authorized {
            token: secrecy::SecretString::from("gho_realtoken"),
            scope: Some("repo".to_owned()),
        };
        assert!(!format!("{poll:?}").contains("gho_realtoken"));
    }

    #[test]
    fn secrets_round_trip_through_expose() {
        let start = DeviceFlowStart {
            device_code: secrecy::SecretString::from("abc"),
            user_code: "U".to_owned(),
            verification_uri: "u".to_owned(),
            verification_uri_complete: None,
            expires_in_secs: 1,
            interval_secs: 1,
        };
        assert_eq!(start.device_code.expose_secret(), "abc");
    }
}
