//! 托管平台 Actions 命令（T4.9）：run 列表、job 列表、取消/重跑与**流式日志**。
//!
//! # 日志流式加载（M4 验收：>5MB 不卡 UI）
//!
//! `repo_actions_job_logs` 立即返回 `jobId`（JobRunner 长任务），任务体里
//! 用专属 current-thread 运行时消费 `reqwest` 的字节流：
//!
//! - **按完整行切分**后经 `actions:log-chunk` 事件推送（`text` 字段）。
//!   按行而不是按字节切，是为了 UTF-8 不被块边界劈开；尾部不完整行留在
//!   缓冲里等下一块。事件里带 `totalLines`（累计行数）与 `jobId`（前端
//!   按它过滤，多个日志窗口互不串流）。
//! - **取消**走通用 `job_cancel`：块间检查取消令牌，取消即 `CANCELLED`
//!   结束（`job:failed`），已推送的行已经在前端，不回滚。
//! - 结束时 `job:done` 载荷 `{ totalLines }`；4xx（如 run 未完成、日志
//!   未生成）按全局映射报错。
//!
//! 事件名 `actions:log-chunk` 是契约（docs/API.md §3），载荷不带用户可见
//! 文案；`text` 是日志原文（CI 日志不是模板，直接透传）。
//!
//! 其余命令与 PR/Issue 同一形态：全部 `Network`、不走 SnapshotManager、
//! 令牌解析在 services 层。

use serde::Deserialize;
use tauri::{AppHandle, Emitter, State};

use forgedesk_domain::{AppError, AppResult, ErrorCode};
use forgedesk_provider::{RunJob, RunPage};

use crate::repository::JobIdDto;
use crate::state::AppState;

/// 日志分块事件名（契约：docs/API.md §3）。
pub const EVENT_ACTIONS_LOG_CHUNK: &str = "actions:log-chunk";

/// `actions:log-chunk` 的载荷。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ActionsLogChunkPayload {
    job_id: String,
    /// 本块的完整行（含行尾换行；UTF-8 已按行边界安全解码）。
    text: String,
    /// 截至本块的累计行数。
    total_lines: u64,
}

/// `repo_actions_runs_list` 的请求体。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunListRequest {
    /// 站点。
    pub host: String,
    /// 所有者。
    pub owner: String,
    /// 仓库名。
    pub repo: String,
    /// 本地仓库 id（可省）。
    #[serde(default)]
    pub repo_id: Option<i64>,
    /// 页码。
    #[serde(default)]
    pub page: Option<u32>,
    /// 每页条数。
    #[serde(default)]
    pub per_page: Option<u32>,
}

fn validate_host(host: &str) -> AppResult<String> {
    let host = host.trim().to_ascii_lowercase();
    if host.is_empty() {
        return Err(AppError::new(
            ErrorCode::Validation,
            "provider host must not be empty",
        ));
    }
    Ok(host)
}

fn validate_run_id(run_id: u64) -> AppResult<u64> {
    if run_id == 0 {
        return Err(AppError::new(
            ErrorCode::Validation,
            "run id must be positive",
        ));
    }
    Ok(run_id)
}

/// 列出 workflow run。能力等级：`Network`。
#[tauri::command]
pub async fn repo_actions_runs_list(
    state: State<'_, AppState>,
    request: RunListRequest,
) -> AppResult<RunPage> {
    let host = validate_host(&request.host)?;
    let target = forgedesk_services::host_repos::RemoteRepoRef {
        host,
        repo_id: request.repo_id,
        owner: request.owner,
        repo: request.repo,
    };
    state
        .host_repos
        .list_runs(&target, request.page, request.per_page)
        .await
}

/// 一个 run 的 job 列表。能力等级：`Network`。
#[tauri::command]
pub async fn repo_actions_run_jobs(
    state: State<'_, AppState>,
    host: String,
    owner: String,
    repo: String,
    run_id: u64,
    repo_id: Option<i64>,
) -> AppResult<Vec<RunJob>> {
    let host = validate_host(&host)?;
    let run_id = validate_run_id(run_id)?;
    state
        .host_repos
        .list_run_jobs(&host, repo_id, &owner, &repo, run_id)
        .await
}

/// 取消一个 run。能力等级：`Network`。
#[tauri::command]
pub async fn repo_actions_run_cancel(
    state: State<'_, AppState>,
    host: String,
    owner: String,
    repo: String,
    run_id: u64,
    repo_id: Option<i64>,
) -> AppResult<()> {
    let host = validate_host(&host)?;
    let run_id = validate_run_id(run_id)?;
    state
        .host_repos
        .cancel_run(&host, repo_id, &owner, &repo, run_id)
        .await
}

/// 重跑一个 run。能力等级：`Network`。
#[tauri::command]
pub async fn repo_actions_run_rerun(
    state: State<'_, AppState>,
    host: String,
    owner: String,
    repo: String,
    run_id: u64,
    repo_id: Option<i64>,
) -> AppResult<()> {
    let host = validate_host(&host)?;
    let run_id = validate_run_id(run_id)?;
    state
        .host_repos
        .rerun_run(&host, repo_id, &owner, &repo, run_id)
        .await
}

/// 流式加载一个 job 的日志（长任务）。能力等级：`Network`。
///
/// 返回 `jobId`；日志行经 `actions:log-chunk` 事件分块推送，结束见模块
/// 文档。取消走通用 `job_cancel`。
#[tauri::command]
pub fn repo_actions_job_logs(
    state: State<'_, AppState>,
    app: AppHandle,
    host: String,
    owner: String,
    repo: String,
    job_id: u64,
    repo_id: Option<i64>,
) -> AppResult<JobIdDto> {
    let host = validate_host(&host)?;
    if job_id == 0 {
        return Err(AppError::new(
            ErrorCode::Validation,
            "job id must be positive",
        ));
    }
    let repos = std::sync::Arc::clone(&state.host_repos);
    let event_job_id = job_id.to_string();
    let job_id = state
        .jobs
        .spawn(crate::jobs::reporter_for(app.clone()), move |context| {
            context.progress("downloading", None, None, None);
            let app = app.clone();
            // 日志流是 async 而任务体是阻塞线程：给这次下载一个专属运行时
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| {
                    AppError::new(
                        ErrorCode::Internal,
                        "failed to create async runtime for log streaming",
                    )
                    .with_detail(error.to_string())
                })?;
            runtime.block_on(async move {
                let mut response = repos
                    .job_logs_response(&host, repo_id, &owner, &repo, job_id)
                    .await?;

                let mut pending: Vec<u8> = Vec::new();
                let mut total_lines: u64 = 0;
                let emit_chunk = |text: String, total_lines: u64, app: &AppHandle| {
                    // 事件投递失败不终止下载：日志已在远端，前端只是这一块没收到
                    let payload = ActionsLogChunkPayload {
                        job_id: event_job_id.clone(),
                        text,
                        total_lines,
                    };
                    if let Err(error) = app.emit(EVENT_ACTIONS_LOG_CHUNK, payload) {
                        tracing::warn!(%error, "投递日志分块事件失败");
                    }
                };

                loop {
                    if context.is_cancelled() {
                        return Err(AppError::new(
                            ErrorCode::Cancelled,
                            "log download cancelled",
                        ));
                    }
                    let chunk = match response.chunk().await {
                        Ok(Some(chunk)) => chunk,
                        Ok(None) => break,
                        Err(error) => {
                            return Err(AppError::new(
                                ErrorCode::Network,
                                "log stream failed mid-download",
                            )
                            .with_detail(error.to_string()))
                        }
                    };
                    pending.extend_from_slice(&chunk);
                    // 只推完整行：尾部不完整行（可能是半个 UTF-8 字符）留待下一块
                    if let Some(pos) = pending.iter().rposition(|&byte| byte == b'\n') {
                        let text = String::from_utf8_lossy(&pending[..=pos]).into_owned();
                        pending.drain(..=pos);
                        total_lines += text.lines().count() as u64;
                        emit_chunk(text, total_lines, &app);
                    }
                }
                if !pending.is_empty() {
                    let text = String::from_utf8_lossy(&pending).into_owned();
                    total_lines += text.lines().count() as u64;
                    emit_chunk(text, total_lines, &app);
                }
                Ok(serde_json::json!({ "totalLines": total_lines }))
            })
        });

    Ok(JobIdDto {
        job_id: job_id.as_str().to_owned(),
    })
}
