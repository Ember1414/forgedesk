//! Pull Request 子服务的 GitHub 实现（T4.7 后端第一批）。
//!
//! # 与 T4.5 仓库服务同一套 HTTP 底座
//!
//! 自建 [`ApiRequest`] 路径：限流头逐响应进 tracker、错误统一映射、
//! 重试/超时/代理全部生效。PLAN M4.2 提到"GraphQL 优先"：那是为
//! 时间线（评论+checks+status 混合加载）省请求数；列表/详情/合并
//! 每次动作本来就只有一个请求，REST 形态已足够，GraphQL 留给
//! 时间线视图落地时再做（回退路径 REST 即现状）。
//!
//! # 合并的错误语义（PLAN：失败返回可读原因）
//!
//! GitHub 合并端点的失败状态码在此语境下含义明确，映射为：
//!
//! | GitHub | 语义 | ErrorCode / hint |
//! | --- | --- | --- |
//! | 405 | PR 不可合并（冲突未解 / 分支保护 / 已合并） | `VALIDATION` + `not-mergeable` |
//! | 409 | head 与服务器记录不一致 | `GIT_CONFLICT` + `conflict` |
//! | 422 | `sha` 预检不匹配（打开详情后远端又有新提交） | `VALIDATION` + `head-changed` |
//!
//! GitHub 的 message（如 "Pull Request is not mergeable"）原样保留在
//! `message`/`detail` 里——它就是"可读原因"本体。
//!
//! # 合并与删除分支是两个动作
//!
//! `delete_branch = true` 时在合并成功**之后**单独调 refs API；
//! 删除失败不回滚合并（合并已生效，只记警告并在结果里如实上报）。

use secrecy::SecretString;
use serde::Deserialize;

use forgedesk_domain::{AppError, ErrorCode};

use crate::client::{map_transport_error, ApiRequest};
use crate::github::GitHubProvider;
use crate::repos::MAX_PER_PAGE;
use crate::traits::PullService;

/// PR 列表的状态过滤。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PullState {
    /// 只列开启中的。
    Open,
    /// 只列已关闭/已合并的。
    Closed,
    /// 全部。
    All,
}

impl PullState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
            Self::All => "all",
        }
    }
}

/// 合并策略（对应 GitHub 的 merge_method）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeStrategy {
    /// 产生合并提交。
    Merge,
    /// 压扁为单个提交。
    Squash,
    /// 变基式线性追加。
    Rebase,
}

impl MergeStrategy {
    fn as_str(self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::Squash => "squash",
            Self::Rebase => "rebase",
        }
    }
}

/// PR 列表条目。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestSummary {
    /// PR 编号。
    pub number: u64,
    /// 标题。
    pub title: String,
    /// `open` / `closed`（已合并的 PR 状态也是 closed，看 `merged`）。
    pub state: String,
    /// 是否草稿。
    pub draft: bool,
    /// 是否已合并。
    pub merged: bool,
    /// 发起人。
    pub author: String,
    /// 源分支标签（`owner:branch`）。
    pub head_label: String,
    /// 目标分支标签。
    pub base_label: String,
    /// 网页地址。
    pub html_url: String,
    /// 创建时间（RFC3339）。
    pub created_at: Option<String>,
    /// 最近更新时间（RFC3339）。
    pub updated_at: Option<String>,
}

/// PR 一页 + 下一页游标。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PullPage {
    /// 本页内容。
    pub items: Vec<PullRequestSummary>,
    /// 下一页页码；`None` 表示没有更多。
    pub next_page: Option<u32>,
}

/// PR 详情（合并条件判断的全部输入）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestDetail {
    /// 列表字段全量。
    pub summary: PullRequestSummary,
    /// 描述原文（Markdown，未清洗）。
    ///
    /// `skip_serializing`：原始 Markdown **永不**越过 IPC（展示层只拿
    /// services 消毒后的 `bodyHtml`，与 README 同一规则）。
    #[serde(skip_serializing)]
    pub body_markdown: Option<String>,
    /// 变更文件数。
    pub changed_files: u64,
    /// 新增行数。
    pub additions: u64,
    /// 删除行数。
    pub deletions: u64,
    /// 是否可合并（GitHub 计算；`None` = 计算中）。
    pub mergeable: Option<bool>,
    /// 合并状态（`clean` / `dirty` / `blocked` / `unstable`…，原样透出供 UI 分档）。
    pub mergeable_state: Option<String>,
    /// 当前 head 的 sha（合并预检用）。
    pub head_sha: String,
}

/// 一条 review。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PullReview {
    /// review id。
    pub id: u64,
    /// reviewer。
    pub author: String,
    /// `APPROVED` / `CHANGES_REQUESTED` / `COMMENTED`。
    pub state: String,
    /// review 正文。
    pub body: Option<String>,
    /// 提交时间（RFC3339）。
    pub submitted_at: Option<String>,
}

/// 合并请求参数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergePullRequest {
    /// 合并策略。
    pub strategy: MergeStrategy,
    /// 自定义提交标题（仅 merge/squash 生效；缺省用 GitHub 默认）。
    pub commit_title: Option<String>,
    /// 自定义提交正文。
    pub commit_message: Option<String>,
    /// 预检 head sha：打开详情时的 sha；期间远端有新提交则合并被拒（422）。
    pub expected_head_sha: Option<String>,
    /// 合并成功后删除源分支。
    ///
    /// 实际删除还需要 [`Self::head_branch`] 给出分支名（GitHub 的 refs API
    /// 认 branch 而非 `owner:branch` 标签）：两者齐备才执行，
    /// 否则如实跳过并在结果里报告 `branch_deleted = false`。
    pub delete_branch: bool,
    /// 源分支名（来自详情 `head_label` 的 branch 段）。
    pub head_branch: Option<String>,
}

impl MergePullRequest {
    /// 是否要在合并成功后删除源分支（参数齐备才执行）。
    fn wants_branch_delete(&self) -> bool {
        self.delete_branch && self.head_branch.is_some()
    }
}

/// 合并结果。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeOutcome {
    /// GitHub 确认已合并。
    pub merged: bool,
    /// 合并提交的 sha。
    pub sha: Option<String>,
    /// GitHub 的说明。
    pub message: Option<String>,
    /// 是否执行了源分支删除。
    pub branch_deleted: bool,
}

#[derive(Debug, Deserialize)]
struct GitHubPull {
    number: u64,
    title: String,
    state: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    merged: bool,
    #[serde(default)]
    user: Option<GitHubLogin>,
    #[serde(default)]
    head: Option<GitHubRef>,
    #[serde(default)]
    base: Option<GitHubRef>,
    #[serde(default)]
    html_url: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    updated_at: Option<String>,
    // 以下字段列表响应为 null/缺省、详情响应才有值
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    changed_files: Option<u64>,
    #[serde(default)]
    additions: Option<u64>,
    #[serde(default)]
    deletions: Option<u64>,
    #[serde(default)]
    mergeable: Option<bool>,
    #[serde(default)]
    mergeable_state: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GitHubLogin {
    login: String,
}

#[derive(Debug, Deserialize)]
struct GitHubRef {
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    sha: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GitHubReview {
    id: u64,
    #[serde(default)]
    user: Option<GitHubLogin>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    submitted_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MergeResponse {
    #[serde(default)]
    merged: bool,
    #[serde(default)]
    sha: Option<String>,
    #[serde(default)]
    message: Option<String>,
}

impl From<GitHubPull> for PullRequestSummary {
    fn from(pull: GitHubPull) -> Self {
        Self {
            number: pull.number,
            title: pull.title,
            state: pull.state,
            draft: pull.draft,
            merged: pull.merged,
            author: pull.user.map(|user| user.login).unwrap_or_default(),
            head_label: pull.head.and_then(|head| head.label).unwrap_or_default(),
            base_label: pull.base.and_then(|base| base.label).unwrap_or_default(),
            html_url: pull.html_url.unwrap_or_default(),
            created_at: pull.created_at,
            updated_at: pull.updated_at,
        }
    }
}

impl GitHubProvider {
    /// `pulls[/number]` 的路径段（number 必须为正，防止拼出错误 URL）。
    fn pull_path(&self, owner: &str, repo: &str, number: Option<u64>) -> Result<String, AppError> {
        if let Some(number) = number {
            if number == 0 {
                return Err(AppError::new(
                    ErrorCode::Validation,
                    "pull request number must be positive",
                ));
            }
        }
        Ok(match number {
            Some(number) => format!("{}/pulls/{number}", self.repo_path(owner, repo)?),
            None => format!("{}/pulls", self.repo_path(owner, repo)?),
        })
    }

    /// 解析 PR 的完整详情（列表与详情共用反序列化，字段按响应取舍）。
    fn pull_detail(&self, pull: GitHubPull) -> PullRequestDetail {
        let head_sha = pull
            .head
            .as_ref()
            .and_then(|head| head.sha.clone())
            .unwrap_or_default();
        PullRequestDetail {
            summary: PullRequestSummary {
                number: pull.number,
                title: pull.title,
                state: pull.state,
                draft: pull.draft,
                merged: pull.merged,
                author: pull.user.map(|user| user.login).unwrap_or_default(),
                head_label: pull.head.and_then(|head| head.label).unwrap_or_default(),
                base_label: pull.base.and_then(|base| base.label).unwrap_or_default(),
                html_url: pull.html_url.unwrap_or_default(),
                created_at: pull.created_at,
                updated_at: pull.updated_at,
            },
            body_markdown: pull.body,
            changed_files: pull.changed_files.unwrap_or(0),
            additions: pull.additions.unwrap_or(0),
            deletions: pull.deletions.unwrap_or(0),
            mergeable: pull.mergeable,
            mergeable_state: pull.mergeable_state,
            head_sha,
        }
    }
}

#[async_trait::async_trait]
impl PullService for GitHubProvider {
    async fn list_pulls(
        &self,
        token: SecretString,
        owner: &str,
        repo: &str,
        state: PullState,
        page: Option<u32>,
        per_page: Option<u32>,
    ) -> Result<PullPage, AppError> {
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
        let request = ApiRequest::get(self.pull_path(owner, repo, None)?)
            .with_bearer(token)
            .with_query(query);
        let response = self.http().send(&request).await?;
        let next_page = response
            .headers()
            .get("link")
            .and_then(|value| value.to_str().ok())
            .and_then(crate::repos::next_page_from_link_header);
        let items = response
            .json::<Vec<GitHubPull>>()
            .await
            .map_err(map_transport_error)?
            .into_iter()
            .map(Into::into)
            .collect();
        Ok(PullPage { items, next_page })
    }

    async fn get_pull(
        &self,
        token: SecretString,
        owner: &str,
        repo: &str,
        number: u64,
    ) -> Result<PullRequestDetail, AppError> {
        let request =
            ApiRequest::get(self.pull_path(owner, repo, Some(number))?).with_bearer(token);
        let response = self.http().send(&request).await?;
        let pull: GitHubPull = response.json().await.map_err(map_transport_error)?;
        Ok(self.pull_detail(pull))
    }

    async fn list_reviews(
        &self,
        token: SecretString,
        owner: &str,
        repo: &str,
        number: u64,
    ) -> Result<Vec<PullReview>, AppError> {
        let request = ApiRequest::get(format!(
            "{}/reviews",
            self.pull_path(owner, repo, Some(number))?
        ))
        .with_bearer(token);
        let response = self.http().send(&request).await?;
        let reviews = response
            .json::<Vec<GitHubReview>>()
            .await
            .map_err(map_transport_error)?
            .into_iter()
            .map(|review| PullReview {
                id: review.id,
                author: review.user.map(|user| user.login).unwrap_or_default(),
                state: review.state.unwrap_or_default(),
                body: review.body,
                submitted_at: review.submitted_at,
            })
            .collect();
        Ok(reviews)
    }

    async fn merge_pull(
        &self,
        token: SecretString,
        owner: &str,
        repo: &str,
        number: u64,
        merge: MergePullRequest,
    ) -> Result<MergeOutcome, AppError> {
        let mut body = serde_json::json!({ "merge_method": merge.strategy.as_str() });
        if let Some(title) = &merge.commit_title {
            body["commit_title"] = serde_json::Value::String(title.clone());
        }
        if let Some(message) = &merge.commit_message {
            body["commit_message"] = serde_json::Value::String(message.clone());
        }
        if let Some(sha) = &merge.expected_head_sha {
            body["sha"] = serde_json::Value::String(sha.clone());
        }
        let merge_url = format!("{}/merge", self.pull_path(owner, repo, Some(number))?);
        let request = ApiRequest::put_json(merge_url, body).with_bearer(token.clone());
        let response = match self.http().send(&request).await {
            Ok(response) => response,
            Err(error) => return Err(remap_merge_error(error)),
        };
        let outcome: MergeResponse = response.json().await.map_err(map_transport_error)?;

        let mut branch_deleted = false;
        if outcome.merged && merge.wants_branch_delete() {
            // unwrap 安全：wants_branch_delete 已保证 head_branch 存在
            let branch = merge.head_branch.clone().unwrap_or_default();
            let delete_url = format!("{}/git/refs/heads/{branch}", self.repo_path(owner, repo)?);
            let delete_request = ApiRequest::delete(delete_url).with_bearer(token.clone());
            match self.http().send(&delete_request).await {
                Ok(_) => branch_deleted = true,
                // 删除失败不回滚合并：合并已生效，如实上报即可
                Err(delete_error) => {
                    tracing::warn!(error = %delete_error, "PR 源分支删除失败（合并已生效）");
                }
            }
        }
        Ok(MergeOutcome {
            merged: outcome.merged,
            sha: outcome.sha,
            message: outcome.message,
            branch_deleted,
        })
    }
}

/// 合并语境下的错误重映射（见模块文档的状态码表）。
///
/// 通用映射把 405/409 归入兜底；在合并语境里它们含义明确，此处改写
/// 错误码并带上 UI 可判定的 hint。GitHub 的可读 message 原样保留。
fn remap_merge_error(mut error: AppError) -> AppError {
    match error.code {
        // 405：不可合并（冲突未解 / 分支保护 / 已合并）
        ErrorCode::Internal => {
            error.code = ErrorCode::Validation;
            error.hint = Some("not-mergeable".to_owned());
            error
        }
        // 409：head 与服务器记录不一致
        ErrorCode::GitConflict => {
            error.hint = Some("conflict".to_owned());
            error
        }
        // 422：sha 预检不匹配（或载荷校验失败）
        ErrorCode::Validation => {
            if error.message.contains("sha")
                || error
                    .detail
                    .as_deref()
                    .is_some_and(|detail| detail.contains("sha"))
            {
                error.hint = Some("head-changed".to_owned());
            } else {
                error.hint = Some("not-mergeable".to_owned());
            }
            error
        }
        _ => error,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{MergePullRequest, MergeStrategy, PullState};
    use crate::client::{GitHubHttp, HttpConfig};
    use crate::github::GitHubProvider;
    use crate::traits::HostProvider;
    use forgedesk_domain::ErrorCode;
    use secrecy::SecretString;
    use std::time::Duration;
    use wiremock::matchers::{method, path, query_param};
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
        SecretString::from("ghp_pulls".to_owned())
    }

    fn pull_json(number: u64, title: &str) -> serde_json::Value {
        serde_json::json!({
            "number": number, "title": title, "state": "open", "draft": false,
            "merged": false, "user": {"login": "octocat"},
            "head": {"label": "octocat:feature", "sha": "abc123"},
            "base": {"label": "github:main"}, "html_url": "https://github.com/octocat/x/pull/1",
            "created_at": "2026-09-30T00:00:00Z", "updated_at": "2026-09-30T01:00:00Z",
            "body": "描述正文", "changed_files": 2, "additions": 10, "deletions": 4,
            "mergeable": true, "mergeable_state": "clean",
            "some_future_field": true
        })
    }

    fn merge_request() -> MergePullRequest {
        MergePullRequest {
            strategy: MergeStrategy::Squash,
            commit_title: None,
            commit_message: None,
            expected_head_sha: Some("abc123".to_owned()),
            delete_branch: true,
            head_branch: Some("feature".to_owned()),
        }
    }

    #[tokio::test]
    async fn list_maps_fields_and_pagination_cursor() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/octocat/x/pulls"))
            .and(query_param("state", "open"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Link", format!("<{}/x?page=2>; rel=\"next\"", server.uri()))
                    .set_body_json(vec![pull_json(1, "Add feature")]),
            )
            .mount(&server)
            .await;

        let page = provider_at(&server)
            .pulls()
            .list_pulls(token(), "octocat", "x", PullState::Open, None, None)
            .await
            .unwrap();

        assert_eq!(page.items.len(), 1);
        assert_eq!(page.next_page, Some(2));
        let summary = &page.items[0];
        assert_eq!(summary.author, "octocat");
        assert_eq!(summary.head_label, "octocat:feature");
        assert!(!summary.draft);
    }

    #[tokio::test]
    async fn detail_carries_mergeability_and_head_sha() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/octocat/x/pulls/1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(pull_json(1, "Add feature")))
            .mount(&server)
            .await;

        let detail = provider_at(&server)
            .pulls()
            .get_pull(token(), "octocat", "x", 1)
            .await
            .unwrap();

        assert_eq!(detail.summary.number, 1);
        assert_eq!(detail.mergeable, Some(true));
        assert_eq!(detail.mergeable_state.as_deref(), Some("clean"));
        assert_eq!(detail.head_sha, "abc123");
        assert_eq!(detail.changed_files, 2);
        assert_eq!(detail.body_markdown.as_deref(), Some("描述正文"));
    }

    #[tokio::test]
    async fn reviews_map_author_state_and_body() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/octocat/x/pulls/1/reviews"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                { "id": 9, "user": {"login": "hubot"}, "state": "APPROVED",
                  "body": "lgtm", "submitted_at": "2026-09-30T02:00:00Z" },
                { "id": 10, "state": "CHANGES_REQUESTED" }
            ])))
            .mount(&server)
            .await;

        let reviews = provider_at(&server)
            .pulls()
            .list_reviews(token(), "octocat", "x", 1)
            .await
            .unwrap();

        assert_eq!(reviews.len(), 2);
        assert_eq!(reviews[0].author, "hubot");
        assert_eq!(reviews[0].state, "APPROVED");
        assert_eq!(reviews[1].author, "", "缺 user 时留空而不是猜");
    }

    #[tokio::test]
    async fn merge_sends_strategy_sha_and_deletes_the_branch_after_success() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/repos/octocat/x/pulls/1/merge"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "merged": true, "sha": "deadbeef", "message": "Pull Request successfully merged"
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/repos/octocat/x/git/refs/heads/feature"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        let outcome = provider_at(&server)
            .pulls()
            .merge_pull(token(), "octocat", "x", 1, merge_request())
            .await
            .unwrap();

        assert!(outcome.merged);
        assert_eq!(outcome.sha.as_deref(), Some("deadbeef"));
        assert!(outcome.branch_deleted);
        server.verify().await;
    }

    #[tokio::test]
    async fn merge_without_branch_delete_makes_no_delete_call() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "merged": true, "sha": "d" })),
            )
            .expect(1)
            .mount(&server)
            .await;
        // 不挂 DELETE 的 mock：多余的删除调用会得到 404，expect(0) 会失败
        Mock::given(method("DELETE"))
            .respond_with(ResponseTemplate::new(204))
            .expect(0)
            .mount(&server)
            .await;

        let request = MergePullRequest {
            strategy: MergeStrategy::Merge,
            commit_title: None,
            commit_message: None,
            expected_head_sha: None,
            delete_branch: false,
            head_branch: None,
        };
        let outcome = provider_at(&server)
            .pulls()
            .merge_pull(token(), "octocat", "x", 1, request)
            .await
            .unwrap();

        assert!(outcome.merged);
        assert!(!outcome.branch_deleted);
        server.verify().await;
    }

    #[tokio::test]
    async fn merge_failures_map_to_readable_codes() {
        let server = MockServer::start().await;
        // 405：不可合并
        Mock::given(method("PUT"))
            .and(path("/repos/octocat/x/pulls/1/merge"))
            .respond_with(
                ResponseTemplate::new(405)
                    .set_body_string(r#"{"message":"Pull Request is not mergeable"}"#),
            )
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path("/repos/octocat/x/pulls/2/merge"))
            .respond_with(
                ResponseTemplate::new(422)
                    .set_body_string(r#"{"message":"head sha was not valid"}"#),
            )
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path("/repos/octocat/x/pulls/3/merge"))
            .respond_with(
                ResponseTemplate::new(409).set_body_string(r#"{"message":"head out of date"}"#),
            )
            .mount(&server)
            .await;

        let provider = provider_at(&server);
        let not_mergeable = provider
            .pulls()
            .merge_pull(token(), "octocat", "x", 1, merge_request())
            .await
            .unwrap_err();
        assert_eq!(not_mergeable.code, ErrorCode::Validation);
        assert_eq!(not_mergeable.hint.as_deref(), Some("not-mergeable"));
        assert!(not_mergeable.message.contains("not mergeable"));

        let head_changed = provider
            .pulls()
            .merge_pull(token(), "octocat", "x", 2, merge_request())
            .await
            .unwrap_err();
        assert_eq!(head_changed.hint.as_deref(), Some("head-changed"));

        let conflict = provider
            .pulls()
            .merge_pull(token(), "octocat", "x", 3, merge_request())
            .await
            .unwrap_err();
        assert_eq!(conflict.code, ErrorCode::GitConflict);
    }

    #[tokio::test]
    async fn merge_errors_never_leak_the_token() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .respond_with(
                ResponseTemplate::new(401).set_body_string(r#"{"message":"Bad credentials"}"#),
            )
            .mount(&server)
            .await;

        let error = provider_at(&server)
            .pulls()
            .merge_pull(
                SecretString::from("ghp_supersecret".to_owned()),
                "octocat",
                "x",
                1,
                merge_request(),
            )
            .await
            .unwrap_err();

        assert!(!format!("{error:?}").contains("ghp_supersecret"));
    }

    #[test]
    fn zero_and_invalid_references_are_rejected_locally() {
        let server_uri = "http://127.0.0.1:1".to_owned();
        let http = GitHubHttp::new(HttpConfig::default()).unwrap();
        let provider =
            GitHubProvider::with_endpoints("github.com", "x", http, server_uri.clone(), server_uri);

        let error = provider.pull_path("octocat", "x", Some(0)).unwrap_err();
        assert_eq!(error.code, ErrorCode::Validation);
        let error = provider.pull_path("", "x", None).unwrap_err();
        assert_eq!(error.code, ErrorCode::Validation);
    }
}
