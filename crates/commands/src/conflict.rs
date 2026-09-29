//! 冲突状态机命令（T3.1）：状态采集、标记解决、继续、中止、跳过。
//!
//! # 这一层做什么
//!
//! 1. **IPC 形状**：`ConflictState` / `ConflictContinueOutcome` /
//!    `ConflictAbortOutcome` 派生了 `Serialize`（camelCase），与 stash / cherry-pick
//!    的 `MergeOutcome` 同一约定——直接作为 IPC 形状返回，不再复制一份 DTO。
//! 2. **审计**：四个写操作各一条记录（`git_conflict_state` 是只读探测，不记）。
//!    abort 的结果带 `snapshotId`，`record_with` 会把它写进
//!    `operation_records.snapshot_id`（T2.10 修复的关联断言依赖这个字段存在）。
//! 3. **广播变化**：mark_resolved 动了索引（`Workspace`）；continue / abort / skip
//!    动了工作区还可能动了 HEAD（`Workspace` + `Refs`），与 stash 冲突路径同一处理。
//!
//! # 为什么是同步命令（带 `async` 标注）
//!
//! 与 stash / cherry-pick 同族：本地操作不碰网络，最慢的是大冲突的多文件读取
//! （秒级），不值得为它引入任务通道的进度/取消状态。`#[tauri::command(async)]`
//! 让它们在 Tauri 2 的异步运行时执行而不是主线程（T2.9 的教训：无标注的命令
//! 在主线程执行，重查询会冻结整窗）。

use forgedesk_domain::git::{
    ConflictAbortOutcome, ConflictContinueOutcome, ConflictFileDetail, ConflictState, LineEnding,
    RepoPath, TakeSide,
};
use forgedesk_domain::AppResult;
use forgedesk_platform::watcher::WatchKind;
use forgedesk_services::audit::op_type;
use forgedesk_services::{AuditArgs, AuditEntry};
use tauri::{AppHandle, State};

use crate::audit;
use crate::state::AppState;
use crate::workspace::emit_changed;

/// `git_conflict_apply_resolution` 的请求体。
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyResolutionRequest {
    /// 编辑器产出的完整结果文本（LF 换行；EOL 由后端按原文件形状重建）。
    pub content: String,
    /// 工作区文件的换行风格（来自 file_detail 的探测）。
    pub eol: LineEnding,
    /// 工作区文件是否带 UTF-8 BOM（写回时保持）。
    pub bom: bool,
    /// 工作区文件末尾是否有换行（写回时保持）。
    pub trailing_newline: bool,
}

/// 采集冲突状态。只读探测：无操作时返回空态（`opKind: null`），不报错。
#[tauri::command(async)]
pub fn git_conflict_state(state: State<'_, AppState>, repo_id: i64) -> AppResult<ConflictState> {
    let repo_id = crate::history_ops::require_repo(repo_id)?;
    state.conflict_service().state(repo_id)
}

/// 标记文件已解决（`git add` + 校验 stage 清空）。写操作：审计。
///
/// 路径来自 [`git_conflict_state`] 报告的文件清单；仍冲突的路径会得到
/// `CONFLICT_UNRESOLVED`（`hint` 列出），不会静默成功。
#[tauri::command(async)]
pub fn git_conflict_mark_resolved(
    state: State<'_, AppState>,
    app: AppHandle,
    repo_id: i64,
    paths: Vec<String>,
) -> AppResult<()> {
    let repo_id = crate::history_ops::require_repo(repo_id)?;
    let args = AuditArgs::new().number("paths", paths.len() as i64);
    audit::record(
        &state,
        AuditEntry::new(repo_id, op_type::CONFLICT_RESOLVE).with_args(args),
        || {
            let repo_paths: Vec<RepoPath> = paths
                .iter()
                .map(|path| RepoPath::from(path.as_str()))
                .collect();
            state
                .conflict_service()
                .mark_resolved(repo_id, &repo_paths)?;
            emit_changed(&app, repo_id, WatchKind::Workspace, Vec::new());
            Ok(())
        },
    )
}

/// 继续进行中的操作。写操作：审计。再次停在冲突上是**正常结果**（见返回类型）。
#[tauri::command(async)]
pub fn git_conflict_continue(
    state: State<'_, AppState>,
    app: AppHandle,
    repo_id: i64,
) -> AppResult<ConflictContinueOutcome> {
    let repo_id = crate::history_ops::require_repo(repo_id)?;
    audit::record(
        &state,
        AuditEntry::new(repo_id, op_type::CONFLICT_CONTINUE),
        || {
            let outcome = state.conflict_service().continue_operation(repo_id)?;
            emit_changed(&app, repo_id, WatchKind::Workspace, Vec::new());
            if outcome.has_conflicts() {
                emit_changed(&app, repo_id, WatchKind::Refs, Vec::new());
            }
            Ok(outcome)
        },
    )
}

/// 中止进行中的操作。写操作：快照（services，abort 之前）+ 审计。
#[tauri::command(async)]
pub fn git_conflict_abort(
    state: State<'_, AppState>,
    app: AppHandle,
    repo_id: i64,
) -> AppResult<ConflictAbortOutcome> {
    let repo_id = crate::history_ops::require_repo(repo_id)?;
    audit::record(
        &state,
        AuditEntry::new(repo_id, op_type::CONFLICT_ABORT),
        || {
            let outcome = state.conflict_service().abort_operation(repo_id)?;
            emit_changed(&app, repo_id, WatchKind::Workspace, Vec::new());
            emit_changed(&app, repo_id, WatchKind::Refs, Vec::new());
            Ok(outcome)
        },
    )
}

/// 跳过当前提交（只有 rebase 支持）。写操作：审计。
#[tauri::command(async)]
pub fn git_conflict_skip(
    state: State<'_, AppState>,
    app: AppHandle,
    repo_id: i64,
) -> AppResult<ConflictContinueOutcome> {
    let repo_id = crate::history_ops::require_repo(repo_id)?;
    audit::record(
        &state,
        AuditEntry::new(repo_id, op_type::CONFLICT_SKIP),
        || {
            let outcome = state.conflict_service().skip(repo_id)?;
            emit_changed(&app, repo_id, WatchKind::Workspace, Vec::new());
            if outcome.has_conflicts() {
                emit_changed(&app, repo_id, WatchKind::Refs, Vec::new());
            }
            Ok(outcome)
        },
    )
}

/// 单个冲突文件的详情（三方 blob + 工作区形状 + diff3 合并块）。只读。
#[tauri::command(async)]
pub fn git_conflict_file_detail(
    state: State<'_, AppState>,
    repo_id: i64,
    path: String,
) -> AppResult<ConflictFileDetail> {
    let repo_id = crate::history_ops::require_repo(repo_id)?;
    if path.trim().is_empty() {
        return Err(forgedesk_domain::AppError::new(
            forgedesk_domain::ErrorCode::Validation,
            "the path must not be empty",
        ));
    }
    state
        .conflict_service()
        .file_detail(repo_id, &RepoPath::from(path.as_str()))
}

/// 整个文件采用一方（二进制 / 删除类冲突的"保留一方"）。写操作：审计。
#[tauri::command(async)]
pub fn git_conflict_take_side(
    state: State<'_, AppState>,
    app: AppHandle,
    repo_id: i64,
    path: String,
    side: TakeSide,
) -> AppResult<()> {
    let repo_id = crate::history_ops::require_repo(repo_id)?;
    let args = AuditArgs::new().text("path", &path).text(
        "side",
        match side {
            TakeSide::Ours => "ours",
            TakeSide::Theirs => "theirs",
        },
    );
    audit::record(
        &state,
        AuditEntry::new(repo_id, op_type::CONFLICT_RESOLVE).with_args(args),
        || {
            state
                .conflict_service()
                .take_side(repo_id, &RepoPath::from(path.as_str()), side)?;
            emit_changed(&app, repo_id, WatchKind::Workspace, Vec::new());
            emit_changed(&app, repo_id, WatchKind::Refs, Vec::new());
            Ok(())
        },
    )
}

/// 写回编辑器的结果文本并标记已解决。写操作：审计（EOL/BOM 由引擎重建）。
#[tauri::command(async)]
pub fn git_conflict_apply_resolution(
    state: State<'_, AppState>,
    app: AppHandle,
    repo_id: i64,
    path: String,
    spec: ApplyResolutionRequest,
) -> AppResult<()> {
    let repo_id = crate::history_ops::require_repo(repo_id)?;
    let args = AuditArgs::new()
        .text("path", &path)
        .number("contentLength", spec.content.chars().count() as i64)
        .text(
            "eol",
            match spec.eol {
                LineEnding::Lf => "lf",
                LineEnding::Crlf => "crlf",
                LineEnding::Cr => "cr",
            },
        )
        .flag("bom", spec.bom);
    audit::record(
        &state,
        AuditEntry::new(repo_id, op_type::CONFLICT_RESOLVE).with_args(args),
        || {
            state.conflict_service().apply_resolution(
                repo_id,
                &RepoPath::from(path.as_str()),
                &spec.content,
                spec.eol,
                spec.bom,
                spec.trailing_newline,
            )?;
            emit_changed(&app, repo_id, WatchKind::Workspace, Vec::new());
            emit_changed(&app, repo_id, WatchKind::Refs, Vec::new());
            Ok(())
        },
    )
}

/// 以"删除该文件"解决删除类冲突。写操作：审计。
#[tauri::command(async)]
pub fn git_conflict_remove_file(
    state: State<'_, AppState>,
    app: AppHandle,
    repo_id: i64,
    path: String,
) -> AppResult<()> {
    let repo_id = crate::history_ops::require_repo(repo_id)?;
    let args = AuditArgs::new().text("path", &path);
    audit::record(
        &state,
        AuditEntry::new(repo_id, op_type::CONFLICT_RESOLVE).with_args(args),
        || {
            state
                .conflict_service()
                .remove_file(repo_id, &RepoPath::from(path.as_str()))?;
            emit_changed(&app, repo_id, WatchKind::Workspace, Vec::new());
            Ok(())
        },
    )
}
