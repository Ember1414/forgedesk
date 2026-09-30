//! GitHub 的 HTTP 底座：UA、超时、代理、限流头捕获、5xx 重试与错误映射。
//!
//! # 为什么自建一层而不是直接用 octocrab 的请求
//!
//! PLAN M4.1 要求的中间件行为（UA、30s 超时、代理来自设置、5xx 指数退避
//! 重试 ≤ 3 次、限流头解析进 `RateLimitState`）必须对**所有**请求生效，
//! 包括 Device Flow 这类不走 octocrab REST API 的端点。这一层同时服务于
//! 自建请求与 octocrab（后者通过 [`GitHubHttp::raw_client`] 复用同一个
//! 连接池与 UA）。
//!
//! # octocrab 路径的限流可见性（T4.2 的取舍）
//!
//! octocrab 0.54 的默认 service 栈不开放插层，复刻整套栈会随升级漂移。
//! 因此 octocrab 发出的请求**不**逐响应喂 [`RateLimitTracker`]；取而代之，
//! [`crate::github::GitHubProvider::refresh_rate_limit`] 用 `GET /rate_limit`
//! （不耗配额、值权威）主动刷新，T4.5 起的服务在 403/429 与页面加载前调用。
//! 自管路径（认证、自建端点）仍然逐响应头捕获。octocrab 自身的重试
//! （`Simple(3)`，立即重试 5xx/429，有界）保持默认，不额外配置。
//!
//! # 重试语义（docs/PLAN.md M4.1）
//!
//! - 只重试 **5xx**：服务端瞬时故障，指数退避（base × 2^n），至多重试 3 次；
//! - **4xx 一律不重试**：尤其 429/403 限流——重试只会更快烧光配额，
//!   正确出路是把 reset 时间交给 UI 降级（T4.10）；
//! - 传输层错误（DNS / 连接 / 超时）不自动重试：错误码 NETWORK 本身
//!   标记为可重试，由用户/上层决定何时重发。
//!
//! # 错误映射（稳定错误码，见 `crates/domain/src/error.rs`）
//!
//! | GitHub | 条件 | ErrorCode |
//! | --- | --- | --- |
//! | 401 | 携带了凭据 | `AUTH_EXPIRED`（令牌过期/被撤销） |
//! | 401 | 未携带凭据 | `AUTH_REQUIRED` |
//! | 403 | `x-ratelimit-remaining == 0` 或正文含 rate limit | `RATE_LIMITED` |
//! | 403 | 其余 | `PERMISSION_DENIED`（作用域不足等） |
//! | 404 | — | `NOT_FOUND` |
//! | 422 | — | `VALIDATION`（GitHub 校验失败的标准码） |
//! | 429 | — | `RATE_LIMITED` |
//! | 5xx | 重试耗尽后 | `NETWORK` |
//! | 传输错误 | — | `NETWORK` |
//!
//! 所有进入 `detail` 的原始文本都先过 [`crate::redact::redact_tokens`]。

use secrecy::{ExposeSecret, SecretString};
use std::time::Duration;

use forgedesk_domain::{AppError, ErrorCode};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::rate_limit::{RateLimitState, RateLimitTracker};
use crate::redact::redact_tokens;

/// 统一 User-Agent（docs/PLAN.md M4.1）。
///
/// 版本取自本 crate 的 workspace 版本，与产品版本同步演进。
pub const USER_AGENT: &str = concat!("ForgeDesk/", env!("CARGO_PKG_VERSION"));

/// 默认请求超时（docs/PLAN.md M4.1：30s）。
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// 默认重试次数（对 5xx；docs/PLAN.md M4.1：≤ 3 次）。
pub const DEFAULT_MAX_RETRIES: u32 = 3;

/// 退避基值：第 n 次重试等待 `base * 2^(n-1)`（250ms → 500ms → 1000ms）。
pub const DEFAULT_BACKOFF: Duration = Duration::from_millis(250);

/// 错误 `detail` 的长度上限（与 AuditLog 的摘要策略一致，防日志膨胀）。
const MAX_DETAIL_LEN: usize = 2048;

/// HTTP 层配置。代理来自应用设置（M6 的代理设置页落地前可传 `None`）。
#[derive(Debug, Clone)]
pub struct HttpConfig {
    /// 显式代理 URL（http/https/socks5）。`None` = 跟随系统代理。
    pub proxy: Option<String>,
    /// 单请求超时。
    pub timeout: Duration,
    /// 5xx 最大重试次数。
    pub max_retries: u32,
    /// 退避基值（测试会调小以加速）。
    pub backoff_base: Duration,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            proxy: None,
            timeout: DEFAULT_TIMEOUT,
            max_retries: DEFAULT_MAX_RETRIES,
            backoff_base: DEFAULT_BACKOFF,
        }
    }
}

/// 一个待发送的 API 请求描述。
///
/// 为什么是"描述"而不是 `RequestBuilder`：重试需要**重建**请求，
/// 而 `RequestBuilder` 不可克隆。每一轮尝试都从这份描述重新构建。
#[derive(Debug, Clone)]
pub struct ApiRequest {
    /// HTTP 方法。
    pub method: reqwest::Method,
    /// 完整 URL。
    pub url: String,
    /// Bearer 令牌；`None` 表示匿名请求（决定 401 的错误码走向）。
    pub bearer: Option<SecretString>,
    /// JSON 请求体。
    pub body: Option<serde_json::Value>,
    /// form-urlencoded 请求体（OAuth 端点要求 form 而不是 JSON）。
    /// 与 `body` 互斥：同时给出时以 `form` 为准。
    pub form: Option<Vec<(String, String)>>,
    /// 额外请求头（已校验）。
    pub headers: Vec<(reqwest::header::HeaderName, reqwest::header::HeaderValue)>,
}

impl ApiRequest {
    /// 构造一个 GET 请求。
    pub fn get(url: impl Into<String>) -> Self {
        Self::new(reqwest::Method::GET, url)
    }

    /// 构造一个 POST 请求（JSON 体）。
    pub fn post_json(url: impl Into<String>, body: serde_json::Value) -> Self {
        Self::new(reqwest::Method::POST, url).with_body(body)
    }

    /// 基础构造。
    pub fn new(method: reqwest::Method, url: impl Into<String>) -> Self {
        Self {
            method,
            url: url.into(),
            bearer: None,
            body: None,
            form: None,
            headers: Vec::new(),
        }
    }

    /// 携带 Bearer 令牌。
    #[must_use]
    pub fn with_bearer(mut self, token: SecretString) -> Self {
        self.bearer = Some(token);
        self
    }

    /// 设置 JSON 体。
    #[must_use]
    pub fn with_body(mut self, body: serde_json::Value) -> Self {
        self.body = Some(body);
        self
    }

    /// 设置 form-urlencoded 体（OAuth Device Flow 端点要求）。
    #[must_use]
    pub fn with_form(mut self, form: Vec<(String, String)>) -> Self {
        self.form = Some(form);
        self
    }

    /// 构造一个 POST 请求（form 体）。
    pub fn post_form(url: impl Into<String>, form: Vec<(String, String)>) -> Self {
        Self::new(reqwest::Method::POST, url).with_form(form)
    }

    /// 追加一个请求头（非法名称/值在这里就被拒绝，不等到发送时）。
    pub fn with_header(mut self, name: &str, value: &str) -> Result<Self, AppError> {
        let name = reqwest::header::HeaderName::from_bytes(name.as_bytes()).map_err(|_| {
            AppError::new(
                ErrorCode::Validation,
                format!("invalid header name: {name}"),
            )
        })?;
        let value = reqwest::header::HeaderValue::from_str(value).map_err(|_| {
            AppError::new(
                ErrorCode::Validation,
                format!("invalid header value for {name}"),
            )
        })?;
        self.headers.push((name, value));
        Ok(self)
    }

    /// 是否携带了凭据（401 映射的判定输入）。
    fn has_credentials(&self) -> bool {
        self.bearer.is_some()
    }
}

/// GitHub 的 HTTP 底座：可克隆，供各子服务共享同一个连接池与限流快照。
#[derive(Clone, Debug)]
pub struct GitHubHttp {
    client: reqwest::Client,
    config: HttpConfig,
    rate_limit: RateLimitTracker,
}

impl GitHubHttp {
    /// 按配置构建。代理 URL 非法时返回 `VALIDATION`。
    pub fn new(config: HttpConfig) -> Result<Self, AppError> {
        let mut builder = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(config.timeout);
        if let Some(proxy_url) = &config.proxy {
            let proxy = reqwest::Proxy::all(proxy_url).map_err(|_| {
                AppError::new(
                    ErrorCode::Validation,
                    format!("invalid proxy URL: {proxy_url}"),
                )
            })?;
            // 显式代理必须排除系统代理，否则环境变量里的代理会与设置打架
            builder = builder.no_proxy().proxy(proxy);
        }
        Ok(Self {
            client: builder.build().map_err(|err| {
                AppError::new(ErrorCode::Network, "failed to build HTTP client")
                    .with_detail(redact_tokens(&err.to_string()))
            })?,
            config,
            rate_limit: RateLimitTracker::new(),
        })
    }

    /// 最近一次限流快照（暴露给前端展示剩余额度与重置时间，T4.10）。
    #[must_use]
    pub fn rate_limit(&self) -> Option<RateLimitState> {
        self.rate_limit.snapshot()
    }

    /// 直接写入快照（`GET /rate_limit` 刷新路径用）。
    pub fn set_rate_limit(&self, state: RateLimitState) {
        self.rate_limit.set(state);
    }

    /// 底层 client（octocrab 复用同一连接池时使用）。
    #[must_use]
    pub fn raw_client(&self) -> &reqwest::Client {
        &self.client
    }

    /// 发送请求：限流捕获 → 5xx 重试 → 错误映射。
    pub async fn send(&self, request: &ApiRequest) -> Result<reqwest::Response, AppError> {
        let mut attempt: u32 = 0;
        loop {
            let builder = self.build(request);
            let response = builder.send().await.map_err(map_transport_error)?;

            self.rate_limit.capture(response.headers());
            let status = response.status();

            if status.is_server_error() && attempt < self.config.max_retries {
                attempt += 1;
                let delay = self
                    .config
                    .backoff_base
                    .saturating_mul(2u32.saturating_pow(attempt - 1));
                tracing::warn!(status = %status, attempt, ?delay, "GitHub 5xx, retrying");
                tokio::time::sleep(delay).await;
                continue;
            }

            if !status.is_success() {
                let has_credentials = request.has_credentials();
                let (message, detail) = take_error_body(response).await;
                return Err(map_github_failure(
                    status,
                    message.as_deref(),
                    detail.as_deref(),
                    has_credentials,
                    self.rate_limit.snapshot(),
                ));
            }
            return Ok(response);
        }
    }

    fn build(&self, request: &ApiRequest) -> reqwest::RequestBuilder {
        let mut builder = self
            .client
            .request(request.method.clone(), &request.url)
            .header(reqwest::header::ACCEPT, "application/vnd.github+json");
        if let Some(token) = &request.bearer {
            builder = builder.bearer_auth(token.expose_secret());
        }
        if let Some(body) = &request.body {
            builder = builder.json(body);
        }
        if let Some(form) = &request.form {
            builder = builder.form(form);
        }
        for (name, value) in &request.headers {
            builder = builder.header(name, value);
        }
        builder
    }
}

/// 传输层错误 → `NETWORK`（detail 脱敏：错误里可能带 URL 查询参数）。
pub(crate) fn map_transport_error(err: reqwest::Error) -> AppError {
    let kind = if err.is_timeout() {
        "timed out"
    } else if err.is_connect() {
        "could not connect"
    } else if err.is_decode() {
        "failed to decode response"
    } else {
        "request failed"
    };
    network_error(
        &format!("GitHub request {kind}"),
        redact_tokens(&err.to_string()),
    )
}

/// 传输层失败 → `NETWORK` 的共用入口（octocrab 的 Http/Hyper/Service 也走这里）。
pub(crate) fn network_error(message: &str, detail: String) -> AppError {
    AppError::new(ErrorCode::Network, message).with_detail(detail)
}

/// 读取错误响应正文：抽出 GitHub 的 `message` 字段，全文脱敏后进 `detail`。
async fn take_error_body(response: reqwest::Response) -> (Option<String>, Option<String>) {
    let raw = match response.text().await {
        Ok(text) => text,
        Err(err) => return (None, Some(redact_tokens(&err.to_string()))),
    };
    let message = serde_json::from_str::<serde_json::Value>(&raw)
        .ok()
        .and_then(|v| v.get("message").and_then(|m| m.as_str()).map(str::to_owned));
    let detail = truncate(&redact_tokens(raw.trim()));
    (message, Some(detail))
}

/// 状态码 → `AppError`。`rate` 是**本响应**解析到的限流快照。
///
/// 供本模块与 [`crate::error`]（octocrab 映射）共用，保证同一状态
/// 永远映射到同一错误码。
pub(crate) fn map_github_failure(
    status: reqwest::StatusCode,
    message: Option<&str>,
    detail: Option<&str>,
    has_credentials: bool,
    rate: Option<RateLimitState>,
) -> AppError {
    use ErrorCode::{
        AuthExpired, AuthRequired, Network, NotFound, PermissionDenied, RateLimited, Validation,
    };

    // GitHub 会把请求里的令牌原样回显进 message（"Bad credentials for ghp_…"）。
    // message 与 detail 都是用户可见字段（前端详情区），入映射前统一脱敏
    // （红线 R8）——不依赖调用方自觉，纵深防御。
    let message = message.map(redact_tokens);
    let detail = detail.map(redact_tokens);

    let mut error = match status {
        reqwest::StatusCode::UNAUTHORIZED => {
            // 带着凭据还 401 = 凭据失效（不是"没登录"）；两者的修复动作不同
            if has_credentials {
                AppError::new(
                    AuthExpired,
                    github_message(message.as_deref(), "stored token was rejected"),
                )
            } else {
                AppError::new(
                    AuthRequired,
                    github_message(message.as_deref(), "authentication is required"),
                )
            }
        }
        reqwest::StatusCode::FORBIDDEN => {
            if rate.as_ref().is_some_and(|r| r.remaining == 0)
                || is_rate_limit_text(message.as_deref())
            {
                let mut err =
                    AppError::new(RateLimited, rate_limit_message(message.as_deref(), &rate));
                if let Some(reset) = rate.map(|r| r.reset_unix_secs) {
                    err = err.with_hint(format!("reset={reset}"));
                }
                err
            } else {
                AppError::new(
                    PermissionDenied,
                    github_message(
                        message.as_deref(),
                        "insufficient permission for this resource",
                    ),
                )
            }
        }
        reqwest::StatusCode::NOT_FOUND => AppError::new(
            NotFound,
            github_message(message.as_deref(), "resource not found"),
        ),
        reqwest::StatusCode::UNPROCESSABLE_ENTITY => AppError::new(
            Validation,
            github_message(message.as_deref(), "GitHub rejected the payload"),
        ),
        reqwest::StatusCode::TOO_MANY_REQUESTS => {
            AppError::new(RateLimited, rate_limit_message(message.as_deref(), &rate))
        }
        s if s.is_server_error() => AppError::new(
            Network,
            github_message(message.as_deref(), "GitHub server error after retries"),
        ),
        _ => AppError::new(
            ErrorCode::Internal,
            github_message(message.as_deref(), "unexpected GitHub API response"),
        ),
    };
    if let Some(detail) = detail.as_deref() {
        error = error.with_detail(detail);
    }
    error
}

/// 403 正文里出现 rate limit 字样（次级限流也可能不带 remaining=0 头）。
fn is_rate_limit_text(message: Option<&str>) -> bool {
    message
        .map(|m| m.to_ascii_lowercase().contains("rate limit"))
        .unwrap_or(false)
}

/// 组装限流错误的开发者可读消息（含 reset 的 RFC3339 时间）。
fn rate_limit_message(message: Option<&str>, rate: &Option<RateLimitState>) -> String {
    let reset = rate
        .as_ref()
        .and_then(|r| {
            OffsetDateTime::from_unix_timestamp(i64::try_from(r.reset_unix_secs).ok()?)
                .ok()
                .and_then(|t| t.format(&Rfc3339).ok())
        })
        .unwrap_or_else(|| "unknown".to_owned());
    format!("GitHub API rate limit exhausted; resets at {reset}")
        + message
            .filter(|m| !m.is_empty())
            .map(|m| format!(": {m}"))
            .as_deref()
            .unwrap_or("")
}

/// 用 GitHub 的 message 拼开发者可读消息；缺失时用兜底描述。
fn github_message(message: Option<&str>, fallback: &str) -> String {
    message
        .filter(|m| !m.trim().is_empty())
        .unwrap_or(fallback)
        .to_owned()
}

fn truncate(text: &str) -> String {
    if text.len() <= MAX_DETAIL_LEN {
        text.to_owned()
    } else {
        let mut cut = MAX_DETAIL_LEN;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        format!("{}…", &text[..cut])
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{map_github_failure, truncate, ApiRequest, GitHubHttp, HttpConfig, USER_AGENT};
    use crate::rate_limit::RateLimitState;
    use forgedesk_domain::{AppError, ErrorCode};
    use secrecy::SecretString;
    use std::time::Duration;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn token(value: &str) -> SecretString {
        SecretString::from(value.to_owned())
    }

    async fn client() -> GitHubHttp {
        GitHubHttp::new(HttpConfig {
            backoff_base: Duration::from_millis(1),
            ..HttpConfig::default()
        })
        .unwrap()
    }

    #[tokio::test]
    async fn every_request_carries_the_forgedesk_user_agent() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/rate_limit"))
            .and(header("user-agent", USER_AGENT))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let http = client().await;
        http.send(&ApiRequest::get(format!("{}/rate_limit", server.uri())))
            .await
            .unwrap();

        server.verify().await;
    }

    #[tokio::test]
    async fn server_errors_are_retried_with_backoff_until_success() {
        let server = MockServer::start().await;
        // 两次 500 之后第三次成功：恰好用满"重试两次"的路径
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(500))
            .up_to_n_times(2)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
            .expect(1)
            .mount(&server)
            .await;

        let http = client().await;
        let response = http.send(&ApiRequest::get(server.uri())).await.unwrap();

        assert_eq!(response.status(), 200);
        server.verify().await;
    }

    #[tokio::test]
    async fn retries_stop_after_the_limit_and_surface_as_network() {
        let server = MockServer::start().await;
        // max_retries=3 → 1 次原始 + 3 次重试 = 4 个请求，且结果一定是 NETWORK
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(502))
            .expect(4)
            .mount(&server)
            .await;

        let http = client().await;
        let error = http.send(&ApiRequest::get(server.uri())).await.unwrap_err();

        assert_eq!(error.code, ErrorCode::Network);
        assert!(error.retryable);
        server.verify().await;
    }

    #[tokio::test]
    async fn client_errors_are_never_retried() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404))
            .expect(1)
            .mount(&server)
            .await;

        let http = client().await;
        let error = http.send(&ApiRequest::get(server.uri())).await.unwrap_err();

        assert_eq!(error.code, ErrorCode::NotFound);
        server.verify().await;
    }

    #[tokio::test]
    async fn rate_limit_headers_are_captured_from_successful_responses() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("x-ratelimit-limit", "5000")
                    .insert_header("x-ratelimit-remaining", "4998")
                    .insert_header("x-ratelimit-reset", "1790000000"),
            )
            .mount(&server)
            .await;

        let http = client().await;
        http.send(&ApiRequest::get(server.uri())).await.unwrap();

        let state = http.rate_limit().unwrap();
        assert_eq!(state.remaining, 4998);
        assert_eq!(state.limit, 5000);
    }

    #[tokio::test]
    async fn an_exhausted_quota_maps_to_rate_limited_with_the_reset_time() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(403)
                    .insert_header("x-ratelimit-remaining", "0")
                    .insert_header("x-ratelimit-limit", "5000")
                    .insert_header("x-ratelimit-reset", "1790000000")
                    .set_body_string(r#"{"message":"API rate limit exceeded"}"#),
            )
            .mount(&server)
            .await;

        let http = client().await;
        let error = http
            .send(&ApiRequest::get(server.uri()).with_bearer(token("ghs_test")))
            .await
            .unwrap_err();

        assert_eq!(error.code, ErrorCode::RateLimited);
        assert!(error.retryable, "限流是典型可重试错误");
        assert!(
            error.message.contains("resets at 2026"),
            "{}",
            error.message
        );
        assert_eq!(error.hint.as_deref(), Some("reset=1790000000"));
    }

    #[tokio::test]
    async fn a_plain_forbidden_maps_to_permission_denied() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(403)
                    .insert_header("x-ratelimit-remaining", "4900")
                    .set_body_string(r#"{"message":"Resource not accessible by integration"}"#),
            )
            .mount(&server)
            .await;

        let http = client().await;
        let error = http.send(&ApiRequest::get(server.uri())).await.unwrap_err();

        assert_eq!(error.code, ErrorCode::PermissionDenied);
        assert_eq!(error.message, "Resource not accessible by integration");
    }

    #[tokio::test]
    async fn an_unauthorized_maps_by_whether_credentials_were_sent() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(401).set_body_string(r#"{"message":"Bad credentials"}"#),
            )
            .mount(&server)
            .await;

        let http = client().await;
        let with_token = http
            .send(&ApiRequest::get(server.uri()).with_bearer(token("ghp_expired")))
            .await
            .unwrap_err();
        assert_eq!(with_token.code, ErrorCode::AuthExpired);
        assert!(!with_token.retryable, "重新登录前重试毫无意义");

        let anonymous = http
            .send(&ApiRequest::get(format!("{}/other", server.uri())))
            .await
            .unwrap_err();
        assert_eq!(anonymous.code, ErrorCode::AuthRequired);
    }

    #[tokio::test]
    async fn error_details_never_contain_a_token() {
        let server = MockServer::start().await;
        // 恶意/异常的服务端把请求头回显进错误正文：脱敏必须兜住
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(401).set_body_string(
                r#"{"message":"Bad credentials for ghp_0123456789abcdefghijklmnopqrstuvwxyzABCD"}"#,
            ))
            .mount(&server)
            .await;

        let http = client().await;
        let error = http
            .send(
                &ApiRequest::get(server.uri())
                    .with_bearer(token("ghp_0123456789abcdefghijklmnopqrstuvwxyzABCD")),
            )
            .await
            .unwrap_err();

        let dumped = format!("{error:?}");
        assert!(
            !dumped.contains("0123456789abcdefghijklmnopqrstuvwxyz"),
            "{dumped}"
        );
        assert!(dumped.contains(crate::redact::REDACTED));
    }

    #[tokio::test]
    async fn a_connection_refused_maps_to_network() {
        // 不用"起了再关"的 wiremock 端口：同进程并行的其他测试可能立刻
        // 复用该端口，把"连接拒绝"变成一条 404。127.0.0.1:1 是特权端口，
        // 本进程无法监听，连接拒绝是确定的。
        let http = client().await;
        let error = http
            .send(&ApiRequest::get("http://127.0.0.1:1/"))
            .await
            .unwrap_err();

        assert_eq!(error.code, ErrorCode::Network);
    }

    #[tokio::test]
    async fn a_timeout_maps_to_network() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(2)))
            .mount(&server)
            .await;

        let http = GitHubHttp::new(HttpConfig {
            timeout: Duration::from_millis(50),
            ..HttpConfig::default()
        })
        .unwrap();
        let error = http.send(&ApiRequest::get(server.uri())).await.unwrap_err();

        assert_eq!(error.code, ErrorCode::Network);
    }

    #[test]
    fn an_invalid_proxy_is_rejected_before_any_request() {
        let error = GitHubHttp::new(HttpConfig {
            proxy: Some("not a proxy".to_owned()),
            ..HttpConfig::default()
        })
        .unwrap_err();

        assert_eq!(error.code, ErrorCode::Validation);
    }

    #[test]
    fn a_custom_proxy_disables_system_proxies() {
        // 构造成功即可：no_proxy + 显式代理的组合是构造期的行为
        let http = GitHubHttp::new(HttpConfig {
            proxy: Some("http://127.0.0.1:8118".to_owned()),
            ..HttpConfig::default()
        });
        assert!(http.is_ok());
    }

    /// map_github_failure 是纯函数，表驱动覆盖所有映射行。
    #[test]
    fn status_codes_map_to_the_documented_error_codes() {
        let rate = |remaining: u32| {
            Some(RateLimitState {
                resource: None,
                limit: 5000,
                remaining,
                used: 5000 - remaining,
                reset_unix_secs: 1_790_000_000,
            })
        };
        let cases = [
            (401, None, true, ErrorCode::AuthExpired),
            (401, None, false, ErrorCode::AuthRequired),
            (403, rate(0), true, ErrorCode::RateLimited),
            (403, rate(42), true, ErrorCode::PermissionDenied),
            (404, None, true, ErrorCode::NotFound),
            (422, None, true, ErrorCode::Validation),
            (429, None, true, ErrorCode::RateLimited),
            (502, None, true, ErrorCode::Network),
        ];
        for (status_code, rate_state, has_creds, expected) in cases {
            let status = reqwest::StatusCode::from_u16(status_code).unwrap();
            let error = map_github_failure(status, None, None, has_creds, rate_state);
            assert_eq!(error.code, expected, "HTTP {status_code}");
        }
    }

    #[test]
    fn truncation_never_splits_a_utf8_character() {
        let multi_byte = "中".repeat(3000); // 9000 字节 > 2048
        let cut = truncate(&multi_byte);
        assert!(cut.chars().count() < 3000);
        assert!(cut.ends_with('…'));
    }

    /// ApiRequest 的 Debug 不含明文令牌（红线 R8）。
    #[test]
    fn api_request_debug_redacts_the_bearer_token() {
        let request =
            ApiRequest::get("https://api.github.com/user").with_bearer(token("ghp_secretvalue"));

        let dumped = format!("{request:?}");
        assert!(!dumped.contains("ghp_secretvalue"), "{dumped}");
    }

    /// AppError 已是本文件映射的输出类型：确认脱敏过 detail 一定在错误里。
    #[test]
    fn mapped_errors_carry_the_sanitized_detail() {
        let status = reqwest::StatusCode::UNAUTHORIZED;
        let error = map_github_failure(
            status,
            Some("Bad credentials"),
            Some("Bad credentials ghp_0123456789abcdefghijklmnopqrstuvwxyzABCD"),
            true,
            None,
        );
        assert!(matches!(
            error,
            AppError {
                code: ErrorCode::AuthExpired,
                ..
            }
        ));
        assert!(error.detail.unwrap().contains(crate::redact::REDACTED));
    }
}
