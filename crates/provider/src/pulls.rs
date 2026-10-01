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
//!
//! # 行内（diff 锚定）评论在本地先校验（T4.7 收尾）
//!
//! 行内评论锚定到 diff 的具体行（`side`：LEFT=旧文件 / RIGHT=新文件 + 行号）。
//! 创建前先取 `GET /pulls/{n}/files` 的**当前** diff 做本地校验：路径不在
//! diff 上、行号越界、多行评论跨 hunk 都直接 `VALIDATION`——**不发出写
//! 请求**，用户按下发表立刻得到可读错误，而不是等 GitHub 的 422。文件
//! 没有 `patch`（二进制/超大 diff 被截断）时无法本地校验，放行交给
//! GitHub 兜底。`commit_id` 取同一时刻的 head：校验与锚定基于同一份 diff。
//!
//! 行内评论的列表（review comments）单页取 100 条（与时间线评论同一
//! 取舍）：超过 100 条的 PR 少见，翻页收益不抵每次多出的请求。

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

/// PR 时间线评论（复用 Issue 评论端点）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PullComment {
    /// 评论 id。
    pub id: u64,
    /// 评论者。
    pub author: String,
    /// 正文（Markdown 原文；展示层消毒）。
    pub body: String,
    /// 创建时间（RFC3339）。
    pub created_at: Option<String>,
}

/// 行内（锚定 diff 行）评论：`GET/POST /pulls/{n}/comments` 的 review comment。
///
/// 与 [`PullComment`]（时间线评论，挂 issue 端点）的区别就在锚点：
/// 这一类带 path/side/line，UI 把它们渲染在 diff 的对应行下面。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PullReviewComment {
    /// 评论 id。
    pub id: u64,
    /// 被回复的评论 id（顶层评论为 `None`）。
    pub in_reply_to: Option<u64>,
    /// 评论者。
    pub author: String,
    /// 正文（Markdown 原文；展示层纯文本渲染）。
    pub body: String,
    /// 锚定的文件路径（回复评论 GitHub 也会回显）。
    pub path: Option<String>,
    /// 锚定侧：`LEFT` / `RIGHT`（原样透出）。
    pub side: Option<String>,
    /// 锚定行（多行评论的末行）。
    pub line: Option<u32>,
    /// 多行评论的起始行。
    pub start_line: Option<u32>,
    /// 多行评论的起始侧。
    pub start_side: Option<String>,
    /// 创建时间（RFC3339）。
    pub created_at: Option<String>,
}

/// 行内评论锚定的一侧（GitHub 语义：LEFT 是旧文件，RIGHT 是新文件）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentSide {
    /// 旧文件（删除行与其上下文）。
    Left,
    /// 新文件（新增行与其上下文）。
    Right,
}

impl CommentSide {
    fn as_str(self) -> &'static str {
        match self {
            Self::Left => "LEFT",
            Self::Right => "RIGHT",
        }
    }

    /// 错误消息里的人类可读形式（开发者读；见 CODING_STYLE 错误分工）。
    fn display(self) -> &'static str {
        match self {
            Self::Left => "old file",
            Self::Right => "new file",
        }
    }
}

/// 行内评论的锚点：文件 + 侧 + 行号（多行评论另有起始行）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewCommentAnchor {
    /// 文件路径（与变更文件列表里的 `filename` 精确匹配）。
    pub path: String,
    /// 锚定侧。
    pub side: CommentSide,
    /// 锚定行（多行评论的末行）。
    pub line: u32,
    /// 多行评论的起始行。
    pub start_line: Option<u32>,
    /// 多行评论的起始侧（缺省与 [`Self::side`] 相同）。
    pub start_side: Option<CommentSide>,
}

/// 行内 diff 的一个 hunk（形状与工作区 diff DTO 对齐，前端同一套渲染）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PullDiffHunk {
    /// 旧文件起始行号。
    pub old_start: u32,
    /// 旧文件行数。
    pub old_lines: u32,
    /// 新文件起始行号。
    pub new_start: u32,
    /// 新文件行数。
    pub new_lines: u32,
    /// `@@` 之后的上下文（通常是函数签名）。
    pub header: String,
    /// hunk 内的行。
    pub lines: Vec<PullDiffLine>,
}

/// 行内 diff 的一行（`kind` 与工作区 diff DTO 同名同义）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PullDiffLine {
    /// `context` / `added` / `removed` / `noNewline`。
    pub kind: &'static str,
    /// 行内容（不含前缀与换行）。
    pub content: String,
    /// 旧文件行号（新增行为 `null`）。
    pub old_no: Option<u32>,
    /// 新文件行号（删除行为 `null`）。
    pub new_no: Option<u32>,
}

/// PR 变更文件（`GET /pulls/{n}/files` 条目）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PullFile {
    /// 文件路径（重命名后）。
    pub filename: String,
    /// 重命名前的路径。
    pub previous_filename: Option<String>,
    /// `added` / `removed` / `modified` / `renamed` / `changed` / `copied`。
    pub status: String,
    /// 新增行数。
    pub additions: u64,
    /// 删除行数。
    pub deletions: u64,
    /// 变更行数（additions + deletions）。
    pub changes: Option<u64>,
    /// 统一 diff 的 hunk 原文（二进制/超大 diff 时 GitHub 不给）。
    pub patch: Option<String>,
    /// 从 `patch` 解析出的 hunk（行级渲染与行号校验共用；patch 缺省时为空）。
    pub hunks: Vec<PullDiffHunk>,
}

/// PR 变更文件一页 + 下一页游标。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PullFilePage {
    /// 本页内容。
    pub items: Vec<PullFile>,
    /// 下一页页码；`None` 表示没有更多。
    pub next_page: Option<u32>,
}

/// review 结论。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewEvent {
    /// 批准。
    Approve,
    /// 请求修改。
    RequestChanges,
    /// 仅评论。
    Comment,
}

impl ReviewEvent {
    fn as_str(self) -> &'static str {
        match self {
            Self::Approve => "APPROVE",
            Self::RequestChanges => "REQUEST_CHANGES",
            Self::Comment => "COMMENT",
        }
    }
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
struct GitHubComment {
    id: u64,
    #[serde(default)]
    user: Option<GitHubLogin>,
    body: String,
    #[serde(default)]
    created_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GitHubReviewComment {
    id: u64,
    #[serde(default)]
    in_reply_to: Option<u64>,
    #[serde(default)]
    user: Option<GitHubLogin>,
    body: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    side: Option<String>,
    #[serde(default)]
    line: Option<u32>,
    #[serde(default)]
    start_line: Option<u32>,
    #[serde(default)]
    start_side: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
}

impl From<GitHubReviewComment> for PullReviewComment {
    fn from(comment: GitHubReviewComment) -> Self {
        Self {
            id: comment.id,
            in_reply_to: comment.in_reply_to,
            author: comment.user.map(|user| user.login).unwrap_or_default(),
            body: comment.body,
            path: comment.path,
            side: comment.side,
            line: comment.line,
            start_line: comment.start_line,
            start_side: comment.start_side,
            created_at: comment.created_at,
        }
    }
}

#[derive(Debug, Deserialize)]
struct GitHubFile {
    filename: String,
    #[serde(default)]
    previous_filename: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    additions: Option<u64>,
    #[serde(default)]
    deletions: Option<u64>,
    #[serde(default)]
    changes: Option<u64>,
    #[serde(default)]
    patch: Option<String>,
}

impl From<GitHubFile> for PullFile {
    fn from(file: GitHubFile) -> Self {
        let hunks = file
            .patch
            .as_deref()
            .map(parse_patch_hunks)
            .unwrap_or_default();
        Self {
            filename: file.filename,
            previous_filename: file.previous_filename,
            status: file.status.unwrap_or_default(),
            additions: file.additions.unwrap_or(0),
            deletions: file.deletions.unwrap_or(0),
            changes: file.changes,
            patch: file.patch,
            hunks,
        }
    }
}

impl From<GitHubComment> for PullComment {
    fn from(comment: GitHubComment) -> Self {
        Self {
            id: comment.id,
            author: comment.user.map(|user| user.login).unwrap_or_default(),
            body: comment.body,
            created_at: comment.created_at,
        }
    }
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

    /// 在变更文件里找目标路径并解析其 hunk（行内评论的本地校验输入）。
    ///
    /// files 端点分页返回：逐页找目标路径，页数上限内没有就
    /// `path-not-in-diff`（正常 PR 远达不到上限规模）。
    /// 返回 `None` 表示文件在 diff 里但没有 `patch`（二进制/超大 diff）：
    /// 无法本地校验，放行交给 GitHub 兜底。
    async fn find_file_hunks(
        &self,
        token: SecretString,
        owner: &str,
        repo: &str,
        number: u64,
        path: &str,
    ) -> Result<Option<Vec<PullDiffHunk>>, AppError> {
        const MAX_VALIDATION_PAGES: u32 = 10;
        let mut page = 1_u32;
        loop {
            let url = format!("{}/files", self.pull_path(owner, repo, Some(number))?);
            let request = ApiRequest::get(url)
                .with_bearer(token.clone())
                .with_query(vec![
                    ("per_page".to_owned(), MAX_PER_PAGE.to_string()),
                    ("page".to_owned(), page.to_string()),
                ]);
            let response = self.http().send(&request).await?;
            // Link 头要在 json() 消费 response 之前取
            let has_next = response
                .headers()
                .get("link")
                .and_then(|value| value.to_str().ok())
                .and_then(crate::repos::next_page_from_link_header)
                .is_some();
            let files = response
                .json::<Vec<GitHubFile>>()
                .await
                .map_err(map_transport_error)?;
            if let Some(file) = files.iter().find(|file| file.filename == path) {
                return Ok(file.patch.as_deref().map(parse_patch_hunks));
            }
            if !has_next || page >= MAX_VALIDATION_PAGES {
                return Err(path_not_in_diff(path));
            }
            page += 1;
        }
    }

    /// 当前 head 的 sha（行内评论的 `commit_id`）。
    ///
    /// 取不到时返回 `None`：GitHub 会自行锚定到最新 head，比猜测安全。
    async fn head_sha(
        &self,
        token: SecretString,
        owner: &str,
        repo: &str,
        number: u64,
    ) -> Result<Option<String>, AppError> {
        let request =
            ApiRequest::get(self.pull_path(owner, repo, Some(number))?).with_bearer(token);
        let pull: GitHubPull = self
            .http()
            .send(&request)
            .await?
            .json()
            .await
            .map_err(map_transport_error)?;
        Ok(pull
            .head
            .and_then(|head| head.sha)
            .filter(|sha| !sha.is_empty()))
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

    async fn list_comments(
        &self,
        token: SecretString,
        owner: &str,
        repo: &str,
        number: u64,
    ) -> Result<Vec<PullComment>, AppError> {
        // PR 的评论挂在 issue 端点上（GitHub 的 PR 本就是 issue + diff 的组合）
        let url = format!("{}/issues/{number}/comments", self.repo_path(owner, repo)?);
        let request = ApiRequest::get(url).with_bearer(token);
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

    async fn create_comment(
        &self,
        token: SecretString,
        owner: &str,
        repo: &str,
        number: u64,
        body: &str,
    ) -> Result<PullComment, AppError> {
        let trimmed = body.trim();
        if trimmed.is_empty() {
            return Err(AppError::new(
                ErrorCode::Validation,
                "comment body must not be empty",
            ));
        }
        let url = format!("{}/issues/{number}/comments", self.repo_path(owner, repo)?);
        let request =
            ApiRequest::post_json(url, serde_json::json!({ "body": trimmed })).with_bearer(token);
        let response = self.http().send(&request).await?;
        response
            .json::<GitHubComment>()
            .await
            .map_err(map_transport_error)
            .map(Into::into)
    }

    async fn submit_review(
        &self,
        token: SecretString,
        owner: &str,
        repo: &str,
        number: u64,
        event: ReviewEvent,
        body: Option<&str>,
    ) -> Result<(), AppError> {
        // 无正文的 COMMENT 是空评论，直接拒绝；APPROVE/REQUEST_CHANGES 的
        // body 是可选的（GitHub 允许纯结论）
        let trimmed = body.map(str::trim).unwrap_or("");
        if event == ReviewEvent::Comment && trimmed.is_empty() {
            return Err(AppError::new(
                ErrorCode::Validation,
                "review body must not be empty",
            ));
        }
        let mut payload = serde_json::json!({ "event": event.as_str() });
        if !trimmed.is_empty() {
            payload["body"] = serde_json::Value::String(trimmed.to_owned());
        }
        let url = format!("{}/reviews", self.pull_path(owner, repo, Some(number))?);
        let request = ApiRequest::post_json(url, payload).with_bearer(token);
        self.http().send(&request).await?;
        Ok(())
    }

    async fn list_files(
        &self,
        token: SecretString,
        owner: &str,
        repo: &str,
        number: u64,
        page: Option<u32>,
        per_page: Option<u32>,
    ) -> Result<PullFilePage, AppError> {
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
        let url = format!("{}/files", self.pull_path(owner, repo, Some(number))?);
        let request = ApiRequest::get(url).with_bearer(token).with_query(query);
        let response = self.http().send(&request).await?;
        let next_page = response
            .headers()
            .get("link")
            .and_then(|value| value.to_str().ok())
            .and_then(crate::repos::next_page_from_link_header);
        let items = response
            .json::<Vec<GitHubFile>>()
            .await
            .map_err(map_transport_error)?
            .into_iter()
            .map(Into::into)
            .collect();
        Ok(PullFilePage { items, next_page })
    }

    async fn list_review_comments(
        &self,
        token: SecretString,
        owner: &str,
        repo: &str,
        number: u64,
    ) -> Result<Vec<PullReviewComment>, AppError> {
        let url = format!("{}/comments", self.pull_path(owner, repo, Some(number))?);
        let request = ApiRequest::get(url)
            .with_bearer(token)
            .with_query(vec![("per_page".to_owned(), MAX_PER_PAGE.to_string())]);
        let response = self.http().send(&request).await?;
        let comments = response
            .json::<Vec<GitHubReviewComment>>()
            .await
            .map_err(map_transport_error)?
            .into_iter()
            .map(Into::into)
            .collect();
        Ok(comments)
    }

    async fn create_review_comment(
        &self,
        token: SecretString,
        owner: &str,
        repo: &str,
        number: u64,
        anchor: ReviewCommentAnchor,
        body: &str,
    ) -> Result<PullReviewComment, AppError> {
        let trimmed = body.trim();
        if trimmed.is_empty() {
            return Err(AppError::new(
                ErrorCode::Validation,
                "comment body must not be empty",
            ));
        }
        // 先本地校验再写：路径/行号的错误要在按下发表时立刻可读（模块文档）
        if let Some(hunks) = self
            .find_file_hunks(token.clone(), owner, repo, number, &anchor.path)
            .await?
        {
            validate_anchor(&hunks, &anchor)?;
        }
        let head_sha = self.head_sha(token.clone(), owner, repo, number).await?;
        let mut payload = serde_json::json!({
            "body": trimmed,
            "path": anchor.path,
            "side": anchor.side.as_str(),
            "line": anchor.line,
        });
        if let Some(start) = anchor.start_line {
            payload["start_line"] = serde_json::Value::from(start);
            payload["start_side"] =
                serde_json::Value::from(anchor.start_side.unwrap_or(anchor.side).as_str());
        }
        // 锚定与校验基于同一份 diff（head_sha 为空时缺省由 GitHub 锚定最新 head）
        if let Some(sha) = head_sha {
            payload["commit_id"] = serde_json::Value::from(sha);
        }
        let url = format!("{}/comments", self.pull_path(owner, repo, Some(number))?);
        let request = ApiRequest::post_json(url, payload).with_bearer(token);
        let response = self.http().send(&request).await?;
        response
            .json::<GitHubReviewComment>()
            .await
            .map_err(map_transport_error)
            .map(Into::into)
    }

    async fn reply_review_comment(
        &self,
        token: SecretString,
        owner: &str,
        repo: &str,
        number: u64,
        comment_id: u64,
        body: &str,
    ) -> Result<PullReviewComment, AppError> {
        let trimmed = body.trim();
        if trimmed.is_empty() {
            return Err(AppError::new(
                ErrorCode::Validation,
                "comment body must not be empty",
            ));
        }
        if comment_id == 0 {
            return Err(AppError::new(
                ErrorCode::Validation,
                "comment id must be positive",
            ));
        }
        let url = format!("{}/comments", self.pull_path(owner, repo, Some(number))?);
        let request = ApiRequest::post_json(
            url,
            serde_json::json!({ "body": trimmed, "in_reply_to": comment_id }),
        )
        .with_bearer(token);
        let response = self.http().send(&request).await?;
        response
            .json::<GitHubReviewComment>()
            .await
            .map_err(map_transport_error)
            .map(Into::into)
    }
}

/// 解析 GitHub files API 的 `patch` 字段（裸 hunk，没有 `diff --git` 文件头）。
///
/// 与 `git-engine` 的统一补丁解析**同规则、不共享代码**：架构规则 infra 只能
/// 依赖 domain（provider 不能依赖 git-engine），且那边面向"全量 git diff 按
/// 文件切段"，这里是单文件的裸 hunk。同样按 hunk 头声明的行数**精确消费**——
/// 内容行自己可能以 `@@`/`---` 开头，前缀不可信；畸形 hunk 头被跳过（它的行号
/// 无法校验，锚定它的评论会在本地被拒，而不是发出去被 GitHub 拒）。
fn parse_patch_hunks(patch: &str) -> Vec<PullDiffHunk> {
    let lines: Vec<&str> = patch.split('\n').collect();
    let mut hunks = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let Some((old_start, old_lines, new_start, new_lines, header)) =
            parse_hunk_header(lines[index])
        else {
            index += 1;
            continue;
        };
        index += 1;
        let mut hunk = PullDiffHunk {
            old_start,
            old_lines,
            new_start,
            new_lines,
            header,
            lines: Vec::new(),
        };
        let mut taken_old = 0_u32;
        let mut taken_new = 0_u32;
        while index < lines.len() && (taken_old < old_lines || taken_new < new_lines) {
            let body = lines[index];
            let Some((kind, content)) = split_body_line(body) else {
                break;
            };
            index += 1;
            let (old_no, new_no) = match kind {
                BodyKind::Context => {
                    taken_old += 1;
                    taken_new += 1;
                    (
                        Some(old_start + taken_old - 1),
                        Some(new_start + taken_new - 1),
                    )
                }
                BodyKind::Added => {
                    taken_new += 1;
                    (None, Some(new_start + taken_new - 1))
                }
                BodyKind::Removed => {
                    taken_old += 1;
                    (Some(old_start + taken_old - 1), None)
                }
                BodyKind::NoNewline => (None, None),
            };
            hunk.lines.push(PullDiffLine {
                kind: kind.as_str(),
                content,
                old_no,
                new_no,
            });
        }
        // 结尾的 `\ No newline` 标记不占头部的计数，紧贴本 hunk 内容行之后
        //（下一行要么是 @@ 要么到结尾），贪婪消费安全。
        while index < lines.len() && lines[index].starts_with('\\') {
            hunk.lines.push(PullDiffLine {
                kind: BodyKind::NoNewline.as_str(),
                content: lines[index][1..].to_owned(),
                old_no: None,
                new_no: None,
            });
            index += 1;
        }
        hunks.push(hunk);
    }
    hunks
}

/// hunk 体一行的类别（解析期内部表示）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BodyKind {
    Context,
    Added,
    Removed,
    NoNewline,
}

impl BodyKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Context => "context",
            Self::Added => "added",
            Self::Removed => "removed",
            Self::NoNewline => "noNewline",
        }
    }
}

/// 拆出 hunk 体行的类别与内容（前缀字符是 ASCII，按字节切安全）。
fn split_body_line(body: &str) -> Option<(BodyKind, String)> {
    let (kind, content) = match body.as_bytes().first()? {
        b' ' => (BodyKind::Context, &body[1..]),
        b'+' => (BodyKind::Added, &body[1..]),
        b'-' => (BodyKind::Removed, &body[1..]),
        b'\\' => (BodyKind::NoNewline, &body[1..]),
        _ => return None,
    };
    Some((kind, content.to_owned()))
}

/// 解析 `@@ -a[,b] +c[,d] @@ header`；计数为一时省略（`@@ -1 +1 @@`）。
fn parse_hunk_header(line: &str) -> Option<(u32, u32, u32, u32, String)> {
    let rest = line.strip_prefix("@@ ")?;
    let (range_part, header) = rest.split_once(" @@")?;
    let (old_part, new_part) = range_part.split_once(' ')?;
    let (old_start, old_lines) = parse_hunk_range(old_part.strip_prefix('-')?)?;
    let (new_start, new_lines) = parse_hunk_range(new_part.strip_prefix('+')?)?;
    Some((
        old_start,
        old_lines,
        new_start,
        new_lines,
        header.trim_start().to_owned(),
    ))
}

/// 解析 `a` 或 `a,b`；b 缺省为 1。
fn parse_hunk_range(range: &str) -> Option<(u32, u32)> {
    match range.split_once(',') {
        Some((start, count)) => Some((start.parse().ok()?, count.parse().ok()?)),
        None => Some((range.parse().ok()?, 1)),
    }
}

/// 行号是否落在一个 hunk 的指定侧（old 侧按旧行号轴，new 侧按新行号轴）。
fn hunk_contains(hunk: &PullDiffHunk, side: CommentSide, line: u32) -> bool {
    let (start, count) = match side {
        CommentSide::Left => (hunk.old_start, hunk.old_lines),
        CommentSide::Right => (hunk.new_start, hunk.new_lines),
    };
    count > 0 && line >= start && line < start.saturating_add(count)
}

/// 校验锚点落在此文件的 diff 上；不满足一律 `VALIDATION` + `line-out-of-range`。
///
/// 多行评论（`start_line`）要求两端在同一 hunk 的各自侧上——GitHub 对
/// 跨 hunk 的范围直接 422，这里提前拦截给出同样的错误码。
fn validate_anchor(hunks: &[PullDiffHunk], anchor: &ReviewCommentAnchor) -> Result<(), AppError> {
    if anchor.line == 0 {
        return Err(line_not_in_diff(&anchor.path, anchor.side, anchor.line));
    }
    if anchor
        .start_line
        .is_some_and(|start| start == 0 || start > anchor.line)
    {
        return Err(AppError::new(
            ErrorCode::Validation,
            format!(
                "comment start_line must be positive and not exceed line {}",
                anchor.line
            ),
        )
        .with_hint("line-out-of-range"));
    }
    let start_side = anchor.start_side.unwrap_or(anchor.side);
    let line_hunk = hunks
        .iter()
        .position(|hunk| hunk_contains(hunk, anchor.side, anchor.line));
    let start_hunk = anchor.start_line.and_then(|start| {
        hunks
            .iter()
            .position(|hunk| hunk_contains(hunk, start_side, start))
    });
    match (line_hunk, start_hunk) {
        (None, _) => Err(line_not_in_diff(&anchor.path, anchor.side, anchor.line)),
        (Some(_), None) if anchor.start_line.is_some() => Err(line_not_in_diff(
            &anchor.path,
            start_side,
            anchor.start_line.unwrap_or_default(),
        )),
        // start_line 与 line 各落在不同 hunk：范围跨了 diff 段
        (Some(line_hunk), Some(start_hunk)) if line_hunk != start_hunk => Err(AppError::new(
            ErrorCode::Validation,
            format!(
                "comment range {}..{} spans multiple diff hunks of `{}`",
                anchor.start_line.unwrap_or_default(),
                anchor.line,
                anchor.path
            ),
        )
        .with_hint("line-out-of-range")),
        _ => Ok(()),
    }
}

fn line_not_in_diff(path: &str, side: CommentSide, line: u32) -> AppError {
    AppError::new(
        ErrorCode::Validation,
        format!(
            "line {line} ({}) is not part of the diff of `{path}`",
            side.display()
        ),
    )
    .with_hint("line-out-of-range")
}

fn path_not_in_diff(path: &str) -> AppError {
    AppError::new(
        ErrorCode::Validation,
        format!("path `{path}` is not part of this pull request diff"),
    )
    .with_hint("path-not-in-diff")
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

    #[tokio::test]
    async fn comments_round_trip_through_the_issue_endpoint() {
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
                { "id": 6, "user": {"login": "octocat"}, "body": "pong", "created_at": "2026-10-01T01:00:00Z" }
            )))
            .mount(&server)
            .await;

        let provider = provider_at(&server);
        let comments = provider
            .pulls()
            .list_comments(token(), "octocat", "x", 1)
            .await
            .unwrap();
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].author, "hubot");

        let created = provider
            .pulls()
            .create_comment(token(), "octocat", "x", 1, "  pong  ")
            .await
            .unwrap();
        assert_eq!(created.body, "pong");
    }

    #[tokio::test]
    async fn an_empty_comment_or_comment_only_review_is_rejected_locally() {
        let server = MockServer::start().await;
        let provider = provider_at(&server);

        let error = provider
            .pulls()
            .create_comment(token(), "octocat", "x", 1, "   ")
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Validation);

        let error = provider
            .pulls()
            .submit_review(
                token(),
                "octocat",
                "x",
                1,
                super::ReviewEvent::Comment,
                Some("  "),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Validation);
    }

    #[tokio::test]
    async fn review_submission_carries_the_event_and_optional_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/repos/octocat/x/pulls/1/reviews"))
            .respond_with(ResponseTemplate::new(200))
            .expect(2)
            .mount(&server)
            .await;

        let provider = provider_at(&server);
        provider
            .pulls()
            .submit_review(
                token(),
                "octocat",
                "x",
                1,
                super::ReviewEvent::Approve,
                Some("ship it"),
            )
            .await
            .unwrap();
        provider
            .pulls()
            .submit_review(
                token(),
                "octocat",
                "x",
                1,
                super::ReviewEvent::RequestChanges,
                None,
            )
            .await
            .unwrap();

        server.verify().await;
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

    // ---- 行内（diff 锚定）评论（T4.7 收尾）----

    const SAMPLE_PATCH: &str = "@@ -1,2 +1,3 @@\n context\n-removed\n+added one\n+added two\n@@ -10,1 +12,2 @@\n ctx\n+new at end";

    #[test]
    fn patch_hunk_parsing_handles_github_bare_fragments() {
        let hunks = super::parse_patch_hunks(SAMPLE_PATCH);
        assert_eq!(hunks.len(), 2, "两个 hunk 都要解析出来");

        assert_eq!(hunks[0].old_start, 1);
        assert_eq!(hunks[0].old_lines, 2);
        assert_eq!(hunks[0].new_start, 1);
        assert_eq!(hunks[0].new_lines, 3);
        let first = &hunks[0].lines;
        assert_eq!(first.len(), 4);
        assert_eq!(
            (first[0].kind, first[0].old_no, first[0].new_no),
            ("context", Some(1), Some(1))
        );
        assert_eq!(
            (first[1].kind, first[1].old_no, first[1].new_no),
            ("removed", Some(2), None)
        );
        assert_eq!(
            (first[2].kind, first[2].old_no, first[2].new_no),
            ("added", None, Some(2))
        );
        assert_eq!(
            (first[3].kind, first[3].old_no, first[3].new_no),
            ("added", None, Some(3))
        );

        assert_eq!((hunks[1].old_start, hunks[1].old_lines), (10, 1));
        assert_eq!((hunks[1].new_start, hunks[1].new_lines), (12, 2));
        assert_eq!(hunks[1].lines[0].old_no, Some(10));
        assert_eq!(hunks[1].lines[0].new_no, Some(12));
        assert_eq!(hunks[1].lines[1].new_no, Some(13));
    }

    #[test]
    fn patch_hunk_parsing_omitted_counts_and_no_newline_markers() {
        // 计数为一时 git 省略（`@@ -1 +1 @@`）；标记行不占计数、紧跟内容行
        let patch = "@@ -1 +1 @@\n-old tail\n\\ No newline at end of file\n+new tail";
        let hunks = super::parse_patch_hunks(patch);
        assert_eq!(hunks.len(), 1);
        assert_eq!((hunks[0].old_start, hunks[0].old_lines), (1, 1));
        assert_eq!((hunks[0].new_start, hunks[0].new_lines), (1, 1));
        assert_eq!(hunks[0].lines.len(), 3);
        assert_eq!(hunks[0].lines[0].kind, "removed");
        assert_eq!(hunks[0].lines[0].old_no, Some(1));
        assert_eq!(hunks[0].lines[1].kind, "noNewline");
        assert_eq!(hunks[0].lines[2].kind, "added");
        assert_eq!(hunks[0].lines[2].new_no, Some(1));
    }

    fn file_json(filename: &str, patch: Option<&str>) -> serde_json::Value {
        let mut file = serde_json::json!({
            "filename": filename, "status": "modified",
            "additions": 3, "deletions": 1, "changes": 4,
            "some_future_field": true
        });
        if let Some(patch) = patch {
            file["patch"] = serde_json::Value::from(patch);
        } else {
            // 二进制文件：GitHub 给 "binary": true 且无 patch
            file["binary"] = serde_json::Value::from(true);
        }
        file
    }

    #[tokio::test]
    async fn files_list_maps_hunks_and_pagination_cursor() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/octocat/x/pulls/1/files"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Link", format!("<{}/x?page=2>; rel=\"next\"", server.uri()))
                    .set_body_json(vec![
                        file_json("src/a.rs", Some(SAMPLE_PATCH)),
                        file_json("assets/logo.png", None),
                    ]),
            )
            .mount(&server)
            .await;

        let page = provider_at(&server)
            .pulls()
            .list_files(token(), "octocat", "x", 1, None, None)
            .await
            .unwrap();

        assert_eq!(page.next_page, Some(2));
        assert_eq!(page.items.len(), 2);
        let text = &page.items[0];
        assert_eq!(text.filename, "src/a.rs");
        assert_eq!(text.hunks.len(), 2, "patch 被解析成 hunk 供前端直接渲染");
        assert_eq!(text.patch.as_deref(), Some(SAMPLE_PATCH));
        let binary = &page.items[1];
        assert_eq!(binary.hunks.len(), 0, "没有 patch 就没有 hunk");
        assert_eq!(binary.patch, None);
    }

    #[tokio::test]
    async fn inline_comment_anchors_to_the_current_head_after_local_validation() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/octocat/x/pulls/1/files"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(vec![file_json("src/a.rs", Some(SAMPLE_PATCH))]),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/repos/octocat/x/pulls/1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(pull_json(1, "Add feature")))
            .mount(&server)
            .await;
        // POST 请求体用 partial-json 断言：锚点与 head 预检必须原样带上
        Mock::given(method("POST"))
            .and(path("/repos/octocat/x/pulls/1/comments"))
            .and(body_partial_json(serde_json::json!({
                "path": "src/a.rs", "side": "RIGHT", "line": 2,
                "commit_id": "abc123"
            })))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!(
                { "id": 77, "user": {"login": "octocat"}, "body": "好问题",
                  "path": "src/a.rs", "side": "RIGHT", "line": 2,
                  "created_at": "2026-10-01T00:00:00Z" }
            )))
            .expect(1)
            .mount(&server)
            .await;

        let created = provider_at(&server)
            .pulls()
            .create_review_comment(
                token(),
                "octocat",
                "x",
                1,
                super::ReviewCommentAnchor {
                    path: "src/a.rs".to_owned(),
                    side: super::CommentSide::Right,
                    line: 2,
                    start_line: None,
                    start_side: None,
                },
                "  好问题  ",
            )
            .await
            .unwrap();

        assert_eq!(created.id, 77);
        assert_eq!(created.path.as_deref(), Some("src/a.rs"));
        assert_eq!(created.line, Some(2));
        server.verify().await;
    }

    #[tokio::test]
    async fn inline_comment_with_out_of_range_line_is_rejected_before_any_write() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/octocat/x/pulls/1/files"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(vec![file_json("src/a.rs", Some(SAMPLE_PATCH))]),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/repos/octocat/x/pulls/1/comments"))
            .respond_with(ResponseTemplate::new(201))
            .expect(0)
            .mount(&server)
            .await;

        let provider = provider_at(&server);
        let anchor = |line: u32| super::ReviewCommentAnchor {
            path: "src/a.rs".to_owned(),
            side: super::CommentSide::Right,
            line,
            start_line: None,
            start_side: None,
        };

        let error = provider
            .pulls()
            .create_review_comment(token(), "octocat", "x", 1, anchor(99), "ping")
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Validation);
        assert_eq!(error.hint.as_deref(), Some("line-out-of-range"));
        assert!(error.message.contains("src/a.rs"));

        // 行号 0 不在任何 hunk 的轴上（行号从 1 起）
        let error = provider
            .pulls()
            .create_review_comment(token(), "octocat", "x", 1, anchor(0), "ping")
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Validation);

        server.verify().await;
    }

    #[tokio::test]
    async fn inline_comment_rejects_paths_that_are_not_in_the_diff() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/octocat/x/pulls/1/files"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(vec![file_json("src/a.rs", Some(SAMPLE_PATCH))]),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(201))
            .expect(0)
            .mount(&server)
            .await;

        let error = provider_at(&server)
            .pulls()
            .create_review_comment(
                token(),
                "octocat",
                "x",
                1,
                super::ReviewCommentAnchor {
                    path: "nope.rs".to_owned(),
                    side: super::CommentSide::Right,
                    line: 1,
                    start_line: None,
                    start_side: None,
                },
                "ping",
            )
            .await
            .unwrap_err();

        assert_eq!(error.code, ErrorCode::Validation);
        assert_eq!(error.hint.as_deref(), Some("path-not-in-diff"));
        server.verify().await;
    }

    #[tokio::test]
    async fn multi_line_inline_comment_must_stay_in_one_hunk() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/octocat/x/pulls/1/files"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(vec![file_json("src/a.rs", Some(SAMPLE_PATCH))]),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/repos/octocat/x/pulls/1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(pull_json(1, "Add feature")))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/repos/octocat/x/pulls/1/comments"))
            .and(body_partial_json(serde_json::json!({
                "path": "src/a.rs", "side": "RIGHT", "line": 3, "start_line": 2
            })))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!(
                { "id": 1, "body": "ok", "path": "src/a.rs", "side": "RIGHT", "line": 3 }
            )))
            .expect(1)
            .mount(&server)
            .await;

        let provider = provider_at(&server);
        // 跨 hunk：起始行在 hunk 1（新文件行 2），末行在 hunk 2（新文件行 13）
        let anchor = |line: u32, start: Option<u32>| super::ReviewCommentAnchor {
            path: "src/a.rs".to_owned(),
            side: super::CommentSide::Right,
            line,
            start_line: start,
            start_side: None,
        };
        let error = provider
            .pulls()
            .create_review_comment(token(), "octocat", "x", 1, anchor(13, Some(2)), "span")
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Validation);
        assert_eq!(error.hint.as_deref(), Some("line-out-of-range"));

        // 同一 hunk 内的范围：放行
        provider
            .pulls()
            .create_review_comment(token(), "octocat", "x", 1, anchor(3, Some(2)), "range")
            .await
            .unwrap();
        server.verify().await;
    }

    #[tokio::test]
    async fn inline_comment_without_patch_is_forwarded_without_local_validation() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/octocat/x/pulls/1/files"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(vec![file_json("big.bin", None)]),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/repos/octocat/x/pulls/1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(pull_json(1, "Add feature")))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!(
                { "id": 2, "body": "x" }
            )))
            .expect(1)
            .mount(&server)
            .await;

        let created = provider_at(&server)
            .pulls()
            .create_review_comment(
                token(),
                "octocat",
                "x",
                1,
                super::ReviewCommentAnchor {
                    path: "big.bin".to_owned(),
                    side: super::CommentSide::Right,
                    line: 1,
                    start_line: None,
                    start_side: None,
                },
                "x",
            )
            .await
            .unwrap();
        assert_eq!(created.id, 2);
        server.verify().await;
    }

    #[tokio::test]
    async fn review_comments_list_maps_anchors_and_replies() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/octocat/x/pulls/1/comments"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                { "id": 30, "user": {"login": "hubot"}, "body": "这里有问题",
                  "path": "src/a.rs", "side": "RIGHT", "line": 2,
                  "created_at": "2026-10-01T00:00:00Z" },
                { "id": 31, "user": {"login": "octocat"}, "body": "同意", "in_reply_to": 30 },
                { "id": 32, "body": "范围", "path": "src/a.rs", "side": "RIGHT",
                  "line": 3, "start_line": 2, "start_side": "RIGHT" }
            ])))
            .mount(&server)
            .await;

        let comments = provider_at(&server)
            .pulls()
            .list_review_comments(token(), "octocat", "x", 1)
            .await
            .unwrap();

        assert_eq!(comments.len(), 3);
        assert_eq!(comments[0].path.as_deref(), Some("src/a.rs"));
        assert_eq!(comments[0].side.as_deref(), Some("RIGHT"));
        assert_eq!(comments[0].line, Some(2));
        assert_eq!(comments[1].in_reply_to, Some(30), "回复沿被回复评论定位");
        assert_eq!(comments[2].start_line, Some(2));
    }

    #[tokio::test]
    async fn reply_review_comment_sends_only_body_and_in_reply_to() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/repos/octocat/x/pulls/1/comments"))
            .and(body_partial_json(
                serde_json::json!({ "in_reply_to": 30, "body": "同意" }),
            ))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!(
                { "id": 31, "body": "同意", "in_reply_to": 30 }
            )))
            .expect(1)
            .mount(&server)
            .await;

        let created = provider_at(&server)
            .pulls()
            .reply_review_comment(token(), "octocat", "x", 1, 30, "  同意  ")
            .await
            .unwrap();
        assert_eq!(created.in_reply_to, Some(30));
        server.verify().await;
    }

    #[tokio::test]
    async fn empty_body_or_zero_comment_id_is_rejected_locally_for_inline_comments() {
        let server = MockServer::start().await;
        let provider = provider_at(&server);
        let anchor = super::ReviewCommentAnchor {
            path: "a.rs".to_owned(),
            side: super::CommentSide::Right,
            line: 1,
            start_line: None,
            start_side: None,
        };

        let error = provider
            .pulls()
            .create_review_comment(token(), "octocat", "x", 1, anchor, "   ")
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Validation);

        let error = provider
            .pulls()
            .reply_review_comment(token(), "octocat", "x", 1, 0, "x")
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Validation);

        let error = provider
            .pulls()
            .reply_review_comment(token(), "octocat", "x", 1, 30, "  ")
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Validation);
    }
}
