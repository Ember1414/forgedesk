//! 快照命令（`snapshot_*`，M1 / T1.9；T3.8 补手动打点、占用与清理）。
//!
//! 与其它命令族同一条纪律：本层只做参数校验、DTO 转换与事件投递。
//!
//! | 命令 | 能力等级 | 说明 |
//! | --- | --- | --- |
//! | [`snapshot_list`] | `ReadOnly` | 快照列表（新的在前） |
//! | [`snapshot_diff`] | `ReadOnly` | 快照与当前状态的差异摘要（含未跟踪内容三分类） |
//! | [`snapshot_usage`] | `ReadOnly` | 磁盘占用、配额与孤儿目录 |
//! | [`snapshot_estimate`] | `ReadOnly` | 下一次快照会备份多少未跟踪内容（危险操作对话框用） |
//! | [`snapshot_create`] | `Mutating` | 手动打点；返回内容备份的实情（体积 / 跳过 / 告警） |
//! | [`snapshot_restore`] | `Mutating` | 回滚到快照；成功后发布 `repo:changed` |
//! | [`snapshot_prune`] | `Mutating` | 按保留策略清理（内置默认策略） |
//! | [`snapshot_cleanup`] | `Mutating` | 立即清理：孤儿目录 + 保留策略 + 总占用回收 |
//!
//! `snapshot_restore` 是界面上**破坏性最强**的按钮（它会把工作区、索引与 HEAD
//! 一起搬回过去），因此它必须只被"用户看过差异摘要并确认"的路径调用——
//! 确认对话框是前端的责任，后端的闸门是回滚前自动打保护点与恢复后校验。
//!
//! `snapshot_create` 是唯一**由用户主动发起**的快照：它的返回值因此必须说清
//! "这次打点包含什么、不包含什么"（体积、跳过的未跟踪文件、告警），
//! 而不是像自动快照那样只回一个 id。

use forgedesk_domain::{AppError, AppResult};
use forgedesk_services::audit::op_type;
use forgedesk_services::{AuditArgs, AuditEntry};
use forgedesk_snapshot::{
    CleanupOutcome, RestoreReport, RetentionPolicy, SnapshotError, SnapshotEstimate, SnapshotId,
    SnapshotKind, SnapshotMeta, SnapshotOutcome, SnapshotRequest, SnapshotUsage, SnapshotWarning,
};
use serde::Serialize;
use tauri::{AppHandle, State};

use crate::state::AppState;
use crate::workspace::emit_changed;
use forgedesk_platform::watcher::WatchKind;

/// 列表数量的上限：一次 IPC 拉走整个历史没有意义，翻页（M2）够用。
const MAX_LIST_LIMIT: i64 = 200;

/// 手动快照标签的最大字符数（展示字段：太长会把列表与审计记录撑爆）。
const MAX_LABEL_CHARS: usize = 64;

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
    /// 从内容备份写回工作区的未跟踪文件数（T3.8；v1 快照恒为 0）。
    pub untracked_restored: usize,
    /// 没能恢复的未跟踪文件（备份缺失、写不进去）。
    pub untracked_failed: Vec<String>,
    /// 当前存在、快照里没有的未跟踪文件——**不会被删除**。
    pub untracked_extra: Vec<String>,
    /// 恢复后的完整校验是否通过（HEAD / 索引 / 备份内容逐字节）。
    pub verified: bool,
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
    /// 回滚会写回的未跟踪文件（有备份，且当前缺失或内容不同）。
    pub untracked_restorable: Vec<String>,
    /// 快照里记录过、但没有内容备份的未跟踪文件——回滚**找不回来**。
    pub untracked_missing: Vec<String>,
    /// 当前存在、快照里没有的未跟踪文件——回滚**不会删除**它们。
    pub untracked_extra: Vec<String>,
}

/// 手动创建快照的结果（T3.8）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotOutcomeDto {
    /// 新快照的 id。
    pub id: SnapshotId,
    /// 备份内容的字节总数。
    pub backup_bytes: u64,
    /// 备份成功的文件数。
    pub backed_up: usize,
    /// 快照时刻的未跟踪文件总数（含未备份的）。
    pub untracked_total: usize,
    /// 因超限或复制失败而**没有**进备份的未跟踪路径。
    pub skipped: Vec<String>,
    /// 如实告警（界面按 `kind` 走 i18n）。
    pub warnings: Vec<SnapshotWarningDto>,
    /// 本次顺手清理掉的旧快照。
    pub pruned: Vec<SnapshotId>,
}

/// 一条创建告警。
///
/// 扁平结构 + `kind` 判别（而不是 serde 的 internally tagged enum）：
/// 前端拿到的是一个"永远有全部字段"的对象，渲染时不必处理缺字段的分支；
/// 代价是未用到的字段是 `null`——这比"某个告警类型少了字段导致界面崩"
/// 划算得多。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotWarningDto {
    /// 类型短名（`untrackedBackupSkipped` / `untrackedBackupPartial` /
    /// `backupDirUnavailable` / `spaceReclaimed` / `orphansRemoved`）。
    pub kind: String,
    /// 涉及的文件数（超限跳过、孤儿清理）。
    pub count: Option<usize>,
    /// 涉及的字节数（超限跳过）。
    pub bytes: Option<u64>,
    /// 触发跳过的上限。
    pub limit: Option<u64>,
    /// 涉及的路径（复制失败）。
    pub paths: Vec<String>,
    /// 具体原因（备份目录不可用、复制失败）。
    pub detail: Option<String>,
    /// 被回收的快照（总占用上限触发）。
    pub removed: Vec<SnapshotId>,
    /// 释放的字节数（总占用上限触发）。
    pub freed_bytes: Option<u64>,
}

/// 快照磁盘占用与配额（列表页 / 设置页显示）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotUsageDto {
    /// 仓库 id。
    pub repo_id: i64,
    /// 快照条数。
    pub snapshot_count: i64,
    /// 内容备份占用的字节数。
    pub backup_bytes: u64,
    /// 单份上限（`0` = 不限制）。
    pub max_snapshot_bytes: u64,
    /// 总占用上限（`0` = 不限制）。
    pub max_repo_bytes: u64,
    /// 磁盘上存在、数据库里没有对应快照的目录。
    pub orphan_dirs: Vec<String>,
}

/// 手动清理的结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupOutcomeDto {
    /// 清理掉的孤儿目录数。
    pub orphans_removed: usize,
    /// 为满足上限而清理掉的快照（LRU）。
    pub reclaimed: Vec<SnapshotId>,
    /// 释放的字节数。
    pub freed_bytes: u64,
    /// 清理后的备份总占用。
    pub remaining_bytes: u64,
}

/// 快照体积预估（危险操作对话框在动手前展示）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotEstimateDto {
    /// 将被备份的未跟踪文件数（超限时为全部候选数）。
    pub untracked_count: usize,
    /// 它们的字节数。
    pub untracked_bytes: u64,
    /// 被忽略文件的数量。
    pub ignored_count: usize,
    /// 被忽略文件的字节数。
    pub ignored_bytes: u64,
    /// 当前策略是否包含被忽略文件。
    pub include_ignored: bool,
    /// 单份上限（`0` = 不限制）。
    pub limit_bytes: u64,
    /// 本次是否会完整备份未跟踪内容。
    pub within_limit: bool,
    /// 超限时会被跳过（不备份）的文件数。
    pub would_skip: usize,
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
            untracked_restorable: diff.untracked_restorable,
            untracked_missing: diff.untracked_missing,
            untracked_extra: diff.untracked_extra,
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

/// 手动创建快照。能力等级：`Mutating`（不改仓库，但要写数据库与备份目录）。
///
/// 返回内容备份的实情：体积、跳过的未跟踪文件与告警。界面据此告诉用户
/// "这次打点包含什么、不包含什么"——**不静默跳过**是 T3.8 的硬要求。
#[tauri::command]
pub fn snapshot_create(
    state: State<'_, AppState>,
    repo_id: i64,
    label: Option<String>,
) -> AppResult<SnapshotOutcomeDto> {
    ensure_repo_id(repo_id)?;
    let label = sanitize_label(label.as_deref());
    // 标签与工作区路径都要过一遍仓库解析：不存在的 repo_id 必须在
    // 动数据库与磁盘之前就被挡住
    let workdir = state.workspace_service().resolve_workdir(repo_id)?;

    let operation = state.audit_service().begin(
        &AuditEntry::new(repo_id, op_type::SNAPSHOT_CREATE).with_args(
            AuditArgs::new()
                .text("label", &label)
                .text("kind", SnapshotKind::Manual.key()),
        ),
    );

    let result = state.snapshots.create(&SnapshotRequest {
        repo_id,
        workdir: &workdir,
        label: &label,
        kind: SnapshotKind::Manual,
    });

    let outcome = match result {
        Ok(outcome) => {
            if let Some(operation) = operation {
                // 审计关联这份新快照：它本身就是"可回滚点"
                operation.finish(&Ok::<(), AppError>(()), Some(outcome.id));
            }
            outcome
        }
        Err(error) => {
            let error = snapshot_error(error);
            if let Some(operation) = operation {
                operation.finish::<()>(&Err(error.clone()), None);
            }
            return Err(error);
        }
    };

    // 手动打点不改仓库状态：不发 repo:changed（发了会让历史页白刷一遍）
    Ok(to_outcome_dto(outcome))
}

/// 快照磁盘占用与配额。能力等级：`ReadOnly`。
#[tauri::command]
pub fn snapshot_usage(state: State<'_, AppState>, repo_id: i64) -> AppResult<SnapshotUsageDto> {
    ensure_repo_id(repo_id)?;
    state
        .snapshots
        .usage(repo_id)
        .map_err(snapshot_error)
        .map(to_usage_dto)
}

/// 下一次快照的体积预估。能力等级：`ReadOnly`。
///
/// 危险操作对话框用它提前告诉用户"这次打点会跳过 N 个未跟踪文件（X MB）"，
/// 而不是等操作执行完才发现快照里没有它们。
#[tauri::command]
pub fn snapshot_estimate(
    state: State<'_, AppState>,
    repo_id: i64,
) -> AppResult<SnapshotEstimateDto> {
    ensure_repo_id(repo_id)?;
    state
        .snapshots
        .estimate(repo_id)
        .map_err(snapshot_error)
        .map(to_estimate_dto)
}

/// 立即清理快照缓存（孤儿目录 + 保留策略 + 总占用回收）。能力等级：`Mutating`。
///
/// 与 `snapshot_prune` 的区别：prune 只按保留策略（条数 / 天数）走，
/// cleanup 还会清掉复制中途崩溃留下的孤儿目录，并按**总占用上限**做 LRU 回收。
#[tauri::command]
pub fn snapshot_cleanup(state: State<'_, AppState>, repo_id: i64) -> AppResult<CleanupOutcomeDto> {
    ensure_repo_id(repo_id)?;
    // 清理删掉的是"过去的自己"：和 prune 一样必须留痕。
    // 记录的是 DTO（含回收清单与释放体积）——审计只存 JSON，不认识快照的领域类型
    crate::audit::record(
        &state,
        AuditEntry::new(repo_id, op_type::SNAPSHOT_CLEANUP),
        || {
            state
                .snapshots
                .cleanup(repo_id)
                .map_err(snapshot_error)
                .map(to_cleanup_dto)
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
        untracked_restored: report.untracked_restored,
        untracked_failed: report.untracked_failed.clone(),
        untracked_extra: report.untracked_extra.clone(),
        verified: report.verified,
    }
}

fn to_outcome_dto(outcome: SnapshotOutcome) -> SnapshotOutcomeDto {
    SnapshotOutcomeDto {
        id: outcome.id,
        backup_bytes: outcome.backup_bytes,
        backed_up: outcome.backed_up,
        untracked_total: outcome.untracked_total,
        skipped: outcome.skipped,
        warnings: outcome.warnings.iter().map(to_warning_dto).collect(),
        pruned: outcome.pruned,
    }
}

/// 告警 → DTO：先建一个"全空"的骨架，再按类型补字段。
fn to_warning_dto(warning: &SnapshotWarning) -> SnapshotWarningDto {
    let base = SnapshotWarningDto {
        kind: warning.kind().to_owned(),
        count: None,
        bytes: None,
        limit: None,
        paths: Vec::new(),
        detail: None,
        removed: Vec::new(),
        freed_bytes: None,
    };
    match warning {
        SnapshotWarning::UntrackedBackupSkipped {
            count,
            bytes,
            limit,
        } => SnapshotWarningDto {
            count: Some(*count),
            bytes: Some(*bytes),
            limit: Some(*limit),
            ..base
        },
        SnapshotWarning::UntrackedBackupPartial { paths, detail } => SnapshotWarningDto {
            paths: paths.clone(),
            detail: Some(detail.clone()),
            ..base
        },
        SnapshotWarning::BackupDirUnavailable { detail } => SnapshotWarningDto {
            detail: Some(detail.clone()),
            ..base
        },
        SnapshotWarning::SpaceReclaimed {
            removed,
            freed_bytes,
        } => SnapshotWarningDto {
            removed: removed.clone(),
            freed_bytes: Some(*freed_bytes),
            count: Some(removed.len()),
            ..base
        },
        SnapshotWarning::OrphansRemoved { count } => SnapshotWarningDto {
            count: Some(*count),
            ..base
        },
    }
}

fn to_usage_dto(usage: SnapshotUsage) -> SnapshotUsageDto {
    SnapshotUsageDto {
        repo_id: usage.repo_id,
        snapshot_count: usage.snapshot_count,
        backup_bytes: usage.backup_bytes,
        max_snapshot_bytes: usage.max_snapshot_bytes,
        max_repo_bytes: usage.max_repo_bytes,
        orphan_dirs: usage.orphan_dirs,
    }
}

fn to_cleanup_dto(outcome: CleanupOutcome) -> CleanupOutcomeDto {
    CleanupOutcomeDto {
        orphans_removed: outcome.orphans_removed,
        reclaimed: outcome.reclaimed,
        freed_bytes: outcome.freed_bytes,
        remaining_bytes: outcome.remaining_bytes,
    }
}

fn to_estimate_dto(estimate: SnapshotEstimate) -> SnapshotEstimateDto {
    SnapshotEstimateDto {
        untracked_count: estimate.untracked_count,
        untracked_bytes: estimate.untracked_bytes,
        ignored_count: estimate.ignored_count,
        ignored_bytes: estimate.ignored_bytes,
        include_ignored: estimate.include_ignored,
        limit_bytes: estimate.limit_bytes,
        within_limit: estimate.within_limit,
        would_skip: estimate.would_skip,
    }
}

/// 手动快照的标签净化：它是**展示字段**（进数据库与界面），
/// 去掉控制字符、限制长度，空串回落到 kind 名。
fn sanitize_label(label: Option<&str>) -> String {
    let cleaned: String = label
        .unwrap_or_default()
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_LABEL_CHARS)
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        SnapshotKind::Manual.key().to_owned()
    } else {
        trimmed.to_owned()
    }
}
