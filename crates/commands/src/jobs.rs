//! 长任务相关命令与 Tauri 事件上报（`job_cancel` 与 `job:*` 事件）。
//!
//! # 事件名与载荷是**契约**
//!
//! 事件名与字段形状登记在 `docs/API.md` §3，前端按同一份契约订阅。
//! 载荷一律 camelCase，且**不携带用户可见文案**——`phase` 是阶段标识，
//! 界面按它选 i18n 文案。
//!
//! # 为什么 reporter 在命令层
//!
//! `forgedesk-jobs` 刻意不依赖 Tauri（理由见该 crate 文档）：它只定义
//! [`JobReporter`] 出口。把"出口 → Tauri 事件"的适配放在这里，
//! 是为了让任务编排逻辑能在纯 Rust 测试里跑完。

use std::sync::Arc;

use forgedesk_domain::AppResult;
use forgedesk_jobs::{JobEvent, JobId, JobReporter};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::state::AppState;

/// 进度事件名。
pub const EVENT_JOB_PROGRESS: &str = "job:progress";
/// 任务成功结束的事件名。
pub const EVENT_JOB_DONE: &str = "job:done";
/// 任务失败结束的事件名。
pub const EVENT_JOB_FAILED: &str = "job:failed";

/// `job:done` 的载荷。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct JobDonePayload {
    job_id: JobId,
    result: serde_json::Value,
}

/// `job:failed` 的载荷。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct JobFailedPayload {
    job_id: JobId,
    error: forgedesk_domain::AppError,
}

/// 把任务事件投递成 Tauri 事件。
pub struct TauriJobReporter {
    app: AppHandle,
}

impl TauriJobReporter {
    /// 绑定到某个应用句柄。
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl std::fmt::Debug for TauriJobReporter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TauriJobReporter").finish_non_exhaustive()
    }
}

impl JobReporter for TauriJobReporter {
    fn report(&self, event: JobEvent) {
        let (name, payload) = match event {
            JobEvent::Progress(progress) => (EVENT_JOB_PROGRESS, serde_json::to_value(progress)),
            JobEvent::Done { job_id, result } => (
                EVENT_JOB_DONE,
                serde_json::to_value(JobDonePayload { job_id, result }),
            ),
            JobEvent::Failed { job_id, error } => (
                EVENT_JOB_FAILED,
                serde_json::to_value(JobFailedPayload { job_id, error }),
            ),
        };

        // 序列化失败只可能来自我们自己定义的类型（没有非字符串的 map key），
        // 但仍要处理：这里 panic 会让任务线程带着"未上报的终态"消失
        match payload {
            Ok(value) => {
                if let Err(error) = self.app.emit(name, value) {
                    tracing::warn!(%error, event = name, "投递任务事件失败");
                }
            }
            Err(error) => {
                tracing::warn!(%error, event = name, "序列化任务事件失败");
            }
        }
    }
}

/// 请求取消一个正在运行的任务。
///
/// 能力等级：`ReadOnly`（只改本进程内的任务状态，不碰仓库、不碰网络）。
///
/// 参数 `jobId` 来自任务创建时返回的 id；未知或已结束的 id 返回 `false`
/// 而不是报错——界面据此显示"任务已结束"，这比一个错误提示更有用。
#[tauri::command]
pub fn job_cancel(state: State<'_, AppState>, job_id: String) -> AppResult<bool> {
    Ok(state.jobs.cancel(&JobId::parse(job_id)))
}

/// 构造一个交给 `JobRunner` 的事件上报口。
pub(crate) fn reporter_for(app: AppHandle) -> Arc<dyn JobReporter> {
    Arc::new(TauriJobReporter::new(app))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use forgedesk_domain::{AppError, ErrorCode};
    use forgedesk_jobs::{JobEvent, JobId, JobProgress};

    use super::{EVENT_JOB_DONE, EVENT_JOB_FAILED, EVENT_JOB_PROGRESS};

    #[test]
    fn event_names_match_the_documented_contract() {
        // 前端按这些名字 listen，改名等于破坏契约
        assert_eq!(EVENT_JOB_PROGRESS, "job:progress");
        assert_eq!(EVENT_JOB_DONE, "job:done");
        assert_eq!(EVENT_JOB_FAILED, "job:failed");
    }

    #[test]
    fn progress_payload_is_camel_case_and_carries_no_user_facing_prose() {
        let progress = JobProgress {
            job_id: JobId::parse("job-1"),
            phase: "receiving".to_owned(),
            current: Some(3),
            total: Some(9),
            message: Some("Receiving objects: 33% (3/9)".to_owned()),
        };

        let json = serde_json::to_value(&progress).unwrap();
        assert_eq!(json["jobId"], "job-1");
        assert_eq!(json["phase"], "receiving");
        assert_eq!(json["current"], 3);
        assert_eq!(json["total"], 9);
    }

    #[test]
    fn done_and_failed_payloads_expose_the_documented_shape() {
        let done = serde_json::to_value(super::JobDonePayload {
            job_id: JobId::parse("job-2"),
            result: serde_json::json!({ "recordId": 7 }),
        })
        .unwrap();
        assert_eq!(done["jobId"], "job-2");
        assert_eq!(done["result"]["recordId"], 7);

        let failed = serde_json::to_value(super::JobFailedPayload {
            job_id: JobId::parse("job-3"),
            error: AppError::new(ErrorCode::Network, "could not resolve host"),
        })
        .unwrap();
        assert_eq!(failed["jobId"], "job-3");
        // 错误形状必须与 AppError 一致（docs/API.md §1.1）
        assert_eq!(failed["error"]["code"], "NETWORK");
        assert_eq!(failed["error"]["retryable"], true);
    }

    #[test]
    fn a_progress_event_serialises_without_the_message_when_absent() {
        let event = JobEvent::Progress(JobProgress {
            job_id: JobId::parse("job-4"),
            phase: "counting".to_owned(),
            current: None,
            total: None,
            message: None,
        });

        let JobEvent::Progress(progress) = event else {
            panic!("应仍是进度事件");
        };
        let json = serde_json::to_value(progress).unwrap();
        assert!(json.get("message").is_none(), "缺省字段不应出现：{json}");
        assert!(json["current"].is_null());
    }
}
