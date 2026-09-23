//! 储藏（stash）模型。

/// 一条 stash 记录（`git stash list` 的语义）。
#[derive(Debug, Clone, PartialEq, Eq)]
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
}

impl StashEntry {
    /// `stash@{n}` 形式的引用名。
    pub fn reference(&self) -> String {
        format!("stash@{{{}}}", self.index)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::StashEntry;

    #[test]
    fn reference_is_the_reflog_form_of_the_index() {
        let entry = StashEntry {
            index: 3,
            oid: "abc".to_owned(),
            base_oid: None,
            message: "WIP on main".to_owned(),
            created_at: None,
            includes_untracked: false,
        };

        assert_eq!(entry.reference(), "stash@{3}");
    }
}
