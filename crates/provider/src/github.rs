//! `GitHubProvider`：GitHub（github.com / Enterprise Cloud / GHE Server）的实现。
//!
//! # host → 端点推导（T4.2 企业 Host 的基础）
//!
//! | host | API 基址 | OAuth 基址 |
//! | --- | --- | --- |
//! | `github.com` | `https://api.github.com` | `https://github.com` |
//! | `*.ghe.com`（GHEC 数据驻留） | `https://api.<host>` | `https://<host>` |
//! | 其他（自建 GHE Server） | `https://<host>/api/v3` | `https://<host>` |
//!
//! # client_id 从哪来
//!
//! Device Flow 需要一个 OAuth App 的 client_id（公开值，不是秘密）。
//! 正式值由人类维护者注册后经设置注入；测试与开发期可用占位值——
//! GitHub 会返回 `device_flow_disabled` 之类的错误，错误映射会把它
//! 变成 `VALIDATION` 并带上 description。
//!
//! # 令牌的边界（红线 R8）
//!
//! 本结构体**不持有**任何令牌：令牌只在 `AuthFlow` 方法的参数与返回值里
//! 出现，落地存储由 services 层经 `forgedesk-credentials` 完成（T4.4）。
//! 因此 `GitHubProvider` 的 `Debug` 天然安全。

use secrecy::{ExposeSecret, SecretString};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use std::time::Duration;

use forgedesk_domain::{AppError, ErrorCode};

use crate::auth::{DeviceFlowPoll, DeviceFlowStart, VerifiedAccount};
use crate::client::{map_transport_error, ApiRequest, GitHubHttp};
use crate::model::{ProviderCapabilities, ProviderId};
use crate::rate_limit::RateLimitState;
use crate::traits::{
    AuthFlow, CiService, HostProvider, IssueService, PullService, ReleaseService, RepoService,
};

/// Device Flow 的默认轮询间隔（GitHub 文档值；响应里的 interval 优先）。
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(5);

/// 流程默认有效期（GitHub 文档值 900s；响应里的 expires_in 优先）。
const DEFAULT_FLOW_EXPIRY: Duration = Duration::from_secs(900);

/// GitHub 平台的 provider 实例：一个 host 一个实例。
#[derive(Debug, Clone)]
pub struct GitHubProvider {
    http: GitHubHttp,
    /// 该实例服务的 host（`github.com`、`acme.ghe.com`、自建 GHE 域名）。
    host: String,
    /// REST/GraphQL API 基址。
    api_base: String,
    /// OAuth 端点基址。
    oauth_base: String,
    /// OAuth App 的 client_id（公开值）。
    client_id: String,
}

impl GitHubProvider {
    /// 构建一个指向 `host` 的 GitHub provider。
    pub fn new(
        host: &str,
        client_id: impl Into<String>,
        http: GitHubHttp,
    ) -> Result<Self, AppError> {
        let host = host.trim().trim_end_matches('/').to_ascii_lowercase();
        if host.is_empty() {
            return Err(AppError::new(
                ErrorCode::Validation,
                "provider host must not be empty",
            ));
        }
        let (api_base, oauth_base) = endpoints_for(&host);
        Ok(Self {
            http,
            host,
            api_base,
            oauth_base,
            client_id: client_id.into(),
        })
    }

    /// 用显式端点构建（不按 host 规则推导）。
    ///
    /// 正常业务路径请用 [`Self::new`]。本构造器服务于两类场景：
    /// 其他 crate 的**契约测试**（把端点指到 wiremock 这类本地假服务），
    /// 以及将来"自定义 API 基址"的设置项（用户网络环境特殊时）。
    pub fn with_endpoints(
        host: &str,
        client_id: impl Into<String>,
        http: GitHubHttp,
        api_base: impl Into<String>,
        oauth_base: impl Into<String>,
    ) -> Self {
        Self {
            http,
            host: host.trim().trim_end_matches('/').to_ascii_lowercase(),
            api_base: api_base.into(),
            oauth_base: oauth_base.into(),
            client_id: client_id.into(),
        }
    }

    /// REST API 基址（测试与诊断用）。
    #[must_use]
    pub fn api_base(&self) -> &str {
        &self.api_base
    }

    /// HTTP 底座（同 crate 的子服务模块用它发请求，限流捕获等行为一致）。
    pub(crate) fn http(&self) -> &GitHubHttp {
        &self.http
    }
}

/// 按上表推导 API 与 OAuth 基址（纯函数，表驱动单测）。
fn endpoints_for(host: &str) -> (String, String) {
    if host == "github.com" {
        (
            "https://api.github.com".to_owned(),
            "https://github.com".to_owned(),
        )
    } else if host.ends_with(".ghe.com") {
        (format!("https://api.{host}"), format!("https://{host}"))
    } else {
        (format!("https://{host}/api/v3"), format!("https://{host}"))
    }
}

/// Device Flow 第一步的响应体。
#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    verification_uri_complete: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    interval: Option<u64>,
}

/// 令牌换取的响应体：成功带 `access_token`，失败带 `error`。
///
/// 未知字段一律忽略（docs/PLAN.md M4 风险表："错误码容错（未知字段忽略）"）：
/// GitHub 给响应加新字段不该让旧版应用解析失败。缺字段用 `default` 兜底，
/// 解析失败只发生在必需字段缺失或类型不符时。
#[derive(Debug, Deserialize)]
struct AccessTokenResponse {
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
}

/// `/user` 的响应体。
#[derive(Debug, Deserialize)]
struct GitHubUser {
    login: String,
    #[serde(default)]
    avatar_url: Option<String>,
}

impl GitHubProvider {
    /// 发送请求并解析 JSON（2xx 之外已由 [`GitHubHttp::send`] 映射为错误）。
    async fn request_json<T: DeserializeOwned>(&self, request: ApiRequest) -> Result<T, AppError> {
        let response = self.http.send(&request).await?;
        response.json::<T>().await.map_err(map_transport_error)
    }

    /// 主动刷新限流状态（`GET /rate_limit`，该端点**不消耗** REST 配额）。
    ///
    /// # 为什么 octocrab 路径用"主动刷新"而不是逐响应捕获
    ///
    /// octocrab 0.54 的默认 service 栈不向外部开放插层（`with_layer` 仅在
    /// `with_service` 全自定义栈下可用），复刻整套默认栈会随 octocrab 升级
    /// 漂移。而 `/rate_limit` 返回的是**权威值**（含同一令牌在别处的消耗），
    /// 响应自带 `x-ratelimit-*` 头、会照常进入 [`RateLimitTracker`]，
    /// 请求体还给出各资源桶（core/graphql/search）的细分。策略：
    /// - GitHubHttp 自管路径（认证、自建端点调用）：逐响应头捕获（已有）；
    /// - octocrab 路径：T4.5 起的服务在出错（403/429）与页面加载前调用本方法，
    ///   T4.10 的降级 UI 提供"手动刷新"。
    pub async fn refresh_rate_limit(
        &self,
        token: Option<SecretString>,
    ) -> Result<RateLimitState, AppError> {
        let url = format!("{}/rate_limit", self.api_base);
        let mut request = ApiRequest::get(url);
        if let Some(token) = token {
            request = request.with_bearer(token);
        }
        let response = self.http.send(&request).await?;
        let body: RateLimitResponse = response.json().await.map_err(map_transport_error)?;
        // core 桶是 REST 一切的配额来源；缺失时退回响应头捕获的快照
        let core = body.resources.and_then(|r| r.core).or(body.rate);
        let Some(core) = core else {
            return self.http.rate_limit().ok_or_else(|| {
                AppError::new(
                    ErrorCode::Internal,
                    "rate limit response had no core or rate bucket",
                )
            });
        };
        let state = RateLimitState {
            resource: Some("core".to_owned()),
            limit: core.limit,
            remaining: core.remaining,
            used: core.used,
            reset_unix_secs: core.reset,
        };
        self.http.set_rate_limit(state.clone());
        Ok(state)
    }
}

/// `/rate_limit` 的响应体（只取需要的桶，未知字段忽略）。
#[derive(Debug, Deserialize)]
struct RateLimitResponse {
    #[serde(default)]
    resources: Option<RateLimitBuckets>,
    #[serde(default)]
    rate: Option<RateLimitBucket>,
}

#[derive(Debug, Deserialize)]
struct RateLimitBuckets {
    #[serde(default)]
    core: Option<RateLimitBucket>,
}

#[derive(Debug, Deserialize)]
struct RateLimitBucket {
    limit: u32,
    used: u32,
    remaining: u32,
    reset: u64,
}

#[async_trait::async_trait]
impl AuthFlow for GitHubProvider {
    async fn start_device_flow(&self, scopes: &[&str]) -> Result<DeviceFlowStart, AppError> {
        let url = format!("{}/login/device/code", self.oauth_base);
        let request = ApiRequest::post_form(
            url,
            vec![
                ("client_id".to_owned(), self.client_id.clone()),
                ("scope".to_owned(), scopes.join(" ")),
            ],
        )
        .with_header("Accept", "application/json")?;
        // GitHub 对没有 Accept: application/json 的请求返回 form 编码体，
        // JSON 解析会失败——这个头是端点契约的一部分，不是可选项
        let response: DeviceCodeResponse = self.request_json(request).await?;

        Ok(DeviceFlowStart {
            device_code: SecretString::from(response.device_code),
            user_code: response.user_code,
            verification_uri: response.verification_uri,
            verification_uri_complete: response.verification_uri_complete,
            expires_in_secs: response
                .expires_in
                .unwrap_or_else(|| DEFAULT_FLOW_EXPIRY.as_secs()),
            interval_secs: response
                .interval
                .unwrap_or_else(|| DEFAULT_POLL_INTERVAL.as_secs()),
        })
    }

    async fn poll_device_flow(&self, flow: &DeviceFlowStart) -> Result<DeviceFlowPoll, AppError> {
        let url = format!("{}/login/oauth/access_token", self.oauth_base);
        let request = ApiRequest::post_form(
            url,
            vec![
                ("client_id".to_owned(), self.client_id.clone()),
                (
                    "device_code".to_owned(),
                    flow.device_code.expose_secret().to_owned(),
                ),
                (
                    "grant_type".to_owned(),
                    "urn:ietf:params:oauth:grant-type:device_code".to_owned(),
                ),
            ],
        )
        .with_header("Accept", "application/json")?;
        let response: AccessTokenResponse = self.request_json(request).await?;

        match (response.access_token, response.error.as_deref()) {
            (Some(token), _) => Ok(DeviceFlowPoll::Authorized {
                token: SecretString::from(token),
                scope: response.scope,
            }),
            (None, Some("authorization_pending")) => Ok(DeviceFlowPoll::Pending),
            (None, Some("slow_down")) => Ok(DeviceFlowPoll::SlowDown),
            (None, Some("expired_token")) => Ok(DeviceFlowPoll::Expired),
            (None, Some("access_denied")) => Ok(DeviceFlowPoll::Denied),
            // 其余错误（incorrect_client_credentials / device_flow_disabled /
            // unsupported_grant_type…）都是配置或请求形态问题，不该重试
            (None, Some(other)) => {
                let mut err = AppError::new(
                    ErrorCode::Validation,
                    format!("device flow failed: {other}"),
                );
                if let Some(desc) = response.error_description {
                    err = err.with_detail(desc);
                }
                Err(err)
            }
            // 什么都没带：响应不符合契约，当作内部错误留排查线索
            (None, None) => Err(AppError::new(
                ErrorCode::Internal,
                "token endpoint returned neither a token nor an error",
            )),
        }
    }

    async fn verify_pat(&self, token: SecretString) -> Result<VerifiedAccount, AppError> {
        let url = format!("{}/user", self.api_base);
        let request = ApiRequest::get(url).with_bearer(token);
        let response = self.http.send(&request).await?;
        // 作用域在响应头里：经典 PAT 是逗号分隔，细粒度 PAT 为空
        let scopes = response
            .headers()
            .get("x-oauth-scopes")
            .and_then(|v| v.to_str().ok())
            .map(|raw| {
                raw.split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let user: GitHubUser = response.json().await.map_err(map_transport_error)?;
        Ok(VerifiedAccount {
            login: user.login,
            avatar_url: user.avatar_url,
            scopes,
        })
    }
}

// 各子服务的方法随各自任务落地（T4.7/T4.8/T4.9）；先接上标记 trait，
// 让 `Box<dyn HostProvider>` 的形态从此固定。RepoService 的实现已随
// T4.5 在 repos.rs 落地。
impl PullService for GitHubProvider {}
impl IssueService for GitHubProvider {}
impl CiService for GitHubProvider {}
impl ReleaseService for GitHubProvider {}

impl HostProvider for GitHubProvider {
    fn id(&self) -> ProviderId {
        ProviderId::GitHub
    }

    fn host(&self) -> &str {
        &self.host
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::GITHUB
    }

    fn auth(&self) -> &dyn AuthFlow {
        self
    }
    fn repos(&self) -> &dyn RepoService {
        self
    }
    fn pulls(&self) -> &dyn PullService {
        self
    }
    fn issues(&self) -> &dyn IssueService {
        self
    }
    fn actions(&self) -> &dyn CiService {
        self
    }
    fn releases(&self) -> &dyn ReleaseService {
        self
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{endpoints_for, AccessTokenResponse, DeviceCodeResponse, GitHubProvider};
    use crate::auth::{DeviceFlowPoll, DeviceFlowStart};
    use crate::client::{GitHubHttp, HttpConfig};
    use crate::model::ProviderId;
    use crate::traits::HostProvider;
    use forgedesk_domain::ErrorCode;
    use secrecy::ExposeSecret;
    use std::time::Duration;
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const CLIENT_ID: &str = "Iv1.testclientid";

    fn http() -> GitHubHttp {
        GitHubHttp::new(HttpConfig {
            backoff_base: Duration::from_millis(1),
            ..HttpConfig::default()
        })
        .unwrap()
    }

    fn provider_at(server: &MockServer) -> GitHubProvider {
        // 用 provider 的 host 覆盖机制指向 wiremock：直接构造而不是 new()，
        // 因为 new() 会按 host 规则推导 https:// 端点
        GitHubProvider {
            http: http(),
            host: "github.com".to_owned(),
            api_base: server.uri(),
            oauth_base: server.uri(),
            client_id: CLIENT_ID.to_owned(),
        }
    }

    #[test]
    fn endpoints_follow_the_github_host_flavours() {
        // github.com：SaaS API 域名
        assert_eq!(
            endpoints_for("github.com"),
            (
                "https://api.github.com".to_owned(),
                "https://github.com".to_owned()
            )
        );
        // GHEC 数据驻留：api.<租户域名>
        assert_eq!(
            endpoints_for("acme.ghe.com"),
            (
                "https://api.acme.ghe.com".to_owned(),
                "https://acme.ghe.com".to_owned()
            )
        );
        // 自建 GHE Server：同域 + /api/v3
        assert_eq!(
            endpoints_for("ghe.corp.internal"),
            (
                "https://ghe.corp.internal/api/v3".to_owned(),
                "https://ghe.corp.internal".to_owned()
            )
        );
        // 大小写与尾斜杠的归一化发生在 new()（endpoints_for 只认已归一化的 host）
        let provider = GitHubProvider::new("GitHub.COM/", CLIENT_ID, http()).unwrap();
        assert_eq!(provider.api_base(), "https://api.github.com");
    }

    #[test]
    fn provider_normalizes_host_before_deriving_endpoints() {
        let provider = GitHubProvider::new("GHE.Corp.Internal//", CLIENT_ID, http()).unwrap();
        assert_eq!(provider.host(), "ghe.corp.internal");
        assert_eq!(provider.api_base(), "https://ghe.corp.internal/api/v3");
    }

    #[test]
    fn an_empty_host_is_rejected() {
        let error = GitHubProvider::new("  ", CLIENT_ID, http()).unwrap_err();
        assert_eq!(error.code, ErrorCode::Validation);
    }

    /// 通过 dyn trait 对象走完整启动流程：锁死 `Box<dyn HostProvider>` 的形态。
    #[tokio::test]
    async fn device_flow_start_works_through_the_dyn_trait_object() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/login/device/code"))
            .and(body_string_contains("client_id=Iv1.testclientid"))
            .and(body_string_contains("scope=repo"))
            .and(body_string_contains("read%3Aorg"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "device_code": "dc_123",
                "user_code": "WDJB-MJTK",
                "verification_uri": "https://github.com/login/device",
                "verification_uri_complete": "https://github.com/login/device?user_code=WDJB-MJTK",
                "expires_in": 900,
                "interval": 5
            })))
            .mount(&server)
            .await;

        let provider: Box<dyn HostProvider> = Box::new(provider_at(&server));
        assert_eq!(provider.id(), ProviderId::GitHub);
        assert_eq!(provider.host(), "github.com");
        assert!(provider.capabilities().pulls && provider.capabilities().graphql);

        let start = provider
            .auth()
            .start_device_flow(&["repo", "read:org", "workflow"])
            .await
            .unwrap();

        assert_eq!(start.user_code, "WDJB-MJTK");
        assert_eq!(start.device_code.expose_secret(), "dc_123");
        assert_eq!(start.interval_secs, 5);
        server.verify().await;
    }

    #[tokio::test]
    async fn device_flow_defaults_fill_in_when_github_omits_timing_fields() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "device_code": "dc_1",
                "user_code": "ABCD-1234",
                "verification_uri": "https://github.com/login/device"
            })))
            .mount(&server)
            .await;

        let start: DeviceFlowStart = provider_at(&server)
            .auth()
            .start_device_flow(&["repo"])
            .await
            .unwrap();
        assert_eq!(start.interval_secs, 5);
        assert_eq!(start.expires_in_secs, 900);
        assert_eq!(start.verification_uri_complete, None);
    }

    #[tokio::test]
    async fn polling_covers_every_device_flow_outcome() {
        // (error 字段值, 期望轮询结果)
        let cases: [(&str, DeviceFlowPoll); 4] = [
            ("authorization_pending", DeviceFlowPoll::Pending),
            ("slow_down", DeviceFlowPoll::SlowDown),
            ("expired_token", DeviceFlowPoll::Expired),
            ("access_denied", DeviceFlowPoll::Denied),
        ];

        for (error_field, expected) in cases {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/login/oauth/access_token"))
                .and(body_string_contains("grant_type=urn"))
                .and(body_string_contains("device_code=dc"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "error": error_field,
                    "error_description": "desc"
                })))
                .mount(&server)
                .await;
            let provider = provider_at(&server);
            let flow = DeviceFlowStart {
                device_code: secrecy::SecretString::from("dc"),
                user_code: "U".to_owned(),
                verification_uri: "u".to_owned(),
                verification_uri_complete: None,
                expires_in_secs: 1,
                interval_secs: 1,
            };
            let poll = provider.auth().poll_device_flow(&flow).await.unwrap();
            assert_eq!(
                std::mem::discriminant(&poll),
                std::mem::discriminant(&expected),
                "error={error_field} → {poll:?}"
            );
        }
    }

    #[tokio::test]
    async fn an_authorized_poll_returns_the_token_and_scope() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "gho_issuedtoken",
                "token_type": "bearer",
                "scope": "repo,read:org"
            })))
            .mount(&server)
            .await;

        let provider = provider_at(&server);
        let flow = DeviceFlowStart {
            device_code: secrecy::SecretString::from("dc"),
            user_code: "U".to_owned(),
            verification_uri: "u".to_owned(),
            verification_uri_complete: None,
            expires_in_secs: 1,
            interval_secs: 1,
        };
        let poll = provider.auth().poll_device_flow(&flow).await.unwrap();

        match poll {
            DeviceFlowPoll::Authorized { token, scope } => {
                assert_eq!(token.expose_secret(), "gho_issuedtoken");
                assert_eq!(scope.as_deref(), Some("repo,read:org"));
            }
            other => panic!("期望 Authorized，得到 {other:?}"),
        }
    }

    #[tokio::test]
    async fn an_unknown_poll_error_is_a_validation_error_with_the_description() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "error": "device_flow_disabled",
                "error_description": "device flow is not enabled for this OAuth app"
            })))
            .mount(&server)
            .await;

        let provider = provider_at(&server);
        let flow = DeviceFlowStart {
            device_code: secrecy::SecretString::from("dc"),
            user_code: "U".to_owned(),
            verification_uri: "u".to_owned(),
            verification_uri_complete: None,
            expires_in_secs: 1,
            interval_secs: 1,
        };
        let error = provider.auth().poll_device_flow(&flow).await.unwrap_err();

        assert_eq!(error.code, ErrorCode::Validation);
        assert!(error.detail.unwrap().contains("not enabled"));
    }

    #[tokio::test]
    async fn verify_pat_returns_the_account_with_header_scopes() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/user"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("x-oauth-scopes", "repo, read:org, workflow")
                    .set_body_json(serde_json::json!({
                        "login": "octocat",
                        "avatar_url": "https://avatars.githubusercontent.com/u/1"
                    })),
            )
            .mount(&server)
            .await;

        let account = provider_at(&server)
            .auth()
            .verify_pat(secrecy::SecretString::from("ghp_good"))
            .await
            .unwrap();

        assert_eq!(account.login, "octocat");
        assert_eq!(account.scopes, vec!["repo", "read:org", "workflow"]);
    }

    #[tokio::test]
    async fn a_rejected_pat_maps_to_auth_expired() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(401).set_body_string(r#"{"message":"Bad credentials"}"#),
            )
            .mount(&server)
            .await;

        let error = provider_at(&server)
            .auth()
            .verify_pat(secrecy::SecretString::from("ghp_revoked"))
            .await
            .unwrap_err();

        assert_eq!(error.code, ErrorCode::AuthExpired);
    }

    #[tokio::test]
    async fn rate_limit_refresh_takes_the_authoritative_core_bucket_and_feeds_the_tracker() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/rate_limit"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("x-ratelimit-limit", "60")
                    .insert_header("x-ratelimit-remaining", "59")
                    .insert_header("x-ratelimit-reset", "1790000000")
                    .set_body_json(serde_json::json!({
                        "resources": {
                            "core": {
                                "limit": 5000, "used": 37, "remaining": 4963, "reset": 1790000100
                            },
                            "graphql": {
                                "limit": 5000, "used": 0, "remaining": 5000, "reset": 1790000100
                            }
                        },
                        "rate": { "limit": 5000, "used": 37, "remaining": 4963, "reset": 1790000100 }
                    })),
            )
            .mount(&server)
            .await;

        let provider = provider_at(&server);
        let state = provider
            .refresh_rate_limit(Some(secrecy::SecretString::from("ghp_ok")))
            .await
            .unwrap();

        // 请求体是权威值：core 桶 4963，而不是响应头里的匿名 59
        assert_eq!(state.remaining, 4963);
        assert_eq!(state.limit, 5000);
        assert_eq!(state.reset_unix_secs, 1_790_000_100);
        assert_eq!(state.resource.as_deref(), Some("core"));
        // 快照同步进 tracker：后续的降级 UI / is_exhausted 判定看得到
        assert_eq!(provider.http.rate_limit().unwrap().remaining, 4963);
    }

    #[tokio::test]
    async fn a_rate_limit_refresh_without_buckets_falls_back_to_header_capture() {
        let server = MockServer::start().await;
        // 某些网关会剥掉 body 里的桶或改写结构：头捕获仍然是兜底
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("x-ratelimit-limit", "5000")
                    .insert_header("x-ratelimit-remaining", "42")
                    .insert_header("x-ratelimit-reset", "1790000000")
                    .set_body_json(serde_json::json!({})),
            )
            .mount(&server)
            .await;

        let provider = provider_at(&server);
        let state = provider.refresh_rate_limit(None).await.unwrap();

        assert_eq!(state.remaining, 42);
    }

    /// 容错策略（docs/PLAN.md M4 风险表"未知字段忽略"）：GitHub 给
    /// 令牌响应加新字段（历史上的 token_type 等）不得让旧版解析失败。
    #[test]
    fn access_token_response_tolerates_new_fields() {
        let raw = r#"{"access_token":"x","token_type":"bearer","totally_new":true}"#;
        let parsed: AccessTokenResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(parsed.access_token.as_deref(), Some("x"));
    }

    /// 宽松结构：未知字段被忽略时解析照常成功。
    #[test]
    fn device_code_response_tolerates_new_fields() {
        let raw = r#"{"device_code":"d","user_code":"U","verification_uri":"u","brand_new":true}"#;
        let parsed: DeviceCodeResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(parsed.user_code, "U");
    }
}
