//! 仓库标识与基本信息。

use std::fmt;
use std::path::{Path, PathBuf};

/// 仓库标识。
///
/// 为什么用**路径**而不是 SQLite 里的 `repositories.id`：`crates/git-engine`
/// 是 infra 层，不应依赖 `crates/storage`（分层规则见 docs/ARCHITECTURE.md §3），
/// 而且"用户刚选中的目录"在写入 `repositories` 表之前就已经需要被操作了。
/// 由 `services` 层负责在数据库 id 与路径之间做映射。
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RepoId {
    root: PathBuf,
}

impl RepoId {
    /// 用工作区根目录（裸仓库用仓库根）创建。
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// 工作区根目录。
    pub fn root(&self) -> &Path {
        &self.root
    }
}

impl fmt::Display for RepoId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // 路径可能不是合法 UTF-8，展示层只能 lossy
        f.write_str(&self.root.to_string_lossy())
    }
}

/// 仓库的基本信息（`git rev-parse` / `git status --branch` 能给出的那部分）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryInfo {
    /// 仓库标识。
    pub id: RepoId,
    /// 工作区根目录；裸仓库为 `None`。
    pub workdir: Option<PathBuf>,
    /// `.git` 目录（裸仓库即仓库根）。
    pub git_dir: PathBuf,
    /// 是否为裸仓库。
    pub is_bare: bool,
    /// 是否还没有任何提交（"初始仓库"）。
    ///
    /// 必须与"游离 HEAD"区分：两者都没有分支名，但引导完全不同
    /// （首次提交 vs 回到分支）。
    pub is_empty: bool,
    /// HEAD 指向的分支短名；游离 HEAD 或空仓库为 `None`。
    pub head: Option<String>,
    /// 是否处于游离 HEAD。
    pub detached: bool,
    /// 当前分支的上游短名（如 `origin/main`）。
    pub upstream: Option<String>,
}

impl RepositoryInfo {
    /// 是否处于游离 HEAD 状态（而非空仓库）。
    pub fn is_detached(&self) -> bool {
        self.detached && !self.is_empty
    }

    /// 界面上应展示的分支标签。
    ///
    /// 三个状态必须给出不同的文案依据：空仓库、游离 HEAD、普通分支。
    /// 让界面自己拼字符串会导致三处各拼一遍且互不一致。
    pub fn branch_label(&self) -> BranchLabel<'_> {
        if self.is_empty {
            BranchLabel::Unborn
        } else if self.detached {
            BranchLabel::Detached
        } else {
            BranchLabel::Named(self.head.as_deref().unwrap_or_default())
        }
    }
}

/// 当前分支在界面上的呈现类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchLabel<'a> {
    /// 还没有任何提交。
    Unborn,
    /// 游离 HEAD（`detached HEAD`）。
    Detached,
    /// 普通分支（名字）。
    Named(&'a str),
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{BranchLabel, RepoId, RepositoryInfo};
    use std::path::PathBuf;

    fn info(head: Option<&str>, detached: bool, is_empty: bool) -> RepositoryInfo {
        RepositoryInfo {
            id: RepoId::new("/tmp/repo"),
            workdir: Some(PathBuf::from("/tmp/repo")),
            git_dir: PathBuf::from("/tmp/repo/.git"),
            is_bare: false,
            is_empty,
            head: head.map(str::to_owned),
            detached,
            upstream: None,
        }
    }

    #[test]
    fn repo_id_compares_by_its_root_path() {
        assert_eq!(RepoId::new("/a/b"), RepoId::new(PathBuf::from("/a/b")));
        assert_ne!(RepoId::new("/a/b"), RepoId::new("/a/c"));
        assert_eq!(RepoId::new("/a/b").to_string(), "/a/b");
    }

    #[test]
    fn empty_repository_is_not_reported_as_detached() {
        // 空仓库的 HEAD 也"没有分支名"，但它不是游离 HEAD
        let empty = info(Some("main"), false, true);

        assert!(!empty.is_detached());
        assert_eq!(empty.branch_label(), BranchLabel::Unborn);
    }

    #[test]
    fn detached_head_is_reported_separately_from_a_named_branch() {
        assert_eq!(
            info(None, true, false).branch_label(),
            BranchLabel::Detached
        );
        assert_eq!(
            info(Some("main"), false, false).branch_label(),
            BranchLabel::Named("main")
        );
    }
}
