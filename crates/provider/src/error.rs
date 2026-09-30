//! `octocrab::Error` → `AppError` 的统一映射（docs/PLAN.md M4.1 第 3 点）。
//!
//! # 为什么映射在这一层
//!
//! services / commands 只认 `AppError` 的稳定错误码；octocrab 的错误形态
//! （enum + `GitHubError` 正文）是实现细节，不允许越过 provider 边界。
//! 映射规则与 [`crate::client`] 的状态码映射**共用同一张表**
//! （[`crate::client::map_github_failure`]），保证"自建请求"与
//! "octocrab 请求"对同一个 HTTP 状态给出同一个错误码。
//!
//! # 非状态码错误的归类
//!
//! - `Reqwest` / `Http` / `Hyper` / `Service` → `NETWORK`（传输层）；
//! - `Json` / `Serde` 等 → `INTERNAL`：GitHub 的响应不符合预期，重试与
//!   重新登录都解决不了，属于"需要看日志"的问题；
//! - `UriParse` / `Uri` / `InvalidHeaderValue` / `JWT` → `VALIDATION`：
//!   我们构造的请求就有问题（配置错误），不该外发；
//! - `Installation*` → `AUTH_REQUIRED`：GitHub App 的安装令牌缺失；
//! - `Graphql` → `INTERNAL` 起步：GraphQL 错误没有统一的状态码，
//!   细分（NOT_FOUND / FORBIDDEN 等）等 T4.7 PR 服务接入 GraphQL 时
//!   按第一批真实错误样本再补。
//!
//! 所有 `detail` 均过 [`crate::redact::redact_tokens`]（红线 R8）。

use forgedesk_domain::AppError;

use crate::client::map_github_failure;
use crate::redact::redact_tokens;

/// 把 octocrab 的错误映射为稳定错误码。`has_credentials` 决定 401 的走向。
pub fn map_octocrab_error(err: octocrab::Error, has_credentials: bool) -> AppError {
    use forgedesk_domain::ErrorCode;
    use octocrab::Error as O;

    // octocrab::Error 是 #[non_exhaustive]：通配分支兜住未来新增的变体。
    // detail 在 match 前算好——错误路径不在意这点开销，换来匹配臂可以直接用。
    let fallback_detail = redact_tokens(&err.to_string());
    match err {
        O::GitHub { source, .. } => {
            // detail 用 GitHubError 的完整展示（含 documentation_url），已脱敏
            let detail = redact_tokens(&source.to_string());
            map_github_failure(
                source.status_code,
                Some(&source.message),
                Some(&detail),
                has_credentials,
                None,
            )
        }
        O::Reqwest { source, .. } => crate::client::map_transport_error(source),
        // 各变体的 source 类型互不相同，不能合进同一个匹配臂
        O::Http { source, .. } => crate::client::network_error(
            "GitHub request failed at the transport layer",
            redact_tokens(&source.to_string()),
        ),
        O::Hyper { source, .. } => crate::client::network_error(
            "GitHub request failed at the transport layer",
            redact_tokens(&source.to_string()),
        ),
        O::Service { source, .. } => crate::client::network_error(
            "GitHub request failed at the transport layer",
            redact_tokens(&source.to_string()),
        ),
        O::Installation { .. } | O::InstallationTokenInvalidAuth { .. } => AppError::new(
            ErrorCode::AuthRequired,
            "GitHub App authorization is required for this operation",
        ),
        O::Graphql { source, .. } => {
            AppError::new(ErrorCode::Internal, "GitHub GraphQL query failed")
                .with_detail(redact_tokens(&source.to_string()))
        }
        O::UriParse { source, .. } => AppError::new(
            ErrorCode::Validation,
            "invalid GitHub API request configuration",
        )
        .with_detail(redact_tokens(&source.to_string())),
        O::Uri { source, .. } => AppError::new(
            ErrorCode::Validation,
            "invalid GitHub API request configuration",
        )
        .with_detail(redact_tokens(&source.to_string())),
        O::InvalidHeaderValue { source, .. } => AppError::new(
            ErrorCode::Validation,
            "invalid GitHub API request configuration",
        )
        .with_detail(redact_tokens(&source.to_string())),
        O::JWT { source, .. } => AppError::new(
            ErrorCode::Validation,
            "invalid GitHub API request configuration",
        )
        .with_detail(redact_tokens(&source.to_string())),
        // 响应解析失败 / 编码失败：内部错误，detail 留排查线索
        O::Json { source, .. } => {
            AppError::new(ErrorCode::Internal, "unexpected GitHub API response")
                .with_detail(redact_tokens(&source.to_string()))
        }
        O::Serde { source, .. } => {
            AppError::new(ErrorCode::Internal, "unexpected GitHub API response")
                .with_detail(redact_tokens(&source.to_string()))
        }
        O::SerdeUrlEncoded { source, .. } => {
            AppError::new(ErrorCode::Internal, "unexpected GitHub API response")
                .with_detail(redact_tokens(&source.to_string()))
        }
        O::InvalidUtf8 { source, .. } => {
            AppError::new(ErrorCode::Internal, "unexpected GitHub API response")
                .with_detail(redact_tokens(&source.to_string()))
        }
        O::Encoder { source, .. } => {
            AppError::new(ErrorCode::Internal, "unexpected GitHub API response")
                .with_detail(redact_tokens(&source.to_string()))
        }
        O::Other { source, .. } => {
            AppError::new(ErrorCode::Internal, "unexpected GitHub API response")
                .with_detail(redact_tokens(&source.to_string()))
        }
        _ => AppError::new(ErrorCode::Internal, "unexpected GitHub API response")
            .with_detail(fallback_detail),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::map_octocrab_error;
    use crate::client::{GitHubHttp, HttpConfig};
    use forgedesk_domain::ErrorCode;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// 用 wiremock 起一个"假 GitHub"，octocrab 经我们配置的 reqwest client
    /// 访问它——这是对 octocrab 请求路径的**契约测试**。
    fn octo_at(server: &MockServer, http: &GitHubHttp) -> octocrab::Octocrab {
        octocrab::Octocrab::builder()
            .personal_token("ghs_testtoken123")
            .base_uri(server.uri())
            .unwrap()
            .build_with_reqwest(http.raw_client().clone())
            .unwrap()
    }

    #[derive(Debug, serde::Deserialize)]
    struct User {
        #[allow(dead_code)]
        login: String,
    }

    async fn octocrab_error_for(status: u16, body: &str) -> forgedesk_domain::AppError {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/user"))
            .respond_with(ResponseTemplate::new(status).set_body_string(body))
            .mount(&server)
            .await;
        let http = GitHubHttp::new(HttpConfig::default()).unwrap();
        let octo = octo_at(&server, &http);
        let err = octo
            .get::<User, _, _>("/user", None::<&()>)
            .await
            .unwrap_err();
        map_octocrab_error(err, true)
    }

    #[tokio::test]
    async fn octocrab_not_found_maps_to_the_stable_not_found_code() {
        let error = octocrab_error_for(404, r#"{"message":"Not Found"}"#).await;

        assert_eq!(error.code, ErrorCode::NotFound);
        assert_eq!(error.message, "Not Found");
    }

    #[tokio::test]
    async fn octocrab_bad_credentials_maps_to_auth_expired() {
        let error = octocrab_error_for(401, r#"{"message":"Bad credentials"}"#).await;

        assert_eq!(error.code, ErrorCode::AuthExpired);
        assert!(!error.retryable);
    }

    #[tokio::test]
    async fn octocrab_validation_failure_maps_to_validation() {
        let error = octocrab_error_for(
            422,
            r#"{"message":"Validation Failed","errors":[{"resource":"Issue"}]}"#,
        )
        .await;

        assert_eq!(error.code, ErrorCode::Validation);
        assert!(error.detail.unwrap().contains("Validation Failed"));
    }

    #[tokio::test]
    async fn octocrab_rate_limit_text_maps_to_rate_limited() {
        let error =
            octocrab_error_for(403, r#"{"message":"API rate limit exceeded for 1.2.3.4"}"#).await;

        assert_eq!(error.code, ErrorCode::RateLimited);
    }

    #[tokio::test]
    async fn octocrab_server_error_maps_to_network() {
        let error = octocrab_error_for(500, r#"{"message":"boom"}"#).await;

        assert_eq!(error.code, ErrorCode::Network);
        assert!(error.retryable);
    }

    #[tokio::test]
    async fn an_unparseable_response_maps_to_internal() {
        // 200 但不是 JSON：octocrab 反序列化失败 → INTERNAL（重试无意义）
        let error = octocrab_error_for(200, "<html>not json</html>").await;

        assert_eq!(error.code, ErrorCode::Internal);
    }

    #[tokio::test]
    async fn details_never_leak_the_token_even_when_github_echoes_it() {
        // 恶意服务端把令牌回显进错误正文：detail 必须已脱敏
        let error =
            octocrab_error_for(401, r#"{"message":"Bad credentials ghs_testtoken123"}"#).await;

        let dumped = format!("{error:?}");
        assert!(!dumped.contains("ghs_testtoken123"), "{dumped}");
    }
}
