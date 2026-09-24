//! 写操作的参数与结果规格。
//!
//! 为什么把参数收进结构体而不是给每个方法一长串入参：这些操作最终都要被
//! **序列化进审计日志与快照标签**（红线 R7），结构体天然可以整体脱敏后落库；
//! 而 12 个位置参数的函数在调用点几乎无法阅读，也没法在不改所有调用点的情况下
//! 新增一个选项。

use super::commit::Signature;
use super::path::RepoPath;
use super::refs::RefUpdate;

/// 重置模式（`git reset --soft|--mixed|--hard`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResetMode {
    /// `--soft`：只移动 HEAD，索引与工作区不动。
    Soft,
    /// `--mixed`：移动 HEAD 并重置索引，工作区不动（Git 默认）。
    Mixed,
    /// `--hard`：三者全部重置。**会丢弃工作区改动**（能力等级 Dangerous）。
    Hard,
}

impl ResetMode {
    /// 对应的命令行开关。
    pub const fn as_flag(self) -> &'static str {
        match self {
            Self::Soft => "--soft",
            Self::Mixed => "--mixed",
            Self::Hard => "--hard",
        }
    }

    /// 是否会丢弃工作区改动。
    ///
    /// 界面据此决定是否强制走"预览 + 快照 + 二次确认"（红线 R7）。
    pub const fn is_destructive(self) -> bool {
        matches!(self, Self::Hard)
    }
}

/// 重置操作的参数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResetSpec {
    /// 目标提交（oid、分支名或相对引用）。
    pub revision: String,
    /// 重置模式。
    pub mode: ResetMode,
    /// 只重置这些路径（非空时等价于 `git reset <rev> -- <paths>`，
    /// 此时 `mode` 只允许 `Mixed`）。
    pub paths: Vec<RepoPath>,
}

impl ResetSpec {
    /// 重置整棵工作树到某个提交。
    pub fn to(revision: impl Into<String>, mode: ResetMode) -> Self {
        Self {
            revision: revision.into(),
            mode,
            paths: Vec::new(),
        }
    }

    /// 只重置部分路径的索引。
    pub fn paths(revision: Option<String>, paths: Vec<RepoPath>) -> Self {
        Self {
            revision: revision.unwrap_or_else(|| "HEAD".to_owned()),
            mode: ResetMode::Mixed,
            paths,
        }
    }

    /// 是否为"只重置部分路径"。
    pub fn is_path_scoped(&self) -> bool {
        !self.paths.is_empty()
    }
}

/// 暂存 / 取消暂存的**文件级**参数。
///
/// 行级 / 块级不在这里：`git add` 的最小粒度是文件，而用户要的是
/// "这个文件里只有第 12–15 行"，唯一可靠的通道是 `git apply --cached`。
/// 那条通道由 [`ApplyPatchSpec`] 承担 —— 它还必须支持"先 `--check` 再应用"，
/// 而本枚举表达不了这件事。把补丁塞进来只会让"用哪条通道"变成调用方的自由选择，
/// 而两条通道的失败语义并不一样（补丁可能被拒绝，`git add` 不会）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StageSpec {
    /// 整文件暂存（或取消暂存）。
    Paths(Vec<RepoPath>),
    /// 全部变更（含删除与未跟踪文件）。
    All,
}

/// 补丁应用的作用面。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ApplyTarget {
    /// 索引（`--cached`）：暂存与取消暂存（T1.6）。
    Index,
    /// 工作区：按块丢弃未暂存的修改。
    Worktree,
}

/// 补丁应用的方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ApplyDirection {
    /// 正向：把补丁描述的"新内容"写进作用面（暂存）。
    Forward,
    /// 反向：把补丁的效果撤掉（取消暂存、丢弃）。
    Reverse,
}

/// 应用一份（可能被裁剪过的）补丁。
///
/// # 为什么 `check_only` 长在参数里
///
/// 红线 R7 要求"先预览再执行"，而补丁通道的预览就是 `git apply --check`：
/// 同一份字节先问一次"能不能应用"，再真正应用。两次调用之间没有任何写入，
/// 因此"检查通过而应用失败"只可能来自并发的外部改动 —— 那种情况下失败是正确结果，
/// 而不是"部分应用"。把 dry-run 做成独立方法会让调用方有机会忘记先检查。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyPatchSpec {
    /// 补丁字节（统一 diff；可能是裁剪后的子集）。
    pub patch: Vec<u8>,
    /// 作用面。
    pub target: ApplyTarget,
    /// 方向。
    pub direction: ApplyDirection,
    /// 只做 `--check`，不写任何东西。
    pub check_only: bool,
}

impl ApplyPatchSpec {
    /// 暂存到索引（正向、作用于索引）。
    pub fn stage(patch: Vec<u8>) -> Self {
        Self {
            patch,
            target: ApplyTarget::Index,
            direction: ApplyDirection::Forward,
            check_only: false,
        }
    }

    /// 从索引里撤销（反向、作用于索引）。
    pub fn unstage(patch: Vec<u8>) -> Self {
        Self {
            patch,
            target: ApplyTarget::Index,
            direction: ApplyDirection::Reverse,
            check_only: false,
        }
    }

    /// 撤销工作区的部分修改（反向、作用于工作区）。
    pub fn discard_worktree(patch: Vec<u8>) -> Self {
        Self {
            patch,
            target: ApplyTarget::Worktree,
            direction: ApplyDirection::Reverse,
            check_only: false,
        }
    }

    /// 只做 dry-run。
    #[must_use]
    pub fn checked(mut self) -> Self {
        self.check_only = true;
        self
    }

    /// 空补丁（裁剪后没有任何内容需要写）；服务层据此幂等返回。
    pub fn is_empty(&self) -> bool {
        self.patch.is_empty()
    }

    /// 传给 git 的开关串。
    ///
    /// 只用于错误 `hint` 与日志（`hint` 只放数据，见 CODING_STYLE §2.1）——
    /// 用户与维护者都需要知道"失败的那次到底加没加 `--reverse`"。
    pub fn git_flags(&self) -> String {
        let mut flags = vec!["apply"];
        if self.target == ApplyTarget::Index {
            flags.push("--cached");
        }
        if self.direction == ApplyDirection::Reverse {
            flags.push("--reverse");
        }
        if self.check_only {
            flags.push("--check");
        }
        flags.push("--recount");
        flags.join(" ")
    }
}

/// 提交参数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitSpec {
    /// 提交信息（首行是 subject）。
    pub message: String,
    /// 只提交这些路径（空 = 提交索引里的全部内容）。
    pub paths: Vec<RepoPath>,
    /// 是否 amend 上一个提交。
    pub amend: bool,
    /// 是否允许空提交。
    pub allow_empty: bool,
    /// 是否 GPG 签名。`None` = 跟随仓库/全局配置（不显式传 `-S`/`--no-gpg-sign`）。
    pub sign: Option<bool>,
    /// 覆盖作者身份（amend 时用于保留原作者）。
    pub author: Option<Signature>,
    /// 是否跳过 pre-commit / commit-msg 钩子。
    ///
    /// 默认 `false`：钩子拒绝是**正常流程**（T1.8 要求把 hook 输出展示给用户），
    /// 绕过钩子应当是一个用户显式选择的动作。
    pub no_verify: bool,
}

impl CommitSpec {
    /// 用提交信息创建。
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            paths: Vec::new(),
            amend: false,
            allow_empty: false,
            sign: None,
            author: None,
            no_verify: false,
        }
    }

    /// 提交信息首行（subject）。
    ///
    /// 界面与日志都只展示这一行，因此由领域层统一裁切，
    /// 避免各处自己 `lines().next()` 而行为不一致（例如空字符串的处理）。
    pub fn subject(&self) -> &str {
        self.message.lines().next().unwrap_or_default()
    }
}

/// 合并参数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeSpec {
    /// 要合并进来的引用。
    pub revision: String,
    /// 禁止快进（`--no-ff`），总是产生合并提交。
    pub no_ff: bool,
    /// 只允许快进（`--ff-only`），否则失败。
    pub ff_only: bool,
    /// 合并提交信息（`no_ff` 时使用）。
    pub message: Option<String>,
}

impl MergeSpec {
    /// 默认合并（允许快进）。
    pub fn new(revision: impl Into<String>) -> Self {
        Self {
            revision: revision.into(),
            no_ff: false,
            ff_only: false,
            message: None,
        }
    }
}

/// 拉取策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PullStrategy {
    /// 只允许快进，否则失败（最安全，也是默认）。
    FastForwardOnly,
    /// 允许产生合并提交。
    Merge,
    /// 变基后再快进。
    Rebase,
}

impl PullStrategy {
    /// 对应的命令行开关。
    pub const fn as_flag(self) -> &'static str {
        match self {
            Self::FastForwardOnly => "--ff-only",
            Self::Merge => "--no-rebase",
            Self::Rebase => "--rebase",
        }
    }
}

/// fetch 参数。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FetchSpec {
    /// 远端名。`None` = 当前分支的上游远端（没有上游时用 `origin`）。
    pub remote: Option<String>,
    /// 是否清理远端已删除的跟踪分支（`--prune`）。
    pub prune: bool,
    /// 只取这些引用（空 = 远端默认 refspec）。
    pub refspecs: Vec<String>,
    /// 是否同时取标签。
    pub tags: bool,
}

impl FetchSpec {
    /// 默认 fetch（取默认远端、不清理）。
    pub fn new() -> Self {
        Self::default()
    }

    /// 指定远端。
    #[must_use]
    pub fn with_remote(mut self, remote: impl Into<String>) -> Self {
        self.remote = Some(remote.into());
        self
    }

    /// 开启清理。
    #[must_use]
    pub fn with_prune(mut self, prune: bool) -> Self {
        self.prune = prune;
        self
    }
}

/// pull 参数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullSpec {
    /// 远端名。`None` = 当前分支的上游。
    pub remote: Option<String>,
    /// 远端分支名。`None` = 上游分支。
    pub branch: Option<String>,
    /// 策略。
    pub strategy: PullStrategy,
}

impl Default for PullSpec {
    fn default() -> Self {
        Self {
            remote: None,
            branch: None,
            strategy: PullStrategy::FastForwardOnly,
        }
    }
}

impl PullSpec {
    /// 默认 pull（上游 + 只允许快进）。
    pub fn new() -> Self {
        Self::default()
    }

    /// 指定策略。
    #[must_use]
    pub fn with_strategy(mut self, strategy: PullStrategy) -> Self {
        self.strategy = strategy;
        self
    }
}

/// push 参数。
///
/// **红线 R7**：这里刻意**没有**裸 `force` 字段。远端被拒绝时只有三条路
/// ——先拉取、`--force-with-lease`、取消；裸 `--force` 会无条件覆盖别人的提交，
/// 而它带来的"我推上去了"的错觉正是本产品要消灭的东西。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PushSpec {
    /// 远端名。`None` = 当前分支的上游远端。
    pub remote: Option<String>,
    /// 本地分支名。`None` = 当前分支。
    pub branch: Option<String>,
    /// 设置上游（`-u`）。
    pub set_upstream: bool,
    /// 仅当远端仍指向我们预期的提交时才强推（`--force-with-lease`）。
    pub force_with_lease: bool,
    /// 是否同时推送标签。
    pub tags: bool,
}

impl PushSpec {
    /// 默认 push（当前分支到其上游）。
    pub fn new() -> Self {
        Self::default()
    }

    /// 设置上游。
    #[must_use]
    pub fn with_set_upstream(mut self, set_upstream: bool) -> Self {
        self.set_upstream = set_upstream;
        self
    }

    /// 使用 `--force-with-lease`。
    #[must_use]
    pub fn with_force_with_lease(mut self, force_with_lease: bool) -> Self {
        self.force_with_lease = force_with_lease;
        self
    }
}

/// rebase / 重排计划中的单个步骤。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReorderStep {
    /// 被操作的提交 oid。
    pub oid: String,
    /// 对该提交执行的动作。
    pub action: ReorderAction,
    /// 新的提交信息（`Reword` / `Squash` 使用）。
    pub new_message: Option<String>,
}

/// rebase 计划里对单个提交的动作（与 `git rebase -i` 的指令一一对应）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReorderAction {
    /// 保留提交。
    Pick,
    /// 保留提交但改写信息。
    Reword,
    /// 保留提交并停下让用户修改内容。
    Edit,
    /// 合并进上一个提交并拼接信息。
    Squash,
    /// 合并进上一个提交并丢弃本条信息。
    Fixup,
    /// 丢弃提交。
    Drop,
}

impl ReorderAction {
    /// `git rebase -i` 的指令字。
    pub const fn as_instruction(self) -> &'static str {
        match self {
            Self::Pick => "pick",
            Self::Reword => "reword",
            Self::Edit => "edit",
            Self::Squash => "squash",
            Self::Fixup => "fixup",
            Self::Drop => "drop",
        }
    }

    /// 是否会丢失用户写下的提交内容（用于危险操作提示）。
    pub const fn discards_content(self) -> bool {
        matches!(self, Self::Drop | Self::Fixup)
    }
}

/// rebase / 重排计划。
///
/// M3 才会执行它；T1.2 只定义类型，让 `GitEngine` 的签名提前稳定下来，
/// 避免 M3 时再改 trait（改 trait 意味着两套实现一起改）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReorderSpec {
    /// 新的基点（`--onto`）。
    pub onto: String,
    /// 计划中的步骤，顺序为**从旧到新**（与 `git rebase -i` 的清单一致）。
    pub steps: Vec<ReorderStep>,
}

/// 合并的结果类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MergeKind {
    /// 已经在目标提交上，什么都没做。
    AlreadyUpToDate,
    /// 快进（没有产生新提交）。
    FastForward,
    /// 产生了合并提交。
    MergeCommit,
    /// 产生冲突，停在冲突状态。
    Conflicted,
}

/// 合并结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeOutcome {
    /// 结果类别。
    pub kind: MergeKind,
    /// 合并后的 HEAD oid（冲突时为 `None`）。
    pub oid: Option<String>,
    /// 冲突文件（`kind == Conflicted` 时非空）。
    pub conflicts: Vec<RepoPath>,
}

impl MergeOutcome {
    /// 是否产生了冲突。
    pub fn has_conflicts(&self) -> bool {
        !self.conflicts.is_empty()
    }
}

/// fetch 结果。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FetchOutcome {
    /// 实际使用的远端名。
    pub remote: String,
    /// 引用变更明细。
    pub updates: Vec<RefUpdate>,
}

impl FetchOutcome {
    /// 有实际变更的引用条数（不含 `UpToDate`）。
    pub fn changed_refs(&self) -> usize {
        self.updates
            .iter()
            .filter(|update| update.kind != super::refs::RefUpdateKind::UpToDate)
            .count()
    }
}

/// pull 结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullOutcome {
    /// fetch 阶段的结果。
    pub fetch: FetchOutcome,
    /// 实际使用的策略。
    pub strategy: PullStrategy,
    /// 本地是否本来就已经是最新（此时不会有合并结果）。
    pub up_to_date: bool,
    /// 合并 / 快进阶段的结果；`up_to_date` 时为 `None`。
    pub merge: Option<MergeOutcome>,
}

impl PullOutcome {
    /// 是否产生了冲突。
    pub fn has_conflicts(&self) -> bool {
        self.merge.as_ref().is_some_and(MergeOutcome::has_conflicts)
    }
}

/// push 被拒绝的原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushRejection {
    /// 被拒绝的引用短名。
    pub name: String,
    /// 原因（原始 stderr 片段，已脱敏）。
    pub reason: String,
    /// 是否属于"非快进"这一类——界面据此提供 `force-with-lease` 选项；
    /// 其他原因（权限、hook）给这个选项只会误导用户。
    pub non_fast_forward: bool,
}

/// push 结果。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PushOutcome {
    /// 实际使用的远端名。
    pub remote: String,
    /// 成功推送的引用变更。
    pub updates: Vec<RefUpdate>,
    /// 被拒绝的引用。
    pub rejections: Vec<PushRejection>,
}

impl PushOutcome {
    /// 是否全部成功。
    pub fn is_success(&self) -> bool {
        self.rejections.is_empty()
    }
}

/// 初始化仓库的参数。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InitSpec {
    /// 初始分支名。`None` = 跟随用户的 `init.defaultBranch` 配置。
    ///
    /// 显式传 `-b` 会覆盖用户配置，因此默认不传——本产品的立场是"复刻用户在终端里的行为"。
    pub initial_branch: Option<String>,
    /// 是否创建裸仓库。
    pub bare: bool,
}

impl InitSpec {
    /// 默认初始化。
    pub fn new() -> Self {
        Self::default()
    }

    /// 指定初始分支。
    #[must_use]
    pub fn with_initial_branch(mut self, branch: impl Into<String>) -> Self {
        self.initial_branch = Some(branch.into());
        self
    }
}

/// 克隆参数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloneSpec {
    /// 远端 URL。
    pub url: String,
    /// 克隆到哪个本地目录。
    pub into: std::path::PathBuf,
    /// 浅克隆深度（`--depth`）。`None` = 完整克隆。
    pub depth: Option<u32>,
    /// 只克隆某个分支（`--branch`）。
    pub branch: Option<String>,
    /// 是否克隆为裸仓库。
    pub bare: bool,
    /// 是否递归初始化子模块（`--recurse-submodules`）。
    pub recurse_submodules: bool,
    /// 是否只取单个分支（`--single-branch`）。
    ///
    /// 与 `--branch` 的区别：`--branch` 决定**检出**哪个分支，
    /// `--single-branch` 决定**只取**哪个分支的引用（其余分支不下载）。
    /// 大仓库上这个区别就是几百 MB。
    pub single_branch: bool,
}

impl CloneSpec {
    /// 完整克隆到指定目录。
    pub fn new(url: impl Into<String>, into: impl Into<std::path::PathBuf>) -> Self {
        Self {
            url: url.into(),
            into: into.into(),
            depth: None,
            branch: None,
            bare: false,
            recurse_submodules: false,
            single_branch: false,
        }
    }

    /// 浅克隆。
    #[must_use]
    pub fn with_depth(mut self, depth: u32) -> Self {
        self.depth = Some(depth);
        self
    }

    /// 只克隆某个分支。
    #[must_use]
    pub fn with_branch(mut self, branch: impl Into<String>) -> Self {
        self.branch = Some(branch.into());
        self
    }

    /// 递归初始化子模块。
    #[must_use]
    pub fn with_submodules(mut self, recurse: bool) -> Self {
        self.recurse_submodules = recurse;
        self
    }

    /// 只取单个分支的引用。
    #[must_use]
    pub fn with_single_branch(mut self, single: bool) -> Self {
        self.single_branch = single;
        self
    }
}

/// 切换分支 / 提交的参数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckoutSpec {
    /// 目标分支名或提交 oid。
    pub target: String,
    /// 是否强制切换（`--force`）。**会丢弃工作区改动**（Dangerous）。
    pub force: bool,
    /// 是否强制进入游离 HEAD（`--detach`）。
    pub detach: bool,
    /// 同时创建新分支（`-b`）。
    pub create_branch: Option<String>,
}

/// 放弃指定路径的**工作区**修改（T1.4 状态面板的"放弃"操作）。
///
/// 两组路径分开传：已跟踪路径走 `git restore --worktree`（工作区 ← 索引），
/// 未跟踪路径只能从磁盘删除（它们不在任何树里，git 无法恢复）。
/// 分组由调用方（services）根据 StatusReport 判定，引擎不做二次查询。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DiscardSpec {
    /// 已跟踪路径：用 `git restore --worktree --` 恢复到索引内容。
    pub tracked: Vec<RepoPath>,
    /// 未跟踪路径：直接删除工作区文件（**不可恢复**，调用方必须先经确认对话框）。
    pub untracked: Vec<RepoPath>,
}

impl DiscardSpec {
    /// 是否没有任何要放弃的路径（调用方应提前拒绝空请求）。
    pub fn is_empty(&self) -> bool {
        self.tracked.is_empty() && self.untracked.is_empty()
    }
}

impl CheckoutSpec {
    /// 切换到某个分支或提交。
    pub fn new(target: impl Into<String>) -> Self {
        Self {
            target: target.into(),
            force: false,
            detach: false,
            create_branch: None,
        }
    }

    /// 是否会把工作区改动丢掉。
    pub fn is_destructive(&self) -> bool {
        self.force
    }
}

/// stash 子操作。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StashAction {
    /// 储藏当前改动。
    Push,
    /// 应用某个 stash 但保留它（`apply`）。
    Apply {
        /// `stash@{n}` 里的 n。
        index: usize,
    },
    /// 应用并删除某个 stash（`pop`）。
    Pop {
        /// `stash@{n}` 里的 n。
        index: usize,
    },
    /// 删除某个 stash（`drop`）。**不可逆**（Dangerous）。
    Drop {
        /// `stash@{n}` 里的 n。
        index: usize,
    },
}

impl StashAction {
    /// 该动作是否会丢失数据。
    pub const fn is_destructive(&self) -> bool {
        matches!(self, Self::Drop { .. })
    }
}

/// stash 操作的参数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StashSpec {
    /// 要执行的子操作。
    pub action: StashAction,
    /// 描述信息（仅 `Push` 使用）。
    pub message: Option<String>,
    /// 是否把未跟踪文件一起储藏（`-u`）。
    pub include_untracked: bool,
    /// 是否保留索引（`--keep-index`）。
    pub keep_index: bool,
}

impl StashSpec {
    /// 储藏当前改动。
    pub fn push(message: Option<String>) -> Self {
        Self {
            action: StashAction::Push,
            message,
            include_untracked: false,
            keep_index: false,
        }
    }

    /// 应用某个 stash。
    pub fn pop(index: usize) -> Self {
        Self {
            action: StashAction::Pop { index },
            message: None,
            include_untracked: false,
            keep_index: false,
        }
    }
}

/// 一条 reflog 记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReflogEntry {
    /// `HEAD@{n}` 里的 n（从 0 开始，0 是最新）。
    pub index: usize,
    /// 该记录指向的提交 oid。
    pub oid: String,
    /// 引用短名（如 `HEAD`、`refs/heads/main`）。
    pub reference: String,
    /// 动作（`commit`、`checkout`、`reset`、`rebase`…）。
    pub action: String,
    /// 动作描述（reflog 消息的正文）。
    pub message: String,
    /// 时间（Unix 秒）。
    pub created_at: Option<i64>,
}

impl ReflogEntry {
    /// `HEAD@{n}` 形式的选择器。
    pub fn selector(&self) -> String {
        format!("{}@{{{}}}", self.reference, self.index)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{
        ApplyDirection, ApplyPatchSpec, ApplyTarget, CheckoutSpec, CommitSpec, FetchOutcome,
        InitSpec, MergeKind, MergeOutcome, PullOutcome, PullStrategy, PushOutcome, ReflogEntry,
        ReorderAction, ResetMode, ResetSpec, StageSpec, StashAction, StashSpec,
    };
    use crate::git::refs::{RefUpdate, RefUpdateKind};

    #[test]
    fn init_does_not_override_the_users_default_branch_by_default() {
        assert_eq!(InitSpec::new().initial_branch, None);
        assert_eq!(
            InitSpec::new()
                .with_initial_branch("trunk")
                .initial_branch
                .as_deref(),
            Some("trunk")
        );
    }

    #[test]
    fn only_forced_checkout_is_destructive() {
        assert!(!CheckoutSpec::new("main").is_destructive());
        assert!(CheckoutSpec {
            force: true,
            ..CheckoutSpec::new("main")
        }
        .is_destructive());
    }

    #[test]
    fn only_stash_drop_is_destructive() {
        assert!(!StashAction::Push.is_destructive());
        assert!(!StashAction::Apply { index: 0 }.is_destructive());
        assert!(!StashAction::Pop { index: 0 }.is_destructive());
        assert!(StashAction::Drop { index: 0 }.is_destructive());
    }

    #[test]
    fn stash_pop_uses_pop_rather_than_push() {
        let spec = StashSpec::pop(2);

        assert_eq!(spec.action, StashAction::Pop { index: 2 });
        assert_eq!(spec.message, None);
    }

    #[test]
    fn reflog_selector_is_the_at_brace_form() {
        let entry = ReflogEntry {
            index: 0,
            oid: "abc".to_owned(),
            reference: "HEAD".to_owned(),
            action: "commit".to_owned(),
            message: "commit: initial".to_owned(),
            created_at: Some(1_704_164_645),
        };

        assert_eq!(entry.selector(), "HEAD@{0}");
    }

    #[test]
    fn only_hard_reset_is_destructive() {
        assert!(ResetMode::Hard.is_destructive());
        assert!(!ResetMode::Soft.is_destructive());
        assert!(!ResetMode::Mixed.is_destructive());
        assert_eq!(ResetMode::Hard.as_flag(), "--hard");
    }

    #[test]
    fn path_scoped_reset_forces_mixed_and_defaults_to_head() {
        let spec = ResetSpec::paths(None, vec!["a.txt".into()]);

        assert!(spec.is_path_scoped());
        assert_eq!(spec.revision, "HEAD");
        assert_eq!(spec.mode, ResetMode::Mixed);
    }

    #[test]
    fn commit_subject_is_the_first_line_and_empty_message_is_safe() {
        let multi = CommitSpec::new("subject line\n\nbody text");
        assert_eq!(multi.subject(), "subject line");

        let empty = CommitSpec::new("");
        assert_eq!(empty.subject(), "");
    }

    #[test]
    fn pull_strategies_map_to_their_git_flags() {
        assert_eq!(PullStrategy::FastForwardOnly.as_flag(), "--ff-only");
        assert_eq!(PullStrategy::Merge.as_flag(), "--no-rebase");
        assert_eq!(PullStrategy::Rebase.as_flag(), "--rebase");
    }

    #[test]
    fn push_spec_has_no_bare_force_option() {
        // 这条断言的意义是"红线 R7 在类型层面被表达出来"：
        // 没有字段可以表达裸 --force，因此也不可能有调用点误用它。
        let spec = super::PushSpec::new().with_force_with_lease(true);
        assert!(spec.force_with_lease);
    }

    #[test]
    fn fetch_changed_refs_excludes_unchanged_ones() {
        let outcome = FetchOutcome {
            remote: "origin".to_owned(),
            updates: vec![
                RefUpdate {
                    name: "origin/main".to_owned(),
                    old_oid: Some("a".to_owned()),
                    new_oid: Some("b".to_owned()),
                    kind: RefUpdateKind::Updated,
                    reason: None,
                },
                RefUpdate {
                    name: "origin/other".to_owned(),
                    old_oid: Some("c".to_owned()),
                    new_oid: Some("c".to_owned()),
                    kind: RefUpdateKind::UpToDate,
                    reason: None,
                },
            ],
        };

        assert_eq!(outcome.changed_refs(), 1);
    }

    #[test]
    fn pull_outcome_reports_conflicts_through_its_merge_result() {
        let conflicted = PullOutcome {
            fetch: FetchOutcome::default(),
            strategy: PullStrategy::Merge,
            up_to_date: false,
            merge: Some(MergeOutcome {
                kind: MergeKind::Conflicted,
                oid: None,
                conflicts: vec!["a.txt".into()],
            }),
        };
        let fast_forwarded = PullOutcome {
            merge: Some(MergeOutcome {
                kind: MergeKind::FastForward,
                oid: Some("abc".to_owned()),
                conflicts: Vec::new(),
            }),
            ..conflicted.clone()
        };

        assert!(conflicted.has_conflicts());
        assert!(!fast_forwarded.has_conflicts());
    }

    #[test]
    fn up_to_date_pull_has_no_merge_result_at_all() {
        let outcome = PullOutcome {
            fetch: FetchOutcome::default(),
            strategy: PullStrategy::FastForwardOnly,
            up_to_date: true,
            merge: None,
        };

        assert!(!outcome.has_conflicts());
    }

    #[test]
    fn push_outcome_success_requires_no_rejections() {
        let ok = PushOutcome::default();
        let rejected = PushOutcome {
            remote: "origin".to_owned(),
            updates: Vec::new(),
            rejections: vec![super::PushRejection {
                name: "main".to_owned(),
                reason: "non-fast-forward".to_owned(),
                non_fast_forward: true,
            }],
        };

        assert!(ok.is_success());
        assert!(!rejected.is_success());
    }

    #[test]
    fn reorder_actions_map_to_rebase_instructions_and_flag_content_loss() {
        assert_eq!(ReorderAction::Pick.as_instruction(), "pick");
        assert_eq!(ReorderAction::Fixup.as_instruction(), "fixup");
        assert!(ReorderAction::Drop.discards_content());
        assert!(ReorderAction::Fixup.discards_content());
        assert!(!ReorderAction::Pick.discards_content());
        assert!(!ReorderAction::Squash.discards_content());
    }

    #[test]
    fn stage_spec_only_carries_file_level_granularity() {
        let paths = StageSpec::Paths(vec!["a.txt".into()]);
        let all = StageSpec::All;

        assert_ne!(paths, all);
    }

    #[test]
    fn apply_specs_pick_the_target_and_direction_for_their_use_case() {
        let stage = ApplyPatchSpec::stage(b"patch".to_vec());
        assert_eq!(stage.target, ApplyTarget::Index);
        assert_eq!(stage.direction, ApplyDirection::Forward);
        assert_eq!(stage.git_flags(), "apply --cached --recount");

        let unstage = ApplyPatchSpec::unstage(b"patch".to_vec());
        assert_eq!(unstage.git_flags(), "apply --cached --reverse --recount");

        let discard = ApplyPatchSpec::discard_worktree(b"patch".to_vec());
        assert_eq!(discard.target, ApplyTarget::Worktree);
        assert_eq!(discard.git_flags(), "apply --reverse --recount");

        let checked = ApplyPatchSpec::stage(b"patch".to_vec()).checked();
        assert!(checked.check_only, "dry-run 必须能表达在同一个 spec 里");
        assert_eq!(checked.git_flags(), "apply --cached --check --recount");
    }

    #[test]
    fn an_empty_patch_is_detected_before_spawning_git() {
        assert!(ApplyPatchSpec::stage(Vec::new()).is_empty());
        assert!(!ApplyPatchSpec::stage(b"x".to_vec()).is_empty());
    }
}
