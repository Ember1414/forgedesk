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
use std::time::Duration;
use tokio_util::sync::CancellationToken;

use forgedesk_domain::{AppError, ErrorCode};

use crate::traits::AuthFlow;

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

/// 授权成功后的产物：令牌只在内存里流转，落地存储是 services 层的事。
#[derive(Debug)]
pub struct AuthorizedLogin {
    /// 访问令牌（秘密；下一步由 services 写入凭据库，T4.4）。
    pub token: SecretString,
    /// 实际授予的作用域（空格分隔）。
    pub scope: Option<String>,
}

/// 轮询循环的可调参数（真实用户走 [`Self::DEFAULT`]）。
#[derive(Debug, Clone, Copy)]
pub struct PollOptions {
    /// 收到 `slow_down` 后追加到间隔的时长（RFC 8628 §3.5 规定 5s）。
    pub slow_down_penalty: Duration,
}

impl PollOptions {
    /// RFC 8628 推荐值。
    pub const DEFAULT: Self = Self {
        slow_down_penalty: Duration::from_secs(5),
    };
}

impl Default for PollOptions {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// 驱动 Device Flow 的轮询循环直到授权完成或流程终止。
///
/// # 为什么循环在这里而不在 services
///
/// 轮询节奏（interval、slow_down +5s、过期判定）是 **RFC 8628 的协议语义**，
/// 不是业务编排；放 provider 里让所有 `AuthFlow` 实现共享同一套节奏，
/// services（T4.3 登录向导）只管启动/取消与结果落地。取消令牌与
/// `jobs` crate 的同源（tokio-util），登录向导把 jobId 的令牌直接传进来。
///
/// 错误码约定：用户取消 → `CANCELLED`；流程/设备码过期 → `AUTH_EXPIRED`
/// （前端把它导向"重新登录"，正是正确出路）；用户拒绝 → `AUTH_REQUIRED`
/// （同样导向重新发起，但 hint 会说明是拒绝而非失败）。
pub async fn poll_until_authorized(
    auth: &dyn AuthFlow,
    flow: &DeviceFlowStart,
    cancel: &CancellationToken,
    options: PollOptions,
) -> Result<AuthorizedLogin, AppError> {
    let deadline = std::time::Instant::now() + Duration::from_secs(flow.expires_in_secs);
    let mut interval = Duration::from_secs(flow.interval_secs);

    loop {
        tokio::select! {
            () = cancel.cancelled() => {
                return Err(AppError::new(ErrorCode::Cancelled, "device flow polling was cancelled"));
            }
            () = tokio::time::sleep(interval) => {}
        }

        if std::time::Instant::now() >= deadline {
            return Err(AppError::new(
                ErrorCode::AuthExpired,
                "the device flow expired; start the sign-in again",
            ));
        }

        match auth.poll_device_flow(flow).await? {
            DeviceFlowPoll::Authorized { token, scope } => {
                return Ok(AuthorizedLogin { token, scope });
            }
            DeviceFlowPoll::Pending => {}
            DeviceFlowPoll::SlowDown => {
                interval += options.slow_down_penalty;
            }
            DeviceFlowPoll::Expired => {
                return Err(AppError::new(
                    ErrorCode::AuthExpired,
                    "the device code expired; start the sign-in again",
                ));
            }
            DeviceFlowPoll::Denied => {
                return Err(AppError::new(
                    ErrorCode::AuthRequired,
                    "the authorization request was denied by the user",
                ));
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{
        poll_until_authorized, AuthorizedLogin, DeviceFlowPoll, DeviceFlowStart, PollOptions,
    };
    use crate::traits::AuthFlow;
    use forgedesk_domain::{AppError, ErrorCode};
    use secrecy::{ExposeSecret, SecretString};
    use std::collections::VecDeque;
    use std::sync::Mutex;
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

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

    /// 轮询循环逻辑测试不需要 HTTP：一个按脚本应答的 MockAuth 就够了。
    /// （poll_device_flow 本身的 HTTP 行为在 github.rs 里用 wiremock 覆盖。）
    struct MockAuth {
        responses: Mutex<VecDeque<Result<DeviceFlowPoll, AppError>>>,
        polls: Mutex<usize>,
    }

    impl MockAuth {
        fn new(responses: Vec<Result<DeviceFlowPoll, AppError>>) -> Self {
            Self {
                responses: Mutex::new(responses.into()),
                polls: Mutex::new(0),
            }
        }

        fn poll_count(&self) -> usize {
            *self.polls.lock().unwrap()
        }
    }

    #[async_trait::async_trait]
    impl AuthFlow for MockAuth {
        async fn start_device_flow(&self, _scopes: &[&str]) -> Result<DeviceFlowStart, AppError> {
            Err(AppError::new(ErrorCode::Internal, "not used in this test"))
        }

        async fn poll_device_flow(
            &self,
            _flow: &DeviceFlowStart,
        ) -> Result<DeviceFlowPoll, AppError> {
            *self.polls.lock().unwrap() += 1;
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("test script ran dry")
        }

        async fn verify_pat(
            &self,
            _token: SecretString,
        ) -> Result<super::VerifiedAccount, AppError> {
            Err(AppError::new(ErrorCode::Internal, "not used in this test"))
        }
    }

    fn flow(expires_in_secs: u64, interval_secs: u64) -> DeviceFlowStart {
        DeviceFlowStart {
            device_code: SecretString::from("dc"),
            user_code: "U".to_owned(),
            verification_uri: "u".to_owned(),
            verification_uri_complete: None,
            expires_in_secs,
            interval_secs,
        }
    }

    fn authorized(token: &str) -> Result<DeviceFlowPoll, AppError> {
        Ok(DeviceFlowPoll::Authorized {
            token: SecretString::from(token.to_owned()),
            scope: Some("repo".to_owned()),
        })
    }

    #[tokio::test]
    async fn polling_keeps_going_through_pending_and_returns_the_token() {
        let auth = MockAuth::new(vec![
            Ok(DeviceFlowPoll::Pending),
            Ok(DeviceFlowPoll::Pending),
            authorized("gho_done"),
        ]);
        let cancel = CancellationToken::new();

        let login = poll_until_authorized(&auth, &flow(900, 0), &cancel, PollOptions::default())
            .await
            .unwrap();

        let AuthorizedLogin { token, scope } = login;
        assert_eq!(token.expose_secret(), "gho_done");
        assert_eq!(scope.as_deref(), Some("repo"));
        assert_eq!(auth.poll_count(), 3);
    }

    #[tokio::test]
    async fn slow_down_extends_the_interval_before_the_next_poll() {
        let auth = MockAuth::new(vec![Ok(DeviceFlowPoll::SlowDown), authorized("gho_done")]);

        let started = std::time::Instant::now();
        let login = poll_until_authorized(
            &auth,
            &flow(900, 0),
            &CancellationToken::new(),
            PollOptions {
                slow_down_penalty: Duration::from_millis(80),
            },
        )
        .await
        .unwrap();

        assert_eq!(login.token.expose_secret(), "gho_done");
        // sleep 只保证"至少"：50~80ms 的下限断言是确定性的
        assert!(started.elapsed() >= Duration::from_millis(79));
    }

    #[tokio::test]
    async fn a_user_denial_maps_to_auth_required_and_stops_polling() {
        let auth = MockAuth::new(vec![Ok(DeviceFlowPoll::Denied), authorized("never")]);

        let error = poll_until_authorized(
            &auth,
            &flow(900, 0),
            &CancellationToken::new(),
            PollOptions::default(),
        )
        .await
        .unwrap_err();

        assert_eq!(error.code, ErrorCode::AuthRequired);
        assert_eq!(auth.poll_count(), 1, "拒绝后不得继续轮询");
    }

    #[tokio::test]
    async fn an_expired_device_code_maps_to_auth_expired() {
        let auth = MockAuth::new(vec![Ok(DeviceFlowPoll::Expired)]);

        let error = poll_until_authorized(
            &auth,
            &flow(900, 0),
            &CancellationToken::new(),
            PollOptions::default(),
        )
        .await
        .unwrap_err();

        assert_eq!(error.code, ErrorCode::AuthExpired);
    }

    #[tokio::test]
    async fn an_already_cancelled_token_ends_the_flow_immediately() {
        let auth = MockAuth::new(vec![authorized("never")]);
        let cancel = CancellationToken::new();
        cancel.cancel();

        let error = poll_until_authorized(&auth, &flow(900, 5), &cancel, PollOptions::default())
            .await
            .unwrap_err();

        assert_eq!(error.code, ErrorCode::Cancelled);
        assert_eq!(auth.poll_count(), 0, "取消优先于任何一次轮询");
    }

    #[tokio::test]
    async fn the_flow_deadline_stops_polling_even_when_the_server_keeps_saying_pending() {
        let auth = MockAuth::new(vec![Ok(DeviceFlowPoll::Pending); 1000]);

        let error = poll_until_authorized(
            &auth,
            &flow(0, 0),
            &CancellationToken::new(),
            PollOptions::default(),
        )
        .await
        .unwrap_err();

        assert_eq!(error.code, ErrorCode::AuthExpired);
        assert!(auth.poll_count() < 1000, "超时必须终止循环");
    }
}
