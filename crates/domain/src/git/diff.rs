//! 变更统计与 diff 模型。
//!
//! # 两个层次刻意分开
//!
//! - [`FileStat`]：文件级统计，来自 `git diff --numstat`。**T1.1 就已可用**，
//!   状态面板与"改了 12 行"这类提示只需要它。
//! - [`DiffReport`] / [`FileDiff`] / [`DiffHunk`] / [`DiffLine`]：行级内容，
//!   来自 unified diff 的解析。**解析器在 T1.5**（见该任务），T1.2 只负责
//!   定义类型并让 `GitEngine::diff` 的签名提前稳定。
//!
//! 把两者混成一个类型会逼着"只想知道改了几行"的调用点也去解析整个补丁——
//! 在大仓库上那是几十 MB 的字符串处理。

use super::path::RepoPath;

/// 单个文件的增删行统计。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStat {
    /// 路径（重命名/复制时是**目标**路径）。
    pub path: RepoPath,
    /// 重命名/复制的来源路径，其余情况为 `None`。
    pub original_path: Option<RepoPath>,
    /// 新增行数。二进制文件为 `None`。
    pub additions: Option<u64>,
    /// 删除行数。二进制文件为 `None`。
    pub deletions: Option<u64>,
    /// 是否为二进制文件。
    ///
    /// 单独一个布尔而不是"additions 为 None 就代表二进制"：`--numstat` 用
    /// `-\t-` 表示二进制，用 `0\t0` 表示"内容变了但没有行数变化"（如模式变更）。
    /// 把两者混为一谈会让界面把模式变更显示成"二进制文件"。
    pub binary: bool,
}

impl FileStat {
    /// 是否发生了重命名或复制。
    pub fn is_rename_or_copy(&self) -> bool {
        self.original_path.is_some()
    }

    /// 变更的总行数；二进制文件返回 `None`。
    pub fn changed_lines(&self) -> Option<u64> {
        match (self.additions, self.deletions) {
            (Some(added), Some(removed)) => Some(added.saturating_add(removed)),
            _ => None,
        }
    }
}

/// 要比较哪两侧。
///
/// 用枚举而不是一对 `from`/`to` 引用：后者的组合空间里有大量无意义的取值
/// （例如"从某个提交到索引"），而每一种有意义的组合在 git 里都是一条**不同的命令**。
/// 枚举让"用户看到的选项"与"实际执行的命令"一一对应。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffTarget {
    /// HEAD ↔ 索引：已暂存的变更（`git diff --cached`）。
    Staged,
    /// 索引 ↔ 工作区：尚未暂存的变更（`git diff`）。
    Unstaged,
    /// 两个提交之间（`git diff <from> <to>`）。
    Between {
        /// 起点。
        from: String,
        /// 终点。
        to: String,
    },
    /// 某个提交 ↔ 工作区（`git diff <revision>`）。
    Since(String),
    /// 单个提交与其父提交（`git show`）。
    Commit(String),
}

impl DiffTarget {
    /// 两个提交之间的差异。
    pub fn between(from: impl Into<String>, to: impl Into<String>) -> Self {
        Self::Between {
            from: from.into(),
            to: to.into(),
        }
    }

    /// 该目标是否会把未提交的工作区改动算进来。
    ///
    /// 界面据此决定是否提示"结果会随你继续编辑而变化"。
    pub fn touches_worktree(&self) -> bool {
        matches!(self, Self::Unstaged | Self::Since(_))
    }
}

/// diff 查询条件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffSpec {
    /// 比较目标。
    pub target: DiffTarget,
    /// 忽略空白变化（`-w`）。
    pub ignore_whitespace: bool,
    /// 上下文行数（`-U<n>`）。
    pub context_lines: u32,
    /// 只包含这些路径（空 = 全部）。
    pub paths: Vec<RepoPath>,
    /// 是否启用重命名检测（`-M`）。
    pub detect_renames: bool,
}

/// 默认上下文行数。
pub const DEFAULT_CONTEXT_LINES: u32 = 3;

impl DiffSpec {
    /// 用比较目标创建（其余取默认值）。
    pub fn new(target: DiffTarget) -> Self {
        Self {
            target,
            ignore_whitespace: false,
            context_lines: DEFAULT_CONTEXT_LINES,
            paths: Vec::new(),
            detect_renames: true,
        }
    }

    /// 忽略空白变化。
    #[must_use]
    pub fn with_ignore_whitespace(mut self, ignore: bool) -> Self {
        self.ignore_whitespace = ignore;
        self
    }

    /// 设置上下文行数。
    #[must_use]
    pub fn with_context_lines(mut self, lines: u32) -> Self {
        self.context_lines = lines;
        self
    }

    /// 限定路径。
    #[must_use]
    pub fn with_paths(mut self, paths: Vec<RepoPath>) -> Self {
        self.paths = paths;
        self
    }
}

/// 文件在 diff 中的变更类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiffChangeKind {
    /// 新增文件。
    Added,
    /// 删除文件。
    Deleted,
    /// 内容修改。
    Modified,
    /// 重命名。
    Renamed,
    /// 复制。
    Copied,
    /// 类型变更（普通文件 ↔ 符号链接 / 子模块）。
    TypeChanged,
    /// 无法判定。
    Unknown,
}

impl DiffChangeKind {
    /// 从 porcelain v2 的 `XY` 状态对推断文件级变更类别。
    ///
    /// 规则与 git 的展示一致：删除优先于修改（`D` 出现在任一侧即视为删除），
    /// 其次是新增，然后是重命名/复制、类型变更。
    /// 冲突（`U`）不在这里判定——它属于仓库状态，不属于 diff。
    pub fn from_status_pair(index: u8, worktree: u8) -> Self {
        let has = |byte: u8, expected: u8| byte == expected;
        if has(index, b'D') || has(worktree, b'D') {
            return Self::Deleted;
        }
        if has(index, b'A') || has(worktree, b'A') {
            return Self::Added;
        }
        if has(index, b'R') || has(worktree, b'R') {
            return Self::Renamed;
        }
        if has(index, b'C') || has(worktree, b'C') {
            return Self::Copied;
        }
        if has(index, b'T') || has(worktree, b'T') {
            return Self::TypeChanged;
        }
        if has(index, b'M') || has(worktree, b'M') {
            return Self::Modified;
        }
        Self::Unknown
    }
}

/// 单个文件的 diff。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiff {
    /// 路径（重命名/复制时是目标路径）。
    pub path: RepoPath,
    /// 重命名/复制的来源路径。
    pub original_path: Option<RepoPath>,
    /// 文件级变更类别。
    pub change: DiffChangeKind,
    /// 是否为二进制文件（无行级内容）。
    pub binary: bool,
    /// 新增行数。
    pub additions: u64,
    /// 删除行数。
    pub deletions: u64,
    /// 行级内容。T1.5 之前为空（见模块头）。
    pub hunks: Vec<DiffHunk>,
}

impl FileDiff {
    /// 是否包含行级内容。
    pub fn has_hunks(&self) -> bool {
        !self.hunks.is_empty()
    }
}

/// 一个 hunk（`@@ -a,b +c,d @@ header`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffHunk {
    /// 旧文件起始行号。
    pub old_start: u32,
    /// 旧文件行数。
    pub old_lines: u32,
    /// 新文件起始行号。
    pub new_start: u32,
    /// 新文件行数。
    pub new_lines: u32,
    /// `@@` 之后的上下文（通常是所在函数的签名），可能为空。
    pub header: String,
    /// hunk 内的行。
    pub lines: Vec<DiffLine>,
}

/// hunk 内一行的类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiffLineKind {
    /// 上下文行（未变化）。
    Context,
    /// 新增行。
    Added,
    /// 删除行。
    Removed,
    /// `\ No newline at end of file` 标记。
    ///
    /// 单独一类而不是塞进上下文：它不是文件内容，把它当内容会让"复制这段代码"
    /// 这类操作把标记一起复制走。
    NoNewlineMarker,
}

/// hunk 内的一行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    /// 行类别。
    pub kind: DiffLineKind,
    /// 行内容（不含前导的 `+`/`-`/空格，也不含行尾换行）。
    pub content: String,
    /// 旧文件行号（新增行为 `None`）。
    pub old_lineno: Option<u32>,
    /// 新文件行号（删除行为 `None`）。
    pub new_lineno: Option<u32>,
}

/// 一次 diff 查询的结果。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DiffReport {
    /// 变更的文件。
    pub files: Vec<FileDiff>,
    /// 因为超过大小上限而**没有展开行级内容**的文件数。
    ///
    /// 必须显式告知界面：静默截断会让用户以为"这个文件只改了这几行"，
    /// 而他可能正据此决定要不要回滚。
    pub truncated_files: usize,
}

impl DiffReport {
    /// 变更文件总数。
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// 全部文件的新增/删除行数合计。
    pub fn totals(&self) -> (u64, u64) {
        self.files.iter().fold((0, 0), |(added, removed), file| {
            (
                added.saturating_add(file.additions),
                removed.saturating_add(file.deletions),
            )
        })
    }

    /// 是否有内容被截断。
    pub fn is_truncated(&self) -> bool {
        self.truncated_files > 0
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{DiffChangeKind, DiffReport, DiffSpec, DiffTarget, FileDiff, FileStat};
    use crate::git::path::RepoPath;

    fn stat(path: &str, added: Option<u64>, removed: Option<u64>) -> FileStat {
        FileStat {
            path: RepoPath::from(path),
            original_path: None,
            additions: added,
            deletions: removed,
            binary: added.is_none() && removed.is_none(),
        }
    }

    #[test]
    fn only_targets_that_read_the_worktree_are_marked_as_such() {
        assert!(DiffTarget::Unstaged.touches_worktree());
        assert!(DiffTarget::Since("HEAD~1".to_owned()).touches_worktree());
        assert!(!DiffTarget::Staged.touches_worktree());
        assert!(!DiffTarget::between("a", "b").touches_worktree());
        assert!(!DiffTarget::Commit("abc".to_owned()).touches_worktree());
    }

    #[test]
    fn diff_spec_defaults_keep_renames_detected_and_three_context_lines() {
        let spec = DiffSpec::new(DiffTarget::Staged);

        assert_eq!(spec.context_lines, 3);
        assert!(spec.detect_renames);
        assert!(!spec.ignore_whitespace);
        assert!(spec.paths.is_empty());
    }

    #[test]
    fn deletion_wins_over_modification_when_both_sides_are_changed() {
        // `MD`：索引侧改过、工作区侧删除 → 用户看到的是"文件被删了"
        assert_eq!(
            DiffChangeKind::from_status_pair(b'M', b'D'),
            DiffChangeKind::Deleted
        );
        assert_eq!(
            DiffChangeKind::from_status_pair(b'.', b'M'),
            DiffChangeKind::Modified
        );
        assert_eq!(
            DiffChangeKind::from_status_pair(b'A', b'.'),
            DiffChangeKind::Added
        );
        assert_eq!(
            DiffChangeKind::from_status_pair(b'R', b'.'),
            DiffChangeKind::Renamed
        );
        assert_eq!(
            DiffChangeKind::from_status_pair(b'C', b'.'),
            DiffChangeKind::Copied
        );
        assert_eq!(
            DiffChangeKind::from_status_pair(b'T', b'.'),
            DiffChangeKind::TypeChanged
        );
        assert_eq!(
            DiffChangeKind::from_status_pair(b'.', b'.'),
            DiffChangeKind::Unknown
        );
    }

    #[test]
    fn report_totals_ignore_binary_files_instead_of_faking_zero_lines() {
        let report = DiffReport {
            files: vec![
                FileDiff {
                    path: RepoPath::from("a.txt"),
                    original_path: None,
                    change: DiffChangeKind::Modified,
                    binary: false,
                    additions: 3,
                    deletions: 1,
                    hunks: Vec::new(),
                },
                FileDiff {
                    path: RepoPath::from("bin.dat"),
                    original_path: None,
                    change: DiffChangeKind::Modified,
                    binary: true,
                    additions: 0,
                    deletions: 0,
                    hunks: Vec::new(),
                },
            ],
            truncated_files: 0,
        };

        assert_eq!(report.file_count(), 2);
        assert_eq!(report.totals(), (3, 1));
        assert!(!report.is_truncated());
    }

    #[test]
    fn truncation_is_visible_to_callers() {
        let report = DiffReport {
            files: Vec::new(),
            truncated_files: 2,
        };

        assert!(report.is_truncated());
    }

    #[test]
    fn binary_file_stats_have_no_line_counts() {
        let binary = stat("bin.dat", None, None);
        let mode_change = stat("mode.sh", Some(0), Some(0));

        assert!(binary.binary);
        assert_eq!(binary.changed_lines(), None);
        assert!(!mode_change.binary, "0/0 是模式变更，不是二进制");
        assert_eq!(mode_change.changed_lines(), Some(0));
    }
}
