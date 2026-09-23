//! 索引里的冲突 stage 模型（`git ls-files -u` 的语义）。

use super::path::RepoPath;

/// 冲突条目的 stage 编号。
///
/// 三路合并的三个版本各有固定编号，这是 Git 的索引格式（不是我们的约定），
/// 因此枚举顺序与编号一一对应。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnmergedStage {
    /// stage 1：共同祖先（base）。
    Base,
    /// stage 2：我方（ours）。
    Ours,
    /// stage 3：对方（theirs）。
    Theirs,
}

impl UnmergedStage {
    /// 从索引里的 stage 数字解析。越界返回 `None`。
    pub const fn from_number(number: u32) -> Option<Self> {
        match number {
            1 => Some(Self::Base),
            2 => Some(Self::Ours),
            3 => Some(Self::Theirs),
            _ => None,
        }
    }

    /// 索引里的 stage 数字。
    pub const fn number(self) -> u32 {
        match self {
            Self::Base => 1,
            Self::Ours => 2,
            Self::Theirs => 3,
        }
    }
}

/// 一个 stage 上的文件记录（模式 + blob oid）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageEntry {
    /// 文件模式（八进制，如 `0o100644`）。
    pub mode: u32,
    /// blob oid（十六进制字符串）。
    pub oid: String,
}

/// `git ls-files -u -z` 的一条记录：一个文件在一个 stage 上的版本。
///
/// 注意这里是**扁平**的"每个 stage 一条"，与 Git 的输出一一对应；
/// 按文件聚合后的形态见 [`super::status::ConflictStages`]。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnmergedEntry {
    /// 文件路径。
    pub path: RepoPath,
    /// 该记录属于哪个 stage。
    pub stage: UnmergedStage,
    /// 文件模式（八进制）。
    pub mode: u32,
    /// blob oid（十六进制字符串）。
    pub oid: String,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::UnmergedStage;

    #[test]
    fn stage_numbers_round_trip() {
        for stage in [
            UnmergedStage::Base,
            UnmergedStage::Ours,
            UnmergedStage::Theirs,
        ] {
            assert_eq!(UnmergedStage::from_number(stage.number()), Some(stage));
        }
    }

    #[test]
    fn out_of_range_stage_number_is_rejected() {
        assert_eq!(UnmergedStage::from_number(0), None);
        assert_eq!(UnmergedStage::from_number(4), None);
    }
}
