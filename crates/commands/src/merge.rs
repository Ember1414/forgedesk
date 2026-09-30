//! 合并命令（T3.4）：prepare（预览）/ execute（执行）/ continue（冲突后继续）。
//!
//! # 两段式契约（红线 R7 的计划形态）
//!
//! prepare 生成计划并存入 AppState.merge_plans，返回完整预览（快进裁决、
//! 独有提交清单、冲突预检、默认信息、等价命令）；execute 只认 plan_id 且
//! 取走即失效，HEAD 变了即 `PLAN_STALE`——与重置计划同一约定。
//!
//! # IPC 形状
//!
//! domain 的 `MergePlan` 不带 serde（仓库约定），这里给 DTO；`MergeOutcome`
//! 带 serde 直接作返回值（与 cherry-pick / revert 同一约定，`snapshotId`
//! 字段供 `record_with` 关联审计）。

use forgedesk_domain::git::{MergeOutcome, MergePlan, MergeSpec, MergeStrategy};
use forgedesk_domain::{AppError, AppResult, ErrorCode};
use forgedesk_platform::watcher::WatchKind;
use forgedesk_services::audit::op_type;
use forgedesk_services::{AuditArgs, AuditEntry};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use crate::audit;
use crate::state::AppState;
use crate::workspace::emit_changed;

/// `git_merge_prepare` 的请求体。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeRequest {
    /// 要合并进来的引用（分支名 / tag / oid）。
    pub source: String,
    /// 合并策略；缺省 merge（最常规）。
    #[serde(default)]
    pub strategy: MergeStrategy,
}

/// 计划里一条独有提交。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitSummaryDto {
    /// 提交 oid。
    pub oid: String,
    /// 提交信息首行。
    pub subject: String,
    /// 作者时间（Unix 秒）。
    pub author_time: Option<i64>,
}

/// 合并计划 DTO（与 domain `MergePlan` 同形，补 serde）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergePlanDto {
    /// 计划句柄（execute 回传）。
    pub plan_id: String,
    /// 要合并进来的引用。
    pub source: String,
    /// 合并策略。
    pub strategy: MergeStrategy,
    /// 快进裁决短名（`upToDate` / `fastForward` / `trueMerge`）。
    pub verdict: String,
    /// source 独有的提交（新到旧，最多 30 条）。
    pub source_only_commits: Vec<CommitSummaryDto>,
    /// source 独有提交的精确总数。
    pub source_commit_count: usize,
    /// 预检是否可用（git 太旧时为 false）。
    pub preview_available: bool,
    /// 预检发现的冲突文件。
    pub conflicted: Vec<String>,
    /// 默认合并信息（可编辑）。
    pub default_message: String,
    /// 等价命令字符串。
    pub equivalent_command: String,
}

impl MergePlanDto {
    fn from_plan(plan: MergePlan) -> Self {
        Self {
            plan_id: plan.plan_id,
            source: plan.source,
            strategy: plan.strategy,
            verdict: plan.verdict.as_str().to_owned(),
            source_only_commits: plan
                .source_only_commits
                .iter()
                .map(|commit| CommitSummaryDto {
                    oid: commit.oid.clone(),
                    subject: commit.subject.clone(),
                    author_time: commit.author_time,
                })
                .collect(),
            source_commit_count: plan.source_commit_count,
            preview_available: plan.preview.available,
            conflicted: plan
                .preview
                .conflicted
                .iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect(),
            default_message: plan.default_message,
            equivalent_command: plan.equivalent_command,
        }
    }
}

/// `git_merge_execute` 的请求体。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeExecuteRequest {
    /// prepare 返回的计划句柄。
    pub plan_id: String,
    /// 用户编辑过的合并信息；缺省用计划的默认信息。
    #[serde(default)]
    pub message: Option<String>,
}

/// 策略之外的后端二次校验收口：source 非空。
fn validate_spec(request: &MergeRequest) -> AppResult<MergeSpec> {
    let source = request.source.trim();
    if source.is_empty() {
        return Err(AppError::new(
            ErrorCode::Validation,
            "the merge source must not be empty",
        ));
    }
    Ok(MergeSpec::new(source.to_owned()).with_strategy(request.strategy))
}

/// 生成合并计划（只读预览，不碰工作区）。ReadOnly。
#[tauri::command(async)]
pub fn git_merge_prepare(
    state: State<'_, AppState>,
    repo_id: i64,
    spec: MergeRequest,
) -> AppResult<MergePlanDto> {
    let repo_id = crate::history_ops::require_repo(repo_id)?;
    let spec = validate_spec(&spec)?;
    let plan = state.merge_service().prepare(repo_id, &spec)?;
    Ok(MergePlanDto::from_plan(plan))
}

/// 执行合并计划。写操作：`PreSync` 快照（services）+ 审计。
///
/// 冲突是**结果不是错误**：返回的 `MergeOutcome.kind == "conflicted"` 时
/// 界面把用户送到冲突页。
#[tauri::command(async)]
pub fn git_merge_execute(
    state: State<'_, AppState>,
    app: AppHandle,
    repo_id: i64,
    spec: MergeExecuteRequest,
) -> AppResult<MergeOutcome> {
    let repo_id = crate::history_ops::require_repo(repo_id)?;
    let args = AuditArgs::new()
        .text("planId", &spec.plan_id)
        .flag("customMessage", spec.message.is_some());
    audit::record(
        &state,
        AuditEntry::new(repo_id, op_type::MERGE).with_args(args),
        || {
            let outcome = state
                .merge_service()
                .execute(repo_id, &spec.plan_id, spec.message)?;
            // 合并动了工作区（快进 / squash / 冲突）也可能动了 HEAD（合并提交）
            emit_changed(&app, repo_id, WatchKind::Workspace, Vec::new());
            emit_changed(&app, repo_id, WatchKind::Refs, Vec::new());
            Ok(outcome)
        },
    )
}

/// 冲突解决后的"继续合并"（可选编辑合并信息）。写操作：审计。
#[tauri::command(async)]
pub fn git_merge_continue(
    state: State<'_, AppState>,
    app: AppHandle,
    repo_id: i64,
    message: Option<String>,
) -> AppResult<MergeOutcome> {
    let repo_id = crate::history_ops::require_repo(repo_id)?;
    let args = AuditArgs::new().flag("customMessage", message.is_some());
    audit::record(
        &state,
        AuditEntry::new(repo_id, op_type::MERGE).with_args(args),
        || {
            let outcome = state.merge_service().continue_merge(repo_id, message)?;
            emit_changed(&app, repo_id, WatchKind::Workspace, Vec::new());
            emit_changed(&app, repo_id, WatchKind::Refs, Vec::new());
            Ok(outcome)
        },
    )
}
