//! 写操作的参数与结果规格。
//!
//! 为什么把参数收进结构体而不是给每个方法一长串入参：这些操作最终都要被
//! **序列化进审计日志与快照标签**（红线 R7），结构体天然可以整体脱敏后落库；
//! 而 12 个位置参数的函数在调用点几乎无法阅读，也没法在不改所有调用点的情况下
//! 新增一个选项。

use super::commit::Signature;
use super::path::RepoPath;
use super::refs::RefUpdate;
use super::reset::CommitSummary;

/// 重置模式（`git reset --soft|--mixed|--hard`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
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
///
/// 只派生 `Serialize`（审计与快照标签要落库），**不**派生 `Deserialize`：
/// 它带着 [`RepoPath`]（保真的字节路径），而"从 JSON 反序列化出一条路径"需要一个
/// 线上表示（T1.4 的 DTO 层工作）。IPC 请求侧用的是 commands 里的 `ResetRequest`。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
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

/// `amend` 的两种语义（T1.8）。
///
/// `git commit --amend` 提交的是**当前索引**，所以"只改提交信息"并不是它的
/// 默认行为。同一个动作要不要把暂存区一起并进去，结果完全不同，因此必须由
/// 用户明确回答——这正是它值得成为一个显式参数、而不是布尔开关的理由。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AmendMode {
    /// 把当前暂存区并入上一次提交（`git commit --amend` 的默认行为）。
    #[default]
    IncludeStaged,
    /// 只替换提交信息，索引内容不进提交。
    ///
    /// 实现要点：必须在**隔离索引**（`GIT_INDEX_FILE`）上执行，且该索引先被
    /// 读成 HEAD 的树。直接跑 `git commit --amend` 会把暂存区一并提交——
    /// 那是"改一个错别字"变成"多提交了三个文件"的事故。
    MessageOnly,
}

impl AmendMode {
    /// 稳定的短名（IPC 用；前端据此走 i18n）。
    pub const fn key(self) -> &'static str {
        match self {
            Self::IncludeStaged => "includeStaged",
            Self::MessageOnly => "messageOnly",
        }
    }

    /// 从稳定短名解析；未知取值返回 `None`（由命令层转成 `VALIDATION`，
    /// 而不是静默降级成默认值）。
    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "includeStaged" => Some(Self::IncludeStaged),
            "messageOnly" => Some(Self::MessageOnly),
            _ => None,
        }
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
    /// 是否在提交信息末尾追加 `Signed-off-by`（`--signoff`，T1.7）。
    ///
    /// 与 GPG 签名是**两件不同的事**：`--signoff` 只是往信息里写一行
    /// `Signed-off-by: Name <email>`（很多项目的 DCO 流程要求它），
    /// 而 `sign` 是对提交对象做密码学签名。名字相近，很容易在参数里搞混。
    pub sign_off: bool,
    /// amend 的语义（T1.8）。仅在 `amend` 为真时有意义。
    pub amend_mode: AmendMode,
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
            sign_off: false,
            amend_mode: AmendMode::default(),
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

// ---------------------------------------------------------------- 分支与标签（T2.5）

/// 分支切换时对"工作区不干净"的处理策略（任务书 T2.5 实现要求 1）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SwitchStrategy {
    /// 先 stash（含未跟踪）再切换；切换**成功后自动恢复**（恢复冲突如实上报）。
    #[default]
    Stash,
    /// 强制切换（`checkout --force`）：**丢弃**全部未提交修改，不可逆。
    /// 调用方（services）必须先打快照并在界面上要求显式确认。
    Force,
    /// 只允许干净切换：工作区不干净时直接报错（界面的"取消"分支）。
    Clean,
}

/// 新建分支参数。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchCreateSpec {
    /// 分支名（先过 [`validate_ref_name`]）。
    pub name: String,
    /// 起点（分支/tag/oid）；`None` = 当前 HEAD。
    pub start_point: Option<String>,
    /// 创建后立即切换过去（`-c` + checkout）。
    pub checkout: bool,
    /// 同时设置上游（`--track` 的完整短名，如 `origin/main`）。
    pub track_upstream: Option<String>,
}

/// 分支重命名参数。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchRenameSpec {
    /// 要改名的分支短名。
    pub old: String,
    /// 新名（先过 [`validate_ref_name`]）。
    pub new: String,
    /// 同时重命名上游分支（远端重命名依赖网络，T2.6 前仅在本地 bare 模拟里可用）。
    pub rename_remote: bool,
}

/// 删除分支参数。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchDeleteSpec {
    /// 要删除的分支短名（可多个）。
    pub names: Vec<String>,
    /// 强制删除（`-D`）：允许删除未合并分支。调用方必须已展示
    /// "独有提交"清单并拿到二次确认（services 层强制校验）。
    pub force: bool,
    /// 同时删除对应的远端分支（T2.6 前仅在本地 bare 模拟里可用）。
    pub also_delete_remote: bool,
}

/// 设置上游参数。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchSetUpstreamSpec {
    /// 本地分支短名。
    pub branch: String,
    /// 上游短名；`None` = 取消上游（`--unset-upstream`）。
    pub upstream: Option<String>,
}

/// 创建标签参数。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TagCreateSpec {
    /// 标签名（先过 [`validate_ref_name`]）。
    pub name: String,
    /// 目标（分支/tag/oid）；`None` = 当前 HEAD。
    pub target: Option<String>,
    /// 附注信息；`Some` = 附注标签（annotated），`None` = 轻量标签。
    pub message: Option<String>,
    /// 要求 gpg 签名（`-s`；仓库未配置签名密钥时 git 会报错，如实传播）。
    pub sign: bool,
    /// 同名标签已存在时覆盖（`-f`）。
    pub force: bool,
}

/// 删除标签参数。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TagDeleteSpec {
    /// 要删除的标签名（可多个）。
    pub names: Vec<String>,
    /// 同时删除远端标签（T2.6 前仅在本地 bare 模拟里可用）。
    pub also_delete_remote: bool,
}

/// `git check-ref-format` 的**本地实现**（纯函数，供 services 与测试复用）。
///
/// # 为什么不用 `git check-ref-format --branch`
///
/// 规则简单且多年未变；起一次子进程（Windows 上约 150ms）换 40 行纯逻辑
/// 不划算，而且校验错误要给"人话原因"（实现要求 3），子进程的 stderr 反而
/// 要再解析一遍。规则清单与 `git-check-ref-format(1)` 文档一一对应。
///
/// 返回 `Ok(())` 或"违反了哪条规则"的人话描述。
pub fn validate_ref_name(name: &str) -> Result<(), &'static str> {
    if name.is_empty() {
        return Err("名称不能为空");
    }
    if name.starts_with('.') {
        return Err("不能以点开头");
    }
    if name.starts_with('/') {
        return Err("不能以斜杠开头");
    }
    if name.ends_with('/') {
        return Err("不能以斜杠结尾");
    }
    if name.ends_with('.') {
        return Err("不能以点结尾");
    }
    if name.ends_with(".lock") {
        return Err("不能以 .lock 结尾");
    }
    if name.contains("..") {
        return Err("不能包含连续的点");
    }
    if name.contains("//") {
        return Err("不能包含连续的斜杠");
    }
    for ch in name.chars() {
        if ch.is_whitespace() {
            return Err("不能包含空格或空白字符");
        }
        if matches!(ch, '~' | '^' | ':' | '?' | '*' | '[' | '\\' | '\u{7f}') {
            return Err("包含 git 不允许的字符（~ ^ : ? * [ \\ 或 DEL）");
        }
        if (ch as u32) < 0x20 {
            return Err("包含控制字符");
        }
    }
    // 分量（以 / 分隔）不能以 .lock 结尾，也不能为空
    for component in name.split('/') {
        if component.is_empty() {
            return Err("包含空的路径分量（连续斜杠）");
        }
        if component.ends_with(".lock") {
            return Err("路径分量不能以 .lock 结尾");
        }
        if component == "@" {
            return Err("单个 @ 不是合法的分量");
        }
        if component.contains("@{") {
            return Err("不能包含 @{ 序列");
        }
    }
    Ok(())
}

/// 合并策略（T3.4）。
///
/// `git` 没有原生的 `-s theirs`：`Theirs` 用 `-X theirs` 表达（**冲突偏向对方**，
/// 非冲突变更仍采用本方）——与 `Ours`（`-s ours`，整树采用本方）语义不同，
/// 界面文案必须区分这两者，不能都写成"采用对方"。
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "camelCase")]
pub enum MergeStrategy {
    /// 默认合并：可以快进就快进，否则产生合并提交。
    #[default]
    Merge,
    /// `--no-ff`：总是产生合并提交。
    NoFf,
    /// `--squash`：把对方变更压进索引，**不产生合并提交**（也不自动提交）。
    Squash,
    /// `--ff-only`：只允许快进，否则失败。
    FastForwardOnly,
    /// `-s ours`：整树采用本方（对方的变更全部丢弃）。
    Ours,
    /// `-X theirs`：冲突偏向对方（非冲突变更仍采用本方）。
    Theirs,
}

/// 合并参数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeSpec {
    /// 要合并进来的引用。
    pub revision: String,
    /// 合并策略（T3.4：替代此前的 `no_ff` / `ff_only` 两个布尔——
    /// 策略是互斥的一组选择，两个布尔表达不了 `--squash` / `-s ours`）。
    pub strategy: MergeStrategy,
    /// 合并提交信息（产生合并提交的策略使用）。
    pub message: Option<String>,
}

impl MergeSpec {
    /// 默认合并（允许快进）。
    pub fn new(revision: impl Into<String>) -> Self {
        Self {
            revision: revision.into(),
            strategy: MergeStrategy::Merge,
            message: None,
        }
    }

    /// 指定策略。
    #[must_use]
    pub fn with_strategy(mut self, strategy: MergeStrategy) -> Self {
        self.strategy = strategy;
        self
    }

    /// 指定合并提交信息。
    #[must_use]
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }
}

/// 快进判定（纯函数，T3.4 的合并预览用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FfVerdict {
    /// HEAD 已包含 source，无事可做。
    UpToDate,
    /// 可以快进（HEAD 是 source 的祖先）。
    FastForward,
    /// 需要真正的合并提交。
    TrueMerge,
}

impl FfVerdict {
    /// 稳定短名（DTO 与日志用）。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UpToDate => "upToDate",
            Self::FastForward => "fastForward",
            Self::TrueMerge => "trueMerge",
        }
    }
}

/// 快进判定：`head_oid` / `merge_base_oid` / `source_oid` 三个 oid 的关系。
///
/// 无关历史（merge-base 解析不出来）由调用方传空串 → 判为真合并
/// （git 执行时会拒绝无关历史合并，预检不替它猜）。
pub fn ff_verdict(head_oid: &str, merge_base_oid: &str, source_oid: &str) -> FfVerdict {
    if head_oid == source_oid {
        FfVerdict::UpToDate
    } else if merge_base_oid == head_oid && !merge_base_oid.is_empty() {
        FfVerdict::FastForward
    } else {
        FfVerdict::TrueMerge
    }
}

/// 默认合并信息（git 惯例；`into` 是被合并进的分支名，游离 HEAD 时为 `None`）。
pub fn default_merge_message(source: &str, into: Option<&str>) -> String {
    match into {
        Some(into) => format!("Merge branch '{source}' into {into}"),
        None => format!("Merge branch '{source}'"),
    }
}

/// 等价命令字符串（纯函数）。
///
/// `--squash` 与 `--ff-only` 不产生合并提交，即使给了 message 也不带 `-m`
/// （信息没有载体）；`Merge` / `NoFf` / `Ours` / `Theirs` 产生合并提交，带 `-m`。
pub fn equivalent_merge_command(
    source: &str,
    strategy: MergeStrategy,
    message: Option<&str>,
) -> String {
    let base = match strategy {
        MergeStrategy::Merge => format!("git merge {source}"),
        MergeStrategy::NoFf => format!("git merge --no-ff {source}"),
        MergeStrategy::Squash => format!("git merge --squash {source}"),
        MergeStrategy::FastForwardOnly => format!("git merge --ff-only {source}"),
        MergeStrategy::Ours => format!("git merge -s ours {source}"),
        MergeStrategy::Theirs => format!("git merge -X theirs {source}"),
    };
    let takes_message = matches!(
        strategy,
        MergeStrategy::Merge | MergeStrategy::NoFf | MergeStrategy::Ours | MergeStrategy::Theirs
    );
    match (takes_message, message) {
        (true, Some(message)) => format!("{base} -m {message:?}"),
        _ => base,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod merge_strategy_tests {
    use super::{
        default_merge_message, equivalent_merge_command, ff_verdict, FfVerdict, MergeSpec,
        MergeStrategy,
    };

    #[test]
    fn default_merge_allows_fast_forward_and_chains() {
        let spec = MergeSpec::new("feature");
        assert_eq!(spec.strategy, MergeStrategy::Merge);
        let spec = spec
            .with_strategy(MergeStrategy::NoFf)
            .with_message("merge it");
        assert_eq!(spec.strategy, MergeStrategy::NoFf);
        assert_eq!(spec.message.as_deref(), Some("merge it"));
    }

    #[test]
    fn strategy_serializes_as_camel_case() {
        let value = serde_json::to_value(MergeStrategy::FastForwardOnly).unwrap();
        assert_eq!(value, serde_json::Value::String("fastForwardOnly".into()));
    }

    #[test]
    fn ff_verdict_covers_the_three_relations() {
        assert_eq!(ff_verdict("aaa", "bbb", "aaa"), FfVerdict::UpToDate);
        assert_eq!(ff_verdict("aaa", "aaa", "ccc"), FfVerdict::FastForward);
        assert_eq!(ff_verdict("aaa", "bbb", "ccc"), FfVerdict::TrueMerge);
        // 无关历史：merge-base 解析不出来（空串）——git 会拒绝，预检不猜
        assert_eq!(ff_verdict("aaa", "", "ccc"), FfVerdict::TrueMerge);
    }

    #[test]
    fn default_message_follows_git_conventions() {
        assert_eq!(
            default_merge_message("feature", None),
            "Merge branch 'feature'"
        );
        assert_eq!(
            default_merge_message("feature", Some("main")),
            "Merge branch 'feature' into main"
        );
    }

    #[test]
    fn equivalent_command_maps_every_strategy() {
        assert_eq!(
            equivalent_merge_command("feat", MergeStrategy::Merge, None),
            "git merge feat"
        );
        assert_eq!(
            equivalent_merge_command("feat", MergeStrategy::NoFf, Some("m")),
            "git merge --no-ff feat -m \"m\""
        );
        // squash / ff-only 不产生合并提交：信息没有载体，不带 -m
        assert_eq!(
            equivalent_merge_command("feat", MergeStrategy::Squash, Some("m")),
            "git merge --squash feat"
        );
        assert_eq!(
            equivalent_merge_command("feat", MergeStrategy::FastForwardOnly, Some("m")),
            "git merge --ff-only feat"
        );
        assert_eq!(
            equivalent_merge_command("feat", MergeStrategy::Ours, Some("m")),
            "git merge -s ours feat -m \"m\""
        );
        assert_eq!(
            equivalent_merge_command("feat", MergeStrategy::Theirs, None),
            "git merge -X theirs feat"
        );
    }
}

/// 拉取策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
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
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FetchSpec {
    /// 远端名。`None` = 当前分支的上游远端（没有上游时用 `origin`）。
    pub remote: Option<String>,
    /// 是否清理远端已删除的跟踪分支（`--prune`）。
    pub prune: bool,
    /// 只取这些引用（空 = 远端默认 refspec）。
    pub refspecs: Vec<String>,
    /// 是否同时取标签。
    pub tags: bool,
    /// 浅取深度（`--depth`）。`None` = 完整历史。
    pub depth: Option<u32>,
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
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PullSpec {
    /// 远端名。`None` = 当前分支的上游。
    pub remote: Option<String>,
    /// 远端分支名。`None` = 上游分支。
    pub branch: Option<String>,
    /// 策略。
    pub strategy: PullStrategy,
    /// 工作区不干净时自动 stash 并在完成后恢复（`--autostash`）。
    pub autostash: bool,
    /// 允许合并没有共同祖先的历史（`--allow-unrelated-histories`）。
    pub allow_unrelated: bool,
}

impl Default for PullSpec {
    fn default() -> Self {
        Self {
            remote: None,
            branch: None,
            strategy: PullStrategy::FastForwardOnly,
            autostash: false,
            allow_unrelated: false,
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
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
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
    /// 远端分支名；`None` = 与本地同名，`Some` = 推到不同名（`<branch>:<remote_branch>`）。
    pub remote_branch: Option<String>,
    /// 演练模式（`--dry-run`）：照常计算但不真正更新远端。
    pub dry_run: bool,
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
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReorderStep {
    /// 被操作的提交 oid。
    pub oid: String,
    /// 对该提交执行的动作。
    pub action: ReorderAction,
    /// 新的提交信息（`Reword` / `Squash` 使用）。
    pub new_message: Option<String>,
}

/// rebase 计划里对单个提交的动作（与 `git rebase -i` 的指令一一对应）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MergeKind {
    /// 已经在目标提交上，什么都没做。
    AlreadyUpToDate,
    /// 快进（没有产生新提交）。
    FastForward,
    /// 产生了合并提交。
    MergeCommit,
    /// squash（`--squash`）：变更压进索引、HEAD 不动、**没有**产生任何提交。
    Squash,
    /// 产生冲突，停在冲突状态。
    Conflicted,
}

/// 合并预检报告（T3.4：`git merge-tree --write-tree` 的封装）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergePreviewReport {
    /// 快进裁决（HEAD / merge-base / source 的关系）。
    pub verdict: FfVerdict,
    /// 冲突预检：`available` 为假表示 git 太旧、预检不可用（界面退化为
    /// "执行后再报冲突"——任务书允许的权衡，必须明示而不是假装预检过）。
    pub preview: MergePreview,
}

/// 冲突预检结果。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MergePreview {
    /// 预检是否可用。
    pub available: bool,
    /// 预检发现的冲突文件（available 且非空 = 预检到冲突）。
    pub conflicted: Vec<RepoPath>,
}

/// 合并计划（prepare 的产物；execute 只认 plan_id，HEAD 变了即 PLAN_STALE）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergePlan {
    /// 计划句柄。
    pub plan_id: String,
    /// 要合并进来的引用。
    pub source: String,
    /// 合并策略。
    pub strategy: MergeStrategy,
    /// 快进裁决。
    pub verdict: FfVerdict,
    /// source 独有的提交（HEAD..source，新到旧，最多 30 条）。
    pub source_only_commits: Vec<CommitSummary>,
    /// source 独有提交的精确总数。
    pub source_commit_count: usize,
    /// 冲突预检。
    pub preview: MergePreview,
    /// 默认合并信息（用户可在预览里编辑；编辑值经 execute 传入）。
    pub default_message: String,
    /// 等价命令（教育价值：让用户看到"这就是 git merge …"）。
    pub equivalent_command: String,
    /// 计划生成时的 HEAD（执行时不一致即 [`crate::ErrorCode::PlanStale`]）。
    pub head_before: String,
}

/// 合并结果。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeOutcome {
    /// 结果类别。
    pub kind: MergeKind,
    /// 合并后的 HEAD oid（冲突时为 `None`）。
    pub oid: Option<String>,
    /// 冲突文件（`kind == Conflicted` 时非空）。
    pub conflicts: Vec<RepoPath>,
    /// 操作前打的快照 id（服务层填；引擎构造的临时值恒为 `None`）。
    ///
    /// 它随 IPC 契约（`snapshotId`）流向前端与审计表——`operation_records.
    /// snapshot_id` 就是从这个字段提取的，少了它 cherry-pick / revert 的
    /// "能不能回滚"在操作历史里永远是"否"（T2.10 修复的断链）。
    pub snapshot_id: Option<i64>,
}

impl MergeOutcome {
    /// 是否产生了冲突。
    pub fn has_conflicts(&self) -> bool {
        !self.conflicts.is_empty()
    }
}

/// fetch 结果。
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
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
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PullOutcome {
    /// fetch 阶段的结果。
    pub fetch: FetchOutcome,
    /// 实际使用的策略。
    pub strategy: PullStrategy,
    /// 本地是否本来就已经是最新（此时不会有合并结果）。
    pub up_to_date: bool,
    /// 合并 / 快进阶段的结果；`up_to_date` 时为 `None`。
    pub merge: Option<MergeOutcome>,
    /// 操作前打的 `PreSync` 快照 id（服务层填；引擎构造的临时值恒为 `None`）。
    pub snapshot_id: Option<i64>,
}

impl PullOutcome {
    /// 是否产生了冲突。
    pub fn has_conflicts(&self) -> bool {
        self.merge.as_ref().is_some_and(MergeOutcome::has_conflicts)
    }
}

/// push 被拒绝的原因。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
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
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
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
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
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
    /// 删除**全部** stash（`git stash clear`）。**不可逆**（Dangerous）。
    Clear,
    /// 从某条 stash 创建分支并把它应用过去（`git stash branch <name> <stash>`）。
    ///
    /// 这是"pop 冲突之后的正规出路"：新分支从 stash 的 base 提交开始，
    /// 因此那条 stash 一定可以干净地应用上去（冲突的产生原因是 base 变了）。
    /// 注意它会**切换分支**（移动 HEAD），执行前必须打快照。
    Branch {
        /// `stash@{n}` 里的 n。
        index: usize,
        /// 新分支名（先过 [`validate_ref_name`]）。
        name: String,
    },
}

impl StashAction {
    /// 该动作是否会丢失数据。
    ///
    /// `Drop` / `Clear` 会**删掉** stash 提交：在 `gc` 真正回收之前它们还能按 oid
    /// 找回，但界面上必须按"不可逆"对待（红线 R7）。
    pub const fn is_destructive(&self) -> bool {
        matches!(self, Self::Drop { .. } | Self::Clear)
    }
}

/// stash 操作的参数。
///
/// 与 [`ResetSpec`] 同理只派生 `Serialize`：`paths` 是保真的 [`RepoPath`]，
/// 它的线上表示属于 DTO 层（commands 里的请求结构体负责）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StashSpec {
    /// 要执行的子操作。
    pub action: StashAction,
    /// 描述信息（仅 `Push` 使用）。
    pub message: Option<String>,
    /// 是否把未跟踪文件一起储藏（`-u`）。
    pub include_untracked: bool,
    /// 是否保留索引（`--keep-index`）。
    pub keep_index: bool,
    /// 只储藏这些路径（空 = 全部改动）。
    ///
    /// 与 `git stash push -- <paths>` 同一语义：**只影响 worktree 侧**——
    /// 路径之外的改动留在原地。
    pub paths: Vec<RepoPath>,
    /// `apply` / `pop` 时同时恢复索引（`--index`）。
    ///
    /// 不传时的行为是 git 的默认：把 stash 的全部改动放进**工作区**（不还原暂存态）。
    /// 界面的"恢复暂存状态"勾选框直接映射到它。
    pub restore_index: bool,
}

impl StashSpec {
    /// 储藏当前改动。
    pub fn push(message: Option<String>) -> Self {
        Self {
            action: StashAction::Push,
            message,
            include_untracked: false,
            keep_index: false,
            paths: Vec::new(),
            restore_index: false,
        }
    }

    /// 应用某个 stash（`apply`，保留该条）。
    pub fn apply(index: usize) -> Self {
        Self {
            action: StashAction::Apply { index },
            message: None,
            include_untracked: false,
            keep_index: false,
            paths: Vec::new(),
            restore_index: false,
        }
    }

    /// 应用并删除某个 stash（`pop`）。
    pub fn pop(index: usize) -> Self {
        Self {
            action: StashAction::Pop { index },
            message: None,
            include_untracked: false,
            keep_index: false,
            paths: Vec::new(),
            restore_index: false,
        }
    }

    /// 删除某个 stash（不可逆）。
    pub fn drop(index: usize) -> Self {
        Self {
            action: StashAction::Drop { index },
            message: None,
            include_untracked: false,
            keep_index: false,
            paths: Vec::new(),
            restore_index: false,
        }
    }

    /// 删除全部 stash（不可逆）。
    pub fn clear() -> Self {
        Self {
            action: StashAction::Clear,
            message: None,
            include_untracked: false,
            keep_index: false,
            paths: Vec::new(),
            restore_index: false,
        }
    }

    /// 从某条 stash 创建分支（会切换分支）。
    pub fn branch(index: usize, name: impl Into<String>) -> Self {
        Self {
            action: StashAction::Branch {
                index,
                name: name.into(),
            },
            message: None,
            include_untracked: false,
            keep_index: false,
            paths: Vec::new(),
            restore_index: false,
        }
    }

    /// 只储藏这些路径。
    #[must_use]
    pub fn with_paths(mut self, paths: Vec<RepoPath>) -> Self {
        self.paths = paths;
        self
    }

    /// 同时恢复索引（`apply` / `pop`）。
    #[must_use]
    pub fn with_restore_index(mut self, restore_index: bool) -> Self {
        self.restore_index = restore_index;
        self
    }
}

/// 一条 reflog 记录。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
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

/// 拣选（`git cherry-pick`）参数。
///
/// 结果复用 [`MergeOutcome`]：cherry-pick 与 merge 的冲突语义完全一样（都会让仓库
/// 停在冲突状态并给出冲突文件清单），另造一个 outcome 类型只会让前端多一套分支。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CherryPickSpec {
    /// 要拣选的提交：单个 rev，或 `A..B` 区间（多个提交）。
    pub revision: String,
    /// 在提交信息里附上来源（`-x`，形如 `(cherry picked from commit 1a2b3c4)`）。
    ///
    /// 只在**能追溯来源**的场合有意义：跨分支移植时这是别人事后判断
    /// "这段改动从哪来"的唯一线索。
    pub record_source: bool,
    /// 只应用改动、不提交（`--no-commit`）。区间拣选时用它一次改完再自己写一条提交。
    pub no_commit: bool,
}

impl CherryPickSpec {
    /// 拣选单个提交，默认行为（提交、不记录来源）。
    pub fn new(revision: impl Into<String>) -> Self {
        Self {
            revision: revision.into(),
            record_source: false,
            no_commit: false,
        }
    }
}

/// 反转（`git revert`）参数。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevertSpec {
    /// 要反转的提交：单个 rev，或 `A..B` 区间。
    pub revision: String,
    /// 反转**合并提交**时指定主父（`-m <n>`，从 1 开始）。
    ///
    /// 合并提交有两个父，`revert` 必须知道"以哪一边为主线来反转"：猜错会得到
    /// 与意图相反的结果。因此这里没有默认值，界面必须让用户明确选择。
    pub mainline: Option<u32>,
    /// 只应用改动、不提交（`--no-commit`）。
    pub no_commit: bool,
}

impl RevertSpec {
    /// 反转单个提交，默认行为（提交、不指定主父）。
    pub fn new(revision: impl Into<String>) -> Self {
        Self {
            revision: revision.into(),
            mainline: None,
            no_commit: false,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod t28_wire_shape_tests {
    use super::{CherryPickSpec, RevertSpec, StashAction, StashSpec};

    #[test]
    fn history_operation_specs_serialise_in_the_documented_camel_case_shape() {
        // 契约（docs/API.md §1）：DTO 一律 camelCase。前端把 `recordSource` /
        // `noCommit` / `mainline` 直接拼进请求，形状错了不会报错——只是选项被静默忽略
        // （用户以为勾了"记录来源"，提交信息里却没有）。
        let pick = CherryPickSpec {
            record_source: true,
            no_commit: true,
            ..CherryPickSpec::new("main..feature")
        };
        let json = serde_json::to_value(&pick).unwrap();
        assert_eq!(json["recordSource"], true, "{json}");
        assert_eq!(json["noCommit"], true);
        assert_eq!(json["revision"], "main..feature");

        let revert = RevertSpec {
            mainline: Some(2),
            no_commit: false,
            ..RevertSpec::new("abc123")
        };
        let json = serde_json::to_value(&revert).unwrap();
        assert_eq!(json["mainline"], 2, "{json}");
        assert_eq!(json["noCommit"], false);

        let stash = StashSpec {
            action: StashAction::Drop { index: 1 },
            include_untracked: true,
            ..StashSpec::push(None)
        };
        let json = serde_json::to_value(&stash).unwrap();
        assert_eq!(json["action"]["drop"]["index"], 1, "{json}");
        assert_eq!(json["includeUntracked"], true);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{
        ApplyDirection, ApplyPatchSpec, ApplyTarget, CheckoutSpec, CommitSpec, FetchOutcome,
        InitSpec, MergeKind, MergeOutcome, PullOutcome, PullStrategy, PushOutcome, PushRejection,
        ReflogEntry, ReorderAction, ResetMode, ResetSpec, StageSpec, StashAction, StashSpec,
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
    fn only_stash_drop_and_clear_are_destructive() {
        assert!(!StashAction::Push.is_destructive());
        assert!(!StashAction::Apply { index: 0 }.is_destructive());
        assert!(!StashAction::Pop { index: 0 }.is_destructive());
        // 建分支会移动 HEAD，但那条 stash 还在（由 `--index`/快照兜底），不算丢数据
        assert!(!StashAction::Branch {
            index: 0,
            name: "recover".to_owned()
        }
        .is_destructive());
        assert!(StashAction::Drop { index: 0 }.is_destructive());
        assert!(StashAction::Clear.is_destructive());
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
                snapshot_id: None,
            }),
            snapshot_id: None,
        };
        let fast_forwarded = PullOutcome {
            merge: Some(MergeOutcome {
                kind: MergeKind::FastForward,
                oid: Some("abc".to_owned()),
                conflicts: Vec::new(),
                snapshot_id: None,
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
            snapshot_id: None,
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

    #[test]
    fn sync_outcomes_serialise_in_the_documented_camel_case_shape() {
        // 契约（docs/API.md §1）：DTO 一律 camelCase。前端的 `job:done.result`
        // 直接读这些字段（`upToDate` / `nonFastForward` / `kind`），
        // 形状错了界面只会"看起来没冲突/没被拒"，不会报错。
        let pulled = PullOutcome {
            fetch: FetchOutcome::default(),
            strategy: PullStrategy::FastForwardOnly,
            up_to_date: true,
            merge: None,
            snapshot_id: Some(7),
        };
        let json = serde_json::to_value(&pulled).unwrap();
        assert_eq!(json["upToDate"], true, "{json}");
        assert_eq!(json["strategy"], "fastForwardOnly");
        // snapshotId 也在契约里：审计表的关联列就是从这个字段提取的（T2.10）
        assert_eq!(json["snapshotId"], 7, "{json}");

        let rejection = PushRejection {
            name: "main".to_owned(),
            reason: "non-fast-forward".to_owned(),
            non_fast_forward: true,
        };
        let json = serde_json::to_value(&rejection).unwrap();
        assert_eq!(json["nonFastForward"], true, "{json}");

        let merge = MergeOutcome {
            kind: MergeKind::Conflicted,
            oid: None,
            conflicts: Vec::new(),
            snapshot_id: None,
        };
        assert_eq!(serde_json::to_value(&merge).unwrap()["kind"], "conflicted");
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod ref_name_tests {
    use super::validate_ref_name;

    /// 任务书 T2.5 实现要求 3：≥ 12 个非法用例（每条规则至少一个）。
    #[test]
    fn rejects_the_documented_illegal_names_with_reasons() {
        // (名称, 期望的错误片段)
        let cases: &[(&str, &str)] = &[
            ("", "不能为空"),
            ("feat branch", "空白"),
            ("feat\ttab", "空白"),
            ("feature..backup", "连续的点"),
            ("refs//double", "连续的斜杠"),
            ("feat~1", "不允许的字符"),
            ("feat^2", "不允许的字符"),
            ("tag:v1", "不允许的字符"),
            ("what?isthis", "不允许的字符"),
            ("a*b", "不允许的字符"),
            ("[bracket]", "不允许的字符"),
            ("back\\slash", "不允许的字符"),
            ("v1.lock", ".lock"),
            ("dir/sub.lock/x", ".lock"),
            (".hidden", "以点开头"),
            ("trailing.", "以点结尾"),
            ("ends/", "斜杠结尾"),
            ("/absolute", "斜杠开头"),
            ("@{reflog}", "@{"),
            ("@", "单个 @"),
        ];
        assert!(cases.len() >= 12, "任务书要求至少 12 个非法用例");
        for (name, expected) in cases {
            let error = validate_ref_name(name).expect_err(name);
            assert!(
                error.contains(expected),
                "分支名 {name:?} 应报 {expected:?}，实际 {error:?}"
            );
        }
    }

    #[test]
    fn accepts_typical_branch_and_tag_names() {
        for name in [
            "main",
            "feature/login-flow",
            "release/v1.2.3",
            "hotfix-2026-09-28",
            "v1.0.0",
            "issue/42-fix-crash",
            "pyromanic_underscore-name.42",
        ] {
            assert_eq!(validate_ref_name(name), Ok(()), "{name} 应合法");
        }
    }

    #[test]
    fn multibyte_names_are_accepted_when_not_violating_rules() {
        assert_eq!(validate_ref_name("feature/登录"), Ok(()));
        assert!(
            validate_ref_name("feature/登录 分支").is_err(),
            "含空格仍拒绝"
        );
    }
}
