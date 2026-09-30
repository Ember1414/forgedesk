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

use serde::Serialize;
use tauri::State;

use forgedesk_domain::{AppError, AppResult, ErrorCode};
use forgedesk_provider::{RemoteRepo, RepoListScope, RepoPage};

use crate::account::AccountDto;
use crate::state::AppState;

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
