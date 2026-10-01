//! 托管平台 Issue 命令（T4.8）：列表、详情、创建、编辑、关开、指派、评论。
//!
//! # 能力等级与审计
//!
//! 全部 `Network`（远端状态语义，不触碰本地仓库，不走 SnapshotManager；
//! 与 PR 命令同一结论，见 `remote_repos.rs` 模块头的说明）。
//!
//! # 描述原文不出后端
//!
//! 详情/创建/编辑返回的 `bodyHtml` 来自 services 层的白名单渲染
//! （`forgedesk_services::readme`），原始 Markdown 在 provider 侧就已
//! `skip_serializing`，这里是第三道防线。

use serde::{Deserialize, Serialize};
use tauri::State;

use forgedesk_domain::{AppError, AppResult, ErrorCode};
use forgedesk_provider::{IssueComment, IssuePage, IssueState};
use forgedesk_services::host_repos::IssueDetailView;

use crate::state::AppState;

/// Issue 详情 DTO：结构化字段 + 消毒后的描述 HTML。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueDetailDto {
    /// Issue 编号。
    pub number: u64,
    /// 标题。
    pub title: String,
    /// `open` / `closed`。
    pub state: String,
    /// 发起人。
    pub author: String,
    /// 标签名。
    pub labels: Vec<String>,
    /// 指派人 login。
    pub assignees: Vec<String>,
    /// 评论数。
    pub comments: u64,
    /// 描述的消毒 HTML。
    pub body_html: Option<String>,
    /// 创建时间。
    pub created_at: Option<String>,
    /// 最近更新。
    pub updated_at: Option<String>,
    /// 关闭时间。
    pub closed_at: Option<String>,
}

impl From<IssueDetailView> for IssueDetailDto {
    fn from(view: IssueDetailView) -> Self {
        let summary = view.detail.summary;
        Self {
            number: summary.number,
            title: summary.title,
            state: summary.state,
            author: summary.author,
            labels: summary.labels,
            assignees: summary.assignees,
            comments: summary.comments,
            body_html: view.body_html,
            created_at: summary.created_at,
            updated_at: summary.updated_at,
            closed_at: summary.closed_at,
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

fn parse_number(number: u64) -> AppResult<u64> {
    if number == 0 {
        return Err(AppError::new(
            ErrorCode::Validation,
            "issue number must be positive",
        ));
    }
    Ok(number)
}

fn parse_issue_state(state: Option<String>) -> AppResult<IssueState> {
    match state.as_deref().map(str::trim) {
        None | Some("") | Some("open") => Ok(IssueState::Open),
        Some("closed") => Ok(IssueState::Closed),
        Some("all") => Ok(IssueState::All),
        Some(other) => Err(AppError::new(
            ErrorCode::Validation,
            format!("unknown issue state: {other}"),
        )),
    }
}

/// `repo_issue_list` 的请求体。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueListRequest {
    /// 站点。
    pub host: String,
    /// 所有者。
    pub owner: String,
    /// 仓库名。
    pub repo: String,
    /// 本地仓库 id（可省）。
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

/// `repo_issue_create` 的请求体。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueCreateRequest {
    /// 站点。
    pub host: String,
    /// 所有者。
    pub owner: String,
    /// 仓库名。
    pub repo: String,
    /// 标题（必填）。
    pub title: String,
    /// 描述（Markdown；可省）。
    #[serde(default)]
    pub body: Option<String>,
    /// 本地仓库 id（可省）。
    #[serde(default)]
    pub repo_id: Option<i64>,
}

/// `repo_issue_edit` 的请求体。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueEditRequest {
    /// 站点。
    pub host: String,
    /// 所有者。
    pub owner: String,
    /// 仓库名。
    pub repo: String,
    /// Issue 编号。
    pub number: u64,
    /// 新标题（`null` 不动）。
    #[serde(default)]
    pub title: Option<String>,
    /// 新描述（`null` 不动；传空串清空描述）。
    #[serde(default)]
    pub body: Option<String>,
    /// 本地仓库 id（可省）。
    #[serde(default)]
    pub repo_id: Option<i64>,
}

/// `repo_issue_state_set` 的请求体。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueStateRequest {
    /// 站点。
    pub host: String,
    /// 所有者。
    pub owner: String,
    /// 仓库名。
    pub repo: String,
    /// Issue 编号。
    pub number: u64,
    /// `true` 重新开启 / `false` 关闭。
    pub open: bool,
    /// 本地仓库 id（可省）。
    #[serde(default)]
    pub repo_id: Option<i64>,
}

/// `repo_issue_assignees_set` 的请求体。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueAssigneesRequest {
    /// 站点。
    pub host: String,
    /// 所有者。
    pub owner: String,
    /// 仓库名。
    pub repo: String,
    /// Issue 编号。
    pub number: u64,
    /// 指派人 login 全量（空数组 = 全部取消指派）。
    pub assignees: Vec<String>,
    /// 本地仓库 id（可省）。
    #[serde(default)]
    pub repo_id: Option<i64>,
}

/// 列出 Issue（不含 PR）。能力等级：`Network`。
#[tauri::command]
pub async fn repo_issue_list(
    state: State<'_, AppState>,
    request: IssueListRequest,
) -> AppResult<IssuePage> {
    let host = validate_host(&request.host)?;
    let issue_state = parse_issue_state(request.state_filter)?;
    let target = forgedesk_services::host_repos::RemoteRepoRef {
        host,
        repo_id: request.repo_id,
        owner: request.owner,
        repo: request.repo,
    };
    let query = forgedesk_services::host_repos::IssueListQuery {
        state: issue_state,
        page: request.page,
        per_page: request.per_page,
    };
    state.host_repos.list_issues(&target, query).await
}

/// Issue 详情（描述已消毒为 HTML）。能力等级：`Network`。
#[tauri::command]
pub async fn repo_issue_get(
    state: State<'_, AppState>,
    host: String,
    owner: String,
    repo: String,
    number: u64,
    repo_id: Option<i64>,
) -> AppResult<IssueDetailDto> {
    let host = validate_host(&host)?;
    let number = parse_number(number)?;
    let view = state
        .host_repos
        .get_issue(&host, repo_id, &owner, &repo, number)
        .await?;
    Ok(IssueDetailDto::from(view))
}

/// Issue 的原始描述（Markdown）——只供编辑器预填进 textarea。能力等级：`Network`。
///
/// 展示永远走 `repo_issue_get` 的消毒 HTML；这个命令与评论正文同一边界
/// 判断：原文只作为惰性文本（React 转义、无 innerHTML），不用于渲染。
#[tauri::command]
pub async fn repo_issue_body(
    state: State<'_, AppState>,
    host: String,
    owner: String,
    repo: String,
    number: u64,
    repo_id: Option<i64>,
) -> AppResult<String> {
    let host = validate_host(&host)?;
    let number = parse_number(number)?;
    state
        .host_repos
        .issue_body_raw(&host, repo_id, &owner, &repo, number)
        .await
}

/// 创建 Issue。能力等级：`Network`。
#[tauri::command]
pub async fn repo_issue_create(
    state: State<'_, AppState>,
    request: IssueCreateRequest,
) -> AppResult<IssueDetailDto> {
    let host = validate_host(&request.host)?;
    let view = state
        .host_repos
        .create_issue(
            &host,
            request.repo_id,
            &request.owner,
            &request.repo,
            &request.title,
            request.body.as_deref(),
        )
        .await?;
    Ok(IssueDetailDto::from(view))
}

/// 编辑 Issue 的标题/描述（`null` 字段不动）。能力等级：`Network`。
#[tauri::command]
pub async fn repo_issue_edit(
    state: State<'_, AppState>,
    request: IssueEditRequest,
) -> AppResult<IssueDetailDto> {
    let host = validate_host(&request.host)?;
    let number = parse_number(request.number)?;
    if request.title.is_none() && request.body.is_none() {
        return Err(AppError::new(
            ErrorCode::Validation,
            "issue edit must change title or body",
        ));
    }
    let edit = forgedesk_provider::IssueEdit {
        title: request.title,
        body: request.body,
    };
    let view = state
        .host_repos
        .edit_issue(
            &host,
            request.repo_id,
            &request.owner,
            &request.repo,
            number,
            edit,
        )
        .await?;
    Ok(IssueDetailDto::from(view))
}

/// 关闭 / 重新开启 Issue。能力等级：`Network`。
#[tauri::command]
pub async fn repo_issue_state_set(
    state: State<'_, AppState>,
    request: IssueStateRequest,
) -> AppResult<IssueDetailDto> {
    let host = validate_host(&request.host)?;
    let number = parse_number(request.number)?;
    let view = state
        .host_repos
        .set_issue_state(
            &host,
            request.repo_id,
            &request.owner,
            &request.repo,
            number,
            request.open,
        )
        .await?;
    Ok(IssueDetailDto::from(view))
}

/// 整体替换 Issue 指派人（空数组 = 全部取消）。能力等级：`Network`。
#[tauri::command]
pub async fn repo_issue_assignees_set(
    state: State<'_, AppState>,
    request: IssueAssigneesRequest,
) -> AppResult<IssueDetailDto> {
    let host = validate_host(&request.host)?;
    let number = parse_number(request.number)?;
    let view = state
        .host_repos
        .set_issue_assignees(
            &host,
            request.repo_id,
            &request.owner,
            &request.repo,
            number,
            &request.assignees,
        )
        .await?;
    Ok(IssueDetailDto::from(view))
}

/// Issue 评论列表。能力等级：`Network`。
#[tauri::command]
pub async fn repo_issue_comments_list(
    state: State<'_, AppState>,
    host: String,
    owner: String,
    repo: String,
    number: u64,
    repo_id: Option<i64>,
) -> AppResult<Vec<IssueComment>> {
    let host = validate_host(&host)?;
    let number = parse_number(number)?;
    state
        .host_repos
        .list_issue_comments(&host, repo_id, &owner, &repo, number)
        .await
}

/// 发表一条 Issue 评论。能力等级：`Network`。
#[tauri::command]
pub async fn repo_issue_comment_create(
    state: State<'_, AppState>,
    host: String,
    owner: String,
    repo: String,
    number: u64,
    body: String,
    repo_id: Option<i64>,
) -> AppResult<IssueComment> {
    let host = validate_host(&host)?;
    let number = parse_number(number)?;
    state
        .host_repos
        .create_issue_comment(&host, repo_id, &owner, &repo, number, &body)
        .await
}

/// 可指派人列表。能力等级：`Network`。
#[tauri::command]
pub async fn repo_issue_assignees(
    state: State<'_, AppState>,
    host: String,
    owner: String,
    repo: String,
    repo_id: Option<i64>,
) -> AppResult<Vec<forgedesk_provider::Assignee>> {
    let host = validate_host(&host)?;
    state
        .host_repos
        .list_assignees(&host, repo_id, &owner, &repo)
        .await
}
