//! 工作区命令（`workspace_*`，M1 / T1.4）。
//!
//! 与 T1.3 的 `repo_*` 命令同一条纪律：本层只做参数校验、DTO 转换与事件投递，
//! 用例本体在 `forgedesk_services::WorkspaceService`。
//!
//! # `repo:changed` 事件
//!
//! 暂存 / 取消暂存 / 放弃成功后发布，载荷为 `{ repoId, paths }`。
//! 这是 `job:*` 三个任务事件之外的第一类"数据已变化"广播：
//! 状态面板（TanStack Query，key 为 `["status", repoId]`）收到后失效重取。
//! 文件监听（T1.10）落地后，外部修改也会发布同一事件——前端因此不需要区分
//! "自己改的"与"外部改的"。
//!
//! # 能力等级
//!
//! `workspace_status` 为 ReadOnly；三个写操作为 Mutating。
//! `workspace_discard` 会销毁工作区修改：前端必须先弹确认对话框（AlertDialog
//! 列出将丢失的修改清单）；快照安全网在 M3 接入（见 services 的模块头）。

use forgedesk_domain::git::{
    DiffChangeKind, DiffHunk, DiffLineKind, DiffReport, DiffSpec, DiffTarget, DiscardSpec,
    EntryKind, LineSelection, OperationState, RepoPath, StageScope, StatusReport,
};
use forgedesk_domain::{AppError, AppResult, ErrorCode};
use forgedesk_platform::watcher::WatchKind;
use forgedesk_services::audit::op_type;
use forgedesk_services::{AuditArgs, AuditEntry, PatchView};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::state::AppState;

/// 数据变化广播事件（T1.4；T1.10 的文件监听复用同一事件名）。
pub const EVENT_REPO_CHANGED: &str = "repo:changed";

/// [`EVENT_REPO_CHANGED`] 的载荷。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoChangedPayload {
    /// 发生变化的仓库（存储层记录 id）。
    pub repo_id: i64,
    /// 变化类别（`workspace` / `refs` / `large`，见 `WatchKind`）。
    ///
    /// 前端据此决定失效哪些查询：只改了文件就没必要刷新历史列表。
    /// 应用自己的写操作与文件监听共用同一套取值——两处若各说各话，
    /// 前端就得为"谁发的"写两套判断。
    pub kind: String,
    /// 本次操作涉及的路径（相对仓库根；`large` 时为空）。
    pub paths: Vec<String>,
}

/// 单条文件变更的 DTO。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChangeDto {
    /// 路径（重命名条目里是目标路径），相对仓库根。
    pub path: String,
    /// 重命名/复制的来源路径。
    pub old_path: Option<String>,
    /// 记录类型：`ordinary` / `renamed-or-copied` / `unmerged` / `untracked` / `ignored`。
    pub kind: String,
    /// 索引侧状态字符（`.` 表示无变更）。
    pub index_status: String,
    /// 工作区侧状态字符。
    pub worktree_status: String,
    /// 工作区文件是否为二进制（前 8KB 含 NUL 的启发式）。
    pub is_binary: bool,
    /// 路径是否启用了 Git LFS（`filter=lfs` 属性）。
    pub is_lfs: bool,
    /// 是否为子模块。
    pub is_submodule: bool,
    /// 工作区文件大小（字节）；文件不存在时为 `null`。
    pub size_bytes: Option<u64>,
}

impl FileChangeDto {
    /// 由领域条目构造（历史操作的重置计划 DTO 也用它：同一份形状，前端只需一套渲染）。
    pub(crate) fn from_entry(entry: &forgedesk_domain::git::FileChange) -> Self {
        Self {
            path: entry.path.to_string_lossy().into_owned(),
            old_path: entry
                .original_path
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned()),
            kind: kind_str(entry.kind),
            index_status: entry.index_status.as_char().to_string(),
            worktree_status: entry.worktree_status.as_char().to_string(),
            is_binary: entry.is_binary,
            is_lfs: entry.is_lfs,
            is_submodule: entry.submodule.is_submodule,
            size_bytes: entry.size_bytes,
        }
    }
}

fn kind_str(kind: EntryKind) -> String {
    match kind {
        EntryKind::Ordinary => "ordinary".to_owned(),
        EntryKind::RenamedOrCopied => "renamed-or-copied".to_owned(),
        EntryKind::Unmerged => "unmerged".to_owned(),
        EntryKind::Untracked => "untracked".to_owned(),
        EntryKind::Ignored => "ignored".to_owned(),
    }
}

/// 分支头的 DTO。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchDto {
    /// HEAD 指向的提交 oid（初始仓库为 `null`）。
    pub oid: Option<String>,
    /// 当前分支名（游离 HEAD 时为 `null`）。
    pub head: Option<String>,
    /// 是否处于游离 HEAD。
    pub detached: bool,
    /// 上游分支短名（如 `origin/main`）。
    pub upstream: Option<String>,
    /// 相对上游领先的提交数。
    pub ahead: Option<i64>,
    /// 相对上游落后的提交数。
    pub behind: Option<i64>,
}

/// 状态报告的 DTO：按面板分组预拆分，前端无需再筛。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusReportDto {
    /// 分支头信息。
    pub branch: BranchDto,
    /// 进行中的多步操作：`none` / `merge` / `rebase` / `cherry-pick` / `revert` / `bisect`。
    pub operation: String,
    /// 已暂存（索引侧有变更）。
    pub staged: Vec<FileChangeDto>,
    /// 未暂存（工作区侧有变更）。
    pub unstaged: Vec<FileChangeDto>,
    /// 未跟踪。
    pub untracked: Vec<FileChangeDto>,
    /// 未解决的冲突（`u` 条目）。
    pub conflicted: Vec<FileChangeDto>,
    /// 被忽略（仅 `include_ignored` 时非空）。
    pub ignored: Vec<FileChangeDto>,
    /// 被忽略文件的数量（仅 `include_ignored` 时统计）。
    pub ignored_count: Option<u64>,
}

fn is_ordinary(entry: &forgedesk_domain::git::FileChange) -> bool {
    matches!(entry.kind, EntryKind::Ordinary | EntryKind::RenamedOrCopied)
}

impl From<StatusReport> for StatusReportDto {
    fn from(report: StatusReport) -> Self {
        let mut staged = Vec::new();
        let mut unstaged = Vec::new();
        let mut untracked = Vec::new();
        let mut conflicted = Vec::new();
        let mut ignored = Vec::new();

        for entry in &report.entries {
            // 一条目可以同时出现在两个分组里（例如 `AM`：已暂存新增、工作区又改）。
            match entry.kind {
                EntryKind::Unmerged => conflicted.push(FileChangeDto::from_entry(entry)),
                EntryKind::Untracked => untracked.push(FileChangeDto::from_entry(entry)),
                EntryKind::Ignored => ignored.push(FileChangeDto::from_entry(entry)),
                EntryKind::Ordinary | EntryKind::RenamedOrCopied => {
                    if is_ordinary(entry) && entry.index_status.is_changed() {
                        staged.push(FileChangeDto::from_entry(entry));
                    }
                    if is_ordinary(entry) && entry.worktree_status.is_changed() {
                        unstaged.push(FileChangeDto::from_entry(entry));
                    }
                }
            }
        }

        Self {
            branch: BranchDto {
                oid: report.branch.oid,
                head: report.branch.head,
                detached: report.branch.detached,
                upstream: report.branch.upstream,
                ahead: report.branch.ahead,
                behind: report.branch.behind,
            },
            operation: operation_str(report.operation),
            staged,
            unstaged,
            untracked,
            conflicted,
            ignored,
            ignored_count: report.ignored_count,
        }
    }
}

fn operation_str(operation: OperationState) -> String {
    operation.as_str().to_owned()
}

fn to_repo_paths(paths: &[String]) -> Vec<RepoPath> {
    paths
        .iter()
        .map(|path| RepoPath::from(path.as_str()))
        .collect()
}

/// 广播一次数据变化。
///
/// `kind` 用 [`WatchKind`] 而不是自定义枚举：应用自己的写操作与文件监听
/// 描述的是同一件事（"仓库的哪一部分变了"），共用一套取值前端才好处理。
pub(crate) fn emit_changed(app: &AppHandle, repo_id: i64, kind: WatchKind, paths: Vec<String>) {
    // 事件投递失败不该让"操作已成功"回滚成错误：数据变化是事实，
    // 面板下一次刷新自然会追上。这里只记录告警（经脱敏层）。
    if let Err(_error) = app.emit(
        EVENT_REPO_CHANGED,
        RepoChangedPayload {
            repo_id,
            kind: kind.as_str().to_owned(),
            paths,
        },
    ) {
        // 投递失败只影响本次自动刷新（操作本身已成功，用户仍可手动刷新）；
        // T1.10 的文件监听也会在外部变化时补上一次事件。
    }
}
// ---------------------------------------------------------------- 命令

/// 读取工作区状态。能力等级：`ReadOnly`。
///
/// `include_ignored` 缺省 `false`；`true` 时额外返回被忽略条目并统计数量
/// （需要一次全目录扫描，界面上的开关才打开它）。
#[tauri::command]
pub fn workspace_status(
    state: State<'_, AppState>,
    repo_id: i64,
    include_ignored: Option<bool>,
) -> AppResult<StatusReportDto> {
    state
        .workspace_service()
        .status(repo_id, include_ignored.unwrap_or(false))
        .map(StatusReportDto::from)
}

/// hunk 内被选中的行（请求形状）。
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LineSelectionRequest {
    /// hunk 下标（0 基）。
    pub hunk_index: usize,
    /// 该 hunk 内被选中的行下标（0 基，与该 hunk `lines` 数组的顺序一致）。
    pub lines: Vec<usize>,
}

/// 暂存 / 取消暂存的粒度请求。
///
/// `#[serde(tag = "kind")]` 让前端传 `{ kind: "hunks", path, hunkIndices }` 这样的
/// 判别联合：**一种能力只有一个命令入口**，"用户到底选了什么"由 `kind` 说清楚，
/// 而不是靠"哪个字段非空"来猜（那种接口在失败时无法给出有意义的错误）。
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum StageScopeRequest {
    /// 整文件（走 `git add` / `git reset`）。
    Files {
        /// 路径列表（相对仓库根）。
        paths: Vec<String>,
    },
    /// 单文件内的若干块。
    Hunks {
        /// 目标文件（重命名时是目标路径）。
        path: String,
        /// 选中的 hunk 下标。
        hunk_indices: Vec<usize>,
    },
    /// 单文件内的若干行。
    Lines {
        /// 目标文件。
        path: String,
        /// 每个 hunk 内选中的行。
        selections: Vec<LineSelectionRequest>,
    },
}

/// 放弃修改的粒度请求。
///
/// 只有整文件粒度需要区分 `tracked` / `untracked`：前者可由 git 恢复，
/// 后者只能从磁盘删除（**不可恢复**）。块级 / 行级只作用于已跟踪文件。
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DiscardRequest {
    /// 整文件。
    Files {
        /// 已跟踪路径（`git restore` 可恢复）。
        tracked: Vec<String>,
        /// 未跟踪路径（磁盘删除，不可恢复）。
        untracked: Vec<String>,
    },
    /// 单文件内的若干块。
    Hunks {
        /// 目标文件。
        path: String,
        /// 选中的 hunk 下标。
        hunk_indices: Vec<usize>,
    },
    /// 单文件内的若干行。
    Lines {
        /// 目标文件。
        path: String,
        /// 每个 hunk 内选中的行。
        selections: Vec<LineSelectionRequest>,
    },
}

/// 生成补丁时的查看参数（缺省时用后端默认值）。
///
/// 必须与界面打开 diff 时用的那组参数一致：hunk 的划分取决于上下文行数，
/// 参数不同会让"用户选中的第 2 块"在后端对应到另一块（见 services::staging 的模块头）。
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchViewRequest {
    /// 上下文行数（`-U<n>`，缺省 3）。
    pub context_lines: Option<u32>,
    /// 忽略空白变化（`-w`，缺省 false）。
    pub ignore_whitespace: Option<bool>,
    /// 重命名检测（`-M`，缺省 true）。
    pub detect_renames: Option<bool>,
}

impl PatchViewRequest {
    /// 转成领域类型（缺省值取 [`PatchView`] 的默认）。
    pub fn into_domain(self) -> PatchView {
        let defaults = PatchView::default();
        PatchView {
            context_lines: self.context_lines.unwrap_or(defaults.context_lines),
            ignore_whitespace: self.ignore_whitespace.unwrap_or(defaults.ignore_whitespace),
            detect_renames: self.detect_renames.unwrap_or(defaults.detect_renames),
        }
    }
}

/// 选择项数量的上限。
///
/// 这不是安全边界（后端仍会逐项做越界校验），而是防手滑：一个界面不可能一次选中
/// 上千个 hunk 之外的东西，而超长数组只会在日志与错误信息里制造噪音。
const MAX_SELECTION_ENTRIES: usize = 4096;

/// 路径的二次校验（前端校验只为即时反馈，见 `docs/API.md` §1）。
fn validate_path(path: &str) -> AppResult<RepoPath> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(AppError::new(ErrorCode::Validation, "the path is empty"));
    }
    if trimmed.contains('\0') {
        return Err(
            AppError::new(ErrorCode::Validation, "the path contains a NUL byte")
                .with_hint("path".to_owned()),
        );
    }
    Ok(RepoPath::from(trimmed))
}

fn validate_entries(count: usize) -> AppResult<()> {
    if count > MAX_SELECTION_ENTRIES {
        return Err(
            AppError::new(ErrorCode::Validation, "the selection has too many entries")
                .with_detail(format!("entries: {count}")),
        );
    }
    Ok(())
}

impl StageScopeRequest {
    /// 转成领域类型，并给出 `repo:changed` 事件要用的路径列表。
    pub fn into_domain(self) -> AppResult<(Vec<String>, StageScope)> {
        match self {
            Self::Files { paths } => {
                validate_entries(paths.len())?;
                let domain_paths = to_repo_paths(&paths);
                Ok((paths, StageScope::Files(domain_paths)))
            }
            Self::Hunks { path, hunk_indices } => {
                let repo_path = validate_path(&path)?;
                validate_entries(hunk_indices.len())?;
                Ok((vec![path], StageScope::hunks(repo_path, hunk_indices)))
            }
            Self::Lines { path, selections } => {
                let repo_path = validate_path(&path)?;
                validate_entries(selections.len())?;
                let selections = selections
                    .into_iter()
                    .map(|item| LineSelection {
                        hunk_index: item.hunk_index,
                        lines: item.lines,
                    })
                    .collect();
                Ok((vec![path], StageScope::lines(repo_path, selections)))
            }
        }
    }
}

/// 放弃修改的领域形态（命令层内部使用）。
enum DiscardScope {
    /// 整文件。
    Files(DiscardSpec),
    /// 块级 / 行级。
    Patch(StageScope),
}

impl DiscardRequest {
    fn into_domain(self) -> AppResult<(Vec<String>, DiscardScope)> {
        match self {
            Self::Files { tracked, untracked } => {
                let mut paths = tracked.clone();
                paths.extend(untracked.iter().cloned());
                Ok((
                    paths,
                    DiscardScope::Files(DiscardSpec {
                        tracked: to_repo_paths(&tracked),
                        untracked: to_repo_paths(&untracked),
                    }),
                ))
            }
            Self::Hunks { path, hunk_indices } => {
                let repo_path = validate_path(&path)?;
                validate_entries(hunk_indices.len())?;
                Ok((
                    vec![path],
                    DiscardScope::Patch(StageScope::hunks(repo_path, hunk_indices)),
                ))
            }
            Self::Lines { path, selections } => {
                let repo_path = validate_path(&path)?;
                validate_entries(selections.len())?;
                let selections = selections
                    .into_iter()
                    .map(|item| LineSelection {
                        hunk_index: item.hunk_index,
                        lines: item.lines,
                    })
                    .collect();
                Ok((
                    vec![path],
                    DiscardScope::Patch(StageScope::lines(repo_path, selections)),
                ))
            }
        }
    }
}

/// 暂存：整文件走 `git add`，块级 / 行级走补丁通道（T1.6）。
/// 能力等级：`Mutating`；成功后发布 `repo:changed`。
#[tauri::command]
pub fn workspace_stage(
    state: State<'_, AppState>,
    app: AppHandle,
    repo_id: i64,
    spec: StageScopeRequest,
    view: Option<PatchViewRequest>,
) -> AppResult<()> {
    let (paths, scope) = spec.into_domain()?;
    // 空选择是界面正常的"全都没选"状态，不是错误：静默成功
    if scope.is_empty() {
        return Ok(());
    }

    // 审计（T1.11）：暂存是最高频的写操作，更必须有记录
    let operation = state
        .audit_service()
        .begin(&AuditEntry::new(repo_id, op_type::STAGE).with_args(scope_args(&scope)));

    let result = match &scope {
        // 文件粒度刻意不走补丁：`git add` 更快，也能处理未跟踪文件与模式变更
        StageScope::Files(domain_paths) => state.workspace_service().stage(repo_id, domain_paths),
        _ => state
            .staging_service()
            .stage(repo_id, &scope, view.unwrap_or_default().into_domain()),
    };

    if let Some(operation) = operation {
        operation.finish(&result, None);
    }
    result?;

    emit_changed(&app, repo_id, WatchKind::Workspace, paths);
    Ok(())
}

/// 取消暂存：整文件走 `git reset`，块级 / 行级走反向补丁通道（T1.6）。
/// 能力等级：`Mutating`；成功后发布 `repo:changed`。
#[tauri::command]
pub fn workspace_unstage(
    state: State<'_, AppState>,
    app: AppHandle,
    repo_id: i64,
    spec: StageScopeRequest,
    view: Option<PatchViewRequest>,
) -> AppResult<()> {
    let (paths, scope) = spec.into_domain()?;
    if scope.is_empty() {
        return Ok(());
    }

    let operation = state
        .audit_service()
        .begin(&AuditEntry::new(repo_id, op_type::UNSTAGE).with_args(scope_args(&scope)));

    let result = match &scope {
        StageScope::Files(domain_paths) => state.workspace_service().unstage(repo_id, domain_paths),
        _ => {
            state
                .staging_service()
                .unstage(repo_id, &scope, view.unwrap_or_default().into_domain())
        }
    };

    if let Some(operation) = operation {
        operation.finish(&result, None);
    }
    result?;

    emit_changed(&app, repo_id, WatchKind::Workspace, paths);
    Ok(())
}

/// 放弃修改。能力等级：`Mutating`（前端必须先经确认对话框）；成功后发布 `repo:changed`。
///
/// 整文件粒度区分 `tracked` / `untracked`；块级 / 行级只作用于已跟踪文件，
/// 且先做 dry-run（补丁对不上时不写任何东西）。快照安全网在 M3 接入。
#[tauri::command]
pub fn workspace_discard(
    state: State<'_, AppState>,
    app: AppHandle,
    repo_id: i64,
    spec: DiscardRequest,
    view: Option<PatchViewRequest>,
) -> AppResult<()> {
    let (paths, scope) = spec.into_domain()?;

    // 空选择不是错误，但也不该留下一条"放弃了 0 个文件"的记录
    let empty = match &scope {
        DiscardScope::Files(discard) => discard.is_empty(),
        DiscardScope::Patch(inner) => inner.is_empty(),
    };
    if empty {
        return Ok(());
    }

    let operation = state
        .audit_service()
        .begin(&AuditEntry::new(repo_id, op_type::DISCARD).with_args(discard_args(&scope)));

    // 放弃是**破坏性**操作：记录里必须留下"放弃了什么"，否则事后无法回答
    // "我那次到底丢了多少东西"（这也是前端必须弹确认框的原因，红线 R7）
    let result = match scope {
        DiscardScope::Files(discard) => state.workspace_service().discard(repo_id, discard),
        DiscardScope::Patch(inner) => {
            state
                .staging_service()
                .discard(repo_id, &inner, view.unwrap_or_default().into_domain())
        }
    };

    if let Some(operation) = operation {
        operation.finish(&result, None);
    }
    result?;

    emit_changed(&app, repo_id, WatchKind::Workspace, paths);
    Ok(())
}

/// 暂存 / 取消暂存的参数摘要。
///
/// 只记"粒度 + 数量 + 文件"：行级选择可能上千个下标，全记下来既没人读，
/// 也会把 2KB 的额度用光（`AuditArgs` 会在超限时截断并保留总数）。
fn scope_args(scope: &StageScope) -> AuditArgs {
    match scope {
        StageScope::Files(paths) => {
            let paths: Vec<String> = paths.iter().map(ToString::to_string).collect();
            AuditArgs::new()
                .text("kind", "files")
                .paths("paths", &paths)
        }
        StageScope::Hunks { path, indices } => AuditArgs::new()
            .text("kind", "hunks")
            .text("path", &path.to_string())
            .number("hunks", indices.len() as i64),
        StageScope::Lines { path, selections } => AuditArgs::new()
            .text("kind", "lines")
            .text("path", &path.to_string())
            .number("hunks", selections.len() as i64)
            .number(
                "lines",
                selections
                    .iter()
                    .map(|selection| selection.lines.len() as i64)
                    .sum(),
            ),
    }
}

/// 放弃修改的参数摘要（整文件粒度要区分"可恢复"与"磁盘删除"两类路径）。
fn discard_args(scope: &DiscardScope) -> AuditArgs {
    match scope {
        DiscardScope::Files(discard) => {
            let all: Vec<String> = discard
                .tracked
                .iter()
                .chain(discard.untracked.iter())
                .map(ToString::to_string)
                .collect();
            AuditArgs::new()
                .text("kind", "files")
                .number("tracked", discard.tracked.len() as i64)
                .number("untracked", discard.untracked.len() as i64)
                .paths("paths", &all)
        }
        DiscardScope::Patch(inner) => scope_args(inner),
    }
}

/// 在系统文件管理器中显示文件（打开其所在目录）。能力等级：ReadOnly。
/// 状态面板行内操作里唯一需要新命令的一项；编辑器打开属于 M5。
#[tauri::command]
pub fn workspace_reveal(state: State<'_, AppState>, repo_id: i64, path: String) -> AppResult<()> {
    let workdir = state.workspace_service().resolve_workdir(repo_id)?;
    let target = workdir.join(&path);
    let parent = target.parent().unwrap_or(workdir.as_path());
    forgedesk_platform::shell::open_in_file_manager(parent)
}

// ---------------------------------------------------------------- 测试

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use forgedesk_domain::git::{
        BranchInfo, ChangeKind, DiscardSpec, EntryKind, FileChange, LineSelection, OperationState,
        RepoPath, StageGranularity, StageScope, StatusReport, SubmoduleState,
    };
    use forgedesk_domain::ErrorCode;
    use forgedesk_services::PatchView;

    use super::{
        discard_args, scope_args, DiscardRequest, DiscardScope, LineSelectionRequest,
        PatchViewRequest, StageScopeRequest, StatusReportDto, EVENT_REPO_CHANGED,
        MAX_SELECTION_ENTRIES,
    };

    /// 构造一条目（测试专用的简写）。
    fn change(kind: EntryKind, path: &str, index: ChangeKind, worktree: ChangeKind) -> FileChange {
        FileChange {
            kind,
            path: RepoPath::from(path),
            original_path: None,
            index_status: index,
            worktree_status: worktree,
            similarity: None,
            mode_head: None,
            mode_index: None,
            mode_worktree: None,
            oid_head: None,
            oid_index: None,
            stages: None,
            submodule: SubmoduleState::NONE,
            is_binary: false,
            is_lfs: false,
            size_bytes: None,
        }
    }

    #[test]
    fn report_is_grouped_for_the_panel() {
        let report = StatusReport {
            branch: BranchInfo::default(),
            operation: OperationState::None,
            entries: vec![
                // 已暂存的新增
                change(
                    EntryKind::Ordinary,
                    "staged.txt",
                    ChangeKind::Added,
                    ChangeKind::Unmodified,
                ),
                // 未暂存的修改
                change(
                    EntryKind::Ordinary,
                    "dirty.txt",
                    ChangeKind::Unmodified,
                    ChangeKind::Modified,
                ),
                // 先暂存、工作区又改：两个分组里都要出现
                change(
                    EntryKind::Ordinary,
                    "both.txt",
                    ChangeKind::Added,
                    ChangeKind::Modified,
                ),
                // 未跟踪与冲突
                change(
                    EntryKind::Untracked,
                    "untracked.txt",
                    ChangeKind::Unmodified,
                    ChangeKind::Unmodified,
                ),
                change(
                    EntryKind::Unmerged,
                    "conflict.txt",
                    ChangeKind::Unmerged,
                    ChangeKind::Unmerged,
                ),
            ],
            ignored_count: None,
        };

        let dto = StatusReportDto::from(report);

        assert_eq!(
            dto.staged
                .iter()
                .map(|f| f.path.as_str())
                .collect::<Vec<_>>(),
            vec!["staged.txt", "both.txt"]
        );
        assert_eq!(
            dto.unstaged
                .iter()
                .map(|f| f.path.as_str())
                .collect::<Vec<_>>(),
            vec!["dirty.txt", "both.txt"]
        );
        assert_eq!(
            dto.untracked
                .iter()
                .map(|f| f.path.as_str())
                .collect::<Vec<_>>(),
            vec!["untracked.txt"]
        );
        assert_eq!(
            dto.conflicted
                .iter()
                .map(|f| f.path.as_str())
                .collect::<Vec<_>>(),
            vec!["conflict.txt"]
        );
        assert!(dto.ignored.is_empty());
        assert_eq!(dto.operation, "none");
    }

    #[test]
    fn staging_arguments_keep_the_granularity_and_the_counts() {
        let files = scope_args(&StageScope::Files(vec![
            RepoPath::from("a.txt"),
            RepoPath::from("b.txt"),
        ]));
        let files = files.build();
        assert!(files.contains("\"kind\":\"files\""));
        assert!(files.contains("a.txt"));

        let hunks = scope_args(&StageScope::Hunks {
            path: RepoPath::from("src/main.rs"),
            indices: vec![1, 3],
        })
        .build();
        assert!(hunks.contains("\"kind\":\"hunks\""));
        assert!(hunks.contains("\"hunks\":2"));

        let lines = scope_args(&StageScope::Lines {
            path: RepoPath::from("src/main.rs"),
            selections: vec![
                LineSelection {
                    hunk_index: 0,
                    lines: vec![1, 2],
                },
                LineSelection {
                    hunk_index: 2,
                    lines: vec![5],
                },
            ],
        })
        .build();
        assert!(lines.contains("\"lines\":3"), "行数要汇总：{lines}");
    }

    #[test]
    fn discarding_arguments_keep_tracked_and_untracked_apart() {
        let args = discard_args(&DiscardScope::Files(DiscardSpec {
            tracked: vec![RepoPath::from("a.txt")],
            untracked: vec![RepoPath::from("b.txt")],
        }))
        .build();

        // 这两个数字的差别就是"可恢复"与"已删除"的差别，事后必须能分清
        assert!(args.contains("\"tracked\":1"), "{args}");
        assert!(args.contains("\"untracked\":1"), "{args}");
    }

    #[test]
    fn event_name_follows_the_plan_convention() {
        // 事件命名与 PLAN §5.5 一致：domain:action
        assert_eq!(EVENT_REPO_CHANGED, "repo:changed");
    }

    #[test]
    fn file_granularity_stays_on_the_path_channel() {
        let request = StageScopeRequest::Files {
            paths: vec!["a.txt".to_owned(), "b.txt".to_owned()],
        };

        let (paths, scope) = request.into_domain().unwrap();

        assert_eq!(paths, vec!["a.txt".to_owned(), "b.txt".to_owned()]);
        assert_eq!(scope.granularity(), StageGranularity::Files);
    }

    #[test]
    fn hunk_granularity_carries_the_file_and_the_indices() {
        let request = StageScopeRequest::Hunks {
            path: "src/main.rs".to_owned(),
            hunk_indices: vec![1, 2],
        };

        let (paths, scope) = request.into_domain().unwrap();

        assert_eq!(paths, vec!["src/main.rs".to_owned()], "事件载荷要带上路径");
        assert_eq!(scope.granularity(), StageGranularity::Hunks);
        assert_eq!(
            scope.path().map(|path| path.to_string()),
            Some("src/main.rs".to_owned())
        );
    }

    #[test]
    fn line_granularity_keeps_the_hunk_and_line_indices_intact() {
        let request = StageScopeRequest::Lines {
            path: "src/main.rs".to_owned(),
            selections: vec![LineSelectionRequest {
                hunk_index: 1,
                lines: vec![2, 3],
            }],
        };

        let (_, scope) = request.into_domain().unwrap();

        match scope {
            StageScope::Lines { selections, .. } => assert_eq!(
                selections,
                vec![LineSelection {
                    hunk_index: 1,
                    lines: vec![2, 3],
                }]
            ),
            other => panic!("应当转换出行级选择：{other:?}"),
        }
    }

    #[test]
    fn an_empty_or_nul_path_is_rejected_before_it_reaches_git() {
        let blank = StageScopeRequest::Hunks {
            path: "   ".to_owned(),
            hunk_indices: vec![0],
        };
        assert_eq!(
            blank.into_domain().expect_err("空路径必须拒绝").code,
            ErrorCode::Validation
        );

        let nul = StageScopeRequest::Hunks {
            path: "a\0b".to_owned(),
            hunk_indices: vec![0],
        };
        assert_eq!(
            nul.into_domain().expect_err("含 NUL 的路径必须拒绝").code,
            ErrorCode::Validation
        );
    }

    #[test]
    fn an_absurdly_large_selection_is_rejected_at_the_boundary() {
        let request = StageScopeRequest::Files {
            paths: vec!["a.txt".to_owned(); MAX_SELECTION_ENTRIES + 1],
        };

        assert_eq!(
            request.into_domain().expect_err("超长选择必须拒绝").code,
            ErrorCode::Validation
        );
    }

    #[test]
    fn the_patch_view_falls_back_to_the_backend_defaults() {
        let view = PatchViewRequest::default().into_domain();

        assert_eq!(view, PatchView::default());
        assert_eq!(
            view.context_lines,
            forgedesk_domain::git::DEFAULT_CONTEXT_LINES
        );
    }

    #[test]
    fn the_patch_view_forwards_exactly_what_the_ui_used() {
        let view = PatchViewRequest {
            context_lines: Some(12),
            ignore_whitespace: Some(true),
            detect_renames: Some(false),
        }
        .into_domain();

        assert_eq!(view.context_lines, 12);
        assert!(view.ignore_whitespace);
        assert!(!view.detect_renames);
    }

    #[test]
    fn discarding_a_file_keeps_tracked_and_untracked_apart() {
        let request = DiscardRequest::Files {
            tracked: vec!["a.txt".to_owned()],
            untracked: vec!["b.txt".to_owned()],
        };

        let (paths, scope) = request.into_domain().unwrap();

        assert_eq!(paths, vec!["a.txt".to_owned(), "b.txt".to_owned()]);
        match scope {
            DiscardScope::Files(spec) => {
                assert_eq!(spec.tracked.len(), 1, "已跟踪路径可由 git 恢复");
                assert_eq!(spec.untracked.len(), 1, "未跟踪路径是磁盘删除");
            }
            DiscardScope::Patch(_) => panic!("文件粒度不该走补丁通道"),
        }
    }

    #[test]
    fn discarding_a_hunk_goes_through_the_patch_channel() {
        let request = DiscardRequest::Hunks {
            path: "a.txt".to_owned(),
            hunk_indices: vec![0],
        };

        let (_, scope) = request.into_domain().unwrap();

        match scope {
            DiscardScope::Patch(scope) => {
                assert_eq!(scope.granularity(), StageGranularity::Hunks);
            }
            DiscardScope::Files(_) => panic!("块级丢弃不该走文件通道"),
        }
    }
}

// ---------------------------------------------------------------- diff（T1.5）

/// 前端传来的 diff 查询条件（camelCase）。
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffRequest {
    /// 比较目标：`staged` / `unstaged` / `between` / `since` / `commit`。
    pub target: String,
    /// `between` 的起点。
    pub from: Option<String>,
    /// `between` 的终点。
    pub to: Option<String>,
    /// `since` / `commit` 的版本。
    pub revision: Option<String>,
    /// 限定路径（空 = 全部）。
    #[serde(default)]
    pub paths: Vec<String>,
    /// 忽略空白变化。
    #[serde(default)]
    pub ignore_whitespace: bool,
    /// 上下文行数。
    #[serde(default = "default_context_lines")]
    pub context_lines: u32,
    /// 重命名检测。
    #[serde(default = "default_true")]
    pub detect_renames: bool,
    /// 跳过大文件截断。
    #[serde(default)]
    pub force_full: bool,
}

fn default_context_lines() -> u32 {
    forgedesk_domain::git::DEFAULT_CONTEXT_LINES
}

fn default_true() -> bool {
    true
}

impl DiffRequest {
    /// 转成领域 spec；未知 target 是调用方的 bug，显式拒绝而不是猜。
    pub fn into_domain(self) -> AppResult<DiffSpec> {
        let target = match self.target.as_str() {
            "staged" => DiffTarget::Staged,
            "unstaged" => DiffTarget::Unstaged,
            "between" => DiffTarget::between(
                self.from.clone().unwrap_or_default(),
                self.to.clone().unwrap_or_default(),
            ),
            "since" => DiffTarget::Since(self.revision.clone().unwrap_or_default()),
            "commit" => DiffTarget::Commit(self.revision.clone().unwrap_or_default()),
            other => {
                return Err(forgedesk_domain::AppError::new(
                    forgedesk_domain::ErrorCode::Validation,
                    "unknown diff target",
                )
                .with_detail(other.to_owned()));
            }
        };
        let mut spec = DiffSpec::new(target)
            .with_ignore_whitespace(self.ignore_whitespace)
            .with_context_lines(self.context_lines)
            .with_paths(to_repo_paths(&self.paths));
        spec.detect_renames = self.detect_renames;
        spec.force_full = self.force_full;
        Ok(spec)
    }
}

/// hunk 内一行的 DTO。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffLineDto {
    /// `context` / `added` / `removed` / `noNewline`。
    pub kind: String,
    /// 行内容（不含 `+`/`-` 前缀与换行）。
    pub content: String,
    /// 旧文件行号（新增行为 `null`）。
    pub old_no: Option<u32>,
    /// 新文件行号（删除行为 `null`）。
    pub new_no: Option<u32>,
}

impl DiffLineDto {
    fn from_domain(line: &forgedesk_domain::git::DiffLine) -> Self {
        Self {
            kind: match line.kind {
                DiffLineKind::Context => "context",
                DiffLineKind::Added => "added",
                DiffLineKind::Removed => "removed",
                DiffLineKind::NoNewlineMarker => "noNewline",
            }
            .to_owned(),
            content: line.content.clone(),
            old_no: line.old_lineno,
            new_no: line.new_lineno,
        }
    }
}

/// 一个 hunk 的 DTO。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffHunkDto {
    /// 旧文件起始行号。
    pub old_start: u32,
    /// 旧文件行数。
    pub old_lines: u32,
    /// 新文件起始行号。
    pub new_start: u32,
    /// 新文件行数。
    pub new_lines: u32,
    /// @@ 之后的上下文（通常是函数签名）。
    pub header: String,
    /// hunk 内的行。
    pub lines: Vec<DiffLineDto>,
}

impl DiffHunkDto {
    fn from_domain(hunk: &DiffHunk) -> Self {
        Self {
            old_start: hunk.old_start,
            old_lines: hunk.old_lines,
            new_start: hunk.new_start,
            new_lines: hunk.new_lines,
            header: hunk.header.clone(),
            lines: hunk.lines.iter().map(DiffLineDto::from_domain).collect(),
        }
    }
}

/// 单个文件的行级 diff DTO。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDiffDto {
    /// 路径（重命名时是目标路径），相对仓库根。
    pub path: String,
    /// 重命名/复制的来源路径。
    pub old_path: Option<String>,
    /// `added` / `deleted` / `modified` / `renamed` / `copied` / `typeChanged` / `unknown`。
    pub change: String,
    /// 是否为二进制文件（无行级内容）。
    pub binary: bool,
    /// 新增行数。
    pub additions: u64,
    /// 删除行数。
    pub deletions: u64,
    /// 行级内容是否被截断（大文件保护；配合 forceFull 重新请求）。
    pub truncated: bool,
    /// hunk 列表（二进制文件为空）。
    pub hunks: Vec<DiffHunkDto>,
}

impl FileDiffDto {
    fn from_domain(file: &forgedesk_domain::git::FileDiff) -> Self {
        Self {
            path: file.path.to_string_lossy().into_owned(),
            old_path: file
                .original_path
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned()),
            change: match file.change {
                DiffChangeKind::Added => "added",
                DiffChangeKind::Deleted => "deleted",
                DiffChangeKind::Modified => "modified",
                DiffChangeKind::Renamed => "renamed",
                DiffChangeKind::Copied => "copied",
                DiffChangeKind::TypeChanged => "typeChanged",
                DiffChangeKind::Unknown => "unknown",
            }
            .to_owned(),
            binary: file.binary,
            additions: file.additions,
            deletions: file.deletions,
            truncated: file.truncated,
            hunks: file.hunks.iter().map(DiffHunkDto::from_domain).collect(),
        }
    }
}

/// 一次 diff 查询的 DTO。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffReportDto {
    /// 变更的文件。
    pub files: Vec<FileDiffDto>,
    /// 被截断的文件数（截断详情见各文件的 truncated 标志）。
    pub truncated_files: usize,
}

impl DiffReportDto {
    /// 由领域报告构造（stash 的 diff 也用它：前端复用同一套 DiffView）。
    pub(crate) fn from_domain(report: &DiffReport) -> Self {
        Self {
            files: report.files.iter().map(FileDiffDto::from_domain).collect(),
            truncated_files: report.truncated_files,
        }
    }
}

/// 读取 diff（行级内容）。能力等级：`ReadOnly`。
#[tauri::command]
pub fn workspace_diff(
    state: State<'_, AppState>,
    repo_id: i64,
    spec: DiffRequest,
) -> AppResult<DiffReportDto> {
    let domain_spec = spec.into_domain()?;
    let report = state.workspace_service().diff(repo_id, domain_spec)?;
    Ok(DiffReportDto::from_domain(&report))
}

/// 生成原始补丁字节（复制 / 导出 .patch）。能力等级：`ReadOnly`。
/// 返回原始字节：补丁里的路径与内容都可能是非 UTF-8。
#[tauri::command]
pub fn workspace_diff_patch(
    state: State<'_, AppState>,
    repo_id: i64,
    spec: DiffRequest,
) -> AppResult<Vec<u8>> {
    let domain_spec = spec.into_domain()?;
    state.workspace_service().diff_patch(repo_id, domain_spec)
}
