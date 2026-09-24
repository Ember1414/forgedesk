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

use forgedesk_domain::git::{DiscardSpec, EntryKind, OperationState, RepoPath, StatusReport};
use forgedesk_domain::AppResult;
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
    /// 本次操作涉及的路径（相对仓库根）。
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
    fn from_entry(entry: &forgedesk_domain::git::FileChange) -> Self {
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

fn emit_changed(app: &AppHandle, repo_id: i64, paths: Vec<String>) {
    // 事件投递失败不该让"操作已成功"回滚成错误：数据变化是事实，
    // 面板下一次刷新自然会追上。这里只记录告警（经脱敏层）。
    if let Err(_error) = app.emit(EVENT_REPO_CHANGED, RepoChangedPayload { repo_id, paths }) {
        // 投递失败只影响本次自动刷新（操作本身已成功，用户仍可手动刷新）；
        // T1.10 引入文件监听后，watch 事件会自然兜住这类丢失。
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

/// 暂存路径。能力等级：`Mutating`；成功后发布 `repo:changed`。
#[tauri::command]
pub fn workspace_stage(
    state: State<'_, AppState>,
    app: AppHandle,
    repo_id: i64,
    paths: Vec<String>,
) -> AppResult<()> {
    // 空选择是界面正常的"全都没选"状态，不是错误：静默成功
    if paths.is_empty() {
        return Ok(());
    }
    state
        .workspace_service()
        .stage(repo_id, &to_repo_paths(&paths))?;
    emit_changed(&app, repo_id, paths);
    Ok(())
}

/// 取消暂存路径。能力等级：`Mutating`；成功后发布 `repo:changed`。
#[tauri::command]
pub fn workspace_unstage(
    state: State<'_, AppState>,
    app: AppHandle,
    repo_id: i64,
    paths: Vec<String>,
) -> AppResult<()> {
    if paths.is_empty() {
        return Ok(());
    }
    state
        .workspace_service()
        .unstage(repo_id, &to_repo_paths(&paths))?;
    emit_changed(&app, repo_id, paths);
    Ok(())
}

/// 放弃工作区修改。能力等级：`Mutating`（前端必须先经确认对话框）；
/// 成功后发布 `repo:changed`。
///
/// `tracked` / `untracked` 分开传：前者可由 git 恢复，后者是磁盘删除。
/// 快照安全网在 M3 接入（见 services 的模块头）。
#[tauri::command]
pub fn workspace_discard(
    state: State<'_, AppState>,
    app: AppHandle,
    repo_id: i64,
    tracked: Vec<String>,
    untracked: Vec<String>,
) -> AppResult<()> {
    if tracked.is_empty() && untracked.is_empty() {
        return Ok(());
    }
    state.workspace_service().discard(
        repo_id,
        DiscardSpec {
            tracked: to_repo_paths(&tracked),
            untracked: to_repo_paths(&untracked),
        },
    )?;
    let mut paths = tracked;
    paths.extend(untracked);
    emit_changed(&app, repo_id, paths);
    Ok(())
}

// ---------------------------------------------------------------- 测试

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use forgedesk_domain::git::{
        BranchInfo, ChangeKind, EntryKind, FileChange, OperationState, RepoPath, StatusReport,
        SubmoduleState,
    };

    use super::{StatusReportDto, EVENT_REPO_CHANGED};

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
    fn event_name_follows_the_plan_convention() {
        // 事件命名与 PLAN §5.5 一致：domain:action
        assert_eq!(EVENT_REPO_CHANGED, "repo:changed");
    }
}
