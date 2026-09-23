//! 工作区状态模型（`git status --porcelain=v2` 的语义）。
//!
//! 为什么以 porcelain v2 为原型：它把"索引侧状态"与"工作区侧状态"拆成两个字符
//! （`XY`），并显式给出每个条目的模式与 oid。这两点是 v1 与 `--short` 都做不到的，
//! 而 M1 的状态面板需要精确区分"已暂存的修改"与"尚未暂存的修改"。

use super::index::StageEntry;
use super::path::RepoPath;

/// 单侧的变更类型，对应 porcelain v2 里 `XY` 的一个字符。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChangeKind {
    /// `.`：该侧无变更。
    Unmodified,
    /// `M`：已修改。
    Modified,
    /// `A`：新增。
    Added,
    /// `D`：已删除。
    Deleted,
    /// `R`：重命名。
    Renamed,
    /// `C`：复制。
    Copied,
    /// `T`：类型变更（普通文件 ↔ 符号链接 / 子模块）。
    TypeChanged,
    /// `U`：已更新但未合并。
    Unmerged,
    /// 无法识别的字符。
    ///
    /// 为什么保留而不是解析失败：Git 未来版本可能新增状态字符，
    /// 让整个状态面板因为一个未知字符而空白，比"这个文件的状态显示为未知"糟糕得多。
    Unknown,
}

impl ChangeKind {
    /// 从状态字符解析。无法识别时返回 [`ChangeKind::Unknown`]。
    pub const fn from_byte(byte: u8) -> Self {
        match byte {
            b'.' => Self::Unmodified,
            b'M' => Self::Modified,
            b'A' => Self::Added,
            b'D' => Self::Deleted,
            b'R' => Self::Renamed,
            b'C' => Self::Copied,
            b'T' => Self::TypeChanged,
            b'U' => Self::Unmerged,
            _ => Self::Unknown,
        }
    }

    /// 状态字符本身。未知状态回退为 `?`。
    pub const fn as_char(self) -> char {
        match self {
            Self::Unmodified => '.',
            Self::Modified => 'M',
            Self::Added => 'A',
            Self::Deleted => 'D',
            Self::Renamed => 'R',
            Self::Copied => 'C',
            Self::TypeChanged => 'T',
            Self::Unmerged => 'U',
            Self::Unknown => '?',
        }
    }

    /// 该侧是否存在变更。
    ///
    /// [`ChangeKind::Unknown`] 归入"无变更"：界面无法为一个不认识的字符
    /// 决定显示什么操作，硬算成"有变更"会给出错误的按钮。
    pub const fn is_changed(self) -> bool {
        matches!(
            self,
            Self::Modified
                | Self::Added
                | Self::Deleted
                | Self::Renamed
                | Self::Copied
                | Self::TypeChanged
                | Self::Unmerged
        )
    }
}

/// porcelain v2 的记录类型（行首字符）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntryKind {
    /// `1`：已跟踪文件的普通变更。
    Ordinary,
    /// `2`：重命名或复制（记录里带两个路径）。
    RenamedOrCopied,
    /// `u`：未解决的冲突。
    Unmerged,
    /// `?`：未跟踪。
    Untracked,
    /// `!`：被忽略（仅在 `--ignored` 时出现）。
    Ignored,
}

impl EntryKind {
    /// 从记录前缀字符解析。无法识别时返回 `None`（调用方跳过该记录）。
    pub const fn from_prefix(byte: u8) -> Option<Self> {
        match byte {
            b'1' => Some(Self::Ordinary),
            b'2' => Some(Self::RenamedOrCopied),
            b'u' => Some(Self::Unmerged),
            b'?' => Some(Self::Untracked),
            b'!' => Some(Self::Ignored),
            _ => None,
        }
    }

    /// 该条目是否携带"模式 + oid"字段。
    ///
    /// `?` 与 `!` 条目只有路径，没有索引信息——把它们的缺失字段当成 0 会在界面上
    /// 显示成"文件被删除"。
    pub const fn has_index_details(self) -> bool {
        matches!(
            self,
            Self::Ordinary | Self::RenamedOrCopied | Self::Unmerged
        )
    }
}

/// 子模块状态（porcelain v2 的 `<sub>` 字段，固定 4 字符）。
///
/// 形如 `S.MU`：`S` 表示是子模块，随后三位分别是"提交与索引记录不同"
/// "工作区有已跟踪修改" "工作区有未跟踪文件"。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct SubmoduleState {
    /// 该条目是否为子模块（`S` / `N`）。
    pub is_submodule: bool,
    /// 子模块 HEAD 与索引里记录的提交不同。
    pub commit_changed: bool,
    /// 子模块工作区有已跟踪内容的修改。
    pub modified_content: bool,
    /// 子模块工作区有未跟踪文件。
    pub untracked_content: bool,
}

impl SubmoduleState {
    /// 非子模块（`N...`），也是 `?` / `!` 条目的取值。
    pub const NONE: Self = Self {
        is_submodule: false,
        commit_changed: false,
        modified_content: false,
        untracked_content: false,
    };

    /// 解析 4 字符的 `<sub>` 字段。长度不足或字符未知时按"该位不成立"处理。
    pub fn parse(field: &[u8]) -> Self {
        let flag = |index: usize, expected: u8| field.get(index) == Some(&expected);
        Self {
            is_submodule: flag(0, b'S'),
            commit_changed: flag(1, b'C'),
            modified_content: flag(2, b'M'),
            untracked_content: flag(3, b'U'),
        }
    }
}

/// 冲突文件在三个 stage 上的记录。
///
/// 缺失的 stage 为 `None`：删除/修改类冲突天然只有两个 stage
/// （例如"我方修改、对方删除"只有 base 与 ours），把它补成空对象会让
/// M3 的冲突向导显示一个不存在的版本。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictStages {
    /// stage 1：共同祖先。
    pub base: Option<StageEntry>,
    /// stage 2：我方（ours）。
    pub ours: Option<StageEntry>,
    /// stage 3：对方（theirs）。
    pub theirs: Option<StageEntry>,
}

/// 一个工作区变更条目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    /// 记录类型。
    pub kind: EntryKind,
    /// 路径（重命名/复制条目里是**目标**路径）。
    pub path: RepoPath,
    /// 重命名/复制条目的来源路径，其余条目为 `None`。
    pub original_path: Option<RepoPath>,
    /// 索引侧状态（`X`）。
    pub index_status: ChangeKind,
    /// 工作区侧状态（`Y`）。
    pub worktree_status: ChangeKind,
    /// 重命名/复制的相似度百分比（0–100），仅 `2` 条目有值。
    pub similarity: Option<u8>,
    /// HEAD 侧的文件模式（八进制）。`0`（不存在）解析为 `None`。
    pub mode_head: Option<u32>,
    /// 索引侧的文件模式。
    pub mode_index: Option<u32>,
    /// 工作区侧的文件模式。
    pub mode_worktree: Option<u32>,
    /// HEAD 侧的 blob oid。全零解析为 `None`。
    pub oid_head: Option<String>,
    /// 索引侧的 blob oid。
    pub oid_index: Option<String>,
    /// 冲突条目的三个 stage；仅 `u` 条目有值。
    pub stages: Option<ConflictStages>,
    /// 子模块状态。
    pub submodule: SubmoduleState,
}

impl FileChange {
    /// 是否存在未解决的冲突。
    pub fn is_conflicted(&self) -> bool {
        self.kind == EntryKind::Unmerged
    }

    /// 是否只存在于工作区（未跟踪 / 被忽略）。
    pub fn is_untracked_or_ignored(&self) -> bool {
        matches!(self.kind, EntryKind::Untracked | EntryKind::Ignored)
    }
}

/// 当前分支信息（porcelain v2 的 `# branch.*` 头）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BranchInfo {
    /// HEAD 指向的提交 oid。**初始仓库**（尚无任何提交）为 `None`。
    pub oid: Option<String>,
    /// 当前分支名。游离 HEAD 时为 `None`。
    pub head: Option<String>,
    /// 是否处于游离 HEAD（`# branch.head (detached)`）。
    pub detached: bool,
    /// 上游分支名（如 `origin/main`）。
    pub upstream: Option<String>,
    /// 相对上游领先的提交数。
    pub ahead: Option<i64>,
    /// 相对上游落后的提交数。
    pub behind: Option<i64>,
}

impl BranchInfo {
    /// 是否为"还没有提交"的初始仓库。
    ///
    /// 这个状态必须能与"游离 HEAD"区分：两者都没有分支名，
    /// 但初始仓库要引导用户做首次提交，游离 HEAD 要引导用户回到分支。
    pub fn is_initial(&self) -> bool {
        self.oid.is_none() && !self.detached
    }
}

/// `git status --porcelain=v2 -z --branch` 的解析结果。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StatusReport {
    /// 分支头信息。
    pub branch: BranchInfo,
    /// 全部变更条目，顺序与 git 输出一致（已跟踪 → 未跟踪 → 被忽略）。
    pub entries: Vec<FileChange>,
}

impl StatusReport {
    /// 工作区是否干净（不含未跟踪与被忽略文件）。
    pub fn is_clean(&self) -> bool {
        self.entries.is_empty()
    }

    /// 未解决的冲突条目。
    pub fn conflicts(&self) -> impl Iterator<Item = &FileChange> {
        self.entries.iter().filter(|entry| entry.is_conflicted())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{ChangeKind, EntryKind, SubmoduleState};

    #[test]
    fn change_kind_round_trips_through_its_character() {
        for kind in [
            ChangeKind::Unmodified,
            ChangeKind::Modified,
            ChangeKind::Added,
            ChangeKind::Deleted,
            ChangeKind::Renamed,
            ChangeKind::Copied,
            ChangeKind::TypeChanged,
            ChangeKind::Unmerged,
        ] {
            assert_eq!(ChangeKind::from_byte(kind.as_char() as u8), kind);
        }
    }

    #[test]
    fn unknown_change_character_does_not_claim_a_change() {
        let kind = ChangeKind::from_byte(b'Z');

        assert_eq!(kind, ChangeKind::Unknown);
        assert!(!kind.is_changed());
    }

    #[test]
    fn untracked_and_ignored_entries_have_no_index_details() {
        assert!(!EntryKind::Untracked.has_index_details());
        assert!(!EntryKind::Ignored.has_index_details());
        assert!(EntryKind::Ordinary.has_index_details());
    }

    #[test]
    fn submodule_flags_are_read_positionally() {
        let state = SubmoduleState::parse(b"S.MU");

        assert!(state.is_submodule);
        assert!(!state.commit_changed);
        assert!(state.modified_content);
        assert!(state.untracked_content);
    }

    #[test]
    fn short_or_unknown_submodule_field_is_treated_as_no_flags() {
        assert_eq!(SubmoduleState::parse(b""), SubmoduleState::NONE);
        assert_eq!(SubmoduleState::parse(b"N..."), SubmoduleState::NONE);
        assert_eq!(SubmoduleState::parse(b"XY"), SubmoduleState::NONE);
    }
}
