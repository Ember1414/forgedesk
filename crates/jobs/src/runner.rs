//! 任务执行器：注册、取消与结果上报。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use forgedesk_domain::{AppError, AppResult, ErrorCode};
use serde::Serialize;
use tokio_util::sync::CancellationToken;

use crate::event::{JobEvent, JobId, JobProgress, JobReporter};

/// 交给任务体的上下文。
///
/// 任务体通过它做三件事：报进度、检查是否被取消、拿到取消令牌传给
/// `GitRunOpts::with_cancel`（真正让 git 子进程被杀掉的那一步）。
#[derive(Clone)]
pub struct JobContext {
    id: JobId,
    reporter: Arc<dyn JobReporter>,
    cancellation: CancellationToken,
}

impl std::fmt::Debug for JobContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JobContext")
            .field("id", &self.id)
            .field("cancelled", &self.cancellation.is_cancelled())
            .finish_non_exhaustive()
    }
}

impl JobContext {
    /// 任务标识。
    pub fn id(&self) -> &JobId {
        &self.id
    }

    /// 取消令牌（交给 `GitRunOpts::with_cancel`）。
    pub fn cancellation(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    /// 是否已被请求取消。
    pub fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }

    /// 被取消时返回 [`ErrorCode::Cancelled`]，否则返回 `Ok(())`。
    ///
    /// 任务体应在**可中断的边界**上调用它（每次循环、每个阶段之间），
    /// 这样取消的响应时间取决于边界之间的工作量，而不是整个任务的长度。
    pub fn ensure_not_cancelled(&self) -> AppResult<()> {
        if self.is_cancelled() {
            Err(AppError::new(
                ErrorCode::Cancelled,
                "the job was cancelled before it finished",
            )
            .with_retryable(false))
        } else {
            Ok(())
        }
    }

    /// 投递一条进度。
    ///
    /// `phase` 是**阶段标识**（`receiving` / `checkout`…），不是用户可见文案。
    pub fn progress(
        &self,
        phase: impl Into<String>,
        current: Option<u64>,
        total: Option<u64>,
        message: Option<String>,
    ) {
        self.reporter.report(JobEvent::Progress(JobProgress {
            job_id: self.id.clone(),
            phase: phase.into(),
            current,
            total,
            message,
        }));
    }
}

/// 在跑任务的注册表。
///
/// 为什么单独一个类型而不是塞进 [`JobRunner`]：应用退出时要"取消全部任务"
/// （T1.10 起还会按仓库取消），而测试要能直接检查"注册表里还剩几个"。
#[derive(Debug, Default)]
pub struct JobRegistry {
    running: Mutex<HashMap<JobId, CancellationToken>>,
}

impl JobRegistry {
    /// 空的注册表。
    pub fn new() -> Self {
        Self::default()
    }

    /// 登记一个任务。
    pub fn register(&self, id: JobId, cancellation: CancellationToken) {
        if let Ok(mut running) = self.running.lock() {
            running.insert(id, cancellation);
        }
    }

    /// 注销一个任务（任务已进入终态）。
    pub fn finish(&self, id: &JobId) {
        if let Ok(mut running) = self.running.lock() {
            running.remove(id);
        }
    }

    /// 请求取消一个任务；返回它是否**正在运行**。
    ///
    /// 返回 `false` 有两种情况（都不可区分，也不需要区分）：任务不存在，
    /// 或它已经结束了。界面据此显示"任务已结束"而不是"取消成功"。
    pub fn cancel(&self, id: &JobId) -> bool {
        let Ok(running) = self.running.lock() else {
            return false;
        };
        match running.get(id) {
            Some(token) => {
                token.cancel();
                true
            }
            None => false,
        }
    }

    /// 请求取消全部任务（应用退出时调用）。
    pub fn cancel_all(&self) {
        if let Ok(running) = self.running.lock() {
            for token in running.values() {
                token.cancel();
            }
        }
    }

    /// 当前在跑的任务数量。
    pub fn running_count(&self) -> usize {
        self.running.lock().map(|map| map.len()).unwrap_or(0)
    }
}

/// 任务执行器。
///
/// 一个进程一个（放在 `AppState` 里），持有共享的 [`JobRegistry`]。
#[derive(Debug, Default)]
pub struct JobRunner {
    registry: Arc<JobRegistry>,
}

impl JobRunner {
    /// 创建执行器。
    pub fn new() -> Self {
        Self::default()
    }

    /// 共享的注册表（宿主可以在退出时调用 `cancel_all`）。
    pub fn registry(&self) -> &Arc<JobRegistry> {
        &self.registry
    }

    /// 请求取消一个任务；返回它是否正在运行。
    pub fn cancel(&self, id: &JobId) -> bool {
        self.registry.cancel(id)
    }

    /// 起一个任务，立即返回任务 id。
    ///
    /// 任务体在**独立线程**上运行（理由见 crate 文档）。它返回的 `Ok(T)`
    /// 会被序列化成 `job:done` 的载荷，`Err(AppError)` 变成 `job:failed`。
    ///
    /// 任务体**不要**自己投递终态事件：终态由这里统一投递，否则
    /// "任务体报成功、注册表还留着"这类不一致迟早会出现。
    pub fn spawn<T, F>(&self, reporter: Arc<dyn JobReporter>, body: F) -> JobId
    where
        T: Serialize,
        F: FnOnce(JobContext) -> AppResult<T> + Send + 'static,
    {
        let id = JobId::new();
        let cancellation = CancellationToken::new();
        self.registry.register(id.clone(), cancellation.clone());

        let context = JobContext {
            id: id.clone(),
            reporter: Arc::clone(&reporter),
            cancellation,
        };
        let registry = Arc::clone(&self.registry);
        // 线程闭包拿走一个副本，函数仍然把 id 还给调用方
        let thread_id = id.clone();

        std::thread::spawn(move || {
            let event = match body(context) {
                Ok(value) => match serde_json::to_value(value) {
                    Ok(result) => JobEvent::Done {
                        job_id: thread_id.clone(),
                        result,
                    },
                    Err(error) => JobEvent::Failed {
                        job_id: thread_id.clone(),
                        error: AppError::new(
                            ErrorCode::Internal,
                            "could not serialise the job result",
                        )
                        .with_detail(error.to_string())
                        .with_retryable(false),
                    },
                },
                Err(error) => JobEvent::Failed {
                    job_id: thread_id.clone(),
                    error,
                },
            };

            registry.finish(&thread_id);
            reporter.report(event);
        });

        id
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use forgedesk_domain::{AppError, AppResult, ErrorCode};

    use super::{JobRegistry, JobRunner};
    use crate::event::{JobEvent, JobId, JobReporter};

    /// 把事件收进 Vec 的 reporter（测试替身）。
    #[derive(Default)]
    struct CollectingReporter {
        events: Mutex<Vec<JobEvent>>,
        reported: AtomicUsize,
    }

    impl CollectingReporter {
        fn take(&self) -> Vec<JobEvent> {
            std::mem::take(&mut *self.events.lock().unwrap())
        }
    }

    impl JobReporter for CollectingReporter {
        fn report(&self, event: JobEvent) {
            self.reported.fetch_add(1, Ordering::SeqCst);
            self.events.lock().unwrap().push(event);
        }
    }

    /// 等到 reporter 收到一个终态事件，或超时。
    fn wait_for_terminal(reporter: &CollectingReporter) -> Vec<JobEvent> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let events = reporter.take();
            if events.iter().any(JobEvent::is_terminal) {
                return events;
            }
            if Instant::now() > deadline {
                panic!("等待终态事件超时");
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_successful_job_reports_progress_then_done() {
        let reporter = Arc::new(CollectingReporter::default());
        let runner = JobRunner::new();

        let id = runner.spawn(Arc::clone(&reporter) as Arc<dyn JobReporter>, |ctx| {
            ctx.progress("receiving", Some(1), Some(2), None);
            ctx.ensure_not_cancelled()?;
            Ok(serde_json::json!({ "path": "/tmp/repo" }))
        });

        let events = wait_for_terminal(&reporter);

        assert_eq!(events.len(), 2, "应收到一条进度与一条终态：{events:?}");
        match &events[0] {
            JobEvent::Progress(progress) => {
                assert_eq!(progress.job_id, id);
                assert_eq!(progress.phase, "receiving");
                assert_eq!(progress.current, Some(1));
            }
            other => panic!("第一条应是进度事件，实际：{other:?}"),
        }
        match &events[1] {
            JobEvent::Done { job_id, result } => {
                assert_eq!(job_id, &id);
                assert_eq!(result["path"], "/tmp/repo");
            }
            other => panic!("第二条应是完成事件，实际：{other:?}"),
        }
    }

    #[test]
    fn a_failing_job_reports_the_error_and_keeps_its_code() {
        let reporter = Arc::new(CollectingReporter::default());
        let runner = JobRunner::new();

        runner.spawn(Arc::clone(&reporter) as Arc<dyn JobReporter>, |_ctx| {
            Err::<(), _>(AppError::new(ErrorCode::Network, "could not resolve host"))
        });

        let events = wait_for_terminal(&reporter);
        match &events[0] {
            JobEvent::Failed { error, .. } => {
                assert_eq!(error.code, ErrorCode::Network);
                assert!(error.retryable, "网络错误的可重试标记不应丢失");
            }
            other => panic!("应收到失败事件，实际：{other:?}"),
        }
    }

    #[test]
    fn a_finished_job_leaves_the_registry_empty() {
        let reporter = Arc::new(CollectingReporter::default());
        let runner = JobRunner::new();

        let id = runner.spawn(Arc::clone(&reporter) as Arc<dyn JobReporter>, |_ctx| Ok(()));
        wait_for_terminal(&reporter);

        assert_eq!(runner.registry().running_count(), 0);
        assert!(!runner.cancel(&id), "已结束的任务不应能被取消");
    }

    #[test]
    fn cancelling_a_running_job_is_visible_to_its_body() {
        let reporter = Arc::new(CollectingReporter::default());
        let runner = JobRunner::new();
        let (started_tx, started_rx) = std::sync::mpsc::channel();

        let id = runner.spawn(Arc::clone(&reporter) as Arc<dyn JobReporter>, move |ctx| {
            started_tx.send(()).expect("通知启动失败");
            // 模拟"长任务在可中断边界上轮询"
            let deadline = Instant::now() + Duration::from_secs(5);
            while !ctx.is_cancelled() {
                assert!(Instant::now() < deadline, "取消信号没有到达任务体");
                std::thread::sleep(Duration::from_millis(5));
            }
            ctx.ensure_not_cancelled()?;
            Ok(())
        });

        started_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("任务没有启动");
        assert!(runner.cancel(&id), "正在运行的任务应能被取消");

        let events = wait_for_terminal(&reporter);
        match &events[0] {
            JobEvent::Failed { error, .. } => assert_eq!(error.code, ErrorCode::Cancelled),
            other => panic!("取消应产生失败事件，实际：{other:?}"),
        }
        assert_eq!(runner.registry().running_count(), 0);
    }

    #[test]
    fn cancelling_an_unknown_job_reports_false() {
        let runner = JobRunner::new();
        assert!(!runner.cancel(&JobId::parse("nope")));
    }

    #[test]
    fn the_registry_can_cancel_everything_at_once() {
        let registry = JobRegistry::new();
        let first = JobId::new();
        let second = JobId::new();
        let first_token = tokio_util::sync::CancellationToken::new();
        let second_token = tokio_util::sync::CancellationToken::new();
        registry.register(first.clone(), first_token.clone());
        registry.register(second.clone(), second_token.clone());
        assert_eq!(registry.running_count(), 2);

        registry.cancel_all();

        assert!(first_token.is_cancelled());
        assert!(second_token.is_cancelled());
        // 取消不等于注销：任务体还要跑完自己的清理路径
        assert_eq!(registry.running_count(), 2);

        registry.finish(&first);
        registry.finish(&second);
        assert_eq!(registry.running_count(), 0);
    }

    #[test]
    fn an_unserialisable_result_becomes_an_internal_failure() {
        /// 序列化时必定失败的返回值。
        struct Unserialisable;

        impl serde::Serialize for Unserialisable {
            fn serialize<S: serde::Serializer>(&self, _serializer: S) -> Result<S::Ok, S::Error> {
                Err(serde::ser::Error::custom("nope"))
            }
        }

        let reporter = Arc::new(CollectingReporter::default());
        let runner = JobRunner::new();

        runner.spawn(
            Arc::clone(&reporter) as Arc<dyn JobReporter>,
            |_ctx| -> AppResult<Unserialisable> { Ok(Unserialisable) },
        );

        let events = wait_for_terminal(&reporter);
        match &events[0] {
            JobEvent::Failed { error, .. } => assert_eq!(error.code, ErrorCode::Internal),
            other => panic!("应收到失败事件，实际：{other:?}"),
        }
    }
}
