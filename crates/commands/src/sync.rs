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
    FetchOutcome, FetchSpec, PullOutcome, PullSpec, PullStrategy, PushOutcome, PushSpec, Remote,
};
use forgedesk_domain::{AppError, AppResult};
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

/// 把仓库上下文补进错误里的"测试连接"动作。
///
/// `ErrorCode::default_actions` 只给动作骨架（id / labelKey / command），
/// 因为领域层不知道是哪个仓库出的错；而前端会照 `command` + `args` 直接 invoke。
/// 缺参数的按钮点下去只会得到一条 `VALIDATION`——那比没有按钮更让人困惑。
fn attach_repo_context(error: AppError, repo_id: i64) -> AppError {
    if !error.actions.is_empty() {
        // 已经有更具体的动作了（例如 `PUSH_REJECTED` 的三条修复路径），不叠加
        return error;
    }
    let actions = error.code.default_actions();
    if actions.is_empty() {
        return error;
    }

    let mut enriched = error;
    for action in actions {
        let action = if action.command == "credential_test_remote" {
            action.with_args(serde_json::json!({ "repoId": repo_id }))
        } else {
            action
        };
        enriched = enriched.with_action(action);
    }
    enriched
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
    let credential_gate = state.credential_gate.clone();
    // 审计在任务线程里记（与 push / repo_clone 同一模式：结果在任务里才知道）
    let audit_database = state.database.clone();
    let job_id = state.jobs.spawn(jobs::reporter_for(app), move |context| {
        let progress = jobs::progress_sink(&context);
        let service = SyncService::new(
            &engines,
            forgedesk_storage::RepositoryStore::new(&database),
            snapshots.as_ref(),
        )
        .with_credential_gate(credential_gate.as_deref());
        let spec_ref = &spec;
        let operation = audit::record_with(
            &forgedesk_services::AuditLog::new(forgedesk_storage::OperationStore::new(
                &audit_database,
            )),
            ServicesAuditEntry::new(repo_id, "sync.fetch").with_args(
                AuditArgs::new().text("remote", spec_ref.remote.as_deref().unwrap_or("<upstream>")),
            ),
            || {
                service.fetch(
                    repo_id,
                    spec_ref.clone(),
                    &progress,
                    &context.cancellation(),
                )
            },
        );
        let outcome = operation.map_err(|error| attach_repo_context(error, repo_id))?;
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

    let credential_gate = state.credential_gate.clone();
    let audit_database = state.database.clone();
    let job_id = state.jobs.spawn(jobs::reporter_for(app), move |context| {
        let progress = jobs::progress_sink(&context);
        let service = SyncService::new(
            &engines,
            forgedesk_storage::RepositoryStore::new(&database),
            snapshots.as_ref(),
        )
        .with_credential_gate(credential_gate.as_deref());
        let spec_ref = &spec;
        let operation = audit::record_with(
            &forgedesk_services::AuditLog::new(forgedesk_storage::OperationStore::new(
                &audit_database,
            )),
            ServicesAuditEntry::new(repo_id, "sync.pull").with_args(
                AuditArgs::new()
                    .text("remote", spec_ref.remote.as_deref().unwrap_or("<upstream>"))
                    .text(
                        "strategy",
                        match spec_ref.strategy {
                            PullStrategy::Merge => "merge",
                            PullStrategy::Rebase => "rebase",
                            PullStrategy::FastForwardOnly => "ff-only",
                        },
                    ),
            ),
            || {
                service.pull(
                    repo_id,
                    spec_ref.clone(),
                    &progress,
                    &context.cancellation(),
                )
            },
        );
        let outcome = operation.map_err(|error| attach_repo_context(error, repo_id))?;
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
    let credential_gate = state.credential_gate.clone();
    let job_id = state.jobs.spawn(jobs::reporter_for(app), move |context| {
        let progress = jobs::progress_sink(&context);
        let service = SyncService::new(
            &engines,
            forgedesk_storage::RepositoryStore::new(&database),
            snapshots.as_ref(),
        )
        .with_credential_gate(credential_gate.as_deref());
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
        let outcome = operation.map_err(|error| attach_repo_context(error, repo_id))?;
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use forgedesk_domain::{ErrorCode, FixAction};

    /// 前端会照 `command` + `args` 直接 invoke：参数错了，按钮点下去只会得到
    /// 一条 `VALIDATION`，用户看到的则是"点了没反应"。
    #[test]
    fn the_test_connection_action_carries_the_repository_id() {
        let error = AppError::from_code(ErrorCode::SshKeyRejected);

        let enriched = attach_repo_context(error, 42);

        let action = enriched.actions.first().expect("应当补上一个动作");
        assert_eq!(action.command, "credential_test_remote");
        assert_eq!(
            action
                .args
                .as_ref()
                .and_then(|args| args.get("repoId"))
                .and_then(serde_json::Value::as_i64),
            Some(42)
        );
    }

    #[test]
    fn an_error_that_already_carries_specific_actions_is_left_alone() {
        // `PUSH_REJECTED` 的三条修复路径是更具体的出口，不能被"测试连接"叠加进来
        let error = AppError::from_code(ErrorCode::PushRejected).with_action(FixAction::new(
            "fetch-first",
            "errors:actions.pushFetchFirst",
            "git_fetch",
        ));

        let enriched = attach_repo_context(error, 42);

        assert_eq!(enriched.actions.len(), 1);
        assert_eq!(enriched.actions[0].command, "git_fetch");
    }

    #[test]
    fn codes_without_default_actions_pass_through_unchanged() {
        let error = AppError::from_code(ErrorCode::Network);

        let enriched = attach_repo_context(error.clone(), 42);

        assert!(enriched.actions.is_empty());
        assert_eq!(enriched.code, error.code);
        assert_eq!(enriched.message, error.message);
    }
}
