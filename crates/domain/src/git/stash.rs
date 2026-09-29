//! 储藏（stash）模型。

/// 一条 stash 记录（`git stash list` 的语义）。
///
/// 序列化形状是 IPC 契约（docs/API.md §1：camelCase）：前端的 stash 面板直接读
/// `baseOid` / `includesUntracked` 决定"能不能显示相对基线的 diff"与"要不要提示
/// 未跟踪文件也在里面"。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StashEntry {
    /// `stash@{n}` 里的 `n`。**不稳定的句柄**：任何 stash 操作都会重排它，
    /// 因此它只用于"用户此刻看到的第几项"，真正要长期引用必须用 [`StashEntry::oid`]。
    pub index: usize,
    /// stash 提交的 oid。
    pub oid: String,
    /// stash 的基线提交（stash 的第一个父提交，即"储藏时 HEAD 指向的提交"）。
    pub base_oid: Option<String>,
    /// 描述信息（默认形如 `WIP on main: 1a2b3c4 subject`）。
    pub message: String,
    /// 创建时间（Unix 秒）。
    pub created_at: Option<i64>,
    /// 是否包含未跟踪文件（`git stash -u` 创建的）。
    pub includes_untracked: bool,
    /// 未跟踪文件所在的第三个父提交（只有 `-u` 创建的 stash 才有）。
    ///
    /// 为什么要把它带出来：stash 是"多父提交"，相对 base 的 diff **不包含**
    /// 未跟踪文件——它们在那个独立的父提交里。少了这个 oid，界面只能对用户
    /// 隐瞒"这条 stash 里还有一些改动你看不到"，或者干脆显示一份残缺的 diff。
    pub untracked_oid: Option<String>,
}

impl StashEntry {
    /// `stash@{n}` 形式的引用名。
    pub fn reference(&self) -> String {
        format!("stash@{{{}}}", self.index)
    }
}

/// stash 动作的结果。
///
/// `apply` / `pop` **可能冲突**（stash 的内容与当前工作区不兼容）：这时 git 以非零
/// 退出码结束，但仓库已经进入冲突状态、并且留下了未合并的索引条目——那是要交给用户
/// 的结果，不是错误。把冲突清单带出来，界面才能把他送到冲突页。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StashOutcome {
    /// 冲突的路径（无冲突时为空）。
    pub conflicts: Vec<super::path::RepoPath>,
}

impl StashOutcome {
    /// 是否停在冲突状态。
    pub fn has_conflicts(&self) -> bool {
        !self.conflicts.is_empty()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{StashEntry, StashOutcome};

    #[test]
    fn reference_is_the_reflog_form_of_the_index() {
        let entry = StashEntry {
            index: 3,
            oid: "abc".to_owned(),
            base_oid: None,
            message: "WIP on main".to_owned(),
            created_at: None,
            includes_untracked: false,
            untracked_oid: None,
        };

        assert_eq!(entry.reference(), "stash@{3}");
    }

    #[test]
    fn the_outcome_reports_conflicts_without_being_an_error() {
        // 冲突是**结果**：界面据此把用户送到冲突页，而不是弹一个红色错误
        let outcome = StashOutcome {
            conflicts: vec![crate::git::RepoPath::from("a.txt")],
        };
        assert!(outcome.has_conflicts());
        assert!(!StashOutcome {
            conflicts: Vec::new()
        }
        .has_conflicts());
    }
}
