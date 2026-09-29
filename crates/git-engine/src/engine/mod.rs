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

use crate::process::NetworkAuth;
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

/// 一次"外部探测"的原样结果（`ssh-add -l` 之类）。
///
/// # 为什么只带回输出与退出码
///
/// 判读规则属于调用方（例如 `forgedesk_credentials::parse_agent_listing`
/// 认识 `ssh-add -l` 的三种退出码语义），引擎只负责"把程序安全地跑起来、
/// 如实带回结果"。两件事的变更原因不同：换一个探测命令只是调用方换个解析器，
/// 而进程启动策略（不出 shell、固定环境、超时、可取消）不该跟着变。
///
/// 为什么不用 [`crate::process::GitOutput`]：那个类型带着 stderr 与"成功判定"，
/// 是给 git 命令用的语义；探测的退出码**非零也是正常结局**（例如 `ssh-add -l`
/// 用 1 表示"agent 里没有密钥"），把两者混成一个类型会诱导调用方误用
/// `success()` 去做判断，从而把"没有密钥"当成错误。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeOutput {
    /// 标准输出（按 UTF-8 宽松解码；解码失败不报错，见 `stdout_lossy` 的既有约定）。
    pub stdout: String,
    /// 退出码；被信号终止时为 `None`。
    pub exit_code: Option<i32>,
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

    /// 当前 HEAD 的 oid；空仓库（还没有提交）返回 `None`。
    ///
    /// 快照回滚后的校验用它：读引擎与写引擎是两条独立实现（T1.2 的差分测试
    /// 保证它们对同一状态给出同一结论），用读路径核对写路径的结果才有意义。
    fn head_oid(&self, repo: &RepoId) -> AppResult<Option<String>>;

    /// 实际生效的钩子目录。
    ///
    /// **不能**直接拼 `.git/hooks`：`core.hooksPath` 会改掉它，而它是常见配置
    /// （husky 默认就设成 `.husky`）。看错目录的后果是提交预览里说"不会执行钩子"、
    /// 实际却执行了——用户据以判断的依据是错的。
    fn hooks_dir(&self, repo: &RepoId) -> AppResult<std::path::PathBuf>;

    /// 包含指定提交的远程跟踪分支（短名，如 `origin/main`）。
    ///
    /// 用于回答"改写这个提交会不会影响别人"（amend 前的警示条）。
    ///
    /// **语义边界（决定了界面文案的措辞）**：查的是本地的 `refs/remotes/*`，
    /// 它是"上次 fetch 时远端的印象"，可能已经过期；而且它无法判断远端是否
    /// 仍保有那个对象。所以结论只能支撑"**可能**已推送"这样的提示，
    /// 不能用来断言"一定推过"或"一定没推过"。
    ///
    /// 只有 CLI 实现：它与 `commit` 同属"写历史"的判断族，而 libgit2 侧要自己
    /// 遍历 refs 做可达性计算，收益不抵两套实现之间产生分歧的风险
    /// （见 `docs/GIT-ENGINE-DIFF.md` §4 的能力边界表）。
    fn remote_refs_containing(&self, repo: &RepoId, revision: &str) -> AppResult<Vec<String>>;

    /// 列出仓库作者（按邮箱去重，提交数降序；T2.3 的作者筛选列表）。
    ///
    /// 范围与 `--all` 一致：作者筛选作用于全仓库，而不是当前分支。
    fn authors(&self, repo: &RepoId) -> AppResult<Vec<forgedesk_domain::git::AuthorSummary>>;

    // ---- 分支与标签管理（T2.5；写操作全部走 CLI） ----

    /// 新建分支。
    fn branch_create(
        &self,
        repo: &RepoId,
        spec: &forgedesk_domain::git::BranchCreateSpec,
    ) -> AppResult<()>;
    /// 切换分支（策略由调用方定；stash 编排在 services）。
    fn branch_switch(
        &self,
        repo: &RepoId,
        strategy: forgedesk_domain::git::SwitchStrategy,
        target: &str,
    ) -> AppResult<()>;
    /// 重命名分支。
    fn branch_rename(
        &self,
        repo: &RepoId,
        spec: &forgedesk_domain::git::BranchRenameSpec,
    ) -> AppResult<()>;
    /// 删除一批分支；返回实际删除的名字（失败即中止并报错）。
    fn branch_delete(
        &self,
        repo: &RepoId,
        spec: &forgedesk_domain::git::BranchDeleteSpec,
    ) -> AppResult<Vec<String>>;
    /// 设置 / 取消上游。
    fn branch_set_upstream(
        &self,
        repo: &RepoId,
        spec: &forgedesk_domain::git::BranchSetUpstreamSpec,
    ) -> AppResult<()>;
    /// 比较两个分支：`(ahead, behind)`（a 相对 b）。
    fn branch_compare(&self, repo: &RepoId, a: &str, b: &str) -> AppResult<(u64, u64)>;
    /// a 独有的提交（oid + subject），供"未合并删除确认"清单。
    fn branch_only_commits(
        &self,
        repo: &RepoId,
        a: &str,
        b: &str,
    ) -> AppResult<Vec<(String, String)>>;
    /// 创建标签。
    fn tag_create(
        &self,
        repo: &RepoId,
        spec: &forgedesk_domain::git::TagCreateSpec,
    ) -> AppResult<()>;
    /// 删除一批标签。
    fn tag_delete(
        &self,
        repo: &RepoId,
        spec: &forgedesk_domain::git::TagDeleteSpec,
    ) -> AppResult<()>;

    // ---------------------------------------------------------------- 快照（T1.9）

    /// 把一个 ref 指到指定提交（快照的防 gc 锚点）。
    ///
    /// 快照为什么需要自己的 ref：HEAD 移走之后，若没有任何引用指着旧提交，
    /// `git gc` 会把它当垃圾收掉，"回滚"就永远失败了。
    /// 刻意不用 `git reflog` / `HEAD@{n}` 当依据——reflog 会被外部操作改写或清空。
    fn update_ref(&self, repo: &RepoId, name: &str, oid: &str) -> AppResult<()>;

    /// 删除一个 ref（快照保留策略的清理动作）。
    fn delete_ref(&self, repo: &RepoId, name: &str) -> AppResult<()>;

    /// 一个 ref 是否存在且指向提交。
    ///
    /// 回滚前必须先做这个检查：ref 没了（用户手动清过、或被 gc 机制影响），
    /// 回滚注定失败——先知道，才能给用户一句可操作的话。
    fn ref_exists(&self, repo: &RepoId, name: &str) -> AppResult<bool>;

    /// 把索引读到指定树/提交。写的是**用户真实索引**——快照恢复的语义就是恢复它。
    fn read_tree(&self, repo: &RepoId, treeish: &str) -> AppResult<()>;

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
    ///
    /// `cancel`（T2.6）：取消令牌传进进程层——取消 = kill 子进程（含其孙进程组），
    /// 返回 `Cancelled` 错误。网络操作都可能很慢，不能取消的同步按钮等于挂死按钮。
    fn fetch(
        &self,
        repo: &RepoId,
        spec: FetchSpec,
        progress: &ProgressSink,
        cancel: &tokio_util::sync::CancellationToken,
        auth: &NetworkAuth,
    ) -> AppResult<FetchOutcome>;

    /// 拉取并合并 / 变基。
    fn pull(
        &self,
        repo: &RepoId,
        spec: PullSpec,
        progress: &ProgressSink,
        cancel: &tokio_util::sync::CancellationToken,
        auth: &NetworkAuth,
    ) -> AppResult<PullOutcome>;

    /// 推送。
    fn push(
        &self,
        repo: &RepoId,
        spec: PushSpec,
        progress: &ProgressSink,
        cancel: &tokio_util::sync::CancellationToken,
        auth: &NetworkAuth,
    ) -> AppResult<PushOutcome>;

    /// 探活一个远端（`git ls-remote`）：返回远端上的引用条数（空仓库为 0）。
    ///
    /// 用途：设置页的"测试连接"——用户改完凭据/密钥后需要一个**不改动任何东西**
    /// 的动作来确认"现在能不能连上"。`ls-remote` 只读远端、不写引用、不动工作区。
    ///
    /// 失败时返回的 `AppError` 已经过 [`ErrorCode::classify`]：SSH 主机指纹、
    /// 公钥被拒、证书、代理这些情况各自成为可区分的错误码（T2.7 的验收要求）。
    fn probe_remote(&self, cwd: &Path, url: &str, auth: &NetworkAuth) -> AppResult<usize>;

    /// 读取 ssh-agent 里的密钥清单（`ssh-add -l`）。
    ///
    /// # 为什么放在引擎层
    ///
    /// 引擎拥有"如何安全地跑外部程序"这份能力：一律以数组传参（没有 shell 包装）、
    /// 固定 locale 与交互开关、有超时、可取消。换到别处实现等于再写一份进程启动逻辑，
    /// 而进程启动恰恰是最容易写漏安全细节的地方。SSH 探测与 [`Self::probe_remote`]
    /// 同属"连接与认证的诊断"家族，放在一起也便于将来统一调整超时与代理环境。
    ///
    /// # 退出码的语义由调用方解释
    ///
    /// `ssh-add -l`：0 = 列出了密钥、1 = agent 在跑但没有密钥、2 = agent 没运行。
    /// **非零不是错误**，因此这里只有在**程序本身跑不起来**时才返回 `Err`
    /// （没装 `ssh-add`、超时）——"agent 没运行"与"命令不存在"需要完全不同的建议。
    fn probe_ssh_agent(&self) -> AppResult<ProbeOutput>;

    /// 添加远端（`git remote add`）。名称与 URL 先经 services 校验。
    fn remote_add(&self, repo: &RepoId, name: &str, url: &str) -> AppResult<()>;
    /// 删除远端。
    fn remote_remove(&self, repo: &RepoId, name: &str) -> AppResult<()>;
    /// 重命名远端。
    fn remote_rename(&self, repo: &RepoId, old: &str, new: &str) -> AppResult<()>;
    /// 改远端 URL。
    fn remote_set_url(&self, repo: &RepoId, name: &str, url: &str) -> AppResult<()>;

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
