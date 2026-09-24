//! 命令层的应用级共享状态。
//!
//! 由 `src-tauri` 在启动时构建（打开数据库 + 执行迁移 + 解析日志目录 + 建引擎 +
//! 建任务执行器）并通过 `manage` 注入；命令只借出只读引用。
//!
//! 为什么单独一个模块：T0.7 时它住在 `settings.rs` 里，而 T0.8 的日志命令也要用它。
//! 一旦多个命令族共享同一个状态，"它归谁"就不该由先写它的那个模块决定——
//! 否则每加一个命令族都要去改不相干的文件。
//!
//! # 为什么把引擎与任务执行器也放进来
//!
//! 两者都是**有状态的重资源**，必须全进程共享：
//!
//! - `GitEngines` 里的 `CliGitEngine` 持有一个驱动异步进程执行器的线程；
//!   每个命令各建一个等于每次调用都起一条线程。
//! - `JobRunner` 的注册表要能被 `job_cancel` 找到——每个命令各建一个注册表，
//!   取消就永远找不到目标。
//!
//! `OpenRepoRegistry` 同理：它是"哪些仓库正开着"的唯一真相源。

use std::path::PathBuf;
use std::sync::Arc;

use forgedesk_jobs::JobRunner;
use forgedesk_services::repository::OpenRepoRegistry;
use forgedesk_services::{
    CommitPlanRegistry, CommitService, GitEngines, RepositoryService, StagingService,
    WorkspaceService,
};
use forgedesk_snapshot::SnapshotManager;
use forgedesk_storage::{Database, OperationStore, RepositoryStore};

/// 应用级共享状态。
#[derive(Debug)]
pub struct AppState {
    /// 本地数据库（并发策略见 `forgedesk_storage::Database`）。
    pub database: Arc<Database>,
    /// 日志目录（由宿主用 Tauri 的 `app_log_dir()` 解析后传入）。
    ///
    /// 为什么由宿主传入而不是在这里解析：`platform` crate 刻意不依赖 Tauri，
    /// 这样日志与会话逻辑能在纯 Rust 测试里跑，不必启动桌面运行时。
    pub log_dir: PathBuf,
    /// 读引擎（libgit2）与写引擎（系统 git CLI）。
    pub engines: Arc<GitEngines>,
    /// 长任务执行器（进度、取消、结果上报）。
    pub jobs: Arc<JobRunner>,
    /// 当前会话中已打开的仓库。
    pub open_repos: Arc<OpenRepoRegistry>,
    /// 快照管理器。
    ///
    /// M3 / T1.9 之前注入的是"未启用"实现（`NoopSnapshotManager`）——提交链路已经
    /// 在正确的位置调用它，只是它如实回答"没有快照"。换成真实实现不需要改上层。
    pub snapshots: Arc<dyn SnapshotManager>,
    /// 待执行的提交计划（进程内、带 5 分钟有效期）。
    ///
    /// 必须全进程共享：`commit_prepare` 与 `commit_execute` 是两次独立调用，
    /// 各自 new 一个注册表会让 `execute` 永远找不到 `prepare` 放进去的计划。
    pub commit_plans: Arc<CommitPlanRegistry>,
}

impl AppState {
    /// 绑定当前状态构造仓库用例服务。
    ///
    /// 为什么在这里提供工厂方法而不是让每个命令自己拼：三个字段（引擎、
    /// 仓储、已打开集合）的**借用关系**必须一致，散在各处拼装迟早出现
    /// "某个命令用了另一个注册表"这种极难发现的问题。
    pub fn repository_service(&self) -> RepositoryService<'_> {
        RepositoryService::new(
            &self.engines,
            RepositoryStore::new(&self.database),
            &self.open_repos,
        )
    }

    /// 绑定当前状态构造工作区用例服务（状态 / 文件级暂存 / 放弃）。
    pub fn workspace_service(&self) -> WorkspaceService<'_> {
        WorkspaceService::new(
            &self.engines,
            RepositoryStore::new(&self.database),
            &self.open_repos,
        )
    }

    /// 绑定当前状态构造部分暂存服务（行级 / 块级，T1.6）。
    ///
    /// `StagingService` 内部持有一个 `WorkspaceService`（复用 `repo_id → 工作区路径`
    /// 的解析），因此两个工厂方法必须用**同一批引擎与同一个注册表** —— 各自 new 一个
    /// 会变成"两个服务看到两个不同的世界"。
    pub fn staging_service(&self) -> StagingService<'_> {
        StagingService::new(self.workspace_service(), &self.engines)
    }

    /// 绑定当前状态构造提交用例服务（prepare / execute / 提示，T1.7）。
    ///
    /// 与其它工厂方法共用同一批引擎、同一个数据库与**同一个计划注册表**：
    /// 注册表是 `prepare` 与 `execute` 之间的唯一纽带，换一个就等于把计划丢了。
    pub fn commit_service(&self) -> CommitService<'_> {
        CommitService::new(
            &self.engines,
            RepositoryStore::new(&self.database),
            OperationStore::new(&self.database),
            self.snapshots.as_ref(),
            &self.commit_plans,
        )
    }
}
