//! 快照命令（`snapshot_*`，M1 / T1.9）。
//!
//! 与其它命令族同一条纪律：本层只做参数校验、DTO 转换与事件投递。
//!
//! | 命令 | 能力等级 | 说明 |
//! | --- | --- | --- |
//! | [`snapshot_list`] | `ReadOnly` | 快照列表（新的在前） |
//! | [`snapshot_diff`] | `ReadOnly` | 快照与当前状态的差异摘要 |
//! | [`snapshot_restore`] | `Mutating` | 回滚到快照；成功后发布 `repo:changed` |
//! | [`snapshot_prune`] | `Mutating` | 按保留策略清理（v1 用内置默认策略） |
//!
//! `snapshot_restore` 是界面上**破坏性最强**的按钮（它会把工作区、索引与 HEAD
//! 一起搬回过去），因此它必须只被"用户看过差异摘要并确认"的路径调用——
//! 确认对话框是前端的责任，后端的闸门是回滚前自动打保护点与恢复后校验。

use forgedesk_domain::{AppError, AppResult};
use forgedesk_services::audit::op_type;
use forgedesk_services::{AuditArgs, AuditEntry};
use forgedesk_snapshot::{RestoreReport, RetentionPolicy, SnapshotError, SnapshotId, SnapshotMeta};
use serde::Serialize;
use tauri::{AppHandle, State};

use crate::state::AppState;
use crate::workspace::emit_changed;
use forgedesk_platform::watcher::WatchKind;

/// 列表数量的上限：一次 IPC 拉走整个历史没有意义，翻页（M2）够用。
const MAX_LIST_LIMIT: i64 = 200;

/// 快照列表行。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotMetaDto {
    /// 主键（回滚时回传）。
    pub id: SnapshotId,
    /// 展示标签（如 `pre-commit`）。
    pub label: String,
    /// 场景短名。
    pub kind: String,
    /// 快照时刻的 HEAD oid（全量，界面自行截短）。
    pub head_oid: String,
    /// 当时的分支名。
    pub branch: Option<String>,
    /// 是否游离 HEAD。
    pub detached: bool,
    /// 创建时间（Unix 毫秒）。
    pub created_at_ms: i64,
}

/// 回滚结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreReportDto {
    /// 被恢复的快照。
    pub restored_snapshot_id: SnapshotId,
    /// 恢复后的 HEAD oid。
    pub head_oid: String,
    /// 恢复后的索引树 oid。
    pub index_tree_oid: String,
    /// 回滚前自动打的保护点。
    pub pre_restore_snapshot_id: Option<SnapshotId>,
    /// 快照时刻的未跟踪文件路径（v1 只记录不恢复）。
    pub untracked_paths: Vec<String>,
}

/// 快照与当前状态的差异摘要。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotDiffDto {
    /// HEAD 已经不是快照时的位置。
    pub head_changed: bool,
    /// 索引已经不是快照时的树。
    pub index_changed: bool,
    /// 当前的 HEAD oid（空仓库为 `null`）。
    pub current_head_oid: Option<String>,
    /// 当前的索引树（索引有未合并条目时为 `null`）。
    pub current_index_tree_oid: Option<String>,
    /// 锚点丢失——这个快照不可恢复。
    pub ref_missing: bool,
}

/// 快照列表。能力等级：`ReadOnly`。
#[tauri::command]
pub fn snapshot_list(
    state: State<'_, AppState>,
    repo_id: i64,
    limit: Option<i64>,
) -> AppResult<Vec<SnapshotMetaDto>> {
    ensure_repo_id(repo_id)?;
    let limit = limit.unwrap_or(50).clamp(1, MAX_LIST_LIMIT);

    state
        .snapshots
        .list(repo_id, limit)
        .map_err(snapshot_error)
        .map(|metas| metas.into_iter().map(to_meta_dto).collect())
}

/// 快照与当前状态的差异。能力等级：`ReadOnly`。
#[tauri::command]
pub fn snapshot_diff(
    state: State<'_, AppState>,
    repo_id: i64,
    snapshot_id: i64,
) -> AppResult<SnapshotDiffDto> {
    ensure_repo_id(repo_id)?;
    ensure_snapshot_id(snapshot_id)?;

    state
        .snapshots
        .diff(repo_id, snapshot_id)
        .map_err(snapshot_error)
        .map(|diff| SnapshotDiffDto {
            head_changed: diff.head_changed,
            index_changed: diff.index_changed,
            current_head_oid: diff.current_head_oid,
            current_index_tree_oid: diff.current_index_tree_oid,
            ref_missing: diff.ref_missing,
        })
}

/// 回滚到快照。能力等级：`Mutating`；成功后发布 `repo:changed`。
///
/// 前端必须先展示差异摘要并经确认对话框（红线 R7 在 UI 层的闸门）；
/// 后端的闸门是"回滚前自动打保护点 + 恢复后校验"。
#[tauri::command]
pub fn snapshot_restore(
    state: State<'_, AppState>,
    app: AppHandle,
    repo_id: i64,
    snapshot_id: i64,
) -> AppResult<RestoreReportDto> {
    ensure_repo_id(repo_id)?;
    ensure_snapshot_id(snapshot_id)?;

    // 审计（T1.11）：回滚是破坏性最强的一步，记录里必须有"回到了哪个快照"
    // 与"有没有保护点可回"（后者是 `reversible` 的唯一依据）
    let operation = state.audit_service().begin(
        &AuditEntry::new(repo_id, op_type::SNAPSHOT_RESTORE)
            .with_args(AuditArgs::new().number("snapshotId", snapshot_id)),
    );

    let result = state.snapshots.restore(repo_id, snapshot_id);
    let report = match result {
        Ok(report) => {
            if let Some(operation) = operation {
                operation.finish(&Ok::<(), AppError>(()), report.pre_restore_snapshot_id);
            }
            report
        }
        Err(error) => {
            let error = snapshot_error(error);
            if let Some(operation) = operation {
                operation.finish::<()>(&Err(error.clone()), None);
            }
            return Err(error);
        }
    };

    // 回滚把 HEAD、索引与工作区一起搬回去了：按"引用变化"上报，
    // 让历史、分支与状态面板全部失效（工作区那一路由文件监听补上）
    emit_changed(&app, repo_id, WatchKind::Refs, Vec::new());
    Ok(to_report_dto(&report))
}

/// 按保留策略清理。能力等级：`Mutating`（删除快照与其锚点 ref）。
///
/// v1 使用内置默认策略（50 条 / 30 天，先到者生效）；把策略暴露成设置项
/// 属于设置页的工作（M3 的 T3.8 会连同磁盘占用阈值一起做）。
#[tauri::command]
pub fn snapshot_prune(state: State<'_, AppState>, repo_id: i64) -> AppResult<Vec<SnapshotId>> {
    ensure_repo_id(repo_id)?;
    let policy = RetentionPolicy::default();

    // 清理也要留痕：删掉的是"过去的自己"，事后要能回答"什么时候删的、按什么策略"
    crate::audit::record(
        &state,
        AuditEntry::new(repo_id, op_type::SNAPSHOT_PRUNE).with_args(
            AuditArgs::new()
                .number("maxCount", i64::from(policy.max_count))
                .number("maxAgeDays", i64::from(policy.max_age_days)),
        ),
        || {
            state
                .snapshots
                .prune(repo_id, &policy)
                .map_err(snapshot_error)
        },
    )
}

// ---------------------------------------------------------------- 内部

fn ensure_repo_id(repo_id: i64) -> AppResult<()> {
    if repo_id <= 0 {
        return Err(forgedesk_domain::AppError::new(
            forgedesk_domain::ErrorCode::Validation,
            "repoId must be a positive record id",
        )
        .with_hint("repo_id"));
    }
    Ok(())
}

fn ensure_snapshot_id(snapshot_id: i64) -> AppResult<()> {
    if snapshot_id <= 0 {
        return Err(forgedesk_domain::AppError::new(
            forgedesk_domain::ErrorCode::Validation,
            "snapshotId must be a positive record id",
        )
        .with_hint("snapshot_id"));
    }
    Ok(())
}

/// 快照错误 → `AppError`（错误码由 [`SnapshotError::code`] 给，文案保持英文）。
fn snapshot_error(error: SnapshotError) -> forgedesk_domain::AppError {
    forgedesk_domain::AppError::new(error.code(), error.message())
}

fn to_meta_dto(meta: SnapshotMeta) -> SnapshotMetaDto {
    SnapshotMetaDto {
        id: meta.id,
        label: meta.label,
        kind: meta.kind,
        head_oid: meta.head_oid,
        branch: meta.branch,
        detached: meta.detached,
        created_at_ms: meta.created_at,
    }
}

fn to_report_dto(report: &RestoreReport) -> RestoreReportDto {
    RestoreReportDto {
        restored_snapshot_id: report.restored_snapshot_id,
        head_oid: report.head_oid.clone(),
        index_tree_oid: report.index_tree_oid.clone(),
        pre_restore_snapshot_id: report.pre_restore_snapshot_id,
        untracked_paths: report.untracked_paths.clone(),
    }
}
