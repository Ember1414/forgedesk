//! 远端同步命令（T2.6）：`git_fetch` / `git_pull` / `git_push` 与 Remote CRUD。
//!
//! # 能力等级与审计
//!
//! fetch 只读远端引用（`Network`，不改本地历史）→ 不快照；pull 合并/变基会
//! 移动 HEAD → `Mutating`，**快照在 services 层**（`PreSync`，与提交路径共用
//! 快照历史）；push 更新远端 → `Network` + 审计。全部写审计。
//!
//! # 长任务与取消（任务书要求 6）
//!
//! 三个同步命令都走 [`JobRunner`]：立即返回任务 id，进度经 `job:progress`
//! 推送，结果经 `job:done` / `job:failed`；取消经 `job_cancel` →
//! `CancellationToken` → 进程层 kill 子进程。Remote CRUD 是瞬时操作，同步返回。
//!
//! # 凭据（任务书要求 5 的本任务部分）
//!
//! 进程层已固化 `GIT_TERMINAL_PROMPT=0`：任何需要交互输入的远端都会立刻失败
//! 并被分类为 `AUTH_REQUIRED`（T2.7 接入凭据库后同一通道自动注入凭据）。
//! 本任务不把任何 secret 放进命令行或 URL（红线 R8）。

use forgedesk_domain::git::{
    FetchOutcome, FetchSpec, PullOutcome, PullSpec, PushOutcome, PushSpec, Remote,
};
use forgedesk_domain::AppResult;
use serde::Serialize;
use tauri::{AppHandle, State};

use crate::audit;
use crate::jobs;
use crate::state::AppState;
use forgedesk_services::AuditEntry;
use forgedesk_services::{AuditArgs, AuditEntry as ServicesAuditEntry, SyncService};

/// 任务结果（`job:done` 的 payload；都是结构化 outcome）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncJobResult {
    /// 远端名（fetch/pull/push 共用形状里的最低公分母）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// fetch 的结果（`git_fetch` 任务成功时存在）。
    pub fetch: Option<FetchOutcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// pull 的结果（`git_pull` 任务成功时存在）。
    pub pull: Option<PullOutcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// push 的结果（`git_push` 任务成功时存在）。
    pub push: Option<PushOutcome>,
}

/// `git_fetch`：立即返回任务 id；进度与结果经 `job:*` 事件。
#[tauri::command]
pub fn git_fetch(
    state: State<'_, AppState>,
    app: AppHandle,
    repo_id: i64,
    spec: FetchSpec,
) -> AppResult<crate::repository::JobIdDto> {
    let engines = std::sync::Arc::clone(&state.engines);
    let database = std::sync::Arc::clone(&state.database);
    let snapshots = std::sync::Arc::clone(&state.snapshots);
    let job_id = state.jobs.spawn(jobs::reporter_for(app), move |context| {
        let progress = jobs::progress_sink(&context);
        let service = SyncService::new(
            &engines,
            forgedesk_storage::RepositoryStore::new(&database),
            snapshots.as_ref(),
        );
        let outcome = service.fetch(repo_id, spec, &progress, &context.cancellation())?;
        Ok(SyncJobResult {
            remote: Some(outcome.remote.clone()),
            fetch: Some(outcome),
            pull: None,
            push: None,
        })
    });

    Ok(crate::repository::JobIdDto {
        job_id: job_id.as_str().to_owned(),
    })
}

/// `git_pull`：先打 `PreSync` 快照（services 层），再拉取并合并/变基。
#[tauri::command]
pub fn git_pull(
    state: State<'_, AppState>,
    app: AppHandle,
    repo_id: i64,
    spec: PullSpec,
) -> AppResult<crate::repository::JobIdDto> {
    let engines = std::sync::Arc::clone(&state.engines);
    let database = std::sync::Arc::clone(&state.database);
    let snapshots = std::sync::Arc::clone(&state.snapshots);

    let job_id = state.jobs.spawn(jobs::reporter_for(app), move |context| {
        let progress = jobs::progress_sink(&context);
        let service = SyncService::new(
            &engines,
            forgedesk_storage::RepositoryStore::new(&database),
            snapshots.as_ref(),
        );
        let outcome = service.pull(repo_id, spec, &progress, &context.cancellation())?;
        // pull 移动了 HEAD / 工作区：同步事件让状态页与历史页刷新
        Ok(SyncJobResult {
            remote: Some(outcome.fetch.remote.clone()),
            fetch: Some(outcome.fetch.clone()),
            pull: Some(outcome),
            push: None,
        })
    });

    Ok(crate::repository::JobIdDto {
        job_id: job_id.as_str().to_owned(),
    })
}

/// `git_push`。non-fast-forward 拒绝在任务内转成 `PUSH_REJECTED`（带 actions），
/// 前端经 `job:failed` 收到同一套统一错误。
#[tauri::command]
pub fn git_push(
    state: State<'_, AppState>,
    app: AppHandle,
    repo_id: i64,
    spec: PushSpec,
) -> AppResult<crate::repository::JobIdDto> {
    let engines = std::sync::Arc::clone(&state.engines);
    let database = std::sync::Arc::clone(&state.database);
    let snapshots = std::sync::Arc::clone(&state.snapshots);

    // push 的审计在任务线程里记（与 repo_clone 同一模式：结果在任务里才知道）
    let audit_database = state.database.clone();
    let job_id = state.jobs.spawn(jobs::reporter_for(app), move |context| {
        let progress = jobs::progress_sink(&context);
        let service = SyncService::new(
            &engines,
            forgedesk_storage::RepositoryStore::new(&database),
            snapshots.as_ref(),
        );
        let spec_ref = &spec;
        let operation = audit::record_with(
            &forgedesk_services::AuditLog::new(forgedesk_storage::OperationStore::new(
                &audit_database,
            )),
            ServicesAuditEntry::new(repo_id, "sync.push").with_args(
                AuditArgs::new()
                    .text("remote", spec_ref.remote.as_deref().unwrap_or("<upstream>"))
                    .text("branch", spec_ref.branch.as_deref().unwrap_or("<current>"))
                    .flag("forceWithLease", spec_ref.force_with_lease)
                    .flag("dryRun", spec_ref.dry_run),
            ),
            || {
                service.push(
                    repo_id,
                    spec_ref.clone(),
                    &progress,
                    &context.cancellation(),
                )
            },
        );
        let outcome = operation?;
        Ok(SyncJobResult {
            remote: Some(outcome.remote.clone()),
            fetch: None,
            pull: None,
            push: Some(outcome),
        })
    });

    Ok(crate::repository::JobIdDto {
        job_id: job_id.as_str().to_owned(),
    })
}

// ---------------------------------------------------------------- Remote CRUD（同步命令）

/// `git_remote_list`：列出远端（瞬时命令，同步返回）。
#[tauri::command]
pub fn git_remote_list(state: State<'_, AppState>, repo_id: i64) -> AppResult<Vec<Remote>> {
    state.sync_service().remote_list(repo_id)
}

/// `git_remote_add`：新增远端（写审计；名称与 URL 在服务层二次校验）。
#[tauri::command]
pub fn git_remote_add(
    state: State<'_, AppState>,
    repo_id: i64,
    name: String,
    url: String,
) -> AppResult<()> {
    let args = AuditArgs::new().text("name", &name).text("url", &url);
    audit::record(
        &state,
        AuditEntry::new(repo_id, "remote.add").with_args(args),
        || state.sync_service().remote_add(repo_id, &name, &url),
    )
}

/// `git_remote_remove`：删除远端（写审计；同时清理其远端跟踪引用）。
#[tauri::command]
pub fn git_remote_remove(state: State<'_, AppState>, repo_id: i64, name: String) -> AppResult<()> {
    let args = AuditArgs::new().text("name", &name);
    audit::record(
        &state,
        AuditEntry::new(repo_id, "remote.remove").with_args(args),
        || state.sync_service().remote_remove(repo_id, &name),
    )
}

/// `git_remote_rename`：重命名远端（写审计；远端跟踪引用随之改名）。
#[tauri::command]
pub fn git_remote_rename(
    state: State<'_, AppState>,
    repo_id: i64,
    old: String,
    new: String,
) -> AppResult<()> {
    let args = AuditArgs::new().text("old", &old).text("new", &new);
    audit::record(
        &state,
        AuditEntry::new(repo_id, "remote.rename").with_args(args),
        || state.sync_service().remote_rename(repo_id, &old, &new),
    )
}

/// `git_remote_set_url`：改写远端 URL（写审计；URL 在服务层二次校验）。
#[tauri::command]
pub fn git_remote_set_url(
    state: State<'_, AppState>,
    repo_id: i64,
    name: String,
    url: String,
) -> AppResult<()> {
    let args = AuditArgs::new().text("name", &name).text("url", &url);
    audit::record(
        &state,
        AuditEntry::new(repo_id, "remote.setUrl").with_args(args),
        || state.sync_service().remote_set_url(repo_id, &name, &url),
    )
}
