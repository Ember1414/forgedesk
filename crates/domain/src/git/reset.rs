//! 重置（reset）的**计划**与结果（T2.8）。
//!
//! # 为什么 reset 要先出计划
//!
//! `git reset --hard` 会同时改动 HEAD、索引与工作区，而**被丢弃的内容没有任何预告**。
//! 红线 R7 要求破坏性操作一律走"计划预览 → 快照 → 执行 → 可回滚"，因此 reset 是
//! 两段式的：先算出将被丢弃的东西（提交、已暂存改动、工作区改动、未跟踪文件），
//! 再由用户确认后执行。
//!
//! 计划里必须回答一个用户真正关心的问题：**远端是否已有这些提交**。它是"丢弃后
//! 还能不能从远端找回"的唯一依据——只存在于本地的提交被丢弃后，就只能靠本地 reflog
//! （默认两周）或我们执行前打的快照了。
//!
//! # 这里的类型不带 serde
//!
//! 与 [`super::commit_plan`] 同一个约定：领域层说"有什么"，IPC 形状由 commands 的
//! DTO 决定（本模块的字段里有 [`RepoPath`] 与 [`FileChange`]——它们的线上表示是
//! T1.4 的 DTO 层工作，在领域层随便定一个形状只会在前端接上时返工）。

use super::path::RepoPath;
use super::spec::ResetMode;
use super::status::FileChange;

/// 计划里的一条提交摘要。
///
/// 只带界面要显示的三样，而不是整个 [`super::commit::Commit`]：计划预览要列出
/// 几十条提交，"每条都带上作者邮箱、签名状态、refs 装饰"会让这个响应大一个量级，
/// 而那些字段在"我要丢弃哪些提交"这个问题上没有用。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitSummary {
    /// 提交 oid。
    pub oid: String,
    /// 提交信息首行。
    pub subject: String,
    /// 作者时间（Unix 秒）。
    pub author_time: Option<i64>,
}

/// 被丢弃的提交与远端的关系。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResetRemoteImpact {
    /// 当前分支的上游（如 `origin/main`）；没有上游时为 `None`
    /// （此时 `not_on_remote` 等于提交总数：本地没有跟踪目标，自然谈不上远端有）。
    pub upstream: Option<String>,
    /// 将被丢弃、且**远端也找不到**的提交数。
    ///
    /// `0` = 远端已有全部将丢弃的提交（它们仍能从远端取回）；
    /// `> 0` = 这些提交只存在于本地。
    pub not_on_remote: usize,
}

/// 重置的影响摘要（执行前的预览）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResetPlan {
    /// 计划句柄；执行时回传，防止"看了 A 却执行了 B"。
    pub plan_id: String,
    /// 重置模式。
    pub mode: ResetMode,
    /// 目标提交 oid（执行前已解析，避免执行时再解析一次而目标已经变了）。
    pub target_oid: String,
    /// 目标提交的首行信息（让用户确认"是这条"）。
    pub target_subject: String,
    /// 计划生成时的 HEAD（与执行时的 HEAD 不一致即 [`crate::ErrorCode::PlanStale`]）。
    pub head_before: String,
    /// 将被丢弃的提交（从新到旧，最多 [`ResetPlan::MAX_LISTED_COMMITS`] 条）。
    pub discarded: Vec<CommitSummary>,
    /// `discarded` 是否被截断（即还有更多没列出）。
    pub discarded_truncated: bool,
    /// 将被丢弃的提交总数（精确值，不受截断影响）。
    pub discarded_count: usize,
    /// 将被丢弃的**已暂存**改动（`--mixed` / `--hard`）。
    pub lost_staged: Vec<FileChange>,
    /// 将被丢弃的**工作区**改动（仅 `--hard`；其余模式为空）。
    pub lost_worktree: Vec<FileChange>,
    /// 将被删除的未跟踪文件（仅 `--hard`；`git reset --hard` 会删掉它们）。
    pub untracked_to_remove: Vec<RepoPath>,
    /// 远端影响。
    pub remote: ResetRemoteImpact,
    /// 是否需要用户**输入确认词**（`--hard` 需要）。
    pub requires_confirmation: bool,
    /// 执行前是否必须打快照。四种组合共用一条规则：**HEAD 会移动就打**。
    pub snapshot_required: bool,
}

impl ResetPlan {
    /// 计划里最多列出多少条将被丢弃的提交。
    ///
    /// 30 条足够回答"我要丢的是不是我以为的那些"，而 `--hard` 到很旧的提交时
    /// 列出几千条只会把对话框变成一片噪声。
    pub const MAX_LISTED_COMMITS: usize = 30;

    /// 计划要求的确认词（`requires_confirmation` 为真时）。
    ///
    /// 用**动词本身**而不是 "yes"：用户要输入的东西应当让他停顿一下想清楚自己
    /// 正在做什么。
    pub const CONFIRMATION_WORD: &'static str = "reset";

    /// 用户输入是否算通过确认。
    ///
    /// 去掉首尾空白、忽略大小写：移动端键盘会自动大写首字母，为此卡住用户
    /// 只会让他以为输入框坏了。
    pub fn confirmation_matches(input: &str) -> bool {
        input.trim().eq_ignore_ascii_case(Self::CONFIRMATION_WORD)
    }
}

/// 重置的执行结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResetOutcome {
    /// 实际执行的模式。
    pub mode: ResetMode,
    /// 执行前的 HEAD。
    pub head_before: String,
    /// 执行后的 HEAD。
    pub head_after: String,
    /// 被丢弃的提交数（与计划里的 `discarded_count` 对照，用于发现"执行期间又提交了"）。
    pub discarded_count: usize,
    /// 执行前打的快照 id（回滚入口）。
    pub snapshot_id: Option<i64>,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::ResetPlan;

    #[test]
    fn the_confirmation_word_is_accepted_case_insensitively_and_without_surrounding_space() {
        assert!(ResetPlan::confirmation_matches("reset"));
        assert!(ResetPlan::confirmation_matches("  RESET "));
        // 近似词一律不算：这正是要防的"手快"
        assert!(!ResetPlan::confirmation_matches("resets"));
        assert!(!ResetPlan::confirmation_matches("yes"));
        assert!(!ResetPlan::confirmation_matches(""));
    }
}
