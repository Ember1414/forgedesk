//! `GitEngine` 抽象与两套实现。
//!
//! # 为什么要抽象
//!
//! 读与写的最优解不同（`docs/PLAN.md` §5.6）：
//!
//! | 操作 | 主实现 | 理由 |
//! | --- | --- | --- |
//! | 读（status/diff/log/show/…） | libgit2 | 无进程开销，可高频调用 |
//! | 写（commit/merge/reset/…） | 系统 git CLI | 完整复刻用户环境（hooks、attributes、签名、filter） |
//! | 网络（fetch/pull/push） | 系统 git CLI | 复用 SSH/凭据/代理配置 |
//!
//! 因此业务层只依赖本 trait，而**不是**依赖某个具体实现。谁来做哪一半由
//! `services` 层决定（它同时持有两个实现，读走 libgit2、写走 CLI）。
//!
//! # 接口为什么是同步的
//!
//! `docs/PLAN.md` §5.6 给出的签名是同步的，现有 6 个 Tauri 命令也全是同步的。
//! `GitProcess` 是异步的，两者之间由 [`bridge::BlockingBridge`] 衔接
//! （见该模块头：在独立线程上驱动，避免"在运行时里再起一个运行时"的 panic）。
//!
//! # 本层不做安全网
//!
//! 写操作**不在这里创建快照**：快照属于 `crates/snapshot`，由 `services` 层
//! 在调用引擎之前编排（红线 R7 要求"计划预览 → 快照 → 执行 → 可回滚"，
//! 而"计划"是 services 的概念）。引擎只负责"把这一步做对"。

pub mod bridge;
pub mod cli;
pub mod enrich;
pub mod libgit2_engine;
pub mod progress;

pub use bridge::BlockingBridge;
pub use cli::CliGitEngine;
pub use enrich::{
    apply_lfs, count_ignored, detect_operation, enrich_filesystem, lfs_paths_from_check_attr,
    query_lfs_paths, resolve_git_dir,
};
pub use libgit2_engine::Libgit2Engine;
pub use progress::{parse_progress_line, ProgressEvent, ProgressPhase, ProgressSink};

use std::path::Path;

use forgedesk_domain::git::{
    ApplyPatchSpec, Branch, CheckoutSpec, CloneSpec, Commit, CommitSpec, DiffReport, DiffSpec,
    DiscardSpec, FetchOutcome, FetchSpec, InitSpec, LogQuery, MergeOutcome, MergeSpec, Page,
    PullOutcome, PullSpec, PushOutcome, PushSpec, ReflogEntry, Remote, ReorderSpec, RepoId,
    RepositoryInfo, ResetSpec, StageSpec, StashEntry, StashSpec, StatusQuery, StatusReport, Tag,
};
use forgedesk_domain::{AppError, AppResult, ErrorCode};

/// 引擎标识（日志、诊断与"降级提示"用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EngineId {
    /// 系统 git 命令行。
    Cli,
    /// libgit2（进程内库）。
    Libgit2,
}

impl EngineId {
    /// 稳定的短名。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Cli => "cli",
            Self::Libgit2 => "libgit2",
        }
    }
}

impl std::fmt::Display for EngineId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 构造"本引擎不支持该操作"的错误。
///
/// 与"尚未实现"区分开：前者是**设计如此**（libgit2 不承担写操作），
/// 用户看到的是"这个功能需要系统 git"；后者是开发进度问题。
pub fn unsupported(engine: EngineId, operation: &str) -> AppError {
    AppError::new(
        ErrorCode::UnsupportedByEngine,
        format!(
            "{operation} is not supported by the {} engine",
            engine.as_str()
        ),
    )
    .with_hint(engine.as_str().to_owned())
    .with_retryable(false)
}

/// 构造"尚未实现"的错误。
///
/// `task` 必须是任务号（如 `T3.6`），让读到这条日志的人知道去哪看进度，
/// 而不是在代码里搜"not implemented"。
pub fn not_implemented(operation: &str, task: &str) -> AppError {
    AppError::new(
        ErrorCode::Internal,
        format!("{operation} is not implemented yet ({task})"),
    )
    .with_hint(task.to_owned())
    .with_retryable(false)
}

/// Git 引擎：读操作与写操作。
///
/// 未在某个实现里支持的写操作**必须**返回
/// [`ErrorCode::UnsupportedByEngine`](forgedesk_domain::ErrorCode::UnsupportedByEngine)
/// 或 [`not_implemented`]，不允许静默成功——静默成功会让 services 层的
/// "预览 → 快照 → 执行"链路以为操作已经完成。
pub trait GitEngine: Send + Sync {
    /// 引擎标识。
    fn id(&self) -> EngineId;

    // ---------------------------------------------------------------- 读

    /// 从任意目录向上查找仓库。
    ///
    /// 路径不在任何仓库内时返回
    /// [`ErrorCode::PathNotRepo`](forgedesk_domain::ErrorCode::PathNotRepo)。
    fn discover(&self, path: &Path) -> AppResult<RepositoryInfo>;

    /// 工作区状态。
    ///
    /// `query.include_ignored` 为 `true` 时返回被忽略条目并统计 `ignored_count`；
    /// 默认忽略文件不返回（它们可能数以万计，状态面板几乎不需要）。
    fn status(&self, repo: &RepoId, query: &StatusQuery) -> AppResult<StatusReport>;

    /// 放弃指定路径的**工作区**修改（`git restore --worktree`；未跟踪文件直接删除）。
    ///
    /// 只影响工作区、不碰索引：对"已暂存 + 工作区又改"的文件，
    /// 放弃工作区改动后保留已暂存的版本。这是界面"放弃"按钮的语义。
    fn discard_worktree(&self, repo: &RepoId, spec: &DiscardSpec) -> AppResult<()>;

    /// 文件级变更统计（行级内容由 T1.5 填充，见 `domain::git::diff` 模块头）。
    fn diff(&self, repo: &RepoId, spec: DiffSpec) -> AppResult<DiffReport>;

    /// 生成原始补丁文本（复制 / 导出 .patch；T1.6 部分暂存的底稿）。
    /// 返回原始字节：补丁里的路径与内容都可能是非 UTF-8。
    fn diff_patch(&self, repo: &RepoId, spec: &DiffSpec) -> AppResult<Vec<u8>>;

    /// 分页查询提交历史。
    fn log(&self, repo: &RepoId, query: LogQuery) -> AppResult<Page<Commit>>;

    /// 查询单条提交（含正文）。
    fn show(&self, repo: &RepoId, revision: &str) -> AppResult<Commit>;

    /// 分支列表（含远程跟踪分支）。
    fn branch_list(&self, repo: &RepoId) -> AppResult<Vec<Branch>>;

    /// 标签列表。
    fn tag_list(&self, repo: &RepoId) -> AppResult<Vec<Tag>>;

    /// 远端列表。
    fn remote_list(&self, repo: &RepoId) -> AppResult<Vec<Remote>>;

    /// stash 列表（最新在前）。
    fn stash_list(&self, repo: &RepoId) -> AppResult<Vec<StashEntry>>;

    /// reflog（最新在前）。
    fn reflog(&self, repo: &RepoId, limit: usize) -> AppResult<Vec<ReflogEntry>>;

    // ---------------------------------------------------------------- 写

    /// 初始化仓库。
    fn init(&self, path: &Path, spec: InitSpec) -> AppResult<RepositoryInfo>;

    /// 克隆仓库。
    fn clone(&self, spec: CloneSpec, progress: &ProgressSink) -> AppResult<RepositoryInfo>;

    /// 暂存。
    fn stage(&self, repo: &RepoId, spec: StageSpec) -> AppResult<()>;

    /// 取消暂存。
    fn unstage(&self, repo: &RepoId, spec: StageSpec) -> AppResult<()>;

    /// 应用一份补丁（行级 / 块级暂存与取消暂存、按块丢弃；T1.6）。
    ///
    /// `spec.check_only` 为真时只做 dry-run（`git apply --check`）—— 红线 R7
    /// 的"先预览再执行"在补丁通道上的形态：服务层用同一份字节先检查再应用。
    /// 空补丁是幂等成功（裁剪后没有内容可写），实现不得为此报错。
    fn apply_patch(&self, repo: &RepoId, spec: &ApplyPatchSpec) -> AppResult<()>;

    /// 当前索引的树 oid（`git write-tree`）。
    ///
    /// 提交计划用它做"索引指纹"：`prepare` 与 `execute` 之间索引若被别处改过，
    /// 指纹就会变，计划必须作废——否则会提交出用户没在预览里看过的内容（T1.7）。
    ///
    /// 选 `write-tree` 而不是"自己把 `ls-files --stage` 的输出哈希一遍"：
    /// 树 oid 就是 git 对索引内容的规范摘要，等价而且**不会引入第二份真相**
    /// （自建哈希还得引一个 sha2 依赖，并自己保证跨平台/跨版本稳定）。
    /// 它会往对象库里写一个树对象（不影响引用与工作区，gc 会回收）。
    fn index_tree(&self, repo: &RepoId) -> AppResult<String>;

    /// HEAD 的树 oid；空仓库（还没有提交）返回 `None`。
    ///
    /// 与 [`GitEngine::index_tree`] 一起回答"这次提交是否什么都不会提交"：
    /// 索引为空（空树 oid）或索引内容与 HEAD 完全相同，都属于"没有暂存内容"。
    fn head_tree(&self, repo: &RepoId) -> AppResult<Option<String>>;

    /// 实际生效的钩子目录。
    ///
    /// **不能**直接拼 `.git/hooks`：`core.hooksPath` 会改掉它，而它是常见配置
    /// （husky 默认就设成 `.husky`）。看错目录的后果是提交预览里说"不会执行钩子"、
    /// 实际却执行了——用户据以判断的依据是错的。
    fn hooks_dir(&self, repo: &RepoId) -> AppResult<std::path::PathBuf>;

    /// 提交（含 amend），返回新提交的 oid。
    fn commit(&self, repo: &RepoId, spec: CommitSpec) -> AppResult<String>;

    /// 重置。
    fn reset(&self, repo: &RepoId, spec: ResetSpec) -> AppResult<()>;

    /// 切换分支 / 提交。
    fn checkout(&self, repo: &RepoId, spec: CheckoutSpec) -> AppResult<()>;

    /// 合并。
    fn merge(&self, repo: &RepoId, spec: MergeSpec) -> AppResult<MergeOutcome>;

    /// 拣选提交。
    fn cherry_pick(&self, repo: &RepoId, revision: &str) -> AppResult<MergeOutcome>;

    /// 反转提交。
    fn revert(&self, repo: &RepoId, revision: &str) -> AppResult<MergeOutcome>;

    /// stash 操作。
    fn stash(&self, repo: &RepoId, spec: StashSpec) -> AppResult<()>;

    /// 拉取远端引用。
    fn fetch(
        &self,
        repo: &RepoId,
        spec: FetchSpec,
        progress: &ProgressSink,
    ) -> AppResult<FetchOutcome>;

    /// 拉取并合并 / 变基。
    fn pull(
        &self,
        repo: &RepoId,
        spec: PullSpec,
        progress: &ProgressSink,
    ) -> AppResult<PullOutcome>;

    /// 推送。
    fn push(
        &self,
        repo: &RepoId,
        spec: PushSpec,
        progress: &ProgressSink,
    ) -> AppResult<PushOutcome>;

    /// 按计划重排提交（交互式 rebase）。
    ///
    /// M3 才实现（T3.6）；两个实现当前都返回 [`not_implemented`]。
    /// 提前放进 trait 是为了让签名在 M3 不需要改动——改 trait 意味着
    /// 两套实现与全部调用点一起改。
    fn rebase(
        &self,
        repo: &RepoId,
        plan: ReorderSpec,
        progress: &ProgressSink,
    ) -> AppResult<MergeOutcome>;
}
