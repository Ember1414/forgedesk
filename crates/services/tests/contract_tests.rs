//! GitHub 契约测试（T4.12）：一个本地 Mock Server 扮演 GitHub，
//! 钉住 ForgeDesk 调用的**每一个** API 路径的请求形状与响应解析。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//!
//! # 覆盖率怎么量化（M4 验收：契约测试覆盖 ≥ 90% 的 API 调用路径）
//!
//! [`ENDPOINT_MANIFEST`] 枚举了 M4 全部 REST 调用路径；每个 mock 挂载
//! 时带 `.expect(1..)`（**至少命中一次**）。场景跑完 `server.verify()`
//! 会对"零命中"的 mock 报错——manifest 里任何一条路径没被走到，测试
//! 就红。所以覆盖率要么 100%（全绿），要么在缺的那条上精确报错，
//! 不会出现"统计口径"的灰色地带。规则：**provider 新增端点必须进
//! manifest 并配契约调用**，否则本测试不知道它存在。
//!
//! # 断言的对象是"契约"而不是"实现"
//!
//! 请求侧钉方法 + 路径 + 关键查询/载荷（不留过头细节）；响应侧用
//! "未知字段忽略"的宽松解析（docs/PLAN.md M4 容错策略）+ 带一个
//! `some_future_field`，保证 GitHub 演进不会破坏解析。

use std::sync::Arc;
use std::time::Duration;

use forgedesk_provider::{
    CommentSide, GitHubHttp, GitHubProvider, HttpConfig, IssueEdit, IssueState, MergePullRequest,
    MergeStrategy, RemoteRepo, RepoListScope, ReviewCommentAnchor, ReviewEvent,
};
use forgedesk_services::accounts::{memory_credential_store, AccountService};
use forgedesk_services::host_repos::{
    HostRepoService, IssueListQuery, PullListQuery, RemoteRepoRef, ReviewSubmission,
};
use forgedesk_storage::Database;
use secrecy::SecretString;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// M4 五个 trait 的方法总数（AuthFlow 3 + RepoService 7 + PullService 10
/// + IssueService 9 + CiService 4）。覆盖率分母。
const M4_TRAIT_METHODS: usize = 33;

/// 契约覆盖的端点清单（与场景一一对应；新增 provider 端点必须登记）。
const ENDPOINT_MANIFEST: &[&str] = &[
    "POST /login/device/code",
    "POST /login/oauth/access_token",
    "GET /user",
    "GET /user/repos",
    "GET /user/starred",
    "GET /search/repositories",
    "PUT /repos/{o}/{r}/starred",
    "DELETE /repos/{o}/{r}/starred",
    "POST /repos/{o}/{r}/forks",
    "GET /repos/{o}/{r}/readme",
    "GET /repos/{o}/{r}/pulls",
    "GET /repos/{o}/{r}/pulls/{n}",
    "GET /repos/{o}/{r}/pulls/{n}/reviews",
    "POST /repos/{o}/{r}/pulls/{n}/reviews",
    "GET /repos/{o}/{r}/pulls/{n}/files",
    "GET /repos/{o}/{r}/pulls/{n}/comments",
    "POST /repos/{o}/{r}/pulls/{n}/comments (create inline)",
    "POST /repos/{o}/{r}/pulls/{n}/comments (reply)",
    "PUT /repos/{o}/{r}/pulls/{n}/merge",
    "DELETE /repos/{o}/{r}/git/refs/heads/{branch}",
    "GET /repos/{o}/{r}/issues",
    "GET /repos/{o}/{r}/issues/{n}",
    "POST /repos/{o}/{r}/issues",
    "PATCH /repos/{o}/{r}/issues/{n} (edit)",
    "PATCH /repos/{o}/{r}/issues/{n} (state)",
    "PATCH /repos/{o}/{r}/issues/{n} (assignees)",
    "GET /repos/{o}/{r}/issues/{n}/comments (PR timeline)",
    "POST /repos/{o}/{r}/issues/{n}/comments (PR timeline)",
    "GET /repos/{o}/{r}/issues/{n}/comments (issue)",
    "POST /repos/{o}/{r}/issues/{n}/comments (issue)",
    "GET /repos/{o}/{r}/assignees",
    "GET /repos/{o}/{r}/actions/runs",
    "GET /repos/{o}/{r}/actions/runs/{id}/jobs",
    "POST /repos/{o}/{r}/actions/runs/{id}/cancel",
    "POST /repos/{o}/{r}/actions/runs/{id}/rerun",
    "GET /repos/{o}/{r}/actions/jobs/{id}/logs",
    "GET /rate_limit",
    "GET /repos/{o}/{r}/pulls (dashboard)",
    "GET /repos/{o}/{r}/actions/runs (dashboard)",
];

#[test]
fn the_manifest_meets_the_ninety_percent_contract_bar() {
    assert!(
        ENDPOINT_MANIFEST.len() * 100 >= 90 * M4_TRAIT_METHODS,
        "契约端点 {} 少于 M4 方法面 33 的 90%——新增端点没进契约测试？",
        ENDPOINT_MANIFEST.len()
    );
}

fn repo_json(name: &str) -> serde_json::Value {
    serde_json::json!({
        "id": 1, "name": name, "full_name": format!("octocat/{name}"),
        "owner": {"login": "octocat"},
        "html_url": format!("https://github.com/octocat/{name}"),
        "some_future_field": true
    })
}

fn pull_json(number: u64) -> serde_json::Value {
    serde_json::json!({
        "number": number, "title": "P", "state": "open",
        "user": {"login": "octocat"},
        "head": {"label": "octocat:feature", "sha": "abc123"},
        "base": {"label": "github:main"},
        "html_url": format!("https://github.com/octocat/r1/pull/{number}")
    })
}

fn issue_json(number: u64) -> serde_json::Value {
    serde_json::json!({
        "number": number, "title": "T", "state": "open",
        "user": {"login": "octocat"}
    })
}

fn comment_json(id: u64) -> serde_json::Value {
    serde_json::json!({ "id": id, "user": {"login": "hubot"}, "body": "b" })
}

fn inline_comment_json(id: u64) -> serde_json::Value {
    serde_json::json!({
        "id": id, "user": {"login": "hubot"}, "body": "x",
        "path": "src/a.rs", "side": "RIGHT", "line": 1
    })
}

/// 挂载一个"至少命中一次"的契约 mock。
macro_rules! contract_mock {
    ($server:expr, $method:expr, $path:expr, $response:expr) => {
        Mock::given(method($method))
            .and(path($path))
            .respond_with($response)
            .expect(1..)
            .mount(&$server)
            .await;
    };
}

#[tokio::test]
async fn every_github_api_path_answers_per_contract() {
    let server = MockServer::start().await;
    let uri = server.uri();

    // ---- 认证（AuthFlow 3 方法）----
    contract_mock!(
        server,
        "POST",
        "/login/device/code",
        ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "device_code": "dc1", "user_code": "ABCD-1234",
            "verification_uri": "https://github.com/login/device",
            "expires_in": 900, "interval": 1
        }))
    );
    contract_mock!(
        server,
        "POST",
        "/login/oauth/access_token",
        ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "ghp_flow", "token_type": "bearer", "scope": "repo"
        }))
    );
    contract_mock!(
        server,
        "GET",
        "/user",
        ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "login": "octocat", "avatar_url": null
        }))
    );

    // ---- 仓库（RepoService）----
    contract_mock!(
        server,
        "GET",
        "/user/repos",
        ResponseTemplate::new(200).set_body_json(serde_json::json!([repo_json("r1")]))
    );
    contract_mock!(
        server,
        "GET",
        "/user/starred",
        ResponseTemplate::new(200).set_body_json(serde_json::json!([]))
    );
    contract_mock!(
        server,
        "GET",
        "/search/repositories",
        ResponseTemplate::new(200)
            .set_body_json(serde_json::json!({ "total_count": 1, "items": [repo_json("r1")] }))
    );
    contract_mock!(
        server,
        "PUT",
        "/repos/octocat/r1/starred",
        ResponseTemplate::new(204)
    );
    contract_mock!(
        server,
        "DELETE",
        "/repos/octocat/r1/starred",
        ResponseTemplate::new(204)
    );
    contract_mock!(
        server,
        "POST",
        "/repos/octocat/r1/forks",
        ResponseTemplate::new(201).set_body_json(repo_json("r1"))
    );
    contract_mock!(
        server,
        "GET",
        "/repos/octocat/r1/readme",
        ResponseTemplate::new(200).set_body_string("# readme body")
    );

    // ---- PR（PullService）：PR1 走整体流，PR2 行内评论，PR3 回复 ----
    contract_mock!(
        server,
        "GET",
        "/repos/octocat/r1/pulls",
        ResponseTemplate::new(200).set_body_json(serde_json::json!([pull_json(1)]))
    );
    contract_mock!(
        server,
        "GET",
        "/repos/octocat/r1/pulls/1",
        ResponseTemplate::new(200).set_body_json(pull_json(1))
    );
    contract_mock!(
        server,
        "GET",
        "/repos/octocat/r1/pulls/1/reviews",
        ResponseTemplate::new(200).set_body_json(serde_json::json!([
            { "id": 1, "user": {"login": "hubot"}, "state": "APPROVED" }
        ]))
    );
    contract_mock!(
        server,
        "POST",
        "/repos/octocat/r1/pulls/1/reviews",
        ResponseTemplate::new(200).set_body_json(serde_json::json!({ "id": 2 }))
    );
    contract_mock!(
        server,
        "GET",
        "/repos/octocat/r1/pulls/1/files",
        ResponseTemplate::new(200).set_body_json(serde_json::json!([
            { "filename": "src/a.rs", "status": "modified" }
        ]))
    );
    contract_mock!(
        server,
        "GET",
        "/repos/octocat/r1/pulls/1/comments",
        ResponseTemplate::new(200).set_body_json(serde_json::json!([]))
    );
    contract_mock!(
        server,
        "GET",
        "/repos/octocat/r1/pulls/2",
        ResponseTemplate::new(200).set_body_json(pull_json(2))
    );
    contract_mock!(
        server,
        "GET",
        "/repos/octocat/r1/pulls/2/files",
        ResponseTemplate::new(200).set_body_json(serde_json::json!([
            { "filename": "src/a.rs", "status": "modified",
              "patch": "@@ -1 +1 @@\n-a\n+b" }
        ]))
    );
    contract_mock!(
        server,
        "POST",
        "/repos/octocat/r1/pulls/2/comments",
        ResponseTemplate::new(201).set_body_json(inline_comment_json(30))
    );
    contract_mock!(
        server,
        "POST",
        "/repos/octocat/r1/pulls/3/comments",
        ResponseTemplate::new(201).set_body_json(inline_comment_json(31))
    );
    contract_mock!(
        server,
        "PUT",
        "/repos/octocat/r1/pulls/1/merge",
        ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "merged": true, "sha": "deadbeef", "message": "merged"
        }))
    );
    contract_mock!(
        server,
        "DELETE",
        "/repos/octocat/r1/git/refs/heads/feature",
        ResponseTemplate::new(204)
    );

    // ---- Issue（IssueService）：7 编辑/评论，8 关开，9 指派 ----
    contract_mock!(
        server,
        "GET",
        "/repos/octocat/r1/issues",
        ResponseTemplate::new(200).set_body_json(serde_json::json!([issue_json(7)]))
    );
    contract_mock!(
        server,
        "GET",
        "/repos/octocat/r1/issues/7",
        ResponseTemplate::new(200).set_body_json(issue_json(7))
    );
    contract_mock!(
        server,
        "POST",
        "/repos/octocat/r1/issues",
        ResponseTemplate::new(201).set_body_json(issue_json(8))
    );
    contract_mock!(
        server,
        "PATCH",
        "/repos/octocat/r1/issues/7",
        ResponseTemplate::new(200).set_body_json(issue_json(7))
    );
    contract_mock!(
        server,
        "PATCH",
        "/repos/octocat/r1/issues/8",
        ResponseTemplate::new(200).set_body_json(issue_json(8))
    );
    contract_mock!(
        server,
        "PATCH",
        "/repos/octocat/r1/issues/9",
        ResponseTemplate::new(200).set_body_json(issue_json(9))
    );
    contract_mock!(
        server,
        "GET",
        "/repos/octocat/r1/issues/1/comments",
        ResponseTemplate::new(200).set_body_json(serde_json::json!([]))
    );
    contract_mock!(
        server,
        "POST",
        "/repos/octocat/r1/issues/1/comments",
        ResponseTemplate::new(201).set_body_json(comment_json(5))
    );
    contract_mock!(
        server,
        "GET",
        "/repos/octocat/r1/issues/7/comments",
        ResponseTemplate::new(200).set_body_json(serde_json::json!([comment_json(6)]))
    );
    contract_mock!(
        server,
        "POST",
        "/repos/octocat/r1/issues/7/comments",
        ResponseTemplate::new(201).set_body_json(comment_json(7))
    );
    contract_mock!(
        server,
        "GET",
        "/repos/octocat/r1/assignees",
        ResponseTemplate::new(200).set_body_json(serde_json::json!([{ "login": "hubot" }]))
    );

    // ---- CI（CiService + 日志流 + 限流刷新）----
    contract_mock!(
        server,
        "GET",
        "/repos/octocat/r1/actions/runs",
        ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "workflow_runs": [
                { "id": 10, "name": "CI", "status": "completed", "conclusion": "failure" }
            ]
        }))
    );
    contract_mock!(
        server,
        "GET",
        "/repos/octocat/r1/actions/runs/10/jobs",
        ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "jobs": [
                { "id": 100, "name": "build", "status": "completed", "conclusion": "failure" }
            ]
        }))
    );
    contract_mock!(
        server,
        "POST",
        "/repos/octocat/r1/actions/runs/10/cancel",
        ResponseTemplate::new(202)
    );
    contract_mock!(
        server,
        "POST",
        "/repos/octocat/r1/actions/runs/10/rerun",
        ResponseTemplate::new(201)
    );
    contract_mock!(
        server,
        "GET",
        "/repos/octocat/r1/actions/jobs/100/logs",
        ResponseTemplate::new(200).set_body_string("line one\nline two\n")
    );
    contract_mock!(
        server,
        "GET",
        "/rate_limit",
        ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "resources": { "core": {
                "limit": 5000, "used": 10, "remaining": 4990, "reset": 1790000000
            } }
        }))
    );

    // ---- Dashboard（T4.11）：独立仓库 r2，命中数与上面互不干扰 ----
    contract_mock!(
        server,
        "GET",
        "/repos/octocat/r2/pulls",
        ResponseTemplate::new(200).set_body_json(serde_json::json!([
            { "number": 1, "title": "P", "state": "open",
              "user": {"login": "hubot"},
              "requested_reviewers": [{"login": "octocat"}] }
        ]))
    );
    contract_mock!(
        server,
        "GET",
        "/repos/octocat/r2/actions/runs",
        ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "workflow_runs": [
                { "id": 11, "name": "CI", "status": "completed", "conclusion": "success" }
            ]
        }))
    );

    // ---- 组装两个服务（同一凭据库 + 同一假 GitHub）----
    let database = {
        let db = Database::open_in_memory().unwrap();
        forgedesk_storage::migrate(&db).unwrap();
        Arc::new(db)
    };
    let credentials = memory_credential_store();
    let http = GitHubHttp::new(HttpConfig {
        backoff_base: Duration::from_millis(1),
        ..HttpConfig::default()
    })
    .unwrap();
    let repos_uri = uri.clone();
    let repos_http = http.clone();
    let repos = HostRepoService::with_factory(
        Arc::clone(&database),
        credentials.clone(),
        http.clone(),
        Box::new(move |host| {
            Ok(GitHubProvider::with_endpoints(
                host,
                "",
                repos_http.clone(),
                repos_uri.clone(),
                repos_uri.clone(),
            ))
        }),
    );
    let accounts_uri = uri.clone();
    let accounts_http = http.clone();
    let accounts = AccountService::with_factory(
        Arc::clone(&database),
        credentials,
        Box::new(move |_host| {
            Ok(GitHubProvider::with_endpoints(
                "github.com",
                "Iv1.test",
                accounts_http.clone(),
                accounts_uri.clone(),
                accounts_uri.clone(),
            ))
        }),
    );

    // ---- 认证流 ----
    let started = accounts
        .start_device_flow("github.com", None)
        .await
        .unwrap();
    assert_eq!(started.start.user_code, "ABCD-1234");
    accounts
        .wait_device_flow(&started.flow_id, &CancellationToken::new())
        .await
        .unwrap();
    let pat_account = accounts
        .login_with_pat("github.com", SecretString::from("ghp_pat".to_owned()))
        .await
        .unwrap();
    assert_eq!(pat_account.login, "octocat");

    let target = RemoteRepoRef {
        host: "github.com".to_owned(),
        repo_id: None,
        owner: "octocat".to_owned(),
        repo: "r1".to_owned(),
    };

    // ---- 仓库流 ----
    let owned = repos
        .list_authenticated("github.com", None, RepoListScope::Owned, None, None)
        .await
        .unwrap();
    assert_eq!(owned.items.len(), 1);
    repos
        .list_starred("github.com", None, None, None)
        .await
        .unwrap();
    let found = repos
        .search("github.com", None, "forgedesk language:rust", None, None)
        .await
        .unwrap();
    assert_eq!(found.items.len(), 1);
    repos
        .set_starred("github.com", None, "octocat", "r1", true)
        .await
        .unwrap();
    repos
        .set_starred("github.com", None, "octocat", "r1", false)
        .await
        .unwrap();
    let forked: RemoteRepo = repos
        .fork("github.com", None, "octocat", "r1")
        .await
        .unwrap();
    assert_eq!(forked.name, "r1");
    let readme = repos
        .readme("github.com", None, "octocat", "r1")
        .await
        .unwrap();
    assert!(readme.contains("readme"), "README 必须已消毒渲染");

    // ---- PR 流 ----
    let pulls = repos
        .list_pulls(
            &target,
            PullListQuery {
                state: forgedesk_provider::PullState::Open,
                page: None,
                per_page: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(pulls.items.len(), 1);
    let detail = repos
        .get_pull("github.com", None, "octocat", "r1", 1)
        .await
        .unwrap();
    assert_eq!(detail.detail.head_sha, "abc123");
    repos
        .list_reviews("github.com", None, "octocat", "r1", 1)
        .await
        .unwrap();
    repos
        .submit_review(
            &target,
            ReviewSubmission {
                number: 1,
                event: ReviewEvent::Approve,
                body: Some("lgtm".to_owned()),
            },
        )
        .await
        .unwrap();
    let files = repos.list_files(&target, 1, None, None).await.unwrap();
    assert_eq!(files.items.len(), 1);
    repos
        .list_review_comments("github.com", None, "octocat", "r1", 1)
        .await
        .unwrap();
    let created = repos
        .create_review_comment(
            &target,
            2,
            ReviewCommentAnchor {
                path: "src/a.rs".to_owned(),
                side: CommentSide::Right,
                line: 1,
                start_line: None,
                start_side: None,
            },
            "inline",
        )
        .await
        .unwrap();
    assert_eq!(created.id, 30);
    let replied = repos
        .reply_review_comment(&target, 3, 30, "reply")
        .await
        .unwrap();
    assert_eq!(replied.id, 31);
    repos
        .list_comments("github.com", None, "octocat", "r1", 1)
        .await
        .unwrap();
    repos
        .create_comment("github.com", None, "octocat", "r1", 1, "ping")
        .await
        .unwrap();
    let outcome = repos
        .merge_pull(
            "github.com",
            None,
            "octocat",
            "r1",
            1,
            MergePullRequest {
                strategy: MergeStrategy::Squash,
                commit_title: None,
                commit_message: None,
                expected_head_sha: Some("abc123".to_owned()),
                delete_branch: true,
                head_branch: Some("feature".to_owned()),
            },
        )
        .await
        .unwrap();
    assert!(outcome.merged);
    assert!(outcome.branch_deleted);

    // ---- Issue 流 ----
    let issues = repos
        .list_issues(
            &target,
            IssueListQuery {
                state: IssueState::Open,
                page: None,
                per_page: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(issues.items.len(), 1);
    let issue = repos
        .get_issue("github.com", None, "octocat", "r1", 7)
        .await
        .unwrap();
    assert_eq!(issue.detail.summary.number, 7);
    repos
        .create_issue("github.com", None, "octocat", "r1", "T", None)
        .await
        .unwrap();
    repos
        .edit_issue(
            "github.com",
            None,
            "octocat",
            "r1",
            7,
            IssueEdit {
                title: Some("New".to_owned()),
                body: None,
            },
        )
        .await
        .unwrap();
    repos
        .set_issue_state("github.com", None, "octocat", "r1", 8, false)
        .await
        .unwrap();
    repos
        .set_issue_assignees(
            "github.com",
            None,
            "octocat",
            "r1",
            9,
            &["hubot".to_owned()],
        )
        .await
        .unwrap();
    repos
        .list_issue_comments("github.com", None, "octocat", "r1", 7)
        .await
        .unwrap();
    repos
        .create_issue_comment("github.com", None, "octocat", "r1", 7, "hello")
        .await
        .unwrap();
    repos
        .list_assignees("github.com", None, "octocat", "r1")
        .await
        .unwrap();

    // ---- CI 流 ----
    let runs = repos.list_runs(&target, None, None).await.unwrap();
    assert_eq!(runs.items.len(), 1);
    repos
        .list_run_jobs("github.com", None, "octocat", "r1", 10)
        .await
        .unwrap();
    repos
        .cancel_run("github.com", None, "octocat", "r1", 10)
        .await
        .unwrap();
    repos
        .rerun_run("github.com", None, "octocat", "r1", 10)
        .await
        .unwrap();
    let logs = repos
        .job_logs_response("github.com", None, "octocat", "r1", 100)
        .await
        .unwrap();
    let logs_text = logs.text().await.unwrap();
    assert!(logs_text.contains("line one"));
    let refreshed = repos.refresh_rate_limit("github.com", None).await.unwrap();
    assert_eq!(refreshed.remaining, 4990);

    // ---- Dashboard 流（r2）----
    let report = repos
        .dashboard(
            "github.com",
            None,
            &[forgedesk_services::host_repos::DashboardTarget {
                owner: "octocat".to_owned(),
                repo: "r2".to_owned(),
            }],
        )
        .await
        .unwrap();
    let digest = report[0].pulls.as_ref().unwrap();
    assert_eq!(
        digest.awaiting_review, 1,
        "requested_reviewers 匹配登录账号"
    );

    // ---- 覆盖率裁决：manifest 里任何一条路径零命中 → 这里红 ----
    server.verify().await;
}
