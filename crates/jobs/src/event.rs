//! 任务标识、进度载荷与事件出口。

use forgedesk_domain::AppError;
use serde::{Deserialize, Serialize};

/// 任务标识。
///
/// 由后端生成、前端只做透传：前端自造 id 会让"同一个任务被点两次"产生两条
/// 互不相关的记录（`src/stores/jobStore.ts` 的约定）。
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct JobId(String);

impl JobId {
    /// 生成一个新的任务 id（UUID v4）。
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }

    /// 从外部字符串还原（IPC 参数里的 `jobId`）。
    ///
    /// 不做格式校验：id 只是一个不透明的关联键，非法值只会导致"找不到该任务"，
    /// 而拒绝它并不会让用户得到更有用的信息。
    pub fn parse(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// 字符串形式。
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for JobId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for JobId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// 一条进度更新（`job:progress` 的载荷）。
///
/// 字段刻意保持"可序列化且无文案"：`phase` 是**阶段标识**（如 `receiving`），
/// 不是给用户看的句子；用户可见文案由前端按 `phase` 走 i18n
/// （`docs/API.md` §3 的"事件不携带用户可见文案"）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobProgress {
    /// 所属任务。
    pub job_id: JobId,
    /// 阶段标识（稳定短名，前端据此选文案）。
    pub phase: String,
    /// 已完成的量；无法量化时为 `None`。
    pub current: Option<u64>,
    /// 总量；git 在开始阶段可能给不出，因此是 `None`。
    pub total: Option<u64>,
    /// 原始进度行（**已脱敏**）。保留它是为了让"进度卡住"时能看到真实输出。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl JobProgress {
    /// 完成比例（0.0–1.0）；总量未知或为 0 时返回 `None`。
    ///
    /// 与 `ProgressEvent::ratio` 同一个语义：`None` 表示"进度条应显示为不确定状态"，
    /// 而不是"0%"——后者会让进度条在开始时看起来卡住。
    pub fn ratio(&self) -> Option<f64> {
        match (self.current, self.total) {
            (Some(current), Some(total)) if total > 0 => {
                Some((current as f64 / total as f64).clamp(0.0, 1.0))
            }
            _ => None,
        }
    }
}

/// 任务事件。
///
/// 三个变体对应 `docs/API.md` §3 事件表里的三条事件，由宿主（`commands` 层）
/// 决定各自的事件名与载荷形状。
#[derive(Debug, Clone)]
pub enum JobEvent {
    /// 进度更新。
    Progress(JobProgress),
    /// 成功结束，携带任务的返回值。
    ///
    /// 用 `serde_json::Value` 而不是泛型：事件要在**不知道任务类型**的地方
    /// （reporter）被投递，泛型参数在那里无法确定。
    Done {
        /// 所属任务。
        job_id: JobId,
        /// 任务返回值（已序列化）。
        result: serde_json::Value,
    },
    /// 失败结束。
    Failed {
        /// 所属任务。
        job_id: JobId,
        /// 失败原因（已分类、已脱敏）。
        error: AppError,
    },
}

impl JobEvent {
    /// 该事件属于哪个任务。
    pub fn job_id(&self) -> &JobId {
        match self {
            Self::Progress(progress) => &progress.job_id,
            Self::Done { job_id, .. } | Self::Failed { job_id, .. } => job_id,
        }
    }

    /// 是否是终态事件（不再有后续）。
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Done { .. } | Self::Failed { .. })
    }
}

/// 事件出口。
///
/// 实现必须是 `Send + Sync`：任务在独立线程上跑，而 reporter 通常被 `Arc`
/// 共享给多个任务（例如同一个 Tauri `AppHandle`）。
pub trait JobReporter: Send + Sync {
    /// 投递一条事件。
    ///
    /// 实现**不得阻塞**：它在任务线程上被调用，阻塞会让进度反馈拖慢任务本身。
    fn report(&self, event: JobEvent);
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{JobEvent, JobId, JobProgress};
    use forgedesk_domain::{AppError, ErrorCode};

    #[test]
    fn job_ids_are_unique_and_round_trip_through_strings() {
        let first = JobId::new();
        let second = JobId::new();

        assert_ne!(first, second, "两个任务不能拿到同一个 id");
        assert_eq!(JobId::parse(first.as_str()), first);
        assert_eq!(first.to_string(), first.as_str());
    }

    #[test]
    fn progress_serialises_to_camel_case_for_the_frontend() {
        let progress = JobProgress {
            job_id: JobId::parse("job-1"),
            phase: "receiving".to_owned(),
            current: Some(10),
            total: Some(40),
            message: None,
        };

        let json = serde_json::to_string(&progress).unwrap();
        assert!(json.contains("\"jobId\":\"job-1\""), "实际：{json}");
        assert!(json.contains("\"current\":10"));
        assert!(!json.contains("job_id"));
        assert!(
            !json.contains("message"),
            "message 缺省时不应出现在载荷里：{json}"
        );
    }

    #[test]
    fn ratio_is_none_until_the_total_is_known() {
        let mut progress = JobProgress {
            job_id: JobId::parse("job-1"),
            phase: "counting".to_owned(),
            current: Some(3),
            total: None,
            message: None,
        };
        assert_eq!(progress.ratio(), None, "总量未知时应显示不确定进度");

        progress.total = Some(0);
        assert_eq!(progress.ratio(), None, "总量为 0 时不能除以零");

        progress.total = Some(6);
        assert_eq!(progress.ratio(), Some(0.5));

        progress.current = Some(99);
        assert_eq!(progress.ratio(), Some(1.0), "比例必须被夹在 0–1");
    }

    #[test]
    fn terminal_events_are_distinguished_from_progress() {
        let progress = JobEvent::Progress(JobProgress {
            job_id: JobId::parse("a"),
            phase: "receiving".to_owned(),
            current: None,
            total: None,
            message: None,
        });
        assert!(!progress.is_terminal());
        assert_eq!(progress.job_id().as_str(), "a");

        let done = JobEvent::Done {
            job_id: JobId::parse("b"),
            result: serde_json::json!({ "ok": true }),
        };
        assert!(done.is_terminal());
        assert_eq!(done.job_id().as_str(), "b");

        let failed = JobEvent::Failed {
            job_id: JobId::parse("c"),
            error: AppError::new(ErrorCode::Network, "boom"),
        };
        assert!(failed.is_terminal());
        assert_eq!(failed.job_id().as_str(), "c");
    }
}
