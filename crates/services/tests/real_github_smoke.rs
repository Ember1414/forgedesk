//! 真实 GitHub 只读冒烟（T4.12 第 2 项）。
//!
//! # 默认跳过，绝不偷偷联网
//!
//! 只有**同时**设置 `GITHUB_TOKEN` 与 `TEST_REPO`（形如 `owner/repo`）时才会真的
//! 发请求；缺任何一个就打印一行"跳过"并正常通过。CI 上这两个变量一律不配，
//! 因此 `cargo test --workspace` 永远不会因为网络而红。
//!
//! # 只读是硬约束（红线 R7 / M4 验收）
//!
//! 本文件只调用读方法：列 PR / 列 Issue / 列 workflow run。**任何写操作**
//! （评论、merge、发 Issue、取消 run…）都不在这里出现——对真实仓库写数据
//! 必须由人类明确授权（见 `docs/acceptance/M4.md` §4）。要加写操作之前先想清楚
//! 这一点：它会把"跑一次测试"变成"改别人的仓库"。
//!
//! # 令牌的边界（红线 R8）
//!
//! 令牌从环境变量读入后只经 `SecretString` 交给凭据门，落在**内存**凭据库
//! （`memory_credential_store`）里；既不落盘、也不出现在任何打印里。
//! 打印出来的只有"读了什么、读到多少条、剩余配额"。
// 打印是**故意**的：跳过原因与冒烟结果都要留在 `--nocapture` 输出里（联调证据）。
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(clippy::print_stdout)]

use std::sync::Arc;

use forgedesk_provider::{GitHubHttp, IssueState, PullState};
use forgedesk_services::accounts::{memory_credential_store, AccountService};
use forgedesk_services::host_repos::{
    HostRepoService, IssueListQuery, PullListQuery, RemoteRepoRef,
};
use forgedesk_storage::Database;
use secrecy::SecretString;

/// 冒烟目标（`TEST_REPO` 的两种写法：`owner/repo` 或 `github.com/owner/repo`）。
struct SmokeTarget {
    /// 站点。
    host: String,
    /// 所有者。
    owner: String,
    /// 仓库名。
    repo: String,
}

/// 解析 `TEST_REPO`；缺变量或写错形状时返回 `None`（调用方打印跳过原因）。
fn target_from_env() -> Option<SmokeTarget> {
    let raw = std::env::var("TEST_REPO").ok()?;
    let parts: Vec<&str> = raw
        .trim()
        .trim_start_matches("https://")
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    match parts.as_slice() {
        // owner/repo
        [owner, repo] => Some(SmokeTarget {
            host: "github.com".to_owned(),
            owner: (*owner).to_owned(),
            repo: (*repo).to_owned(),
        }),
        // host/owner/repo
        [host, owner, repo] => Some(SmokeTarget {
            host: (*host).to_owned(),
            owner: (*owner).to_owned(),
            repo: (*repo).to_owned(),
        }),
        _ => None,
    }
}

/// 一页最多取多少条：冒烟只证明"读得通、解析得对"，不要拉空整个仓库。
const PER_PAGE: u32 = 5;

#[tokio::test]
async fn reading_a_real_repository_works_with_a_pat_and_stays_read_only() {
    let token = std::env::var("GITHUB_TOKEN")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let Some(token) = token else {
        println!(
            "跳过真实 GitHub 冒烟：未设置 GITHUB_TOKEN（设 GITHUB_TOKEN + TEST_REPO=owner/repo 后运行）"
        );
        return;
    };
    let Some(target) = target_from_env() else {
        println!("跳过真实 GitHub 冒烟：未设置或无法解析 TEST_REPO（形如 owner/repo）");
        return;
    };

    // 内存库 + 内存凭据库：令牌只活在这一刻，不落盘
    let database = {
        let database = Database::open_in_memory().expect("打开内存库失败");
        forgedesk_storage::migrate(&database).expect("迁移失败");
        Arc::new(database)
    };
    let credentials = memory_credential_store();
    let http =
        GitHubHttp::new(forgedesk_provider::HttpConfig::default()).expect("建 HTTP 底座失败");
    let accounts = AccountService::new(Arc::clone(&database), credentials.clone(), http.clone());
    let repos = HostRepoService::new(Arc::clone(&database), credentials, http.clone());

    // 登录 = 用令牌换一次 `/user`（读操作）：返回的账号是令牌有效的证据
    let account = accounts
        .login_with_pat(&target.host, SecretString::from(token))
        .await
        .expect("PAT 登录失败：令牌无效、无网络，或站点不可达");
    assert!(!account.login.is_empty(), "GitHub 必须返回登录名");
    assert_eq!(account.host, target.host);

    let repo_ref = RemoteRepoRef {
        host: target.host.clone(),
        repo_id: None,
        owner: target.owner.clone(),
        repo: target.repo.clone(),
    };

    // ---- 读：PR 列表 ----
    let pulls = repos
        .list_pulls(
            &repo_ref,
            PullListQuery {
                state: PullState::Open,
                page: None,
                per_page: Some(PER_PAGE),
            },
        )
        .await
        .expect("读 PR 列表失败");
    assert!(
        u32::try_from(pulls.items.len()).unwrap_or(u32::MAX) <= PER_PAGE,
        "per_page 没有被遵守：{}",
        pulls.items.len()
    );

    // ---- 读：Issue 列表 ----
    let issues = repos
        .list_issues(
            &repo_ref,
            IssueListQuery {
                state: IssueState::Open,
                page: None,
                per_page: Some(PER_PAGE),
            },
        )
        .await
        .expect("读 Issue 列表失败");
    assert!(u32::try_from(issues.items.len()).unwrap_or(u32::MAX) <= PER_PAGE);

    // ---- 读：Actions run 列表（可能为空：仓库没有流水线是完全正常的） ----
    let runs = repos
        .list_runs(&repo_ref, None, Some(PER_PAGE))
        .await
        .expect("读 workflow run 失败");

    // 限流快照：读响应头被捕获 = 额度信息对界面可用（T4.10 的数据源）
    let snapshot = repos.rate_limit_snapshot();
    assert!(
        snapshot.is_some(),
        "读了几次真实请求却没有限流快照：响应头捕获断了"
    );

    // 人类可读的证据（回报与 M4 验收报告直接引用；不含任何令牌材料）
    println!(
        "真实 GitHub 冒烟通过：{}/{}（登录名 {}）｜PR {} 条｜Issue {} 条｜workflow run {} 条｜剩余核心配额 {}",
        target.owner,
        target.repo,
        account.login,
        pulls.items.len(),
        issues.items.len(),
        runs.items.len(),
        snapshot.map_or_else(|| "-".to_owned(), |state| state.remaining.to_string())
    );
}
