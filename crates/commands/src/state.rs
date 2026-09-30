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
    AuditLog, BranchService, CommitDetailService, CommitPlanRegistry, CommitService,
    ConflictService, CredentialGate, CredentialsService, GitEngines, HistoryOpsService,
    HistoryService, LogPageCache, MergePlanRegistry, MergeService, RebaseService,
    RepositoryService, ResetPlanRegistry, StagingService, StashService, SyncService,
    WorkspaceService,
};
use forgedesk_snapshot::SnapshotManager;
use forgedesk_storage::{Database, OperationStore, RepositoryStore};

use crate::watch::WatcherRegistry;

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
    /// 快照管理器（T1.9 起是 ref 锚点实现，由宿主注入）。
    pub snapshots: Arc<dyn SnapshotManager>,
    /// 仓库文件监听注册表（T1.10）。
    ///
    /// 与 `OpenRepoRegistry` 分工不同：后者记录"哪些仓库开着"，这里持有
    /// "每份监听句柄"。句柄是**资源**（操作系统监听 + 一条线程），
    /// 必须全进程唯一——每个命令各建一个注册表等于每次调用都多一份监听。
    pub watchers: Arc<WatcherRegistry>,
    /// 待执行的提交计划（进程内、带 5 分钟有效期）。
    ///
    /// 必须全进程共享：`commit_prepare` 与 `commit_execute` 是两次独立调用，
    /// 各自 new 一个注册表会让 `execute` 永远找不到 `prepare` 放进去的计划。
    pub commit_plans: Arc<CommitPlanRegistry>,
    /// 凭据存储（T2.7）：密文在系统凭据库，索引在数据目录。
    ///
    /// 与 `credential_gate` 共享**同一份**存储实例：分成两个实例会让"刚保存的凭据"
    /// 在下一次网络操作里查不到（索引各写各的）。
    pub credentials: Arc<CredentialsService>,
    /// 凭据门（T2.7）：网络操作前解析注入方案，并记住连续认证失败次数。
    ///
    /// `None` 表示拿不到自身可执行文件路径（无法充当 askpass 程序）——
    /// 此时不做注入，网络操作退化为匿名/SSH。
    pub credential_gate: Option<Arc<CredentialGate>>,
    /// 待执行的重置计划（T2.8，进程内）。
    ///
    /// 与 `commit_plans` 同理：`git_reset_prepare` 与 `git_reset_execute` 是两次
    /// 独立调用，各自 new 一个注册表会让执行永远找不到刚刚预览过的计划。
    pub reset_plans: Arc<ResetPlanRegistry>,
    /// 待执行的合并计划（T3.4，进程内；与 reset_plans 同理）。
    pub merge_plans: Arc<MergePlanRegistry>,
    /// 日志分页缓存（T2.9）：累积各查询形状的 walk 前缀，深分页与重复首页
    /// 不再从 tip 重扫。
    ///
    /// Arc 而不是裸值，与 `engines`/`jobs` 同一理由：它是**有状态的全进程资源**，
    /// 且未来的长任务闭包只拿得到 Arc 克隆（T2.7 的"漏接"教训）。
    pub log_pages: Arc<LogPageCache>,
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
        // 克隆也要凭据：私有仓库没有它必然失败（T2.7）
        .with_credential_gate(self.credential_gate.as_deref())
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

    /// 绑定当前状态构造审计服务（T1.11）。
    ///
    /// 命令层的写操作统一经它留痕；查询与导出也走同一个实例，
    /// 保证"写"与"读"看到的是同一张表、同一套脱敏规则。
    pub fn audit_service(&self) -> AuditLog<'_> {
        AuditLog::new(OperationStore::new(&self.database))
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

    /// 绑定当前状态构造历史查询服务（T2.2）。
    ///
    /// 与其它工厂方法共用同一批引擎与同一个数据库；
    /// `HistoryService` 需要 `RepositoryStore` 来把 `repo_id` 解析为工作区路径。
    /// 日志分页缓存（T2.9）在这里接上：全进程共享同一份前缀。
    pub fn history_service(&self) -> HistoryService<'_> {
        HistoryService::new(&self.engines, RepositoryStore::new(&self.database))
            .with_log_cache(self.log_pages.as_ref())
    }

    /// 绑定当前状态构造远端同步服务（T2.6）。
    ///
    /// 快照管理器与提交/分支路径共享同一个实例：pull 的 `PreSync` 快照
    /// 落在同一份历史里。
    pub fn sync_service(&self) -> SyncService<'_> {
        SyncService::new(
            &self.engines,
            RepositoryStore::new(&self.database),
            self.snapshots.as_ref(),
        )
        // 有凭据门就接上（T2.7）：fetch/pull/push 才会带上保存过的凭据
        .with_credential_gate(self.credential_gate.as_deref())
    }

    /// 凭据存储（T2.7）：设置页的账号面板直接用它。
    pub fn credentials_service(&self) -> &CredentialsService {
        &self.credentials
    }

    /// 绑定当前状态构造储藏服务（T2.8）。
    pub fn stash_service(&self) -> StashService<'_> {
        StashService::new(
            &self.engines,
            RepositoryStore::new(&self.database),
            self.snapshots.as_ref(),
        )
    }

    /// 绑定当前状态构造冲突状态机服务（T3.1）。
    ///
    /// 快照管理器与 stash / sync 路径共享同一个实例：abort 的
    /// `PreHeadMove` 快照落在同一份历史里，回滚页一并列出。
    pub fn conflict_service(&self) -> ConflictService<'_> {
        ConflictService::new(
            &self.engines,
            RepositoryStore::new(&self.database),
            self.snapshots.as_ref(),
        )
    }

    /// 绑定当前状态构造历史操作服务（T2.8：拣选 / 反转 / 重置 / reflog）。
    ///
    /// 重置计划的注册表来自状态本身：与 `commit_plans` 同理，
    /// prepare 与 execute 必须看到同一个注册表。
    pub fn history_ops_service(&self) -> HistoryOpsService<'_> {
        HistoryOpsService::new(
            &self.engines,
            RepositoryStore::new(&self.database),
            self.snapshots.as_ref(),
            &self.reset_plans,
        )
    }

    /// 绑定当前状态构造合并服务（T3.4）。
    ///
    /// 快照管理器与 pull 路径共享同一实例：合并的 `PreSync` 快照
    /// 落在同一份历史里。
    pub fn merge_service(&self) -> MergeService<'_> {
        MergeService::new(
            &self.engines,
            RepositoryStore::new(&self.database),
            self.snapshots.as_ref(),
            &self.merge_plans,
        )
    }

    /// 绑定当前状态构造 rebase 执行服务（T3.7）。
    ///
    /// 快照管理器与 merge 路径共享同一实例：rebase 的 `PreHeadMove`
    /// 快照落在同一份历史里。
    pub fn rebase_service(&self) -> RebaseService<'_> {
        RebaseService::new(
            &self.engines,
            RepositoryStore::new(&self.database),
            self.snapshots.as_ref(),
        )
    }

    /// 绑定当前状态构造分支/标签管理服务（T2.5）。
    ///
    /// 快照管理器与提交路径共享同一个实例：Force 切换 / 未合并强删的
    /// `PreHeadMove` 快照和提交前的快照落在同一份历史里，回滚页一并列出。
    pub fn branch_service(&self) -> BranchService<'_> {
        BranchService::new(
            &self.engines,
            RepositoryStore::new(&self.database),
            self.snapshots.as_ref(),
        )
    }

    /// 绑定当前状态构造提交详情服务（T2.4）。
    ///
    /// 与历史查询共用同一批引擎与同一个数据库；diff/补丁统计在服务内
    /// 路由到 CLI 引擎（唯一数据源），`is_pushed` 用 CLI 的
    /// `remote_refs_containing`（libgit2 侧未实现该读取）。
    pub fn commit_detail_service(&self) -> CommitDetailService<'_> {
        CommitDetailService::new(&self.engines, RepositoryStore::new(&self.database))
    }
}
