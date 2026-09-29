//! diff3 式三方合并的**块计算**（T3.2 三栏编辑器的数据源）。
//!
//! # 职责边界
//!
//! 输入是 T3.1 采集的三个 stage blob 文本（base / ours / theirs），
//! 输出是块的序列：三方一致的上下文、只有一方改过的自动解决段、
//! 双方都改且不同的冲突块。**解决状态不属于这里**：块操作（采用本地 /
//! 远端 / 两者保留 / 手动编辑）是界面的交互状态，由前端维护；
//! 本模块只回答"冲突在哪、各方是什么"。
//!
//! # 为什么自实现而不是找三方合并库
//!
//! Rust 生态里没有维护良好的 diff3 合并实现（`similar` 只有双向 diff），
//! 而双向 diff 恰好是本算法的唯一重活——引入 `similar`（Apache-2.0 OR MIT，
//! Myers 的成熟实现）做行 diff，合并逻辑本身约两百行，纯函数、
//! 20 组人工构造用例覆盖（见 tests）。算法是经典 diff3（Khanna–Kunal–Pierce
//! 的"双方相对 base 的编辑区间在 base 坐标上求重叠"这一标准做法）：
//!
//! 1. 分别对 (base, ours) 与 (base, theirs) 做行 diff，得到两组
//!    "base 区间 → 该方区间"的替换 hunk；
//! 2. 在 base 坐标上扫描两组 hunk，**重叠的 hunk（含相邻、间隙为零）合成一组**；
//! 3. 组内只有一方 → 自动采用该方；两方替换内容相同 → 自动采用；
//!    否则 → 冲突块（base 取组的完整区间，两方各取自己区间的行）。
//!
//! # 行尾与编码
//!
//! 所有行都是**剥离行尾符**的文本（`str::lines()` 语义）：CRLF / LF / CR
//! 由调用方探测并统一重建（见 `git-engine` 的 file detail 与 apply resolution），
//! 这里不掺和行尾，否则"base 是 CRLF、ours 是 LF"这种仓库现实会把 diff
//! 全部污染成假冲突。

use similar::TextDiff;

use super::path::RepoPath;

/// 自动解决段的内容来源（展示文案用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeSource {
    /// 只有我方改了这一段。
    Ours,
    /// 只有对方改了这一段。
    Theirs,
    /// 双方改成了相同内容。
    Both,
}

/// 三方合并后的块序列。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum MergeBlock {
    /// 三方一致的上下文（展示用，参与结果文本的拼接）。
    Context {
        /// 该段的所有行。
        lines: Vec<String>,
    },
    /// 只有一方（或双方一致）改过的段——**自动解决**，无需用户操作。
    Resolved {
        /// 合并后的行。
        lines: Vec<String>,
        /// 内容来源。
        source: ChangeSource,
    },
    /// 双方都改了且不同——需要用户逐块决策。
    Conflict {
        /// base 行。
        base: Vec<String>,
        /// 我方行。
        ours: Vec<String>,
        /// 对方行。
        theirs: Vec<String>,
    },
}

impl MergeBlock {
    /// 该块是否需要用户决策。
    pub const fn is_conflict(&self) -> bool {
        matches!(self, Self::Conflict { .. })
    }
}

/// 一个文件的三方合并结果：块序列 + 原始路径。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeBlockReport {
    /// 文件路径（来自 stage 采集）。
    pub path: RepoPath,
    /// 块序列（顺序即文件顺序）。
    pub blocks: Vec<MergeBlock>,
}

impl MergeBlockReport {
    /// 冲突块总数。
    pub fn conflict_count(&self) -> usize {
        self.blocks
            .iter()
            .filter(|block| block.is_conflict())
            .count()
    }
}

/// 单侧相对 base 的替换 hunk（base 区间 → 该方区间；插入时区间为空）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Hunk {
    base_start: usize,
    base_end: usize,
    side_start: usize,
    side_end: usize,
}

/// 对 base 与某一侧做行 diff，产出替换 hunk 列表（按 base 顺序）。
fn diff_hunks(base: &[&str], side: &[&str]) -> Vec<Hunk> {
    let diff = TextDiff::configure()
        .algorithm(similar::Algorithm::Myers)
        .diff_slices(base, side);
    diff.ops()
        .iter()
        .filter(|op| !matches!(op, similar::DiffOp::Equal { .. }))
        .map(|op| Hunk {
            base_start: op.old_range().start,
            base_end: op.old_range().end,
            side_start: op.new_range().start,
            side_end: op.new_range().end,
        })
        .collect()
}

/// 计算三方合并块。
pub fn compute_merge_blocks(base: &str, ours: &str, theirs: &str) -> Vec<MergeBlock> {
    let base_lines: Vec<&str> = base.lines().collect();
    let ours_lines: Vec<&str> = ours.lines().collect();
    let theirs_lines: Vec<&str> = theirs.lines().collect();

    let mut ours_hunks = diff_hunks(&base_lines, &ours_lines);
    let mut theirs_hunks = diff_hunks(&base_lines, &theirs_lines);
    // 两组各自有序，合并扫描时按 base_start 归并取小
    ours_hunks.reverse();
    theirs_hunks.reverse();

    let mut blocks: Vec<MergeBlock> = Vec::new();
    let mut pos = 0_usize;
    loop {
        let next = [
            ours_hunks.last().map(|hunk| hunk.base_start),
            theirs_hunks.last().map(|hunk| hunk.base_start),
        ]
        .into_iter()
        .flatten()
        .min();
        let Some(group_start) = next else {
            break;
        };

        // 组前的 base 行三方一致 → Context
        if group_start > pos {
            blocks.push(MergeBlock::Context {
                lines: base_lines[pos..group_start]
                    .iter()
                    .map(|l| (*l).to_owned())
                    .collect(),
            });
        }

        // 收集与组重叠的全部 hunk：相邻（间隙为零）也并入——这是 git 的
        // diff3 行为，分开渲染两条紧挨着的冲突只会让用户困惑
        let mut group_end = group_start;
        let mut ours_group: Vec<Hunk> = Vec::new();
        let mut theirs_group: Vec<Hunk> = Vec::new();
        loop {
            let mut grew = false;
            while let Some(hunk) = ours_hunks.last() {
                if hunk.base_start <= group_end {
                    let hunk = ours_hunks.pop().unwrap();
                    group_end = group_end.max(hunk.base_end);
                    ours_group.push(hunk);
                    grew = true;
                } else {
                    break;
                }
            }
            while let Some(hunk) = theirs_hunks.last() {
                if hunk.base_start <= group_end {
                    let hunk = theirs_hunks.pop().unwrap();
                    group_end = group_end.max(hunk.base_end);
                    theirs_group.push(hunk);
                    grew = true;
                } else {
                    break;
                }
            }
            if !grew {
                break;
            }
        }
        ours_group.reverse();
        theirs_group.reverse();

        if let Some(block) = resolve_group(
            &base_lines,
            &ours_lines,
            &theirs_lines,
            &ours_group,
            &theirs_group,
        ) {
            blocks.push(block);
        }
        pos = group_end;
    }

    if pos < base_lines.len() {
        blocks.push(MergeBlock::Context {
            lines: base_lines[pos..].iter().map(|l| (*l).to_owned()).collect(),
        });
    }
    blocks
}

/// 把一组重叠的 hunk 折叠成一个块。
fn resolve_group(
    base_lines: &[&str],
    ours_lines: &[&str],
    theirs_lines: &[&str],
    ours_group: &[Hunk],
    theirs_group: &[Hunk],
) -> Option<MergeBlock> {
    let group_start = [
        ours_group.first().map(|hunk| hunk.base_start),
        theirs_group.first().map(|hunk| hunk.base_start),
    ]
    .into_iter()
    .flatten()
    .min()
    .unwrap_or(0);
    let group_end = [
        ours_group.last().map(|hunk| hunk.base_end),
        theirs_group.last().map(|hunk| hunk.base_end),
    ]
    .into_iter()
    .flatten()
    .max()
    .unwrap_or(0);

    // 同侧组内的间隙行是该侧**未修改**的行（否则它们自己就是 hunk），因此
    // side 坐标在整个组区间上连续映射 base：按"gap 原样重放 + hunk 取 side 区间"
    // 逐段拼接，才不会丢掉"对方改了、本方没改"的那几行（相邻 hunk 的冲突）
    let replay_side = |group: &[Hunk],
                       side_lines: &[&str],
                       group_start: usize,
                       group_end: usize|
     -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let Some(first) = group.first() else {
            return out;
        };
        let mut base_pos = group_start;
        let mut side_pos = first.side_start - (first.base_start - group_start);
        for hunk in group {
            while base_pos < hunk.base_start {
                out.push(side_lines[side_pos].to_owned());
                base_pos += 1;
                side_pos += 1;
            }
            for line in &side_lines[hunk.side_start..hunk.side_end] {
                out.push((*line).to_owned());
            }
            base_pos = hunk.base_end;
            side_pos = hunk.side_end;
        }
        while base_pos < group_end {
            out.push(side_lines[side_pos].to_owned());
            base_pos += 1;
            side_pos += 1;
        }
        out
    };

    match (ours_group.is_empty(), theirs_group.is_empty()) {
        // 单侧修改：自动采用；删空了的段不产出块——删除的语义已经
        // 体现在前后的上下文里，一个空块只会是界面上的噪音
        (false, true) => {
            let lines = replay_side(ours_group, ours_lines, group_start, group_end);
            lines.is_empty().then_some(()).map_or_else(
                || {
                    Some(MergeBlock::Resolved {
                        lines,
                        source: ChangeSource::Ours,
                    })
                },
                |_| None,
            )
        }
        (true, false) => {
            let lines = replay_side(theirs_group, theirs_lines, group_start, group_end);
            lines.is_empty().then_some(()).map_or_else(
                || {
                    Some(MergeBlock::Resolved {
                        lines,
                        source: ChangeSource::Theirs,
                    })
                },
                |_| None,
            )
        }
        // 双方都改：内容相同 → 自动采用；不同 → 冲突
        _ => {
            let ours_text = replay_side(ours_group, ours_lines, group_start, group_end);
            let theirs_text = replay_side(theirs_group, theirs_lines, group_start, group_end);
            if ours_text == theirs_text {
                return (!ours_text.is_empty()).then_some(MergeBlock::Resolved {
                    lines: ours_text,
                    source: ChangeSource::Both,
                });
            }
            Some(MergeBlock::Conflict {
                base: base_lines[group_start..group_end]
                    .iter()
                    .map(|l| (*l).to_owned())
                    .collect(),
                ours: ours_text,
                theirs: theirs_text,
            })
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{compute_merge_blocks, ChangeSource, MergeBlock};

    /// 20 组人工构造的三方输入 → 期望的块序列（任务书验收项）。
    /// 每组是一个 (base, ours, theirs, 期望摘要) 的表驱动用例；
    /// 摘要用简短的判别函数断言，避免逐字段比对的噪音。
    #[test]
    fn twenty_hand_built_three_way_inputs_produce_the_expected_blocks() {
        let cases: Vec<(&str, &str, &str, Vec<MergeBlock>)> = vec![
            // 1. 三方一致 → 全上下文
            (
                "a\nb\nc\n",
                "a\nb\nc\n",
                "a\nb\nc\n",
                vec![MergeBlock::Context {
                    lines: vec!["a".into(), "b".into(), "c".into()],
                }],
            ),
            // 2. 只有 ours 改一行
            (
                "a\nb\nc\n",
                "a\nB\nc\n",
                "a\nb\nc\n",
                vec![
                    MergeBlock::Context {
                        lines: vec!["a".into()],
                    },
                    MergeBlock::Resolved {
                        lines: vec!["B".into()],
                        source: ChangeSource::Ours,
                    },
                    MergeBlock::Context {
                        lines: vec!["c".into()],
                    },
                ],
            ),
            // 3. 只有 theirs 改一行
            (
                "a\nb\nc\n",
                "a\nb\nc\n",
                "a\nT\nc\n",
                vec![
                    MergeBlock::Context {
                        lines: vec!["a".into()],
                    },
                    MergeBlock::Resolved {
                        lines: vec!["T".into()],
                        source: ChangeSource::Theirs,
                    },
                    MergeBlock::Context {
                        lines: vec!["c".into()],
                    },
                ],
            ),
            // 4. 双方改不同区域
            (
                "a\nb\nc\nd\ne\n",
                "A\nb\nc\nd\ne\n",
                "a\nb\nc\nD\ne\n",
                vec![
                    MergeBlock::Resolved {
                        lines: vec!["A".into()],
                        source: ChangeSource::Ours,
                    },
                    MergeBlock::Context {
                        lines: vec!["b".into(), "c".into()],
                    },
                    MergeBlock::Resolved {
                        lines: vec!["D".into()],
                        source: ChangeSource::Theirs,
                    },
                    MergeBlock::Context {
                        lines: vec!["e".into()],
                    },
                ],
            ),
            // 5. 双方改同一行成不同内容 → 冲突
            (
                "a\nb\nc\n",
                "a\nO\nc\n",
                "a\nT\nc\n",
                vec![
                    MergeBlock::Context {
                        lines: vec!["a".into()],
                    },
                    MergeBlock::Conflict {
                        base: vec!["b".into()],
                        ours: vec!["O".into()],
                        theirs: vec!["T".into()],
                    },
                    MergeBlock::Context {
                        lines: vec!["c".into()],
                    },
                ],
            ),
            // 6. ours 删除、theirs 修改同一行 → 冲突
            (
                "a\nb\nc\n",
                "a\nc\n",
                "a\nB2\nc\n",
                vec![
                    MergeBlock::Context {
                        lines: vec!["a".into()],
                    },
                    MergeBlock::Conflict {
                        base: vec!["b".into()],
                        ours: vec![],
                        theirs: vec!["B2".into()],
                    },
                    MergeBlock::Context {
                        lines: vec!["c".into()],
                    },
                ],
            ),
            // 7. 双方在不同位置插入
            (
                "a\nc\n",
                "a\nO1\nO2\nc\n",
                "a\nc\nT1\n",
                vec![
                    MergeBlock::Context {
                        lines: vec!["a".into()],
                    },
                    MergeBlock::Resolved {
                        lines: vec!["O1".into(), "O2".into()],
                        source: ChangeSource::Ours,
                    },
                    MergeBlock::Context {
                        lines: vec!["c".into()],
                    },
                    MergeBlock::Resolved {
                        lines: vec!["T1".into()],
                        source: ChangeSource::Theirs,
                    },
                ],
            ),
            // 8. 双方在同一位置插入不同内容 → 冲突
            (
                "a\nc\n",
                "a\nO\nc\n",
                "a\nT\nc\n",
                vec![
                    MergeBlock::Context {
                        lines: vec!["a".into()],
                    },
                    MergeBlock::Conflict {
                        base: vec![],
                        ours: vec!["O".into()],
                        theirs: vec!["T".into()],
                    },
                    MergeBlock::Context {
                        lines: vec!["c".into()],
                    },
                ],
            ),
            // 9. 双方改成相同内容 → 自动采用
            (
                "a\nb\nc\n",
                "a\nsame\nc\n",
                "a\nsame\nc\n",
                vec![
                    MergeBlock::Context {
                        lines: vec!["a".into()],
                    },
                    MergeBlock::Resolved {
                        lines: vec!["same".into()],
                        source: ChangeSource::Both,
                    },
                    MergeBlock::Context {
                        lines: vec!["c".into()],
                    },
                ],
            ),
            // 10. base 为空、双方各加 → 冲突
            (
                "",
                "O\n",
                "T\n",
                vec![MergeBlock::Conflict {
                    base: vec![],
                    ours: vec!["O".into()],
                    theirs: vec!["T".into()],
                }],
            ),
            // 11. base 为空、只有 ours 加 → 自动采用
            (
                "",
                "O1\nO2\n",
                "",
                vec![MergeBlock::Resolved {
                    lines: vec!["O1".into(), "O2".into()],
                    source: ChangeSource::Ours,
                }],
            ),
            // 12. theirs 删末行、ours 动前面
            (
                "a\nb\nc\n",
                "A\nb\nc\n",
                "a\nb\n",
                vec![
                    MergeBlock::Resolved {
                        lines: vec!["A".into()],
                        source: ChangeSource::Ours,
                    },
                    MergeBlock::Context {
                        lines: vec!["b".into()],
                    },
                ],
            ),
            // 13. 双方删同一行 → 自动采用（删除也算"相同修改"）
            (
                "a\nb\nc\n",
                "a\nc\n",
                "a\nc\n",
                vec![
                    MergeBlock::Context {
                        lines: vec!["a".into()],
                    },
                    MergeBlock::Context {
                        lines: vec!["c".into()],
                    },
                ],
            ),
            // 14. 相邻 hunk（ours 改 b、theirs 改 c，中间无间隔行）→ 合并成一个冲突
            (
                "a\nb\nc\nd\n",
                "a\nB\nc\nd\n",
                "a\nb\nC\nd\n",
                vec![
                    MergeBlock::Context {
                        lines: vec!["a".into()],
                    },
                    MergeBlock::Conflict {
                        base: vec!["b".into(), "c".into()],
                        ours: vec!["B".into(), "c".into()],
                        theirs: vec!["b".into(), "C".into()],
                    },
                    MergeBlock::Context {
                        lines: vec!["d".into()],
                    },
                ],
            ),
            // 15. 多行替换 vs 多行替换（不同长度）重叠 → 冲突
            (
                "a\n1\n2\n3\nz\n",
                "a\nX1\nX2\nz\n",
                "a\nY1\nY2\nY3\nz\n",
                vec![
                    MergeBlock::Context {
                        lines: vec!["a".into()],
                    },
                    MergeBlock::Conflict {
                        base: vec!["1".into(), "2".into(), "3".into()],
                        ours: vec!["X1".into(), "X2".into()],
                        theirs: vec!["Y1".into(), "Y2".into(), "Y3".into()],
                    },
                    MergeBlock::Context {
                        lines: vec!["z".into()],
                    },
                ],
            ),
            // 16. ours 改首行、theirs 删同一行
            (
                "a\nb\n",
                "A\nb\n",
                "b\n",
                vec![
                    MergeBlock::Conflict {
                        base: vec!["a".into()],
                        ours: vec!["A".into()],
                        theirs: vec![],
                    },
                    MergeBlock::Context {
                        lines: vec!["b".into()],
                    },
                ],
            ),
            // 17. 中文内容不被破坏
            (
                "你好\n世界\n",
                "你好\n世界！\n",
                "你好\n世界\n",
                vec![
                    MergeBlock::Context {
                        lines: vec!["你好".into()],
                    },
                    MergeBlock::Resolved {
                        lines: vec!["世界！".into()],
                        source: ChangeSource::Ours,
                    },
                ],
            ),
            // 18. 空行是有效内容
            (
                "a\n\nb\n",
                "a\n\n\nb\n",
                "a\n\nb\n",
                vec![
                    MergeBlock::Context {
                        lines: vec!["a".into(), "".into()],
                    },
                    MergeBlock::Resolved {
                        lines: vec!["".into()],
                        source: ChangeSource::Ours,
                    },
                    MergeBlock::Context {
                        lines: vec!["b".into()],
                    },
                ],
            ),
            // 19. 复杂交错：冲突 + 两个自动解决 + 上下文
            (
                "h1\na\nb\nc\nd\nh2\n",
                "h1\nA1\nA2\nb\nc\nD\nh2\n",
                "h1\na\nb\nB1\nB2\nd\nh2\n",
                vec![
                    MergeBlock::Context {
                        lines: vec!["h1".into()],
                    },
                    // theirs 没动 a：ours 的替换是单侧自动解决
                    MergeBlock::Resolved {
                        lines: vec!["A1".into(), "A2".into()],
                        source: ChangeSource::Ours,
                    },
                    MergeBlock::Context {
                        lines: vec!["b".into()],
                    },
                    // theirs 改 c、ours 改 d：相邻（间隙为零）合并成一个冲突
                    MergeBlock::Conflict {
                        base: vec!["c".into(), "d".into()],
                        ours: vec!["c".into(), "D".into()],
                        theirs: vec!["B1".into(), "B2".into(), "d".into()],
                    },
                    MergeBlock::Context {
                        lines: vec!["h2".into()],
                    },
                ],
            ),
            // 20. ours 把一行改成与 theirs 的多行相同开头（重叠但内容不同）→ 冲突
            (
                "a\nb\nc\n",
                "a\nb1\nb2\nc\n",
                "a\nb9\nc\n",
                vec![
                    MergeBlock::Context {
                        lines: vec!["a".into()],
                    },
                    MergeBlock::Conflict {
                        base: vec!["b".into()],
                        ours: vec!["b1".into(), "b2".into()],
                        theirs: vec!["b9".into()],
                    },
                    MergeBlock::Context {
                        lines: vec!["c".into()],
                    },
                ],
            ),
        ];

        for (index, (base, ours, theirs, expected)) in cases.into_iter().enumerate() {
            let actual = compute_merge_blocks(base, ours, theirs);
            assert_eq!(actual, expected, "case #{} 失败", index + 1);
        }
    }

    #[test]
    fn conflict_count_is_exposed_for_the_file_list_badge() {
        let blocks = compute_merge_blocks("a\n", "A\n", "T\n");
        assert_eq!(blocks.iter().filter(|b| b.is_conflict()).count(), 1);
    }
}
