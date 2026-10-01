//! Issue 子服务的 GitHub 实现（T4.8）。
//!
//! # 与 PR 共用的端点
//!
//! GitHub 的 Issue 与 PR 共享 issue 端点族；区别在**列表语义**：
//! `GET /repos/{o}/{r}/issues` 会把 PR 也当 issue 返回（每个 PR 都有
//! 一个隐式 issue），靠条目里的 `pull_request` 键区分——列表映射时
//! 必须把它滤掉，否则"Issue 列表"里会混进 PR。评论端点则完全同形，
//! 类型直接复用 [`crate::pulls::PullComment`]（别名 [`IssueComment`]）。
//!
//! # 编辑/关开/指派都是同一个 PATCH
//!
//! `PATCH /repos/{o}/{r}/issues/{n}` 接受部分载荷：`title`/`body` 编辑、
//! `state`（open/closed）关开、`assignees` 指派（空数组 = 全部取消指派，
//! 这是 GitHub 的显式语义而不是遗漏）。这里拆成三个 trait 方法让调用方
//! 意图明确；`edit_issue` 对"两个字段都缺"的载荷在本地拒绝，不发空写。

use secrecy::SecretString;
use serde::Deserialize;

use forgedesk_domain::{AppError, ErrorCode};

use crate::client::{map_transport_error, ApiRequest};
use crate::github::GitHubProvider;
use crate::pulls::{GitHubComment, GitHubLogin, PullComment};
use crate::repos::MAX_PER_PAGE;
use crate::traits::IssueService;

/// Issue 列表的状态过滤。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueState {
    /// 只列开启中的。
    Open,
    /// 只列已关闭的。
    Closed,
    /// 全部。
    All,
}

impl IssueState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
            Self::All => "all",
        }
    }
}

/// Issue 列表条目。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueSummary {
    /// Issue 编号（同仓库内与 PR 共用一号段）。
    pub number: u64,
    /// 标题。
    pub title: String,
    /// `open` / `closed`。
    pub state: String,
    /// 发起人。
    pub author: String,
    /// 标签名（展示用；标签管理不在 T4.8 范围）。
    pub labels: Vec<String>,
    /// 当前指派人的 login。
    pub assignees: Vec<String>,
    /// 评论数。
    pub comments: u64,
    /// 创建时间（RFC3339）。
    pub created_at: Option<String>,
    /// 最近更新时间（RFC3339）。
    pub updated_at: Option<String>,
    /// 关闭时间（RFC3339）。
    pub closed_at: Option<String>,
}

/// Issue 详情（描述原文不出后端，services 层消毒为 HTML）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueDetail {
    /// 列表字段全量。
    pub summary: IssueSummary,
    /// 描述原文（Markdown，未清洗）。
    ///
    /// `skip_serializing`：与 PR 详情同一规则（红线 R8 的展示边界——
    /// 原始 Markdown 永不越过 IPC，展示层只拿消毒后的 HTML）。
    #[serde(skip_serializing)]
    pub body_markdown: Option<String>,
}

/// Issue 评论（与 PR 时间线评论同端点同形状，类型别名复用）。
pub type IssueComment = PullComment;

/// 可指派人（`GET /repos/{o}/{r}/assignees` 条目）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Assignee {
    /// login。
    pub login: String,
}

/// 一次 Issue 编辑的载荷（`None` 字段不动）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IssueEdit {
    /// 新标题。
    pub title: Option<String>,
    /// 新描述。
    pub body: Option<String>,
}

impl IssueEdit {
    /// 是否是空载荷（两个字段都没给）。
    fn is_empty(&self) -> bool {
        self.title.is_none() && self.body.is_none()
    }
}

/// Issue 一页 + 下一页游标。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssuePage {
    /// 本页内容。
    pub items: Vec<IssueSummary>,
    /// 下一页页码；`None` 表示没有更多。
    pub next_page: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
struct GitHubIssue {
    number: u64,
    title: String,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    user: Option<GitHubLogin>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    labels: Option<Vec<GitHubLabel>>,
    #[serde(default)]
    assignees: Option<Vec<GitHubLogin>>,
    #[serde(default)]
    comments: Option<u64>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    updated_at: Option<String>,
    #[serde(default)]
    closed_at: Option<String>,
    // 有这个键的就是 PR（隐式 issue），列表里要滤掉
    #[serde(default)]
    pull_request: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
struct GitHubLabel {
    name: String,
}

impl From<GitHubIssue> for IssueSummary {
    fn from(issue: GitHubIssue) -> Self {
        Self {
            number: issue.number,
            title: issue.title,
            state: issue.state.unwrap_or_default(),
            author: issue.user.map(|user| user.login).unwrap_or_default(),
            labels: issue
                .labels
                .unwrap_or_default()
                .into_iter()
                .map(|label| label.name)
                .collect(),
            assignees: issue
                .assignees
                .unwrap_or_default()
                .into_iter()
                .map(|user| user.login)
                .collect(),
            comments: issue.comments.unwrap_or(0),
            created_at: issue.created_at,
            updated_at: issue.updated_at,
            closed_at: issue.closed_at,
        }
    }
}

impl From<GitHubIssue> for IssueDetail {
    fn from(issue: GitHubIssue) -> Self {
        Self {
            summary: IssueSummary::from(issue.clone()),
            body_markdown: issue.body,
        }
    }
}

impl GitHubProvider {
    /// `issues[/number]` 的路径段（number 必须为正，防止拼出错误 URL）。
    fn issue_path(&self, owner: &str, repo: &str, number: Option<u64>) -> Result<String, AppError> {
        if let Some(number) = number {
            if number == 0 {
                return Err(AppError::new(
                    ErrorCode::Validation,
                    "issue number must be positive",
                ));
            }
        }
        Ok(match number {
            Some(number) => format!("{}/issues/{number}", self.repo_path(owner, repo)?),
            None => format!("{}/issues", self.repo_path(owner, repo)?),
        })
    }
}

#[async_trait::async_trait]
impl IssueService for GitHubProvider {
    async fn list_issues(
        &self,
        token: SecretString,
        owner: &str,
        repo: &str,
        state: IssueState,
        page: Option<u32>,
        per_page: Option<u32>,
    ) -> Result<IssuePage, AppError> {
        if let Some(value) = per_page {
            if value > MAX_PER_PAGE {
                return Err(AppError::new(
                    ErrorCode::Validation,
                    format!("per_page must not exceed {MAX_PER_PAGE}"),
                ));
            }
        }
        let mut query = vec![("state".to_owned(), state.as_str().to_owned())];
        if let Some(page) = page {
            if page > 1 {
                query.push(("page".to_owned(), page.to_string()));
            }
        }
        if let Some(per_page) = per_page {
            query.push(("per_page".to_owned(), per_page.to_string()));
        }
        let request = ApiRequest::get(self.issue_path(owner, repo, None)?)
            .with_bearer(token)
            .with_query(query);
        let response = self.http().send(&request).await?;
        let next_page = response
            .headers()
            .get("link")
            .and_then(|value| value.to_str().ok())
            .and_then(crate::repos::next_page_from_link_header);
        // 先取 Link 头再消费 body（json() 拿走 response 的所有权）
        let items = response
            .json::<Vec<GitHubIssue>>()
            .await
            .map_err(map_transport_error)?
            .into_iter()
            .filter(|issue| issue.pull_request.is_none())
            .map(IssueSummary::from)
            .collect();
        Ok(IssuePage { items, next_page })
    }

    async fn get_issue(
        &self,
        token: SecretString,
        owner: &str,
        repo: &str,
        number: u64,
    ) -> Result<IssueDetail, AppError> {
        let request =
            ApiRequest::get(self.issue_path(owner, repo, Some(number))?).with_bearer(token);
        let response = self.http().send(&request).await?;
        response
            .json::<GitHubIssue>()
            .await
            .map_err(map_transport_error)
            .map(Into::into)
    }

    async fn create_issue(
        &self,
        token: SecretString,
        owner: &str,
        repo: &str,
        title: &str,
        body: Option<&str>,
    ) -> Result<IssueDetail, AppError> {
        let trimmed = title.trim();
        if trimmed.is_empty() {
            return Err(AppError::new(
                ErrorCode::Validation,
                "issue title must not be empty",
            ));
        }
        let mut payload = serde_json::json!({ "title": trimmed });
        if let Some(body) = body.map(str::trim).filter(|body| !body.is_empty()) {
            payload["body"] = serde_json::Value::String(body.to_owned());
        }
        let request =
            ApiRequest::post_json(self.issue_path(owner, repo, None)?, payload).with_bearer(token);
        let response = self.http().send(&request).await?;
        response
            .json::<GitHubIssue>()
            .await
            .map_err(map_transport_error)
            .map(Into::into)
    }

    async fn edit_issue(
        &self,
        token: SecretString,
        owner: &str,
        repo: &str,
        number: u64,
        edit: IssueEdit,
    ) -> Result<IssueDetail, AppError> {
        if edit.is_empty() {
            return Err(AppError::new(
                ErrorCode::Validation,
                "issue edit must change title or body",
            ));
        }
        let mut payload = serde_json::json!({});
        if let Some(title) = edit.title.map(|title| title.trim().to_owned()) {
            if title.is_empty() {
                return Err(AppError::new(
                    ErrorCode::Validation,
                    "issue title must not be empty",
                ));
            }
            payload["title"] = serde_json::Value::String(title);
        }
        if let Some(body) = edit.body {
            payload["body"] = serde_json::Value::String(body);
        }
        let request = ApiRequest::patch_json(self.issue_path(owner, repo, Some(number))?, payload)
            .with_bearer(token);
        let response = self.http().send(&request).await?;
        response
            .json::<GitHubIssue>()
            .await
            .map_err(map_transport_error)
            .map(Into::into)
    }

    async fn set_issue_state(
        &self,
        token: SecretString,
        owner: &str,
        repo: &str,
        number: u64,
        open: bool,
    ) -> Result<IssueDetail, AppError> {
        let payload = serde_json::json!({ "state": if open { "open" } else { "closed" } });
        let request = ApiRequest::patch_json(self.issue_path(owner, repo, Some(number))?, payload)
            .with_bearer(token);
        let response = self.http().send(&request).await?;
        response
            .json::<GitHubIssue>()
            .await
            .map_err(map_transport_error)
            .map(Into::into)
    }

    async fn set_issue_assignees(
        &self,
        token: SecretString,
        owner: &str,
        repo: &str,
        number: u64,
        assignees: &[String],
    ) -> Result<IssueDetail, AppError> {
        // 空数组是"全部取消指派"，GitHub 的显式语义，原样放行
        let payload = serde_json::json!({ "assignees": assignees });
        let request = ApiRequest::patch_json(self.issue_path(owner, repo, Some(number))?, payload)
            .with_bearer(token);
        let response = self.http().send(&request).await?;
        response
            .json::<GitHubIssue>()
            .await
            .map_err(map_transport_error)
            .map(Into::into)
    }

    async fn list_issue_comments(
        &self,
        token: SecretString,
        owner: &str,
        repo: &str,
        number: u64,
    ) -> Result<Vec<IssueComment>, AppError> {
        let url = format!("{}/comments", self.issue_path(owner, repo, Some(number))?);
        let request = ApiRequest::get(url)
            .with_bearer(token)
            .with_query(vec![("per_page".to_owned(), MAX_PER_PAGE.to_string())]);
        let response = self.http().send(&request).await?;
        let comments = response
            .json::<Vec<GitHubComment>>()
            .await
            .map_err(map_transport_error)?
            .into_iter()
            .map(Into::into)
            .collect();
        Ok(comments)
    }

    async fn create_issue_comment(
        &self,
        token: SecretString,
        owner: &str,
        repo: &str,
        number: u64,
        body: &str,
    ) -> Result<IssueComment, AppError> {
        let trimmed = body.trim();
        if trimmed.is_empty() {
            return Err(AppError::new(
                ErrorCode::Validation,
                "comment body must not be empty",
            ));
        }
        let url = format!("{}/comments", self.issue_path(owner, repo, Some(number))?);
        let request =
            ApiRequest::post_json(url, serde_json::json!({ "body": trimmed })).with_bearer(token);
        let response = self.http().send(&request).await?;
        response
            .json::<GitHubComment>()
            .await
            .map_err(map_transport_error)
            .map(Into::into)
    }

    async fn list_assignees(
        &self,
        token: SecretString,
        owner: &str,
        repo: &str,
    ) -> Result<Vec<Assignee>, AppError> {
        let url = format!("{}/assignees", self.repo_path(owner, repo)?);
        let request = ApiRequest::get(url)
            .with_bearer(token)
            .with_query(vec![("per_page".to_owned(), MAX_PER_PAGE.to_string())]);
        let response = self.http().send(&request).await?;
        let assignees = response
            .json::<Vec<GitHubLogin>>()
            .await
            .map_err(map_transport_error)?
            .into_iter()
            .map(|user| Assignee { login: user.login })
            .collect();
        Ok(assignees)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use crate::client::{GitHubHttp, HttpConfig};
    use crate::github::GitHubProvider;
    use crate::issues::{IssueEdit, IssueState};
    use crate::traits::HostProvider;
    use forgedesk_domain::ErrorCode;
    use secrecy::SecretString;
    use std::time::Duration;
    use wiremock::matchers::{body_partial_json, method, path, query_param};
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
        SecretString::from("ghp_issues".to_owned())
    }

    fn issue_json(number: u64, title: &str) -> serde_json::Value {
        serde_json::json!({
            "number": number, "title": title, "state": "open",
            "user": {"login": "octocat"}, "body": "描述正文",
            "labels": [{"name": "bug"}, {"name": "p1"}],
            "assignees": [{"login": "hubot"}], "comments": 2,
            "created_at": "2026-10-01T00:00:00Z", "updated_at": "2026-10-01T01:00:00Z",
            "some_future_field": true
        })
    }

    #[tokio::test]
    async fn list_maps_fields_and_filters_pull_requests_out() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/octocat/x/issues"))
            .and(query_param("state", "open"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Link", format!("<{}/x?page=2>; rel=\"next\"", server.uri()))
                    .set_body_json(vec![
                        issue_json(1, "Crash on open"),
                        // GitHub 把 PR 混进 issue 列表：带 pull_request 键，必须滤掉
                        serde_json::json!({
                            "number": 2, "title": "A pull request", "state": "open",
                            "pull_request": {"html_url": "https://github.com/octocat/x/pull/2"}
                        }),
                    ]),
            )
            .mount(&server)
            .await;

        let page = provider_at(&server)
            .issues()
            .list_issues(token(), "octocat", "x", IssueState::Open, None, None)
            .await
            .unwrap();

        assert_eq!(page.next_page, Some(2));
        assert_eq!(page.items.len(), 1, "PR 不能混进 Issue 列表");
        let issue = &page.items[0];
        assert_eq!(issue.number, 1);
        assert_eq!(issue.author, "octocat");
        assert_eq!(issue.labels, vec!["bug".to_owned(), "p1".to_owned()]);
        assert_eq!(issue.assignees, vec!["hubot".to_owned()]);
        assert_eq!(issue.comments, 2);
    }

    #[tokio::test]
    async fn detail_carries_body_and_summary() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/octocat/x/issues/1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(issue_json(1, "Crash on open")))
            .mount(&server)
            .await;

        let detail = provider_at(&server)
            .issues()
            .get_issue(token(), "octocat", "x", 1)
            .await
            .unwrap();

        assert_eq!(detail.summary.title, "Crash on open");
        assert_eq!(detail.body_markdown.as_deref(), Some("描述正文"));
    }

    #[tokio::test]
    async fn create_sends_title_and_optional_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/repos/octocat/x/issues"))
            .and(body_partial_json(
                serde_json::json!({ "title": "新问题", "body": "步骤…" }),
            ))
            .respond_with(ResponseTemplate::new(201).set_body_json(issue_json(3, "新问题")))
            .expect(1)
            .mount(&server)
            .await;

        let created = provider_at(&server)
            .issues()
            .create_issue(token(), "octocat", "x", "  新问题  ", Some("  步骤…  "))
            .await
            .unwrap();
        assert_eq!(created.summary.number, 3);
        server.verify().await;
    }

    #[tokio::test]
    async fn empty_title_is_rejected_locally_for_create() {
        let server = MockServer::start().await;
        let error = provider_at(&server)
            .issues()
            .create_issue(token(), "octocat", "x", "   ", None)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Validation);
    }

    #[tokio::test]
    async fn edit_sends_only_the_fields_that_changed() {
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .and(path("/repos/octocat/x/issues/1"))
            .and(body_partial_json(serde_json::json!({ "title": "改名" })))
            .respond_with(ResponseTemplate::new(200).set_body_json(issue_json(1, "改名")))
            .expect(1)
            .mount(&server)
            .await;

        let edited = provider_at(&server)
            .issues()
            .edit_issue(
                token(),
                "octocat",
                "x",
                1,
                IssueEdit {
                    title: Some("  改名  ".to_owned()),
                    body: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(edited.summary.title, "改名");
        server.verify().await;
    }

    #[tokio::test]
    async fn an_empty_edit_is_rejected_without_a_request() {
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;

        let error = provider_at(&server)
            .issues()
            .edit_issue(token(), "octocat", "x", 1, IssueEdit::default())
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Validation);
        server.verify().await;
    }

    #[tokio::test]
    async fn state_and_assignees_go_through_the_same_patch_endpoint() {
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .and(path("/repos/octocat/x/issues/1"))
            .and(body_partial_json(serde_json::json!({ "state": "closed" })))
            .respond_with(ResponseTemplate::new(200).set_body_json(issue_json(1, "x")))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("PATCH"))
            .and(path("/repos/octocat/x/issues/2"))
            .and(body_partial_json(
                serde_json::json!({ "assignees": ["hubot", "octocat"] }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(issue_json(2, "y")))
            .expect(1)
            .mount(&server)
            .await;

        let provider = provider_at(&server);
        provider
            .issues()
            .set_issue_state(token(), "octocat", "x", 1, false)
            .await
            .unwrap();
        provider
            .issues()
            .set_issue_assignees(
                token(),
                "octocat",
                "x",
                2,
                &["hubot".to_owned(), "octocat".to_owned()],
            )
            .await
            .unwrap();
        server.verify().await;
    }

    #[tokio::test]
    async fn comments_round_trip_and_empty_body_is_rejected_locally() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/octocat/x/issues/1/comments"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                { "id": 5, "user": {"login": "hubot"}, "body": "ping", "created_at": "2026-10-01T00:00:00Z" }
            ])))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/repos/octocat/x/issues/1/comments"))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!(
                { "id": 6, "user": {"login": "octocat"}, "body": "pong" }
            )))
            .mount(&server)
            .await;

        let provider = provider_at(&server);
        let comments = provider
            .issues()
            .list_issue_comments(token(), "octocat", "x", 1)
            .await
            .unwrap();
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].author, "hubot");

        let created = provider
            .issues()
            .create_issue_comment(token(), "octocat", "x", 1, "  pong  ")
            .await
            .unwrap();
        assert_eq!(created.body, "pong");

        let error = provider
            .issues()
            .create_issue_comment(token(), "octocat", "x", 1, "   ")
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Validation);
    }

    #[tokio::test]
    async fn assignees_list_maps_logins() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/octocat/x/assignees"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                { "login": "hubot" }, { "login": "octocat" }
            ])))
            .mount(&server)
            .await;

        let assignees = provider_at(&server)
            .issues()
            .list_assignees(token(), "octocat", "x")
            .await
            .unwrap();
        assert_eq!(
            assignees
                .iter()
                .map(|a| a.login.as_str())
                .collect::<Vec<_>>(),
            vec!["hubot", "octocat"]
        );
    }

    #[test]
    fn zero_and_invalid_references_are_rejected_locally() {
        let server_uri = "http://127.0.0.1:1".to_owned();
        let http = GitHubHttp::new(HttpConfig::default()).unwrap();
        let provider =
            GitHubProvider::with_endpoints("github.com", "x", http, server_uri.clone(), server_uri);

        let error = provider.issue_path("octocat", "x", Some(0)).unwrap_err();
        assert_eq!(error.code, ErrorCode::Validation);
        let error = provider.issue_path("", "x", None).unwrap_err();
        assert_eq!(error.code, ErrorCode::Validation);
    }
}
