//! rebase 执行命令（T3.7）：preview_only（预演）/ execute（执行）/ continue_after_edit。
//!
//! # 两段式与暂停语义
//!
//! execute 打 `PreHeadMove` 快照（rebase 重写历史）后跑引擎，结局三选一
//! （完成 / 冲突暂停 / edit 暂停），暂停是**结果不是错误**：冲突走冲突页，
//! edit 由 `git_rebase_continue_edit` 在用户改完内容后恢复。

use forgedesk_domain::git::{RebaseOutcome, RebasePlan, ReorderStep};
use forgedesk_domain::{AppError, AppResult, ErrorCode};
use forgedesk_platform::watcher::WatchKind;
use forgedesk_services::audit::op_type;
use forgedesk_services::{AuditArgs, AuditEntry};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use crate::audit;
use crate::state::AppState;
use crate::workspace::emit_changed;

/// rebase 预演的 DTO（与 domain `RebasePreview` 同形，补 serde）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RebasePreviewDto {
    /// 执行后存活的提交（新顺序；oid 是重写前占位）。
    pub surviving: Vec<RebaseSurvivingDto>,
    /// 被丢弃的提交 oid。
    pub dropped: Vec<String>,
    /// 信息被改写的提交 oid。
    pub reworded: Vec<String>,
    /// 被并入其他提交的记录。
    pub squashed: Vec<String>,
    /// 受影响的提交总数。
    pub affected_count: usize,
    /// 区间内有已推送提交将被重写（需要 force-with-lease 提示）。
    pub touches_pushed: bool,
}

/// 预演里一条存活提交。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RebaseSurvivingDto {
    /// 原提交 oid（占位）。
    /// 被操作的提交 oid。
    pub oid: String,
    /// 信息草案。
    pub subject: String,
}

impl RebasePreviewDto {
    fn from_domain(preview: forgedesk_domain::git::RebasePreview) -> Self {
        Self {
            surviving: preview
                .surviving
                .iter()
                .map(|commit| RebaseSurvivingDto {
                    oid: commit.oid.clone(),
                    subject: commit.subject.clone(),
                })
                .collect(),
            dropped: preview.dropped,
            reworded: preview.reworded,
            squashed: preview.squashed,
            affected_count: preview.affected_count,
            touches_pushed: preview.touches_pushed,
        }
    }
}

/// `git_rebase_execute` 的请求体。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RebaseExecuteRequest {
    /// 新的基点（`--onto` 的目标）。
    pub base: String,
    /// 区间右端。
    pub head: String,
    /// 步骤清单（从旧到新）。
    pub steps: Vec<RebaseStepRequest>,
    /// 允许压平 merge 提交。
    #[serde(default)]
    /// 允许压平 merge 提交（丢弃合并结构，必须显式开启）。
    pub allow_flatten_merges: bool,
    #[serde(default)]
    /// todo 启用 autosquash 语义（fixup!/squash! 自动归并）。
    pub autosquash: bool,
}

/// 步骤请求体。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RebaseStepRequest {
    /// 被操作的提交 oid。
    pub oid: String,
    /// 动作（pick/reword/edit/squash/fixup/drop）。
    pub action: forgedesk_domain::git::ReorderAction,
    /// reword / squash 的新信息草案。
    #[serde(default)]
    pub new_message: Option<String>,
}

/// rebase 结局 DTO（tag = kind，与 `RebaseOutcome` 的 serde 形状一致）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum RebaseOutcomeDto {
    /// 完成：HEAD 是重写后的新顶端。
    Completed {
        /// 完成后的 HEAD oid。
        oid: String,
    },
    /// 停在冲突上：走冲突页（T3.1 状态机）。
    PausedConflict {
        /// 冲突文件清单。
        conflicts: Vec<String>,
    },
    /// 停在 edit 步骤：`git_rebase_continue_edit` 恢复。
    PausedEdit {
        /// 被编辑的提交 oid。
        oid: String,
    },
}

impl RebaseOutcomeDto {
    fn from_outcome(outcome: RebaseOutcome) -> Self {
        match outcome {
            RebaseOutcome::Completed { oid } => Self::Completed { oid },
            RebaseOutcome::PausedConflict { conflicts } => Self::PausedConflict {
                conflicts: conflicts
                    .iter()
                    .map(|path| path.to_string_lossy().into_owned())
                    .collect(),
            },
            RebaseOutcome::PausedEdit { oid } => Self::PausedEdit { oid },
        }
    }
}

fn validate_plan(request: &RebaseExecuteRequest) -> AppResult<RebasePlan> {
    if request.base.trim().is_empty() || request.head.trim().is_empty() {
        return Err(AppError::new(
            ErrorCode::Validation,
            "the rebase base and head must not be empty",
        ));
    }
    if request.steps.is_empty() {
        return Err(AppError::new(
            ErrorCode::Validation,
            "the rebase plan has no steps",
        ));
    }
    Ok(RebasePlan {
        base: request.base.trim().to_owned(),
        head: request.head.trim().to_owned(),
        steps: request
            .steps
            .iter()
            .map(|step| ReorderStep {
                oid: step.oid.trim().to_owned(),
                action: step.action,
                new_message: step.new_message.clone(),
            })
            .collect(),
        allow_flatten_merges: request.allow_flatten_merges,
        autosquash: request.autosquash,
    })
}

/// 预演 rebase 计划（只读：装配区间图 → 校验 → 预览）。ReadOnly。
#[tauri::command(async)]
pub fn git_rebase_preview_only(
    state: State<'_, AppState>,
    repo_id: i64,
    spec: RebaseExecuteRequest,
) -> AppResult<RebasePreviewDto> {
    let repo_id = crate::history_ops::require_repo(repo_id)?;
    let plan = validate_plan(&spec)?;
    let preview = state.rebase_service().preview_only(repo_id, &plan)?;
    Ok(RebasePreviewDto::from_domain(preview))
}

/// 执行 rebase 计划。写操作：`PreHeadMove` 快照（services）+ 审计。
#[tauri::command(async)]
pub fn git_rebase_execute(
    state: State<'_, AppState>,
    app: AppHandle,
    repo_id: i64,
    spec: RebaseExecuteRequest,
) -> AppResult<RebaseOutcomeDto> {
    let repo_id = crate::history_ops::require_repo(repo_id)?;
    let plan = validate_plan(&spec)?;
    let args = AuditArgs::new()
        .text("base", &spec.base)
        .text("head", &spec.head)
        .number("steps", spec.steps.len() as i64);
    audit::record(
        &state,
        AuditEntry::new(repo_id, op_type::REBASE).with_args(args),
        || {
            let outcome = state.rebase_service().execute(repo_id, &plan)?;
            // rebase 动了历史（重写）与工作区
            emit_changed(&app, repo_id, WatchKind::Large, Vec::new());
            emit_changed(&app, repo_id, WatchKind::Refs, Vec::new());
            Ok(RebaseOutcomeDto::from_outcome(outcome))
        },
    )
}

/// edit 暂停的恢复（amend 接住用户改动 + rebase --continue）。写操作：审计。
#[tauri::command(async)]
pub fn git_rebase_continue_edit(
    state: State<'_, AppState>,
    app: AppHandle,
    repo_id: i64,
) -> AppResult<RebaseOutcomeDto> {
    let repo_id = crate::history_ops::require_repo(repo_id)?;
    audit::record(&state, AuditEntry::new(repo_id, op_type::REBASE), || {
        let outcome = state.rebase_service().continue_after_edit(repo_id)?;
        emit_changed(&app, repo_id, WatchKind::Large, Vec::new());
        emit_changed(&app, repo_id, WatchKind::Refs, Vec::new());
        Ok(RebaseOutcomeDto::from_outcome(outcome))
    })
}
