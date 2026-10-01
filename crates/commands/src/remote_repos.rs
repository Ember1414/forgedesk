//! 托管平台远端仓库命令（T4.5）：列表、搜索、星标、fork 与每仓库账号绑定。
//!
//! # 能力等级与审计
//!
//! | 命令 | 能力等级 | 说明 |
//! | --- | --- | --- |
//! | `repo_remote_list` | Network | 列出账号可见/星标仓库（分页） |
//! | `repo_remote_starred` | Network | 列出星标仓库 |
//! | `repo_remote_search` | Network | 搜索（匿名可用；有账号用高配额） |
//! | `repo_remote_star` | Network | 加星/取消加星（远端状态变更，但不触碰本地仓库） |
//! | `repo_remote_fork` | Network | fork（GitHub 202：副本异步创建） |
//! | `repo_account_binding_get` | ReadOnly | 读仓库级绑定 |
//! | `repo_account_binding_set` | Mutating | 写/解除仓库级绑定（只写设置，不碰仓库内容） |
//!
//! 不走 SnapshotManager：这些命令不改本地仓库的任何内容（红线 R7 管
//! 的是仓库状态）；星标/fork 是远端语义，可重复执行。
//!
//! # host 是外部输入
//!
//! 一律校验非空并小写化；owner/repo 的合法性由 provider 层在拼 URL 前
//! 二次校验（含 `/` 或空段直接 `VALIDATION`）。

use serde::{Deserialize, Serialize};
use tauri::State;

use forgedesk_domain::{AppError, AppResult, ErrorCode};
use forgedesk_provider::{
    MergeOutcome, MergePullRequest, MergeStrategy, PullComment, PullPage, PullReview, PullState,
    RemoteRepo, RepoListScope, RepoPage, ReviewEvent,
};

use crate::account::AccountDto;
use crate::state::AppState;
use forgedesk_services::host_repos::PullDetailView;

/// PR 详情 DTO：结构化字段 + 消毒后的描述 HTML（原文不出后端）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PullDetailDto {
    /// PR 编号。
    pub number: u64,
    /// 标题。
    pub title: String,
    /// `open` / `closed`。
    pub state: String,
    /// 是否草稿。
    pub draft: bool,
    /// 是否已合并。
    pub merged: bool,
    /// 发起人。
    pub author: String,
    /// 源分支标签。
    pub head_label: String,
    /// 目标分支标签。
    pub base_label: String,
    /// 当前 head sha（合并预检用）。
    pub head_sha: String,
    /// 网页地址。
    pub html_url: String,
    /// 描述的消毒 HTML。
    pub body_html: Option<String>,
    /// 变更文件数。
    pub changed_files: u64,
    /// 新增行数。
    pub additions: u64,
    /// 删除行数。
    pub deletions: u64,
    /// 是否可合并。
    pub mergeable: Option<bool>,
    /// 合并状态（`clean`/`dirty`/`blocked`…）。
    pub mergeable_state: Option<String>,
    /// 创建时间。
    pub created_at: Option<String>,
    /// 最近更新。
    pub updated_at: Option<String>,
}

impl From<PullDetailView> for PullDetailDto {
    fn from(view: PullDetailView) -> Self {
        let detail = view.detail;
        let summary = detail.summary;
        Self {
            number: summary.number,
            title: summary.title,
            state: summary.state,
            draft: summary.draft,
            merged: summary.merged,
            author: summary.author,
            head_label: summary.head_label,
            base_label: summary.base_label,
            head_sha: detail.head_sha,
            html_url: summary.html_url,
            body_html: view.body_html,
            changed_files: detail.changed_files,
            additions: detail.additions,
            deletions: detail.deletions,
            mergeable: detail.mergeable,
            mergeable_state: detail.mergeable_state,
            created_at: summary.created_at,
            updated_at: summary.updated_at,
        }
    }
}

/// `repo_pull_list` 的请求体。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullListRequest {
    /// 站点。
    pub host: String,
    /// 所有者。
    pub owner: String,
    /// 仓库名。
    pub repo: String,
    /// 本地仓库 id（绑定解析来源；远端浏览场景可省）。
    #[serde(default)]
    pub repo_id: Option<i64>,
    /// 状态过滤（`open` / `closed` / `all`；缺省 open）。
    #[serde(default)]
    pub state_filter: Option<String>,
    /// 页码。
    #[serde(default)]
    pub page: Option<u32>,
    /// 每页条数。
    #[serde(default)]
    pub per_page: Option<u32>,
}

/// `repo_pull_merge` 的请求体。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullMergeRequest {
    /// 站点。
    pub host: String,
    /// 所有者。
    pub owner: String,
    /// 仓库名。
    pub repo: String,
    /// PR 编号。
    pub number: u64,
    /// 合并策略（`merge` / `squash` / `rebase`）。
    pub strategy: String,
    /// 本地仓库 id（可省）。
    #[serde(default)]
    pub repo_id: Option<i64>,
    /// 自定义提交标题。
    #[serde(default)]
    pub commit_title: Option<String>,
    /// 自定义提交正文。
    #[serde(default)]
    pub commit_message: Option<String>,
    /// 预检 head sha（远端版 PLAN_STALE 的判定输入）。
    #[serde(default)]
    pub expected_head_sha: Option<String>,
    /// 合并后删除源分支（需要 `headBranch` 齐备）。
    #[serde(default)]
    pub delete_branch: Option<bool>,
    /// 源分支名（refs API 认 branch 而非 `owner:branch`）。
    #[serde(default)]
    pub head_branch: Option<String>,
}

fn parse_pull_state(state: Option<String>) -> AppResult<PullState> {
    match state.as_deref().map(str::trim) {
        None | Some("") | Some("open") => Ok(PullState::Open),
        Some("closed") => Ok(PullState::Closed),
        Some("all") => Ok(PullState::All),
        Some(other) => Err(AppError::new(
            ErrorCode::Validation,
            format!("unknown pull state: {other}"),
        )),
    }
}

fn parse_strategy(strategy: &str) -> AppResult<MergeStrategy> {
    match strategy.trim() {
        "merge" => Ok(MergeStrategy::Merge),
        "squash" => Ok(MergeStrategy::Squash),
        "rebase" => Ok(MergeStrategy::Rebase),
        other => Err(AppError::new(
            ErrorCode::Validation,
            format!("unknown merge strategy: {other}"),
        )),
    }
}

fn parse_number(number: u64) -> AppResult<u64> {
    if number == 0 {
        return Err(AppError::new(
            ErrorCode::Validation,
            "pull request number must be positive",
        ));
    }
    Ok(number)
}

/// 列出 PR。能力等级：`Network`。
#[tauri::command]
pub async fn repo_pull_list(
    state: State<'_, AppState>,
    request: PullListRequest,
) -> AppResult<PullPage> {
    let host = validate_host(&request.host)?;
    let pull_state = parse_pull_state(request.state_filter)?;
    let target = forgedesk_services::host_repos::RemoteRepoRef {
        host,
        repo_id: request.repo_id,
        owner: request.owner,
        repo: request.repo,
    };
    let query = forgedesk_services::host_repos::PullListQuery {
        state: pull_state,
        page: request.page,
        per_page: request.per_page,
    };
    state.host_repos.list_pulls(&target, query).await
}

/// PR 详情（描述已消毒为 HTML）。能力等级：`Network`。
#[tauri::command]
pub async fn repo_pull_get(
    state: State<'_, AppState>,
    host: String,
    owner: String,
    repo: String,
    number: u64,
    repo_id: Option<i64>,
) -> AppResult<PullDetailDto> {
    let host = validate_host(&host)?;
    let number = parse_number(number)?;
    let view = state
        .host_repos
        .get_pull(&host, repo_id, &owner, &repo, number)
        .await?;
    Ok(PullDetailDto::from(view))
}

/// PR 的 review 列表。能力等级：`Network`。
#[tauri::command]
pub async fn repo_pull_reviews(
    state: State<'_, AppState>,
    host: String,
    owner: String,
    repo: String,
    number: u64,
    repo_id: Option<i64>,
) -> AppResult<Vec<PullReview>> {
    let host = validate_host(&host)?;
    let number = parse_number(number)?;
    state
        .host_repos
        .list_reviews(&host, repo_id, &owner, &repo, number)
        .await
}

/// 合并 PR。能力等级：`Network`。
///
/// `expectedHeadSha` 是 UI 打开详情时抓下的 head：远端又有新提交时合并
/// 会被 422 拒绝（`hint = "head-changed"`），这是远端版 PLAN_STALE。
#[tauri::command]
pub async fn repo_pull_merge(
    state: State<'_, AppState>,
    request: PullMergeRequest,
) -> AppResult<MergeOutcome> {
    let host = validate_host(&request.host)?;
    let number = parse_number(request.number)?;
    let merge = MergePullRequest {
        strategy: parse_strategy(&request.strategy)?,
        commit_title: request.commit_title,
        commit_message: request.commit_message,
        expected_head_sha: request.expected_head_sha,
        delete_branch: request.delete_branch.unwrap_or(false),
        head_branch: request.head_branch,
    };
    state
        .host_repos
        .merge_pull(
            &host,
            request.repo_id,
            &request.owner,
            &request.repo,
            number,
            merge,
        )
        .await
}

/// 远端仓库分页（provider 的 [`RepoPage`] 已是 camelCase，原样透出）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoPageDto {
    /// 本页内容。
    pub items: Vec<RemoteRepoDto>,
    /// 下一页页码；`None` 表示没有更多。
    pub next_page: Option<u32>,
}

/// 远端仓库条目（provider 的 [`RemoteRepo`] 原样透出）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteRepoDto {
    /// 平台内数字 id。
    pub id: u64,
    /// 所有者。
    pub owner: String,
    /// 仓库名。
    pub name: String,
    /// `owner/name`。
    pub full_name: String,
    /// 描述。
    pub description: Option<String>,
    /// 网页地址。
    pub html_url: String,
    /// 默认分支。
    pub default_branch: Option<String>,
    /// 是否私有。
    pub private: bool,
    /// 是否 fork。
    pub fork: bool,
    /// Star 数。
    pub stars: u64,
    /// 最近 push 时间。
    pub pushed_at: Option<String>,
}

impl From<RepoPage> for RepoPageDto {
    fn from(page: RepoPage) -> Self {
        Self {
            items: page.items.into_iter().map(Into::into).collect(),
            next_page: page.next_page,
        }
    }
}

impl From<RemoteRepo> for RemoteRepoDto {
    fn from(repo: RemoteRepo) -> Self {
        Self {
            id: repo.id,
            owner: repo.owner,
            name: repo.name,
            full_name: repo.full_name,
            description: repo.description,
            html_url: repo.html_url,
            default_branch: repo.default_branch,
            private: repo.private,
            fork: repo.fork,
            stars: repo.stars,
            pushed_at: repo.pushed_at,
        }
    }
}

fn validate_host(host: &str) -> AppResult<String> {
    let host = host.trim().to_ascii_lowercase();
    if host.is_empty() {
        return Err(AppError::new(
            ErrorCode::Validation,
            "provider host must not be empty",
        ));
    }
    Ok(host)
}

fn parse_scope(scope: Option<String>) -> AppResult<RepoListScope> {
    match scope.as_deref().map(str::trim) {
        None | Some("") | Some("owned") => Ok(RepoListScope::Owned),
        Some("all") => Ok(RepoListScope::All),
        Some(other) => Err(AppError::new(
            ErrorCode::Validation,
            format!("unknown repo list scope: {other}"),
        )),
    }
}

/// 列出账号可见的仓库。能力等级：`Network`。
#[tauri::command]
pub async fn repo_remote_list(
    state: State<'_, AppState>,
    host: String,
    repo_id: Option<i64>,
    scope: Option<String>,
    page: Option<u32>,
    per_page: Option<u32>,
) -> AppResult<RepoPageDto> {
    let host = validate_host(&host)?;
    let scope = parse_scope(scope)?;
    let page = state
        .host_repos
        .list_authenticated(&host, repo_id, scope, page, per_page)
        .await?;
    Ok(RepoPageDto::from(page))
}

/// 列出账号星标的仓库。能力等级：`Network`。
#[tauri::command]
pub async fn repo_remote_starred(
    state: State<'_, AppState>,
    host: String,
    repo_id: Option<i64>,
    page: Option<u32>,
    per_page: Option<u32>,
) -> AppResult<RepoPageDto> {
    let host = validate_host(&host)?;
    let page = state
        .host_repos
        .list_starred(&host, repo_id, page, per_page)
        .await?;
    Ok(RepoPageDto::from(page))
}

/// 搜索仓库。能力等级：`Network`。
#[tauri::command]
pub async fn repo_remote_search(
    state: State<'_, AppState>,
    host: String,
    query: String,
    repo_id: Option<i64>,
    page: Option<u32>,
    per_page: Option<u32>,
) -> AppResult<RepoPageDto> {
    let host = validate_host(&host)?;
    let page = state
        .host_repos
        .search(&host, repo_id, &query, page, per_page)
        .await?;
    Ok(RepoPageDto::from(page))
}

/// 加星 / 取消加星。能力等级：`Network`。
#[tauri::command]
pub async fn repo_remote_star(
    state: State<'_, AppState>,
    host: String,
    owner: String,
    repo: String,
    starred: bool,
    repo_id: Option<i64>,
) -> AppResult<()> {
    let host = validate_host(&host)?;
    state
        .host_repos
        .set_starred(&host, repo_id, &owner, &repo, starred)
        .await
}

/// fork 到当前账号名下。能力等级：`Network`。
#[tauri::command]
pub async fn repo_remote_fork(
    state: State<'_, AppState>,
    host: String,
    owner: String,
    repo: String,
    repo_id: Option<i64>,
) -> AppResult<RemoteRepoDto> {
    let host = validate_host(&host)?;
    let forked = state.host_repos.fork(&host, repo_id, &owner, &repo).await?;
    Ok(RemoteRepoDto::from(forked))
}

/// 拉取并安全渲染仓库 README（T4.6）。能力等级：`Network`。
///
/// 返回**已消毒**的 HTML 片段（清洗规则与 XSS 用例在
/// `forgedesk_services::readme`）：前端直接渲染，不再接触原始 Markdown。
/// 仓库没有 README 时返回 `NOT_FOUND`。
#[tauri::command]
pub async fn repo_remote_readme(
    state: State<'_, AppState>,
    host: String,
    owner: String,
    repo: String,
    repo_id: Option<i64>,
) -> AppResult<String> {
    let host = validate_host(&host)?;
    state.host_repos.readme(&host, repo_id, &owner, &repo).await
}

/// PR 时间线评论列表。能力等级：`Network`。
#[tauri::command]
pub async fn repo_pull_comments_list(
    state: State<'_, AppState>,
    host: String,
    owner: String,
    repo: String,
    number: u64,
    repo_id: Option<i64>,
) -> AppResult<Vec<PullComment>> {
    let host = validate_host(&host)?;
    let number = parse_number(number)?;
    state
        .host_repos
        .list_comments(&host, repo_id, &owner, &repo, number)
        .await
}

/// 发表一条 PR 时间线评论。能力等级：`Network`。
#[tauri::command]
pub async fn repo_pull_comment_create(
    state: State<'_, AppState>,
    host: String,
    owner: String,
    repo: String,
    number: u64,
    body: String,
    repo_id: Option<i64>,
) -> AppResult<PullComment> {
    let host = validate_host(&host)?;
    let number = parse_number(number)?;
    state
        .host_repos
        .create_comment(&host, repo_id, &owner, &repo, number, &body)
        .await
}

/// `repo_pull_review_submit` 的请求体。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullReviewSubmitRequest {
    /// 站点。
    pub host: String,
    /// 所有者。
    pub owner: String,
    /// 仓库名。
    pub repo: String,
    /// PR 编号。
    pub number: u64,
    /// 结论（`APPROVE` / `REQUEST_CHANGES` / `COMMENT`）。
    pub event: String,
    /// 正文（COMMENT 时必填）。
    #[serde(default)]
    pub body: Option<String>,
    /// 本地仓库 id（可省）。
    #[serde(default)]
    pub repo_id: Option<i64>,
}

/// 提交一次 review（批准 / 请求修改 / 评论）。能力等级：`Network`。
#[tauri::command]
pub async fn repo_pull_review_submit(
    state: State<'_, AppState>,
    request: PullReviewSubmitRequest,
) -> AppResult<()> {
    let host = validate_host(&request.host)?;
    let number = parse_number(request.number)?;
    let event = match request.event.trim() {
        "APPROVE" => ReviewEvent::Approve,
        "REQUEST_CHANGES" => ReviewEvent::RequestChanges,
        "COMMENT" => ReviewEvent::Comment,
        other => {
            return Err(AppError::new(
                ErrorCode::Validation,
                format!("unknown review event: {other}"),
            ))
        }
    };
    let target = forgedesk_services::host_repos::RemoteRepoRef {
        host,
        repo_id: request.repo_id,
        owner: request.owner,
        repo: request.repo,
    };
    let submission = forgedesk_services::host_repos::ReviewSubmission {
        number,
        event,
        body: request.body,
    };
    state.host_repos.submit_review(&target, submission).await
}

/// 读取仓库绑定的账号。能力等级：`ReadOnly`。
#[tauri::command]
pub fn repo_account_binding_get(
    state: State<'_, AppState>,
    repo_id: i64,
) -> AppResult<Option<AccountDto>> {
    Ok(state.host_repos.binding(repo_id)?.map(AccountDto::from))
}

/// 设置/解除仓库绑定的账号。能力等级：`Mutating`。
#[tauri::command]
pub fn repo_account_binding_set(
    state: State<'_, AppState>,
    repo_id: i64,
    account_id: Option<String>,
) -> AppResult<Option<AccountDto>> {
    let binding = state
        .host_repos
        .set_binding(repo_id, account_id.as_deref())?;
    Ok(binding.map(AccountDto::from))
}
