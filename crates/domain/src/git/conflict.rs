//! 冲突状态机模型（T3.1）。
//!
//! # 冲突是结果不是错误（T2.8 立下的约定）
//!
//! merge / rebase / cherry-pick / revert 撞上冲突时，仓库进入一个**可描述的
//! 状态**：哪些文件在哪个 stage 上有版本、rebase 进行到第几步、能不能继续。
//! 本模块把这个状态建模为纯数据：引擎负责从 git 的 index stage
//! （`git ls-files -u` 与 `git show :N:<path>`）采集，服务层负责编排
//! continue / abort / skip，界面据此把用户送到冲突页。
//!
//! # 数据来源的红线（T3.1 任务书第 1 条）
//!
//! 冲突信息**必须**来自 index stage，不得依赖工作区文件里的 `<<<<<<<` 标记：
//! 用户可能已经手动编辑过文件（标记被删掉但 index 仍然冲突），也可能在
//! 没有冲突的文件里写下这些字符。工作区只回答 [`ConflictFile::worktree_exists`]
//! 这一个问题，其余一切以 index 为准。
//!
//! # 与 IPC 的关系
//!
//! 与 `MergeOutcome` / `StashOutcome` 同一约定：这里派生 `serde::Serialize`
//!（camelCase），由 commands 层直接作为 IPC 形状返回——冲突查询没有第二份
//! DTO，避免"domain 一份字段名、IPC 一份字段名"的漂移。

use super::merge_blocks::MergeBlock;
use super::path::RepoPath;

/// 结果文本写回工作区时使用的换行风格（T3.2：保持原文件风格）。
///
/// 派生 `Deserialize`：它随写回请求从 IPC 进来（前端回传探测结果）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LineEnding {
    /// LF：`\n`（Unix 与现代默认）。
    Lf,
    /// CRLF：`\r\n`（Windows）。
    Crlf,
    /// CR：`\r`（经典 Mac，罕见但存在）。
    Cr,
}

/// 整个文件采用一方的解决动作（二进制冲突与"文件级快捷操作"共用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TakeSide {
    /// 采用我方版本（`git checkout --ours` 后标记已解决）。
    Ours,
    /// 采用对方版本（`git checkout --theirs` 后标记已解决）。
    Theirs,
}

/// 单个冲突文件的完整详情：三方 blob、工作区文件形状、合并块（T3.2）。
///
/// 与 [`ConflictFile`] 的区别：它是**打开单文件编辑器时**的惰性查询——
/// 块计算要对三份文本跑 diff3，冲突清单页只给文件名与类别就够了。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictFileDetail {
    /// 文件路径。
    pub path: RepoPath,
    /// 冲突类别。
    pub kind: ConflictKind,
    /// 共同祖先（stage 1）。
    pub base: Option<FileBlob>,
    /// 我方版本（stage 2）。
    pub ours: Option<FileBlob>,
    /// 对方版本（stage 3）。
    pub theirs: Option<FileBlob>,
    /// 工作区里该文件是否还存在。
    pub worktree_exists: bool,
    /// 工作区文件的换行风格（写回时保持）。
    pub eol: LineEnding,
    /// 工作区文件是否带 UTF-8 BOM（写回时保持）。
    pub bom: bool,
    /// 工作区文件末尾是否有换行（写回时保持）。
    pub trailing_newline: bool,
    /// diff3 合并块（三方内容可用且都是文本时非空；二进制为空）。
    pub blocks: Vec<MergeBlock>,
}

/// 冲突来源的操作类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ConflictOpKind {
    /// 合并（`.git/MERGE_HEAD`）。
    Merge,
    /// 变基（`.git/rebase-merge` / `.git/rebase-apply` 目录）。
    Rebase,
    /// 拣选（`.git/CHERRY_PICK_HEAD`）。
    CherryPick,
    /// 反转（`.git/REVERT_HEAD`）。
    Revert,
}

impl ConflictOpKind {
    /// 稳定短名（日志与测试用；IPC 形状走 serde 的 camelCase）。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::Rebase => "rebase",
            Self::CherryPick => "cherry-pick",
            Self::Revert => "revert",
        }
    }
}

/// 单个 stage 的 blob 快照（base / ours / theirs 共用同一形状）。
///
/// [`Self::content`] 为 `None` 的两种情况：内容超过
/// [`MAX_CONFLICT_BLOB_BYTES`]（不把大文件整个读进内存，`size` 仍然有效），
/// 或 blob 是二进制（界面只能走"采用一方 / 手动替换"路径，T3.2）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileBlob {
    /// 字节大小（`git cat-file -s` 的结果；无论是否读内容都有效）。
    pub size: u64,
    /// 是否二进制（git 的启发式：字节里含 NUL）。
    pub is_binary: bool,
    /// 编码提示：内容是合法 UTF-8 时为 `Some("utf-8")`；其他编码**不猜测**
    /// （`None`）——猜错编码比不猜更糟，界面把 `None` 交给用户决定。
    pub encoding_hint: Option<String>,
    /// 文本内容；二进制或超大时为 `None`。
    pub content: Option<String>,
}

/// 冲突 blob 的内容读取上限（2 MiB，T3.1 任务书）。
///
/// 为什么按字节而不是字符：上限的意义是"不把内存吃穿"，字节才是内存的单位；
/// 截断到一半的 UTF-8 字符没有意义，所以超限直接整体置 `None`。
pub const MAX_CONFLICT_BLOB_BYTES: u64 = 2 * 1024 * 1024;

/// 冲突文件的类别——由三个 stage 的**存在性**推导，与标记符无关。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ConflictKind {
    /// 三方都在且都是文本。
    Text,
    /// 任一 stage 的 blob 是二进制。
    Binary,
    /// 我方删除、对方修改（index 只有 base 与 theirs）。
    DeletedByUs,
    /// 对方删除、我方修改（index 只有 base 与 ours）。
    DeletedByThem,
    /// 两边都新增了同名文件（没有 base）。
    AddedByBoth,
    /// 只有我方新增（没有 base 与 theirs）。
    AddedByUs,
    /// 只有对方新增（没有 base 与 ours）。
    AddedByThem,
}

impl ConflictKind {
    /// 从三个 stage 的 blob 推导类别（`None` = 该 stage 在 index 里不存在）。
    ///
    /// 这就是"不依赖标记符"的判定核心：类别只看 index 里有哪些 stage，
    /// 工作区文件长什么样不参与。
    pub fn classify(
        base: Option<&FileBlob>,
        ours: Option<&FileBlob>,
        theirs: Option<&FileBlob>,
    ) -> Self {
        match (base, ours, theirs) {
            (None, Some(_), Some(_)) => Self::AddedByBoth,
            (None, Some(_), None) => Self::AddedByUs,
            (None, None, Some(_)) => Self::AddedByThem,
            (Some(_), None, Some(_)) => Self::DeletedByUs,
            (Some(_), Some(_), None) => Self::DeletedByThem,
            // 两边都删（index 只剩 stage 1）git 会自动解决，正常到不了这里；
            // 真到了（畸形 index / 外部工具干预），按"无内容可看"的文本处理
            (Some(_), None, None) => Self::Text,
            // 三个 stage 全空同样是畸形 index（`ls-files -u` 不会为正常冲突给出
            // 这种组合）；按"无内容可看"的文本处理，不 panic
            (None, None, None) => Self::Text,
            (Some(base), Some(ours), Some(theirs)) => {
                if base.is_binary || ours.is_binary || theirs.is_binary {
                    Self::Binary
                } else {
                    Self::Text
                }
            }
        }
    }
}

/// 一个冲突文件的三方版本与类别。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictFile {
    /// 文件路径（原始字节，见 [`RepoPath`] 的说明）。
    pub path: RepoPath,
    /// 冲突类别（由 stage 存在性推导，见 [`ConflictKind::classify`]）。
    pub kind: ConflictKind,
    /// 共同祖先（stage 1）；两边新增的冲突没有 base。
    pub base: Option<FileBlob>,
    /// 我方版本（stage 2）。
    pub ours: Option<FileBlob>,
    /// 对方版本（stage 3）。
    pub theirs: Option<FileBlob>,
    /// 工作区里该文件是否还存在（删除类冲突为 `false`）。
    pub worktree_exists: bool,
}

/// 冲突状态机（T3.1 任务书的数据结构）。
///
/// `files` 是**仍未解决**的文件（index 里还有未合并 stage 的）；
/// 用户 `git add` 之后文件就从这里消失，全部消失时
/// [`Self::can_continue`] 才为真。
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictState {
    /// 冲突来源操作；`None` = 当前没有进行中的多步操作
    /// （探测空态：冲突页需要能区分"没有冲突"与"有冲突"）。
    pub op_kind: Option<ConflictOpKind>,
    /// 操作是否进行中（有标记文件 / 目录）。
    pub op_in_progress: bool,
    /// rebase 当前进度（从 `rebase-merge/msgnum` 读取；其余操作为 `None`）。
    pub current_step: Option<u32>,
    /// rebase 总步数（`rebase-merge/end`；其余操作为 `None`）。
    pub total_steps: Option<u32>,
    /// 被 rebase 的分支名（`rebase-merge/head-name`；其余操作为 `None`）。
    pub head_name: Option<String>,
    /// 变更并入的分支名（merge / cherry-pick / revert 时的当前分支；
    /// 游离 HEAD 或 rebase 中为 `None`）。
    pub into_branch: Option<String>,
    /// 仍未解决的冲突文件。
    pub files: Vec<ConflictFile>,
    /// 是否可以继续（操作进行中且全部文件已解决）。
    pub can_continue: bool,
    /// 是否可以中止（四种操作都支持 `--abort`）。
    pub can_abort: bool,
    /// 是否可以跳过当前提交（只有 rebase 支持 `--skip`）。
    pub can_skip: bool,
}

impl ConflictState {
    /// 引擎采集完原始字段后调用：按状态机规则推导 `op_in_progress` 与 `can_*`。
    ///
    /// 规则集中在这里而不是散在引擎里，因为它们是**产品语义**：
    /// - `can_abort`：有进行中的操作即可中止；
    /// - `can_continue`：操作进行中**且**没有未解决文件——有未解决文件时
    ///   git 自己也会拒绝 continue，提前关掉按钮比让用户撞一次报错好；
    /// - `can_skip`：只有 rebase 支持 `--skip`。
    pub fn derive_flags(&mut self) {
        self.op_in_progress = self.op_kind.is_some();
        self.can_abort = self.op_kind.is_some();
        self.can_continue = self.op_kind.is_some() && self.files.is_empty();
        self.can_skip = self.op_kind == Some(ConflictOpKind::Rebase);
    }

    /// 是否还有未解决的冲突文件。
    pub fn has_unresolved_files(&self) -> bool {
        !self.files.is_empty()
    }
}

/// `continue` / `skip` 的结果。
///
/// 两者都可能**再次停在冲突上**（rebase / cherry-pick 序列重放下一个提交时
/// 撞新的冲突）：`conflicts` 非空就是又停住了，此时 `oid` 为 `None`；
/// 全部完成时 `oid` 是完成后的 HEAD。
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictContinueOutcome {
    /// 完成后的 HEAD oid；再次停在冲突上时为 `None`。
    pub oid: Option<String>,
    /// 新产生的冲突文件（非空 = 操作又停在了冲突状态）。
    pub conflicts: Vec<RepoPath>,
}

impl ConflictContinueOutcome {
    /// 是否又停在了冲突上。
    pub fn has_conflicts(&self) -> bool {
        !self.conflicts.is_empty()
    }
}

/// `abort` 的结果。
///
/// `snapshot_id` 由服务层填（abort 前打的 `PreHeadMove` 快照），随 IPC 契约
/// 流向审计表（`record_with` 从 `snapshotId` 字段提取，见 T2.10 修复的断链）。
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictAbortOutcome {
    /// 中止后的 HEAD oid。
    pub head_oid: Option<String>,
    /// 中止后的分支名（游离 HEAD 为 `None`）。
    pub head_ref: Option<String>,
    /// 中止前打的快照 id（服务层填；引擎构造的临时值恒为 `None`）。
    pub snapshot_id: Option<i64>,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{ConflictKind, ConflictOpKind, ConflictState, FileBlob};

    fn text_blob(content: &str) -> FileBlob {
        FileBlob {
            size: content.len() as u64,
            is_binary: false,
            encoding_hint: Some("utf-8".to_owned()),
            content: Some(content.to_owned()),
        }
    }

    #[test]
    fn classify_covers_all_stage_combinations() {
        let base = text_blob("base");
        let ours = text_blob("ours");
        let theirs = text_blob("theirs");

        assert_eq!(
            ConflictKind::classify(None, Some(&ours), Some(&theirs)),
            ConflictKind::AddedByBoth
        );
        assert_eq!(
            ConflictKind::classify(None, Some(&ours), None),
            ConflictKind::AddedByUs
        );
        assert_eq!(
            ConflictKind::classify(None, None, Some(&theirs)),
            ConflictKind::AddedByThem
        );
        assert_eq!(
            ConflictKind::classify(Some(&base), None, Some(&theirs)),
            ConflictKind::DeletedByUs
        );
        assert_eq!(
            ConflictKind::classify(Some(&base), Some(&ours), None),
            ConflictKind::DeletedByThem
        );
        assert_eq!(
            ConflictKind::classify(Some(&base), Some(&ours), Some(&theirs)),
            ConflictKind::Text
        );
    }

    #[test]
    fn classify_reports_binary_when_any_stage_is_binary() {
        let base = text_blob("base");
        let ours = text_blob("ours");
        let mut theirs = text_blob("theirs");
        theirs.is_binary = true;

        assert_eq!(
            ConflictKind::classify(Some(&base), Some(&ours), Some(&theirs)),
            ConflictKind::Binary
        );
    }

    #[test]
    fn flags_follow_the_state_machine_rules() {
        // 没有操作：全部为否
        let mut state = ConflictState::default();
        state.derive_flags();
        assert!(!state.op_in_progress);
        assert!(!state.can_abort);
        assert!(!state.can_continue);
        assert!(!state.can_skip);

        // merge 有未解决文件：能中止、不能继续
        let mut state = ConflictState {
            op_kind: Some(ConflictOpKind::Merge),
            files: vec![],
            ..ConflictState::default()
        };
        state.files.push(unfinished_file());
        state.derive_flags();
        assert!(state.op_in_progress);
        assert!(state.can_abort);
        assert!(!state.can_continue);
        assert!(!state.can_skip);
        assert!(state.has_unresolved_files());

        // 全部解决：能继续
        state.files.clear();
        state.derive_flags();
        assert!(state.can_continue);
        assert!(!state.has_unresolved_files());

        // rebase：能跳过
        let mut state = ConflictState {
            op_kind: Some(ConflictOpKind::Rebase),
            ..ConflictState::default()
        };
        state.derive_flags();
        assert!(state.can_skip);
        assert!(state.can_continue);
    }

    fn unfinished_file() -> super::ConflictFile {
        super::ConflictFile {
            path: super::RepoPath::from("src/a.rs"),
            kind: ConflictKind::Text,
            base: Some(text_blob("base")),
            ours: Some(text_blob("ours")),
            theirs: Some(text_blob("theirs")),
            worktree_exists: true,
        }
    }
}
