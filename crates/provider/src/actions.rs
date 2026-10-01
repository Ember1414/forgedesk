//! CI（Actions）子服务的 GitHub 实现（T4.9）。
//!
//! # 日志是流式端点
//!
//! [`CiService::job_logs`] 返回 `reqwest::Response` 而不是组装好的文本：
//! 日志可以远超 5MB（M4 验收"流式加载大日志不卡 UI"），一次性读进内存
//! 再整体过 IPC 会把两者同时卡死。流式的消费在命令层（JobRunner 任务
//! 分块读取、经事件推送，见 `commands/actions.rs` 的模块文档）；本层只
//! 负责拿到跟随 302 重定向后的响应体。也正因为返回的是未封装的
//! Response，这个方法**不在** `CiService` 上——它是 `GitHubProvider`
//! 的自有方法，等 trait 层找到跨平台签名时再上移。
//!
//! # 结论语义
//!
//! `status`（queued/in_progress/completed）是调度状态，`conclusion`
//! （success/failure/cancelled/…）只有 completed 后才有值；UI 判定
//! "可取消 / 可重跑"必须先看 status 再看 conclusion。取消未运行的 run
//! GitHub 返回 409，原样映射为 `GIT_CONFLICT`。

use secrecy::SecretString;
use serde::Deserialize;

use forgedesk_domain::{AppError, ErrorCode};

use crate::client::{map_transport_error, ApiRequest};
use crate::github::GitHubProvider;
use crate::pulls::GitHubLogin;
use crate::repos::MAX_PER_PAGE;
use crate::traits::CiService;

/// run 的调度状态。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRunSummary {
    /// run id。
    pub id: u64,
    /// 展示标题（GitHub 的 `display_title`）。
    pub name: String,
    /// 触发的分支。
    pub head_branch: Option<String>,
    /// 触发的 sha。
    pub head_sha: Option<String>,
    /// `queued` / `in_progress` / `completed`。
    pub status: String,
    /// 结论（`success` / `failure` / `cancelled`…；未完成为 null）。
    pub conclusion: Option<String>,
    /// 触发事件（`push` / `pull_request`…）。
    pub event: Option<String>,
    /// 触发者。
    pub actor: String,
    /// 仓库内递增的运行号。
    pub run_number: u64,
    /// 创建时间（RFC3339）。
    pub created_at: Option<String>,
    /// 最近更新时间（RFC3339）。
    pub updated_at: Option<String>,
    /// 网页地址。
    pub html_url: String,
}

/// run 一页 + 下一页游标。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunPage {
    /// 本页内容。
    pub items: Vec<WorkflowRunSummary>,
    /// 下一页页码；`None` 表示没有更多。
    pub next_page: Option<u32>,
}

/// run 的一个 job。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunJob {
    /// job id（日志端点的定位键）。
    pub id: u64,
    /// job 名。
    pub name: String,
    /// `queued` / `in_progress` / `completed`。
    pub status: String,
    /// 结论（未完成为 null）。
    pub conclusion: Option<String>,
    /// 开始时间（RFC3339）。
    pub started_at: Option<String>,
    /// 结束时间（RFC3339）。
    pub completed_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GitHubRun {
    id: u64,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    display_title: Option<String>,
    #[serde(default)]
    head_branch: Option<String>,
    #[serde(default)]
    head_sha: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    conclusion: Option<String>,
    #[serde(default)]
    event: Option<String>,
    #[serde(default)]
    actor: Option<GitHubLogin>,
    #[serde(default)]
    run_number: Option<u64>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    updated_at: Option<String>,
    #[serde(default)]
    html_url: Option<String>,
}

impl From<GitHubRun> for WorkflowRunSummary {
    fn from(run: GitHubRun) -> Self {
        Self {
            id: run.id,
            // 老字段 name 与 display_title 并存：display_title 更完整，回退 name
            name: run.display_title.or(run.name).unwrap_or_default(),
            head_branch: run.head_branch,
            head_sha: run.head_sha,
            status: run.status.unwrap_or_default(),
            conclusion: run.conclusion,
            event: run.event,
            actor: run.actor.map(|actor| actor.login).unwrap_or_default(),
            run_number: run.run_number.unwrap_or(0),
            created_at: run.created_at,
            updated_at: run.updated_at,
            html_url: run.html_url.unwrap_or_default(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct GitHubJob {
    id: u64,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    conclusion: Option<String>,
    #[serde(default)]
    started_at: Option<String>,
    #[serde(default)]
    completed_at: Option<String>,
}

impl From<GitHubJob> for RunJob {
    fn from(job: GitHubJob) -> Self {
        Self {
            id: job.id,
            name: job.name.unwrap_or_default(),
            status: job.status.unwrap_or_default(),
            conclusion: job.conclusion,
            started_at: job.started_at,
            completed_at: job.completed_at,
        }
    }
}

impl GitHubProvider {
    /// `actions/runs[/id]` 的路径段。
    fn runs_path(&self, owner: &str, repo: &str, run_id: Option<u64>) -> Result<String, AppError> {
        if let Some(run_id) = run_id {
            if run_id == 0 {
                return Err(AppError::new(
                    ErrorCode::Validation,
                    "run id must be positive",
                ));
            }
        }
        Ok(match run_id {
            Some(run_id) => format!("{}/actions/runs/{run_id}", self.repo_path(owner, repo)?),
            None => format!("{}/actions/runs", self.repo_path(owner, repo)?),
        })
    }

    /// 一个 job 的日志响应（跟随重定向后的明文，可能很大——流式消费）。
    pub async fn job_logs_response(
        &self,
        token: SecretString,
        owner: &str,
        repo: &str,
        job_id: u64,
    ) -> Result<reqwest::Response, AppError> {
        if job_id == 0 {
            return Err(AppError::new(
                ErrorCode::Validation,
                "job id must be positive",
            ));
        }
        let url = format!(
            "{}/actions/jobs/{job_id}/logs",
            self.repo_path(owner, repo)?
        );
        let request = ApiRequest::get(url).with_bearer(token);
        self.http().send(&request).await
    }
}

#[async_trait::async_trait]
impl CiService for GitHubProvider {
    async fn list_runs(
        &self,
        token: SecretString,
        owner: &str,
        repo: &str,
        page: Option<u32>,
        per_page: Option<u32>,
    ) -> Result<RunPage, AppError> {
        if let Some(value) = per_page {
            if value > MAX_PER_PAGE {
                return Err(AppError::new(
                    ErrorCode::Validation,
                    format!("per_page must not exceed {MAX_PER_PAGE}"),
                ));
            }
        }
        let mut query = Vec::new();
        if let Some(page) = page {
            if page > 1 {
                query.push(("page".to_owned(), page.to_string()));
            }
        }
        if let Some(per_page) = per_page {
            query.push(("per_page".to_owned(), per_page.to_string()));
        }
        let request = ApiRequest::get(self.runs_path(owner, repo, None)?)
            .with_bearer(token)
            .with_query(query);
        let response = self.http().send(&request).await?;
        // 先取 Link 头再消费 body（json() 拿走 response 的所有权）
        let next_page = response
            .headers()
            .get("link")
            .and_then(|value| value.to_str().ok())
            .and_then(crate::repos::next_page_from_link_header);
        let body = response
            .json::<GitHubRunList>()
            .await
            .map_err(map_transport_error)?;
        Ok(RunPage {
            items: body.workflow_runs.into_iter().map(Into::into).collect(),
            next_page,
        })
    }

    async fn list_run_jobs(
        &self,
        token: SecretString,
        owner: &str,
        repo: &str,
        run_id: u64,
    ) -> Result<Vec<RunJob>, AppError> {
        let url = format!("{}/jobs", self.runs_path(owner, repo, Some(run_id))?);
        let request = ApiRequest::get(url)
            .with_bearer(token)
            .with_query(vec![("per_page".to_owned(), MAX_PER_PAGE.to_string())]);
        let response = self.http().send(&request).await?;
        let body = response
            .json::<GitHubJobList>()
            .await
            .map_err(map_transport_error)?;
        Ok(body.jobs.into_iter().map(Into::into).collect())
    }

    async fn cancel_run(
        &self,
        token: SecretString,
        owner: &str,
        repo: &str,
        run_id: u64,
    ) -> Result<(), AppError> {
        let url = format!("{}/cancel", self.runs_path(owner, repo, Some(run_id))?);
        let request = ApiRequest::post_json(url, serde_json::json!({})).with_bearer(token);
        self.http().send(&request).await?;
        Ok(())
    }

    async fn rerun_run(
        &self,
        token: SecretString,
        owner: &str,
        repo: &str,
        run_id: u64,
    ) -> Result<(), AppError> {
        let url = format!("{}/rerun", self.runs_path(owner, repo, Some(run_id))?);
        let request = ApiRequest::post_json(url, serde_json::json!({})).with_bearer(token);
        self.http().send(&request).await?;
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct GitHubRunList {
    #[serde(default)]
    workflow_runs: Vec<GitHubRun>,
}

#[derive(Debug, Deserialize)]
struct GitHubJobList {
    #[serde(default)]
    jobs: Vec<GitHubJob>,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use crate::client::{GitHubHttp, HttpConfig};
    use crate::github::GitHubProvider;
    use crate::traits::HostProvider;
    use forgedesk_domain::ErrorCode;
    use secrecy::SecretString;
    use std::time::Duration;
    use wiremock::matchers::{method, path};
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
        SecretString::from("ghp_actions".to_owned())
    }

    fn run_json(id: u64, title: &str) -> serde_json::Value {
        serde_json::json!({
            "id": id, "name": "CI", "display_title": title,
            "head_branch": "main", "head_sha": "abc123",
            "status": "completed", "conclusion": "success",
            "event": "push", "actor": {"login": "octocat"},
            "run_number": 42, "created_at": "2026-10-01T00:00:00Z",
            "updated_at": "2026-10-01T00:05:00Z",
            "html_url": format!("https://github.com/octocat/x/actions/runs/{id}"),
            "some_future_field": true
        })
    }

    #[tokio::test]
    async fn list_runs_maps_fields_and_pagination_cursor() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/octocat/x/actions/runs"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Link", format!("<{}/x?page=2>; rel=\"next\"", server.uri()))
                    .set_body_json(serde_json::json!({
                        "total_count": 2,
                        "workflow_runs": [run_json(10, "Fix crash"), run_json(11, "Add feature")]
                    })),
            )
            .mount(&server)
            .await;

        let page = provider_at(&server)
            .actions()
            .list_runs(token(), "octocat", "x", None, None)
            .await
            .unwrap();

        assert_eq!(page.next_page, Some(2));
        assert_eq!(page.items.len(), 2);
        let run = &page.items[0];
        assert_eq!(run.id, 10);
        assert_eq!(run.name, "Fix crash", "display_title 优先于 name");
        assert_eq!(run.status, "completed");
        assert_eq!(run.conclusion.as_deref(), Some("success"));
        assert_eq!(run.actor, "octocat");
    }

    #[tokio::test]
    async fn run_jobs_map_status_and_conclusion() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/octocat/x/actions/runs/10/jobs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "total_count": 2,
                "jobs": [
                    { "id": 100, "name": "build", "status": "completed", "conclusion": "failure",
                      "started_at": "2026-10-01T00:00:00Z", "completed_at": "2026-10-01T00:03:00Z" },
                    { "id": 101, "name": "test", "status": "in_progress", "conclusion": null }
                ]
            })))
            .mount(&server)
            .await;

        let jobs = provider_at(&server)
            .actions()
            .list_run_jobs(token(), "octocat", "x", 10)
            .await
            .unwrap();

        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs[0].name, "build");
        assert_eq!(jobs[0].conclusion.as_deref(), Some("failure"));
        assert_eq!(jobs[1].conclusion, None);
    }

    #[tokio::test]
    async fn cancel_and_rerun_post_to_the_run_endpoints() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/repos/octocat/x/actions/runs/10/cancel"))
            .respond_with(ResponseTemplate::new(202))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/repos/octocat/x/actions/runs/10/rerun"))
            .respond_with(ResponseTemplate::new(201))
            .expect(1)
            .mount(&server)
            .await;

        let provider = provider_at(&server);
        provider
            .actions()
            .cancel_run(token(), "octocat", "x", 10)
            .await
            .unwrap();
        provider
            .actions()
            .rerun_run(token(), "octocat", "x", 10)
            .await
            .unwrap();
        server.verify().await;
    }

    #[tokio::test]
    async fn zero_ids_are_rejected_locally() {
        let server = MockServer::start().await;
        let provider = provider_at(&server);

        let error = provider
            .actions()
            .list_run_jobs(token(), "octocat", "x", 0)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Validation);

        let error = provider
            .actions()
            .cancel_run(token(), "octocat", "x", 0)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Validation);

        let error = provider
            .job_logs_response(token(), "octocat", "x", 0)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Validation);
    }

    #[tokio::test]
    async fn job_logs_follow_the_redirect_to_a_stream() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/octocat/x/actions/jobs/100/logs"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string("line one\nline two\nline three\n"),
            )
            .mount(&server)
            .await;

        let response = provider_at(&server)
            .job_logs_response(token(), "octocat", "x", 100)
            .await
            .unwrap();
        let text = response.text().await.unwrap();
        assert_eq!(text, "line one\nline two\nline three\n");
    }
}
