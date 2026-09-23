//! 变更统计模型（`git diff --numstat` 的语义）。

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
