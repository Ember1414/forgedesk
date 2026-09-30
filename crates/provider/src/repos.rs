//! 仓库子服务的 GitHub 实现：列表、星标、搜索、fork。
//!
//! # 为什么走自建 HTTP 而不是 octocrab
//!
//! 这批端点是"一问一答"的 REST 列表：自建 [`ApiRequest`] 路径已经具备
//! UA/超时/代理/限流捕获/错误映射，且限流头逐响应进 tracker（octocrab
//! 路径做不到，见 `client.rs` 的取舍说明）。octocrab 留给 T4.7 的
//! GraphQL 优先场景（PR 时间线等复杂查询）。
//!
//! # 分页
//!
//! GitHub 用 `Link` 头表达下一页（`rel="next"`）。这里解析出 `page` 参数
//! 作为游标返回给调用方（UI 的无限滚动每次带 `next_page` 追加一页）；
//! 没有 `Link` 或没有 `next` 就是最后一页。
//!
//! # 匿名与配额
//!
//! `/search/repositories` 与 `GET /repos/{owner}/{repo}` 匿名可用（低配额）；
//! 其余（`/user/repos`、star、fork）必须有令牌——令牌缺失时是 **401 →
//! `AUTH_EXPIRED`/`AUTH_REQUIRED`** 的既有映射，这里不再重复判断。

use secrecy::SecretString;
use serde::Deserialize;

use forgedesk_domain::{AppError, ErrorCode};

use crate::client::{map_transport_error, ApiRequest};
use crate::github::GitHubProvider;
use crate::traits::RepoService;

/// 单页参数的合法上限（GitHub 硬限制 100，超出会 `VALIDATION`）。
pub const MAX_PER_PAGE: u32 = 100;

/// 已登录用户视角的仓库列表范围（对应 affiliation 参数）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepoListScope {
    /// 我拥有的（`affiliation=owner`）。
    Owned,
    /// 我拥有的 + 协作 + 组织成员（`affiliation=owner,collaborator,organization_member`）。
    All,
}

impl RepoListScope {
    fn affiliation(self) -> &'static str {
        match self {
            Self::Owned => "owner",
            Self::All => "owner,collaborator,organization_member",
        }
    }
}

/// 远端托管平台上的一份仓库（列表与详情共用；字段刻意精简）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteRepo {
    /// 平台内的数字 id。
    pub id: u64,
    /// 所有者登录名（用户或组织）。
    pub owner: String,
    /// 仓库名。
    pub name: String,
    /// `owner/name`。
    pub full_name: String,
    /// 描述。
    pub description: Option<String>,
    /// 网页地址。
    pub html_url: String,
    /// 默认分支（空仓库可能为 `None`）。
    pub default_branch: Option<String>,
    /// 是否私有。
    pub private: bool,
    /// 是否 fork。
    pub fork: bool,
    /// Star 数。
    pub stars: u64,
    /// 最近一次 push（RFC3339 字符串，展示层解析）。
    pub pushed_at: Option<String>,
}

/// 一页仓库列表 + 下一页游标（`None` = 没有更多）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoPage {
    /// 本页内容。
    pub items: Vec<RemoteRepo>,
    /// 下一页页码；`None` 表示没有更多了。
    pub next_page: Option<u32>,
}

/// GitHub 仓库 JSON（只取需要的字段，未知字段忽略）。
#[derive(Debug, Deserialize)]
struct GitHubRepo {
    id: u64,
    name: String,
    full_name: String,
    #[serde(default)]
    owner: Option<GitHubOwner>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    html_url: Option<String>,
    #[serde(default)]
    default_branch: Option<String>,
    #[serde(default)]
    private: bool,
    #[serde(default)]
    fork: bool,
    #[serde(default)]
    stargazers_count: u64,
    #[serde(default)]
    pushed_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GitHubOwner {
    login: String,
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    #[serde(default)]
    items: Vec<GitHubRepo>,
}

impl From<GitHubRepo> for RemoteRepo {
    fn from(repo: GitHubRepo) -> Self {
        Self {
            id: repo.id,
            owner: repo.owner.map(|owner| owner.login).unwrap_or_default(),
            name: repo.name,
            full_name: repo.full_name,
            description: repo.description,
            html_url: repo.html_url.unwrap_or_default(),
            default_branch: repo.default_branch,
            private: repo.private,
            fork: repo.fork,
            stars: repo.stargazers_count,
            pushed_at: repo.pushed_at,
        }
    }
}

/// 从 `Link` 头解析 `rel="next"` 的页码。
///
/// 形如 `<https://api.github.com/user/repos?page=2>; rel="next", <…>; rel="last"`。
/// 手写解析（不引 url 依赖）：只需要"有没有 next"和它的 page 值。
#[must_use]
pub fn next_page_from_link_header(header: &str) -> Option<u32> {
    for segment in header.split(',') {
        if !segment.contains("rel=\"next\"") && !segment.contains("rel=next") {
            continue;
        }
        let url = segment.split('<').nth(1)?.split('>').next()?;
        // 页码是查询串里的 page 参数
        for pair in url.split('?').nth(1)?.split('&') {
            if let Some(value) = pair.strip_prefix("page=") {
                return value.parse().ok();
            }
        }
    }
    None
}

fn clamp_per_page(per_page: Option<u32>) -> Result<Option<u32>, AppError> {
    match per_page {
        Some(value) if value > MAX_PER_PAGE => Err(AppError::new(
            ErrorCode::Validation,
            format!("per_page must not exceed {MAX_PER_PAGE}"),
        )),
        other => Ok(other),
    }
}

fn page_query(page: Option<u32>, per_page: Option<u32>) -> Vec<(String, String)> {
    let mut query = Vec::new();
    if let Some(page) = page {
        if page > 1 {
            query.push(("page".to_owned(), page.to_string()));
        }
    }
    if let Some(per_page) = per_page {
        query.push(("per_page".to_owned(), per_page.to_string()));
    }
    query
}

impl GitHubProvider {
    /// 发送列表请求：解析分页与正文（2xx 之外已由 [`GitHubHttp::send`] 映射）。
    async fn repo_page(&self, request: ApiRequest) -> Result<RepoPage, AppError> {
        let response = self.http().send(&request).await?;
        let next_page = response
            .headers()
            .get("link")
            .and_then(|value| value.to_str().ok())
            .and_then(next_page_from_link_header);
        let items = response
            .json::<Vec<GitHubRepo>>()
            .await
            .map_err(map_transport_error)?
            .into_iter()
            .map(Into::into)
            .collect();
        Ok(RepoPage { items, next_page })
    }

    /// `owner/repo` 的 API 路径前缀（含校验：两个段都不能为空或含 `/`）。
    pub(crate) fn repo_path(&self, owner: &str, repo: &str) -> Result<String, AppError> {
        for part in [owner, repo] {
            let part = part.trim();
            if part.is_empty() || part.contains('/') {
                return Err(AppError::new(
                    ErrorCode::Validation,
                    format!("invalid repository reference: {owner}/{repo}"),
                ));
            }
        }
        Ok(format!(
            "{}/repos/{}/{}",
            self.api_base(),
            owner.trim(),
            repo.trim()
        ))
    }
}

#[async_trait::async_trait]
impl RepoService for GitHubProvider {
    async fn list_authenticated(
        &self,
        token: SecretString,
        scope: RepoListScope,
        page: Option<u32>,
        per_page: Option<u32>,
    ) -> Result<RepoPage, AppError> {
        let url = format!("{}/user/repos", self.api_base());
        let per_page = clamp_per_page(per_page)?;
        let mut query = vec![("affiliation".to_owned(), scope.affiliation().to_owned())];
        query.extend(page_query(page, per_page));
        let request = ApiRequest::get(url).with_bearer(token).with_query(query);
        self.repo_page(request).await
    }

    async fn list_starred(
        &self,
        token: SecretString,
        page: Option<u32>,
        per_page: Option<u32>,
    ) -> Result<RepoPage, AppError> {
        let url = format!("{}/user/starred", self.api_base());
        let per_page = clamp_per_page(per_page)?;
        let request = ApiRequest::get(url)
            .with_bearer(token)
            .with_query(page_query(page, per_page));
        self.repo_page(request).await
    }

    async fn search(
        &self,
        query: &str,
        token: Option<SecretString>,
        page: Option<u32>,
        per_page: Option<u32>,
    ) -> Result<RepoPage, AppError> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return Err(AppError::new(
                ErrorCode::Validation,
                "search query must not be empty",
            ));
        }
        let url = format!("{}/search/repositories", self.api_base());
        let per_page = clamp_per_page(per_page)?;
        let mut params = vec![("q".to_owned(), trimmed.to_owned())];
        params.extend(page_query(page, per_page));
        let mut request = ApiRequest::get(url).with_query(params);
        if let Some(token) = token {
            request = request.with_bearer(token);
        }
        let response = self.http().send(&request).await?;
        let next_page = response
            .headers()
            .get("link")
            .and_then(|value| value.to_str().ok())
            .and_then(next_page_from_link_header);
        let items = response
            .json::<SearchResponse>()
            .await
            .map_err(map_transport_error)?
            .items
            .into_iter()
            .map(Into::into)
            .collect();
        Ok(RepoPage { items, next_page })
    }

    async fn get(
        &self,
        owner: &str,
        repo: &str,
        token: Option<SecretString>,
    ) -> Result<RemoteRepo, AppError> {
        let url = self.repo_path(owner, repo)?;
        let mut request = ApiRequest::get(url);
        if let Some(token) = token {
            request = request.with_bearer(token);
        }
        let response = self.http().send(&request).await?;
        response
            .json::<GitHubRepo>()
            .await
            .map_err(map_transport_error)
            .map(Into::into)
    }

    async fn set_starred(
        &self,
        token: SecretString,
        owner: &str,
        repo: &str,
        starred: bool,
    ) -> Result<(), AppError> {
        let url = self.repo_path(owner, repo)?;
        let method = if starred {
            reqwest::Method::PUT
        } else {
            reqwest::Method::DELETE
        };
        let request = ApiRequest::new(method, format!("{url}/starred"))
            .with_bearer(token)
            // star 端点对 202/204 的空体响应不需要 JSON Accept，但
            // GitHub 文档要求带版本化 Accept 防止将来默认行为漂移
            .with_header("Accept", "application/vnd.github+json")?;
        let response = self.http().send(&request).await?;
        // 丢弃空体（204），只确认连接正常关闭
        let _ = response.bytes().await.map_err(map_transport_error)?;
        Ok(())
    }

    async fn fork(
        &self,
        token: SecretString,
        owner: &str,
        repo: &str,
    ) -> Result<RemoteRepo, AppError> {
        let url = format!("{}/forks", self.repo_path(owner, repo)?);
        let request = ApiRequest::post_json(url, serde_json::json!({})).with_bearer(token);
        let response = self.http().send(&request).await?;
        // 202 Accepted：fork 是异步任务，返回的副本此刻可能还在创建中
        response
            .json::<GitHubRepo>()
            .await
            .map_err(map_transport_error)
            .map(Into::into)
    }

    async fn readme(
        &self,
        owner: &str,
        repo: &str,
        token: Option<SecretString>,
    ) -> Result<String, AppError> {
        let url = format!("{}/readme", self.repo_path(owner, repo)?);
        let mut request = ApiRequest::get(url)
            // raw：直接拿 Markdown 原文，省一次 base64 解码与 JSON 包裹
            .with_header("Accept", "application/vnd.github.raw")?;
        if let Some(token) = token {
            request = request.with_bearer(token);
        }
        let response = self.http().send(&request).await?;
        // README 按字节原样回传：先取字节再转 UTF-8，避免 reqwest 按错误
        // 的字符集猜测造成替换符
        let bytes = response.bytes().await.map_err(map_transport_error)?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{next_page_from_link_header, RemoteRepo, RepoListScope};
    use crate::client::{GitHubHttp, HttpConfig};
    use crate::github::GitHubProvider;
    use crate::traits::HostProvider;
    use forgedesk_domain::ErrorCode;
    use secrecy::SecretString;
    use std::time::Duration;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn provider_at(server: &MockServer) -> GitHubProvider {
        let http = GitHubHttp::new(HttpConfig {
            backoff_base: Duration::from_millis(1),
            ..HttpConfig::default()
        })
        .unwrap();
        let uri = server.uri();
        GitHubProvider::with_endpoints("github.com", "Iv1.test", http, uri.clone(), uri)
    }

    fn token() -> SecretString {
        SecretString::from("ghp_list".to_owned())
    }

    fn repo_json(id: u64, name: &str) -> serde_json::Value {
        serde_json::json!({
            "id": id, "name": name, "full_name": format!("octocat/{name}"),
            "owner": {"login": "octocat"},
            "description": "desc", "html_url": format!("https://github.com/octocat/{name}"),
            "default_branch": "main", "private": false, "fork": false,
            "stargazers_count": 3, "pushed_at": "2026-09-30T00:00:00Z",
            "some_future_field": true
        })
    }

    #[tokio::test]
    async fn list_maps_fields_and_parses_the_next_page_cursor() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/user/repos"))
            .and(query_param("affiliation", "owner"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header(
                        "Link",
                        format!(
                            "<{}/user/repos?page=2&per_page=30>; rel=\"next\", <{}/user/repos?page=9>; rel=\"last\"",
                            server.uri(),
                            server.uri()
                        ),
                    )
                    .set_body_json(vec![repo_json(1, "Hello-World"), repo_json(2, "Spoon-Knife")]),
            )
            .mount(&server)
            .await;

        let page = provider_at(&server)
            .repos()
            .list_authenticated(token(), RepoListScope::Owned, None, None)
            .await
            .unwrap();

        assert_eq!(page.items.len(), 2);
        assert_eq!(page.next_page, Some(2));
        let repo: RemoteRepo = page.items[0].clone();
        assert_eq!(repo.full_name, "octocat/Hello-World");
        assert_eq!(repo.owner, "octocat");
        assert_eq!(repo.stars, 3);
        assert!(!repo.private);
    }

    #[tokio::test]
    async fn a_page_without_a_link_header_is_the_last_page() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(Vec::<serde_json::Value>::new()))
            .mount(&server)
            .await;

        let page = provider_at(&server)
            .repos()
            .list_starred(token(), None, None)
            .await
            .unwrap();

        assert!(page.items.is_empty());
        assert_eq!(page.next_page, None);
    }

    #[tokio::test]
    async fn search_carries_the_query_and_works_anonymously() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/search/repositories"))
            .and(query_param("q", "forgedesk language:rust"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                serde_json::json!({ "total_count": 1, "items": [repo_json(7, "forgedesk")] }),
            ))
            .expect(1)
            .mount(&server)
            .await;

        // 不带令牌的匿名搜索
        let page = provider_at(&server)
            .repos()
            .search("forgedesk language:rust", None, None, Some(30))
            .await
            .unwrap();

        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].name, "forgedesk");
    }

    #[tokio::test]
    async fn an_empty_search_query_is_rejected_without_a_request() {
        let server = MockServer::start().await;
        let error = provider_at(&server)
            .repos()
            .search("   ", None, None, None)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Validation);
    }

    #[tokio::test]
    async fn starring_uses_put_and_unstarring_uses_delete() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/repos/octocat/Hello-World/starred"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/repos/octocat/Hello-World/starred"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        let provider = provider_at(&server);
        provider
            .repos()
            .set_starred(token(), "octocat", "Hello-World", true)
            .await
            .unwrap();
        provider
            .repos()
            .set_starred(token(), "octocat", "Hello-World", false)
            .await
            .unwrap();

        server.verify().await;
    }

    #[tokio::test]
    async fn fork_accepts_the_202_and_returns_the_copy() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/repos/octocat/Hello-World/forks"))
            .respond_with(ResponseTemplate::new(202).set_body_json(repo_json(99, "Hello-World")))
            .mount(&server)
            .await;

        let forked = provider_at(&server)
            .repos()
            .fork(token(), "octocat", "Hello-World")
            .await
            .unwrap();

        assert_eq!(forked.id, 99);
        assert_eq!(forked.owner, "octocat");
    }

    #[test]
    fn repo_references_are_validated_before_they_reach_the_url() {
        let server_uri = "http://127.0.0.1:1".to_owned();
        let http = GitHubHttp::new(HttpConfig::default()).unwrap();
        let provider = GitHubProvider::with_endpoints(
            "github.com",
            "Iv1.x",
            http,
            server_uri.clone(),
            server_uri,
        );

        // 空 owner / 带 / 的名字：直接 VALIDATION，不发请求
        let error = provider.repo_path("", "x").unwrap_err();
        assert_eq!(error.code, ErrorCode::Validation);
        let error = provider.repo_path("octocat", "a/b").unwrap_err();
        assert_eq!(error.code, ErrorCode::Validation);
    }

    #[test]
    fn link_header_parsing_covers_both_quote_styles_and_misses() {
        assert_eq!(
            next_page_from_link_header("<https://api/x?page=2>; rel=\"next\""),
            Some(2)
        );
        assert_eq!(
            next_page_from_link_header("<https://api/x?page=7>; rel=next"),
            Some(7)
        );
        assert_eq!(
            next_page_from_link_header(
                "<https://api/x?page=9>; rel=\"last\", <https://api/x?page=2>; rel=\"next\""
            ),
            Some(2)
        );
        assert_eq!(
            next_page_from_link_header("<https://api/x>; rel=\"last\""),
            None
        );
        assert_eq!(next_page_from_link_header(""), None);
    }

    #[tokio::test]
    async fn readme_returns_the_raw_markdown_and_maps_404_to_not_found() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/octocat/Hello-World/readme"))
            .and(header("accept", "application/vnd.github.raw"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                "# Hello

正文 <script>alert(1)</script>",
            ))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/repos/octocat/Empty/readme"))
            .respond_with(ResponseTemplate::new(404).set_body_string(r#"{"message":"Not Found"}"#))
            .mount(&server)
            .await;

        let provider = provider_at(&server);
        let markdown = provider
            .repos()
            .readme("octocat", "Hello-World", None)
            .await
            .unwrap();
        assert!(markdown.starts_with("# Hello"), "{markdown}");

        let error = provider
            .repos()
            .readme("octocat", "Empty", None)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::NotFound);
    }

    /// per_page 超过 GitHub 硬上限 100 时在本地拒绝（避免一次注定失败的请求）。
    #[tokio::test]
    async fn per_page_above_the_limit_is_rejected_locally() {
        let server = MockServer::start().await;
        let error = provider_at(&server)
            .repos()
            .list_authenticated(token(), RepoListScope::All, None, Some(101))
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Validation);
    }

    /// 令牌不进列表请求的日志/错误（红线 R8 的常规复查点）。
    #[tokio::test]
    async fn a_rejected_token_error_carries_no_secret() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(401).set_body_string(r#"{"message":"Bad credentials"}"#),
            )
            .mount(&server)
            .await;

        let error = provider_at(&server)
            .repos()
            .list_authenticated(
                SecretString::from("ghp_supersecret".to_owned()),
                RepoListScope::Owned,
                None,
                None,
            )
            .await
            .unwrap_err();

        assert_eq!(error.code, ErrorCode::AuthExpired);
        assert!(!format!("{error:?}").contains("ghp_supersecret"));
    }
}
