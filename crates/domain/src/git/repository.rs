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
///
/// "打开仓库"这一步的全部输入：既包含界面立即要显示的东西（分支、是否裸仓库），
/// 也包含决定后续交互策略的东西（浅克隆、LFS、worktree、默认分支）。
/// 这些字段都在**同一次**发现里填好，而不是让界面按需再发几轮 IPC——
/// 每一轮 IPC 都是可见延迟（M1 的验收标准是"打开仓库 ≤ 2s"）。
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
    /// 默认分支短名（如 `main`）。
    ///
    /// 取值顺序：`refs/remotes/origin/HEAD` 指向的分支 → 当前分支。
    /// 两者都没有（游离 HEAD 且没有 `origin/HEAD`）时为 `None`。
    /// 用途：新建分支时的建议基点、克隆后展示"默认分支"。
    pub default_branch: Option<String>,
    /// 是否为浅克隆（`git clone --depth`）。
    ///
    /// 浅仓库缺少历史，历史视图与 rebase 类操作必须据此降级提示，
    /// 而不是让用户在"祖先不存在"的错误里困惑。
    pub is_shallow: bool,
    /// 是否启用了 Git LFS。
    ///
    /// 判定依据：仓库根的 `.gitattributes` 里有 `filter=lfs`，
    /// 或 `.git/lfs` 对象目录已存在。这是一个**提示性**字段——
    /// 界面据此提示"该仓库使用 LFS"，不参与任何数据完整性判断。
    pub is_lfs: bool,
    /// 关联的工作区（worktree）列表，**主工作区在首位**。
    ///
    /// 单工作区仓库也返回一个元素（就是仓库自身），这样界面不必区分
    /// "空列表"与"只有一个工作区"两种情形。
    pub worktrees: Vec<Worktree>,
}

impl RepositoryInfo {
    /// 是否处于游离 HEAD 状态（而非空仓库）。
    pub fn is_detached(&self) -> bool {
        self.detached && !self.is_empty
    }

    /// 主工作区（列表首项）；理论上列表恒非空，取不到时返回 `None`。
    pub fn main_worktree(&self) -> Option<&Worktree> {
        self.worktrees.first()
    }

    /// 除主工作区之外还有几个关联工作区。
    pub fn linked_worktree_count(&self) -> usize {
        self.worktrees.len().saturating_sub(1)
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

/// 一个工作区（`git worktree list` 的一条）。
///
/// 工作区与分支是**多对一**的：同一个分支只能被一个工作区检出。
/// 界面在"切换分支"前必须据此判断目标分支是否已被另一个工作区占用，
/// 否则用户会拿到一个 git 层面的失败而不是可理解的提示。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    /// 工作区目录（主工作区即仓库根）。
    pub path: PathBuf,
    /// 该工作区当前检出的提交 oid；空仓库或不可读时为 `None`。
    pub head: Option<String>,
    /// 该工作区检出的分支短名；游离 HEAD 时为 `None`。
    pub branch: Option<String>,
    /// 该工作区是否处于游离 HEAD。
    pub detached: bool,
    /// 该工作区是否为裸仓库（主工作区可能是）。
    pub is_bare: bool,
    /// 是否被 `git worktree lock` 锁定（锁定后不可被 prune）。
    pub locked: bool,
    /// 是否可被 `git worktree prune` 清理（目录已丢失）。
    pub prunable: bool,
}

impl Worktree {
    /// 是否是主工作区（仓库根自身）。
    ///
    /// 判断依据只有"它在不在列表首位"——主工作区一定由 `git worktree list`
    /// 首行给出，而路径比较在 Windows 上要处理大小写与分隔符，容易出错。
    pub fn is_main(&self, main_path: &Path) -> bool {
        self.path == main_path
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{BranchLabel, RepoId, RepositoryInfo, Worktree};
    use std::path::{Path, PathBuf};

    fn worktree(path: &str) -> Worktree {
        Worktree {
            path: PathBuf::from(path),
            head: Some("a".repeat(40)),
            branch: Some("main".to_owned()),
            detached: false,
            is_bare: false,
            locked: false,
            prunable: false,
        }
    }

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
            default_branch: head.map(str::to_owned),
            is_shallow: false,
            is_lfs: false,
            worktrees: vec![worktree("/tmp/repo")],
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

    #[test]
    fn main_worktree_is_the_first_entry_and_linked_ones_are_counted() {
        let mut single = info(Some("main"), false, false);
        assert_eq!(
            single.main_worktree().map(|tree| tree.path.as_path()),
            Some(Path::new("/tmp/repo"))
        );
        assert_eq!(single.linked_worktree_count(), 0);

        single.worktrees.push(worktree("/tmp/repo-feature"));
        assert_eq!(single.linked_worktree_count(), 1);
        assert!(single.worktrees[1].is_main(Path::new("/tmp/repo-feature")));
        assert!(!single.worktrees[1].is_main(Path::new("/tmp/repo")));
    }
}
