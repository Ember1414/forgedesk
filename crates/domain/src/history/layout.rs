//! 提交图的泳道布局（**纯逻辑，无 IO**）。
//!
//! # 为什么布局在 Rust 侧而不是前端
//!
//! 1. 布局是纯计算，与画布无关：放这里可以用后台线程算、可以按
//!    `(repo_id, tip_oids, mode)` 缓存（M2 风险表里的缓解措施）；
//! 2. T2.1 的四条性质（边完整 / 确定性 / 无重叠 / 分页一致）需要用随机 DAG 反复断言，
//!    而这些 DAG 的构造与断言在 Rust 侧做一次就够，前端只管把结果画出来；
//! 3. 5 万节点在 JS 主线程上算会直接卡住交互。
//!
//! （PLAN §6.2.3 写的是"布局用 D3"，与 T2.1 的提示词冲突；按提示词执行，
//! 差异记在 `docs/ARCHITECTURE.md`。D3 只留给 M3 的 rebase 拖拽面板。）
//!
//! # 算法（一次前向扫描 + 一次定边）
//!
//! 输入是按**新 → 旧**排好序的提交（`Commit::parents` 的第一个元素是 first-parent）。
//!
//! **第一遍：分配 lane 与 row。**
//! 维护一张"泳道槽位表"，每个槽位要么空着、要么记着"它在等哪个提交 oid"
//! （即：某个已摆放的孩子正在等这个父提交出现）。
//!
//! 对第 `row` 个提交 `C`：
//! - 若有槽位在等 `C.oid` → 取**最左边**的那个槽位作为 `C` 的 lane（左侧优先，
//!   让主线尽量靠左）；其余在等同一 oid 的槽位立刻释放（它们的"分叉"由别的孩子承担，
//!   这正是 lane 回收的时机）；
//! - 否则 `C` 是一个 tip（没人等它：分支尖端、游离 HEAD、未加载完的窗口边界），
//!   取**最左边空闲**的槽位；没有空闲就追加一个。
//! - 然后为 `C` 的每个父提交预留槽位：first-parent 优先复用 `C` 自己的 lane，
//!   其余父提交取最左边空闲槽位。
//!
//! **第二遍：定边。** 第一遍只记下"孩子 → 预留的槽位"，因为那时还不知道父提交
//! 最终落在哪个 lane（可能有更左的槽位也在等同一个父）。第二遍用"已摆放表"
//! 把 `to_lane` 解析成父提交真实的 lane，并按下面的规则定边类型：
//!
//! | 情况 | 边类型 | 含义 |
//! | --- | --- | --- |
//! | `parent == parents[0]` 且预留槽位就是父的 lane | `Straight` | 主线继续 |
//! | `parent != parents[0]` | `Merge` | 合并进来的支线 |
//! | `parent == parents[0]` 但父落在别人的 lane | `Branch` | 支线汇入已占用的主线 |
//!
//! # 颜色
//!
//! `color_index = lane % PALETTE_SIZE`。lane 在它的生命周期内不变，因此：
//! - 同一条分支在刷新后同色（PLAN M2 验收项）；
//! - 分页加载更多不会改变已加载行的颜色 —— 前向扫描天然满足这条，
//!   这正是"分页一致性"断言要保护的性质。
//!
//! # 折叠已合并分支（可选的第三遍）
//!
//! `LayoutOptions::collapse_merged_branches` 打开且模式为 `AllBranches` 时，
//! 在两遍扫描之后追加一次**只标记、不重排**的折叠：对每个 merge，若其第二父的
//! 所有祖先都在第一父的祖先集中，则该 merge 可折叠为汇总节点——被折叠分支的
//! tip 行置 `hidden`，merge 行的 `collapsed` 记录 tip oid。行不删、lane 与边
//! 不动的取舍与判定细节见 [`LayoutOptions::collapse_merged_branches`] 的文档。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::git::Commit;

/// 调色板容量：颜色索引的取值范围。
pub const PALETTE_SIZE: u16 = 8;

/// 布局模式。
///
/// 注意这两种模式**只管边，不管行**：`FirstParentOnly` 只是不为非第一父提交画线，
/// 输入里若仍带着支线提交，它们照样各占一行（因为没人"等"它们，会当作新的 tip）。
/// "只看主线"的**筛选**是查询的事（`git log --first-parent`），布局器不替它做过滤——
/// 否则同一份输入在两种模式下会得到不同的行号，选中/跳转的下标就对不上了。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LayoutMode {
    /// 全部引用（`git log --all`）：把每个分支尖端都当作起点。
    #[default]
    AllBranches,
    /// 仅 first-parent 链（"只看主线"）：不画非第一父的那些边。
    FirstParentOnly,
}

/// 布局选项。
///
/// 把"模式"与"呈现开关"收进一个结构，避免每加一个开关就给 [`layout()`]
/// 加一个位置参数（旧调用点可用 [`LayoutMode::into`] 构造，见 `From` 实现）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LayoutOptions {
    /// 布局模式（默认 [`LayoutMode::AllBranches`]）。
    pub mode: LayoutMode,
    /// 折叠已合并分支（默认关闭；T2.1 第 4 条，设计先写在文档注释）。
    ///
    /// # 判定
    ///
    /// 对每个合并提交 M（去重后父数 ≥ 2），设第一父为 F、第二父为 S：若
    /// **S 的所有祖先都在 F 的祖先集中**（祖先闭包含自身，即
    /// `reach(S) ∖ {S} ⊆ reach(F)`），则 M 可折叠为汇总节点——S 这条分支
    /// 没有引入任何主线没有的历史（S 自己除外）。典型场景：分支从主线
    /// tip 分出后只提交了一次就并回；或同一分支被合并多次时的重复合并。
    ///
    /// # 标记形状（保守：行不删、lane 与边不动）
    ///
    /// 判定通过时：
    /// - M 行的 `collapsed` 记录被折叠分支的 tip oid（前端据此显示
    ///   "已折叠"徽标）；
    /// - S 行置 `hidden = true`——仅当 S 不经由 F 可达；重复合并里 S 已经
    ///   在 F 的可达集里，经由更早的合并仍然可见，隐藏它会凭空丢一行。
    ///
    /// **行不删**是刻意的：服务层把行号平移成全局行号、游标语义建立在
    /// "行不删"上（`services/history.rs`）；折叠只提供标记，是否隐藏、
    /// 如何呈现由前端（T2.2）决定。lane 与边同样不动：折叠是叠加信息，
    /// 关闭时两份布局逐行逐边一致（property 测试保证）。
    ///
    /// # 边界
    ///
    /// - 仅在 [`LayoutMode::AllBranches`] 下生效：`FirstParentOnly` 已裁掉
    ///   非第一父的边，折叠没有呈现对象。
    /// - 仅在**完整窗口**上生效：输入中任一提交引用的父 oid 不在输入集合
    ///   （分页边界）时，祖先闭包不完整，"⊆"会把窗外祖先误判为不存在，
    ///   因此整体放弃折叠。不做部分折叠：部分折叠会让同一 merge 在不同页
    ///   得到不同结论，前端无法一致呈现。
    /// - v1 只判定第二父；octopus 的第三父及以后保守地不折叠。
    ///
    /// # 复杂度
    ///
    /// 可达集用 `u64` 位集、利用"父下标 > 孩子下标"的输入前提自旧向新
    /// 动态规划：时间 O((V+E)·⌈V/64⌉)，内存 O(V·⌈V/64⌉)。5000 节点时
    /// 每提交约 79 个字、总计约百万次字运算，远低于任务书 200ms 的基准
    /// 门槛；服务层只在最后一页（≤ 500 行）启用，位集更小。
    pub collapse_merged_branches: bool,
}

impl From<LayoutMode> for LayoutOptions {
    /// 用布局模式构造选项（折叠关闭）——旧调用点的等价写法。
    fn from(mode: LayoutMode) -> Self {
        Self {
            mode,
            collapse_merged_branches: false,
        }
    }
}

/// 边的类型（渲染层用不同样式区分）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EdgeKind {
    /// 主线继续（同一 lane）。
    Straight,
    /// 合并：从合并点连到非第一父。
    Merge,
    /// 支线汇入：孩子在自己的 lane 上，父落在别人的 lane。
    Branch,
}

/// 一个提交在图上的位置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphRow {
    /// 提交 oid。
    pub oid: String,
    /// 泳道（0 基，左侧为 0）。
    pub lane: u16,
    /// 行号（与输入下标一致，0 基）。
    pub row: u32,
    /// 颜色索引（`lane % PALETTE_SIZE`）。
    pub color_index: u16,
    /// 是否是合并提交（父提交多于一个）。
    pub is_merge: bool,
    /// 该行是否被"折叠已合并分支"标记为隐藏（默认 `false`）。
    ///
    /// 行本身不删除：服务层的全局行号平移与游标语义建立在"行不删"上，
    /// 是否隐藏、如何呈现由前端决定（见
    /// [`LayoutOptions::collapse_merged_branches`]）。
    #[serde(default)]
    pub hidden: bool,
    /// 该行折叠掉的分支提交 oid（默认空；仅可折叠的 merge 行非空）。
    #[serde(default)]
    pub collapsed: Vec<String>,
}

/// 一条边（孩子 → 父）。
///
/// 方向约定：`from` 是**孩子**（更新的那个），`to` 是**父**（更旧的那个）。
/// 行号上 `from` 在上、`to` 在下，与历史的视觉顺序一致。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphEdge {
    /// 孩子 oid。
    pub from_oid: String,
    /// 父 oid。
    pub to_oid: String,
    /// 孩子的 lane。
    pub from_lane: u16,
    /// 父的 lane。
    pub to_lane: u16,
    /// 边类型。
    pub kind: EdgeKind,
}

/// 一次布局的结果。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GraphLayout {
    /// 每个提交的位置，顺序与输入一致。
    pub rows: Vec<GraphRow>,
    /// 全部边；顺序 = 提交顺序 × 父提交顺序（稳定，便于快照测试）。
    pub edges: Vec<GraphEdge>,
    /// 用到的泳道数（渲染宽度）。
    pub lane_count: u16,
}

impl GraphLayout {
    /// 按 oid 查位置。
    pub fn row_of(&self, oid: &str) -> Option<&GraphRow> {
        self.rows.iter().find(|row| row.oid == oid)
    }
}

/// 计算布局。
///
/// `commits` 必须按新 → 旧排序，且父提交要么在数组里更靠后，要么不在数组里
/// （分页边界）。**不满足这个前提时结果未定义**——排序是调用方的责任
/// （`git log` 保证它），布局器不重复检查：那会把 O(n) 的纯计算变成两遍扫描。
///
/// `options.collapse_merged_branches` 打开时追加折叠标记（只在 `AllBranches`
/// 模式与完整窗口上生效，见 [`LayoutOptions::collapse_merged_branches`]）：
/// 只做标记——行数、lane 与边与关闭时完全一致。
pub fn layout(commits: &[Commit], options: LayoutOptions) -> GraphLayout {
    let mode = options.mode;
    let mut slots: Vec<Option<String>> = Vec::new();
    let mut placements: Vec<GraphRow> = Vec::with_capacity(commits.len());
    // 第一遍的结果：孩子 → (父, 预留槽位, 是否 first-parent)
    let mut pending: Vec<(u16, String, bool)> = Vec::new();
    let mut edges_per_row: Vec<Vec<(u16, String, bool)>> = Vec::with_capacity(commits.len());

    for (index, commit) in commits.iter().enumerate() {
        let row = u32::try_from(index).unwrap_or(u32::MAX);

        // ① 谁在等这个提交？取最左边的那个槽位作为它的 lane。
        //
        // 先把下标收集出来再改 `slots`：直接在迭代里赋值会同时借用可变与不可变，
        // 而"边遍历边释放"的写法在语义上也不清楚（释放的顺序影响不了结果，
        // 但读代码的人得自己想一遍才知道）。
        let waiting: Vec<usize> = slots
            .iter()
            .enumerate()
            .filter(|(_, slot)| slot.as_deref() == Some(commit.oid.as_str()))
            .map(|(index, _)| index)
            .collect();
        let mut lane = None;
        for index in waiting {
            if lane.is_none() {
                lane = Some(u16::try_from(index).unwrap_or(u16::MAX));
            }
            // 其余的槽位也在等同一个父提交：释放。它的分叉由别的孩子承担，
            // 这正是"已合并分支的 lane 可以回收"的时机。
            slots[index] = None;
        }
        // ② 没人等它 → tip：取最左边空闲槽位，必要时追加。
        let lane = lane.unwrap_or_else(|| allocate(&mut slots));

        // ③ 去重父提交。
        //
        // git 允许同一个父写两次（`git commit-tree -p X -p X`），但图上它只有**一条线**：
        // 不去重会画出两条完全重叠的边（"同一 lane 同一行区间不得被两条边占用"这条性质
        // 会直接失败——这是性质测试抓到的第一个真实缺陷）。
        let unique_parents: Vec<&String> = {
            let mut seen = Vec::with_capacity(commit.parents.len());
            for parent in &commit.parents {
                if !seen.contains(&parent) {
                    seen.push(parent);
                }
            }
            seen
        };

        placements.push(GraphRow {
            oid: commit.oid.clone(),
            lane,
            row,
            color_index: lane % PALETTE_SIZE,
            is_merge: unique_parents.len() > 1,
            hidden: false,
            collapsed: Vec::new(),
        });

        // ④ 为父提交预留槽位（并记下待定边）。
        let parents: &[&String] = match mode {
            LayoutMode::AllBranches => &unique_parents,
            LayoutMode::FirstParentOnly => match unique_parents.first() {
                Some(first) => std::slice::from_ref(first),
                None => &[],
            },
        };
        pending.clear();
        for (parent_index, parent) in parents.iter().enumerate() {
            let parent: &String = parent;
            let is_first_parent = parent_index == 0;
            // 顺序即优先级：
            //  ① 已经有槽位在等这个父提交 → 复用（这是"lane 复用"的核心，
            //     否则两个分支共用同一个祖先把无谓地多占一条泳道）；
            //  ② first-parent 且自己的 lane 空着 → 主线接着走这条 lane（直线）；
            //  ③ 其余情况 → 取最左边的空闲槽位。
            let reserved = match slots
                .iter()
                .position(|slot| slot.as_deref() == Some(parent.as_str()))
            {
                Some(existing) => u16::try_from(existing).unwrap_or(u16::MAX),
                None => {
                    let slot = if is_first_parent && slots[lane as usize].is_none() {
                        lane
                    } else {
                        allocate(&mut slots)
                    };
                    slots[slot as usize] = Some(parent.clone());
                    slot
                }
            };
            pending.push((reserved, parent.clone(), is_first_parent));
        }
        edges_per_row.push(pending.clone());
    }

    // 第二遍：把预留槽位解析成父提交真实的 lane，并定边类型。
    let lane_of: HashMap<&str, u16> = placements
        .iter()
        .map(|row| (row.oid.as_str(), row.lane))
        .collect();

    let mut edges = Vec::new();
    for (row, pending) in placements.iter().zip(edges_per_row.iter()) {
        for (reserved, parent_oid, is_first_parent) in pending {
            // 父提交不在窗口内（分页边界）：边仍然发出去，`to_lane` 用预留槽位，
            // 渲染层据此画一条"继续往下"的线；等下一页到了会重算整段布局。
            let to_lane = lane_of
                .get(parent_oid.as_str())
                .copied()
                .unwrap_or(*reserved);
            let kind = if !is_first_parent {
                EdgeKind::Merge
            } else if to_lane == row.lane {
                EdgeKind::Straight
            } else {
                EdgeKind::Branch
            };
            edges.push(GraphEdge {
                from_oid: row.oid.clone(),
                to_oid: parent_oid.clone(),
                from_lane: row.lane,
                to_lane,
                kind,
            });
        }
    }

    // 第三遍（可选）：折叠已合并分支。只做标记，不改行、不改 lane、不改边：
    // 服务层的全局行号平移与游标语义建立在"行不删"上，隐藏呈现由前端（T2.2）
    // 决定（取舍详见 `LayoutOptions::collapse_merged_branches` 的文档）。
    if options.collapse_merged_branches && mode == LayoutMode::AllBranches {
        if let Some((hidden, collapsed)) = collapse_marks(commits) {
            for (placement, is_hidden) in placements.iter_mut().zip(hidden) {
                placement.hidden = is_hidden;
            }
            for (placement, collapsed_tips) in placements.iter_mut().zip(collapsed) {
                placement.collapsed = collapsed_tips;
            }
        }
    }

    let lane_count = placements
        .iter()
        .map(|row| row.lane)
        .max()
        .map_or(0, |max| max.saturating_add(1));

    GraphLayout {
        rows: placements,
        edges,
        lane_count,
    }
}

/// 取最左边的空闲槽位；没有空闲就追加一个。
fn allocate(slots: &mut Vec<Option<String>>) -> u16 {
    if let Some(index) = slots.iter().position(Option::is_none) {
        return u16::try_from(index).unwrap_or(u16::MAX);
    }
    slots.push(None);
    u16::try_from(slots.len() - 1).unwrap_or(u16::MAX)
}

/// 位集的字宽（`u64`）。
const WORD_BITS: usize = 64;

/// 把 `index` 位置 1。
fn set_bit(bits: &mut [u64], index: usize) {
    bits[index / WORD_BITS] |= 1 << (index % WORD_BITS);
}

/// `bits` 的 `index` 位是否为 1。
fn has_bit(bits: &[u64], index: usize) -> bool {
    bits[index / WORD_BITS] & (1 << (index % WORD_BITS)) != 0
}

/// `subset` 去掉 `except` 这一位之后是否 ⊆ `superset`。
fn subset_except(subset: &[u64], superset: &[u64], except: usize) -> bool {
    subset
        .iter()
        .zip(superset)
        .enumerate()
        .all(|(word, (sub, sup))| {
            let mut diff = sub & !sup;
            if word == except / WORD_BITS {
                diff &= !(1 << (except % WORD_BITS));
            }
            diff == 0
        })
}

/// 计算折叠标记（纯函数，T2.1 第 4 条：设计先写在文档注释）。
///
/// 返回 `(hidden, collapsed)`：前者是每行的 hidden 标记，后者是每行的被折叠
/// 分支 tip 列表（只有判定通过的 merge 行非空）。窗口不完整（任一父 oid 不在
/// 输入里）时返回 `None`，调用方整体回退为不折叠。
///
/// # 判定
///
/// 对每个合并提交（去重后父数 ≥ 2）：设第一父 F、第二父 S，若
/// `reach(S) ∖ {S} ⊆ reach(F)`（S 的所有祖先都能从 F 到达）则可折叠。
///
/// # 算法
///
/// 可达集用 `u64` 位集表示，利用"父下标 > 孩子下标"的输入前提自旧向新
/// 动态规划：`reach[i] = {i} ∪ ⋃ reach[parent]`。时间 O((V+E)·⌈V/64⌉)、
/// 内存 O(V·⌈V/64⌉)——5000 节点时每提交约 79 个字、总计约百万次字运算，
/// 对 200ms 的基准门槛无感；服务层只在最后一页（≤ 500 行）启用，位集更小。
fn collapse_marks(commits: &[Commit]) -> Option<(Vec<bool>, Vec<Vec<String>>)> {
    let index_of: HashMap<&str, usize> = commits
        .iter()
        .enumerate()
        .map(|(index, commit)| (commit.oid.as_str(), index))
        .collect();

    // 窗口完整性在下面的 DP 里顺带完成：DP 遍历每个提交的每条父边，遇到
    // 窗口外的父 oid 就返回 `None` 整体回退（祖先闭包不完整会让"⊆"把窗外
    // 祖先误判为不存在，因此不做部分折叠）。
    let word_count = commits.len().div_ceil(WORD_BITS);
    let mut reach: Vec<Vec<u64>> = vec![Vec::new(); commits.len()];
    for index in (0..commits.len()).rev() {
        let mut bits = vec![0_u64; word_count];
        set_bit(&mut bits, index);
        for parent in &commits[index].parents {
            let parent_index = *index_of.get(parent.as_str())?;
            for (word, parent_word) in bits.iter_mut().zip(&reach[parent_index]) {
                *word |= *parent_word;
            }
        }
        reach[index] = bits;
    }

    let mut hidden = vec![false; commits.len()];
    let mut collapsed: Vec<Vec<String>> = vec![Vec::new(); commits.len()];
    for (row, commit) in commits.iter().enumerate() {
        // 与布局同一套父去重规则：重复的父在图上只有一条线，也不参与判定
        let mut unique: Vec<&String> = Vec::with_capacity(commit.parents.len());
        for parent in &commit.parents {
            if !unique.contains(&parent) {
                unique.push(parent);
            }
        }
        // v1 只判定第二父：octopus 的第三父及以后保守地不折叠
        if unique.len() < 2 {
            continue;
        }
        let first = *index_of.get(unique[0].as_str())?;
        let second = *index_of.get(unique[1].as_str())?;

        // S 的所有祖先 ⊆ F 的祖先集 ⇔ reach(S) 去掉 S 自己 ⊆ reach(F)
        if subset_except(&reach[second], &reach[first], second) {
            collapsed[row].push(commits[second].oid.clone());
            // 仅当 S 不经由 F 可达时才隐藏：重复合并里 S 已在主线可达集里，
            // 经由更早的合并仍然可见，隐藏它会凭空丢一行
            if !has_bit(&reach[first], second) {
                hidden[second] = true;
            }
        }
    }

    Some((hidden, collapsed))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::git::{Signature, SignatureStatus};

    fn commit(oid: &str, parents: &[&str]) -> Commit {
        Commit {
            oid: oid.to_owned(),
            parents: parents.iter().map(|parent| (*parent).to_owned()).collect(),
            author: Signature::new("Perf Fixture", "perf@example.invalid"),
            committer: Signature::new("Perf Fixture", "perf@example.invalid"),
            refs: Vec::new(),
            signature: SignatureStatus::Unsigned,
            subject: format!("commit {oid}"),
            body: None,
        }
    }

    fn chain(spec: &[(&str, &[&str])]) -> Vec<Commit> {
        spec.iter()
            .map(|(oid, parents)| commit(oid, parents))
            .collect()
    }

    /// 线性历史的 lane 必须始终为 0，且每一对相邻提交之间有一条直线边。
    #[test]
    fn a_linear_history_stays_on_the_first_lane() {
        let commits = chain(&[("c", &["b"]), ("b", &["a"]), ("a", &[])]);
        let graph = layout(&commits, LayoutMode::AllBranches.into());

        assert_eq!(
            graph.rows.iter().map(|row| row.lane).collect::<Vec<_>>(),
            vec![0_u16, 0, 0]
        );
        assert_eq!(graph.lane_count, 1);
        assert_eq!(graph.edges.len(), 2, "c→b 与 b→a");
        assert!(graph
            .edges
            .iter()
            .all(|edge| edge.kind == EdgeKind::Straight
                && edge.from_lane == 0
                && edge.to_lane == 0));
    }

    /// 分叉后合并：第二父走新 lane，合并边类型是 Merge。
    #[test]
    fn a_merge_sends_the_second_parent_to_a_new_lane() {
        // m 合并了 a（主线）与 b（支线），两者都基于 base
        let commits = chain(&[
            ("m", &["a", "b"]),
            ("a", &["base"]),
            ("b", &["base"]),
            ("base", &[]),
        ]);
        let graph = layout(&commits, LayoutMode::AllBranches.into());

        let lane_of = |oid: &str| graph.row_of(oid).map(|row| row.lane);
        assert_eq!(lane_of("m"), Some(0));
        assert_eq!(lane_of("a"), Some(0), "first-parent 接着走主线");
        assert_eq!(lane_of("b"), Some(1), "第二父另开一条泳道");
        assert_eq!(
            lane_of("base"),
            Some(0),
            "两条泳道汇入同一个父，落在最左的预留槽位"
        );

        let merge_edge = graph
            .edges
            .iter()
            .find(|edge| edge.from_oid == "m" && edge.to_oid == "b")
            .expect("合并边存在");
        assert_eq!(merge_edge.kind, EdgeKind::Merge);
        assert_eq!((merge_edge.from_lane, merge_edge.to_lane), (0, 1));

        let branch_edge = graph
            .edges
            .iter()
            .find(|edge| edge.from_oid == "b" && edge.to_oid == "base")
            .expect("支线边存在");
        assert_eq!(branch_edge.kind, EdgeKind::Branch, "支线汇入别人占着的主线");
        assert_eq!((branch_edge.from_lane, branch_edge.to_lane), (1, 0));

        assert_eq!(graph.lane_count, 2);
    }

    /// 两个 tip 共用同一个父：各自占一条泳道（它们是不同的分支尖端），
    /// 但**父提交只被预留一次**——这就是"lane 复用"要保证的。
    #[test]
    fn two_tips_sharing_a_parent_converge_on_one_lane() {
        let commits = chain(&[("tip-a", &["base"]), ("tip-b", &["base"]), ("base", &[])]);
        let graph = layout(&commits, LayoutMode::AllBranches.into());

        // tip 是"没人等它"的提交，因此必然各占一条 lane（画成并排的两个尖端）；
        // base 只能落在其中一个 lane 上（最左的预留），另一个 tip 的边汇入它。
        assert_eq!(graph.lane_count, 2);
        assert_eq!(graph.row_of("base").map(|row| row.lane), Some(0));
        assert_eq!(
            graph
                .edges
                .iter()
                .filter(|edge| edge.to_oid == "base")
                .count(),
            2
        );
        // base 的 lane 上，两条边一条是主线延续、一条是汇入——不会都是 Straight
        let kinds: Vec<EdgeKind> = graph
            .edges
            .iter()
            .filter(|edge| edge.to_oid == "base")
            .map(|edge| edge.kind)
            .collect();
        assert!(
            kinds.contains(&EdgeKind::Branch),
            "汇入边必须是 Branch：{kinds:?}"
        );
    }

    /// octopus merge（多父）每多一个父就多一条 Merge 边。
    #[test]
    fn an_octopus_merge_emits_one_merge_edge_per_extra_parent() {
        let commits = chain(&[
            ("m", &["a", "b", "c", "d"]),
            ("a", &[]),
            ("b", &[]),
            ("c", &[]),
            ("d", &[]),
        ]);
        let graph = layout(&commits, LayoutMode::AllBranches.into());

        let merges = graph
            .edges
            .iter()
            .filter(|edge| edge.from_oid == "m" && edge.kind == EdgeKind::Merge)
            .count();
        assert_eq!(merges, 3, "四个父提交里有三个是合并进来的");
        assert_eq!(graph.lane_count, 4);
    }

    /// 同一个父被写两次（git 允许 `commit-tree -p X -p X`）：图上只画一条边。
    #[test]
    fn a_duplicated_parent_produces_a_single_edge() {
        let commits = chain(&[("m", &["a", "a"]), ("a", &[])]);
        let graph = layout(&commits, LayoutMode::AllBranches.into());

        assert_eq!(graph.lane_count, 1);
        let duplicates = graph
            .edges
            .iter()
            .filter(|edge| edge.from_oid == "m" && edge.to_oid == "a")
            .count();
        assert_eq!(
            duplicates, 1,
            "重复的父提交在图上是一条线，不是两条重叠的线"
        );
        // 去重后只剩一个父，因此不再算合并提交
        assert_eq!(graph.row_of("m").map(|row| row.is_merge), Some(false));
    }

    /// 互不相关的根提交复用同一条泳道：它们没有父子关系，lane 空闲即可回收。
    #[test]
    fn unrelated_roots_reuse_the_same_lane() {
        let commits = chain(&[("a", &[]), ("b", &[]), ("c", &[])]);
        let graph = layout(&commits, LayoutMode::AllBranches.into());

        // 根提交没有父提交可预留，lane 立刻回到空闲池——三个根都落在 lane 0，
        // 行区间互不重叠，这正是"泳道数应尽量少"的体现
        assert_eq!(
            graph.rows.iter().map(|row| row.lane).collect::<Vec<_>>(),
            vec![0_u16, 0, 0]
        );
        assert_eq!(graph.lane_count, 1);
        assert!(graph.edges.is_empty());
    }

    /// 只看主线：非第一父的**边**被忽略（行仍在——筛选是查询的事，见 `LayoutMode` 的文档）。
    #[test]
    fn first_parent_only_ignores_side_branch_edges() {
        let commits = chain(&[
            ("m", &["a", "b"]),
            ("a", &["base"]),
            ("b", &["base"]),
            ("base", &[]),
        ]);
        let graph = layout(&commits, LayoutMode::FirstParentOnly.into());

        // 行数不变：同一份输入在两种模式下必须给出同样的行号（选中/跳转的下标一致）
        assert_eq!(graph.rows.len(), 4);
        // 被忽略的是"合并进来的那条边"（m→b）；b 自己到它第一父的边仍然保留
        assert!(!graph
            .edges
            .iter()
            .any(|edge| edge.from_oid == "m" && edge.to_oid == "b"));
        // b 是 tip，落在自己的 lane 上，它到 base 的边是跨泳道的汇入（Branch）
        assert!(graph
            .edges
            .iter()
            .any(|edge| edge.from_oid == "b" && edge.kind == EdgeKind::Branch));
    }

    /// 分页边界：父提交不在窗口内时，边仍然存在（渲染层据此画"继续向下"的线）。
    #[test]
    fn edges_leave_the_window_at_a_page_boundary() {
        let commits = chain(&[("b", &["a"]), ("a", &["unknown-parent"])]);
        let graph = layout(&commits, LayoutMode::AllBranches.into());

        let dangling = graph
            .edges
            .iter()
            .find(|edge| edge.to_oid == "unknown-parent")
            .expect("窗口外的父提交也要有一条边");
        assert_eq!(dangling.kind, EdgeKind::Straight);
        assert_eq!((dangling.from_lane, dangling.to_lane), (0, 0));
    }

    /// 颜色索引必须等于 lane % PALETTE_SIZE，且 lane 一旦分配就不变。
    #[test]
    fn colours_follow_the_lane_and_are_stable() {
        let commits = chain(&[
            ("m", &["a", "b"]),
            ("a", &["base"]),
            ("b", &["base"]),
            ("base", &[]),
        ]);
        let graph = layout(&commits, LayoutMode::AllBranches.into());

        for row in &graph.rows {
            assert_eq!(row.color_index, row.lane % PALETTE_SIZE);
        }
        // 同一个 lane 在整张图里只有一个颜色（不会出现"同泳道两种颜色"）
        let mut seen: HashMap<u16, u16> = HashMap::new();
        for row in &graph.rows {
            let colour = *seen.entry(row.lane).or_insert(row.color_index);
            assert_eq!(colour, row.color_index);
        }
    }

    // ---------------------------------------------------------------- 折叠已合并分支

    /// 可折叠 merge：分支从主线 tip 分出、单提交后并回 → 分支行被标记
    /// `hidden`，merge 行的 `collapsed` 记录分支 tip；行数与 lane 不变
    /// （折叠只做标记，不重排）。
    #[test]
    fn a_collapsible_merge_hides_the_merged_branch_tip() {
        // f 从 a（当时的主线 tip）分出、提交一次后由 m 并回：
        // f 的祖先 = {a, base} 全在第一父 a 的祖先集里 → 可折叠
        let commits = chain(&[
            ("m", &["a", "f"]),
            ("f", &["a"]),
            ("a", &["base"]),
            ("base", &[]),
        ]);
        let graph = layout(
            &commits,
            LayoutOptions {
                collapse_merged_branches: true,
                ..LayoutOptions::default()
            },
        );

        assert_eq!(graph.rows.len(), 4, "行不删除");
        let row_of = |oid: &str| graph.row_of(oid).expect("行存在");
        assert!(row_of("f").hidden, "分支 tip 是被折叠的那一行");
        assert_eq!(
            row_of("m").collapsed,
            vec!["f".to_owned()],
            "merge 行记录被折叠的分支 tip"
        );
        assert!(!row_of("m").hidden, "merge 行自己是汇总节点，不隐藏");
        assert!(
            !row_of("a").hidden && !row_of("base").hidden,
            "主线照常可见"
        );
        // lane 分配与边不受折叠影响
        assert_eq!(graph.lane_count, 2);
    }

    /// 跨分支 merge（第二父基于更早的第三方提交）：第二父有祖先不在第一父的
    /// 祖先集里 → 不可折叠，所有行保持可见。
    #[test]
    fn a_cross_branch_merge_is_not_collapsed() {
        // b 基于主线之外的 third：third ∉ reach(a) → 不可折叠
        let commits = chain(&[
            ("m", &["a", "b"]),
            ("a", &["base"]),
            ("b", &["third"]),
            ("third", &["base"]),
            ("base", &[]),
        ]);
        let graph = layout(
            &commits,
            LayoutOptions {
                collapse_merged_branches: true,
                ..LayoutOptions::default()
            },
        );

        assert!(graph.rows.iter().all(|row| !row.hidden), "没有行可折叠");
        assert!(graph.rows.iter().all(|row| row.collapsed.is_empty()));
    }

    /// 嵌套 merge：每一层各自折叠自己的分支 tip（里层折叠后，外层第二父的
    /// 祖先已经全部可从里层 merge 到达）。
    #[test]
    fn nested_merges_collapse_each_branch_tip_at_its_own_merge() {
        // m1 合并 f1（f1 基于 a）；m2 再合并 f2（f2 基于 f1）
        let commits = chain(&[
            ("m2", &["m1", "f2"]),
            ("f2", &["f1"]),
            ("m1", &["a", "f1"]),
            ("f1", &["a"]),
            ("a", &["base"]),
            ("base", &[]),
        ]);
        let graph = layout(
            &commits,
            LayoutOptions {
                collapse_merged_branches: true,
                ..LayoutOptions::default()
            },
        );

        assert_eq!(graph.rows.len(), 6);
        let row_of = |oid: &str| graph.row_of(oid).expect("行存在");
        assert!(row_of("f1").hidden, "f1 的祖先 {{a, base}} ⊆ reach(a)");
        assert_eq!(row_of("m1").collapsed, vec!["f1".to_owned()]);
        assert!(
            row_of("f2").hidden,
            "f2 的祖先 {{f1, a, base}} ⊆ reach(m1)（m1 已并入 f1）"
        );
        assert_eq!(row_of("m2").collapsed, vec!["f2".to_owned()]);
        assert!(
            !row_of("m1").hidden && !row_of("m2").hidden,
            "两个 merge 行都是汇总节点"
        );
    }

    /// FirstParentOnly 下折叠没有呈现对象（非第一父的边已被裁掉）：
    /// 即使开关打开也不产生任何折叠标记。
    #[test]
    fn collapsing_is_skipped_in_first_parent_only_mode() {
        let commits = chain(&[
            ("m", &["a", "f"]),
            ("f", &["a"]),
            ("a", &["base"]),
            ("base", &[]),
        ]);
        let graph = layout(
            &commits,
            LayoutOptions {
                mode: LayoutMode::FirstParentOnly,
                collapse_merged_branches: true,
            },
        );

        assert!(graph.rows.iter().all(|row| !row.hidden));
        assert!(graph.rows.iter().all(|row| row.collapsed.is_empty()));
    }

    /// 窗口不完整（任一父 oid 不在输入里）：祖先闭包不完整会让"⊆"把窗外
    /// 祖先误判为不存在（本例若无回退，f 会被误判为可折叠），因此整体放弃
    /// 折叠——不做部分折叠。
    #[test]
    fn an_incomplete_window_disables_collapsing() {
        // 与可折叠用例同形，但 base 不在输入里（分页边界）
        let commits = chain(&[("m", &["a", "f"]), ("f", &["a"]), ("a", &["base"])]);
        let graph = layout(
            &commits,
            LayoutOptions {
                collapse_merged_branches: true,
                ..LayoutOptions::default()
            },
        );

        assert!(graph.rows.iter().all(|row| !row.hidden));
        assert!(graph.rows.iter().all(|row| row.collapsed.is_empty()));
    }

    // ---------------------------------------------------------------- 性质测试
    //
    // T2.1 要求用 proptest 做属性测试。本环境**无法离线引入 proptest**
    // （不在 Cargo.lock、本地 registry 也没有），因此这里用固定种子的
    // xorshift 生成随机 DAG 并断言同样的性质。差异（自带的生成器）记录在
    // 本模块的文档注释与各 property 用例的注释里，不再另立验收文档：
    // 性质没有打折，只是生成器是自带的。

    /// 固定种子随机数（可复现；不引入依赖）。
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            // xorshift64*
            let mut state = self.0;
            state ^= state >> 12;
            state ^= state << 25;
            state ^= state >> 27;
            self.0 = state;
            state.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }

        fn below(&mut self, bound: usize) -> usize {
            if bound == 0 {
                return 0;
            }
            usize::try_from(self.next() % u64::try_from(bound).unwrap_or(1)).unwrap_or(0)
        }
    }

    /// 生成一个合法的"新 → 旧"提交序列：父提交的下标一定大于孩子。
    ///
    /// 这样构造的图一定是 DAG（无环），且满足布局器的输入前提。
    fn random_history(seed: u64, length: usize, max_parents: usize) -> Vec<Commit> {
        let mut rng = Rng(seed);
        let mut parents_of: Vec<Vec<usize>> = Vec::with_capacity(length);

        for index in 0..length {
            let remaining = length - index - 1;
            let mut parents = Vec::new();
            if remaining > 0 {
                // 至少有一个父提交的概率很高（否则图会碎成一堆孤点）
                let count = 1 + rng.below(max_parents.max(1));
                for _ in 0..count {
                    let parent = index + 1 + rng.below(remaining);
                    if !parents.contains(&parent) || rng.below(8) == 0 {
                        parents.push(parent);
                    }
                }
            }
            parents_of.push(parents);
        }

        let oids: Vec<String> = (0..length).map(|index| format!("c{index:04}")).collect();
        (0..length)
            .map(|index| {
                let parents: Vec<&str> = parents_of[index]
                    .iter()
                    .filter_map(|parent| oids.get(*parent).map(String::as_str))
                    .collect();
                commit(&oids[index], &parents)
            })
            .collect()
    }

    /// P1：所有父子关系都有对应边，且每条边两端的 lane 与节点一致。
    #[test]
    fn property_every_parent_relation_has_an_edge_with_matching_lanes() {
        for seed in 1..=40_u64 {
            let commits = random_history(seed, 60, 3);
            let graph = layout(&commits, LayoutMode::AllBranches.into());

            let lane_of = |oid: &str| graph.row_of(oid).map(|row| row.lane).unwrap_or_default();

            let mut expected = 0;
            for commit in &commits {
                // 布局会对重复的父提交去重（图上只有一条线），
                // 因此这里的期望值也按"去重后的父子关系"计数
                let mut unique: Vec<&String> = Vec::new();
                for parent in &commit.parents {
                    if !unique.contains(&parent) {
                        unique.push(parent);
                    }
                }
                for parent in unique {
                    expected += 1;
                    let edge = graph
                        .edges
                        .iter()
                        .find(|edge| edge.from_oid == commit.oid && edge.to_oid == *parent);
                    let edge = edge.unwrap_or_else(|| {
                        panic!("seed {seed}：缺少 {} → {parent} 的边", commit.oid)
                    });
                    assert_eq!(edge.from_lane, lane_of(&commit.oid), "seed {seed}");
                    assert_eq!(edge.to_lane, lane_of(parent), "seed {seed}");
                }
            }
            assert_eq!(
                graph.edges.len(),
                expected,
                "seed {seed}：边数应与父子关系数一致"
            );
        }
    }

    /// P2：确定性 —— 同输入两次布局结果完全相同。
    #[test]
    fn property_layout_is_deterministic() {
        for seed in 1..=20_u64 {
            let commits = random_history(seed * 7, 80, 4);
            assert_eq!(
                layout(&commits, LayoutMode::AllBranches.into()),
                layout(&commits, LayoutMode::AllBranches.into()),
                "seed {seed}"
            );
        }
    }

    /// P3：无重叠 —— 同一 lane 上，任何两条"同 lane 边"的行区间不得部分重叠。
    ///
    /// 区间定义：`[孩子行, 父行)`。首尾相接（一条边结束处另一条开始）是正常的
    /// （那正是同一条链的延续），不允许的是交叉或包含。
    #[test]
    fn property_edges_in_one_lane_never_overlap() {
        for seed in 1..=40_u64 {
            let commits = random_history(seed * 13, 70, 3);
            let graph = layout(&commits, LayoutMode::AllBranches.into());
            let row_of = |oid: &str| graph.row_of(oid).map(|row| row.row).unwrap_or_default();

            let mut spans: HashMap<u16, Vec<(u32, u32, String)>> = HashMap::new();
            for edge in &graph.edges {
                if edge.from_lane != edge.to_lane {
                    continue;
                }
                let from = row_of(&edge.from_oid);
                let to = row_of(&edge.to_oid);
                if to <= from {
                    continue; // 窗口外的父提交没有行号，跳过（由 P1 保证 lane 一致）
                }
                spans.entry(edge.from_lane).or_default().push((
                    from,
                    to,
                    format!("{}→{}", edge.from_oid, edge.to_oid),
                ));
            }

            for (lane, mut intervals) in spans {
                intervals.sort_by_key(|(from, _, _)| *from);
                for pair in intervals.windows(2) {
                    let (_, first_end, first_label) = &pair[0];
                    let (second_start, _, second_label) = &pair[1];
                    assert!(
                        second_start >= first_end,
                        "seed {seed} lane {lane}：{first_label} 与 {second_label} 的行区间重叠",
                    );
                }
            }
        }
    }

    /// P4：分页一致性 —— 先取前 100 条再取前 200 条，前 100 条的 lane 分配必须一致。
    ///
    /// 这是最容易出错的地方：任何"往回看"或"按整段长度分配"的实现都会在这里翻车。
    #[test]
    fn property_lane_assignment_is_stable_across_page_sizes() {
        for seed in 1..=30_u64 {
            let commits = random_history(seed * 31, 200, 2);
            let short = layout(&commits[..100], LayoutMode::AllBranches.into());
            let long = layout(&commits, LayoutMode::AllBranches.into());

            assert_eq!(short.rows.len(), 100);
            for (index, row) in short.rows.iter().enumerate() {
                assert_eq!(
                    row, &long.rows[index],
                    "seed {seed}：第 {index} 行的位置随窗口长度变了"
                );
            }
            // 前 100 条内的边也必须一模一样（窗口外的父提交不影响已定型的边）
            let inside: Vec<&GraphEdge> = long
                .edges
                .iter()
                .filter(|edge| {
                    short.row_of(&edge.from_oid).is_some() && short.row_of(&edge.to_oid).is_some()
                })
                .collect();
            let short_inside: Vec<&GraphEdge> = short
                .edges
                .iter()
                .filter(|edge| {
                    short.row_of(&edge.from_oid).is_some() && short.row_of(&edge.to_oid).is_some()
                })
                .collect();
            assert_eq!(inside, short_inside, "seed {seed}：窗口内的边集合应当一致");
        }
    }

    /// 随机历史上 lane 数量必须是有限的、且不超过提交数（结构性断言，
    /// 防止"每次 tip 都新开 lane"这类实现错误）。
    #[test]
    fn property_lane_count_stays_within_the_commit_count() {
        for seed in 1..=20_u64 {
            let commits = random_history(seed * 17, 50, 4);
            let graph = layout(&commits, LayoutMode::AllBranches.into());
            assert!(graph.lane_count >= 1);
            assert!(
                usize::from(graph.lane_count) <= commits.len(),
                "seed {seed}"
            );
        }
    }

    /// P5（折叠）：hidden 的行与 collapsed 列表里的 oid，必然落在某个 merge
    /// 第二父的可达集里（折叠只可能标记第二父那条线上的提交）。
    #[test]
    fn property_hidden_rows_are_within_the_merged_sides_reachable_sets() {
        for seed in 1..=30_u64 {
            let commits = random_history(seed * 19, 80, 3);
            let graph = layout(
                &commits,
                LayoutOptions {
                    collapse_merged_branches: true,
                    ..LayoutOptions::default()
                },
            );

            // 测试侧独立重算可达闭包（含起点自身；父下标 > 孩子下标）
            let mut parents_of: HashMap<&str, Vec<&str>> = HashMap::new();
            for commit in &commits {
                parents_of
                    .entry(commit.oid.as_str())
                    .or_default()
                    .extend(commit.parents.iter().map(String::as_str));
            }
            let mut union: Vec<&str> = Vec::new();
            for commit in &commits {
                // 与布局同一套去重规则：重复的父在图上只有一条线
                let mut unique: Vec<&str> = Vec::new();
                for parent in &commit.parents {
                    if !unique.contains(&parent.as_str()) {
                        unique.push(parent.as_str());
                    }
                }
                if unique.len() < 2 {
                    continue;
                }
                for oid in reachable_from(&parents_of, unique[1]) {
                    if !union.contains(&oid) {
                        union.push(oid);
                    }
                }
            }

            for row in &graph.rows {
                if row.hidden {
                    assert!(
                        union.contains(&row.oid.as_str()),
                        "seed {seed}：hidden 的 {} 必须在某个 merge 第二父的可达集里",
                        row.oid
                    );
                }
                for collapsed in &row.collapsed {
                    assert!(
                        union.contains(&collapsed.as_str()),
                        "seed {seed}：collapsed 的 {collapsed} 必须在某个 merge 第二父的可达集里"
                    );
                }
            }
        }
    }

    /// P6（折叠）：折叠只做标记——rows 总数、位置字段（lane/row/color/is_merge）、
    /// edges 与 lane_count 与关闭折叠时完全一致；hidden 之外唯一允许的差异是
    /// merge 行的 collapsed 列表（重复合并里 tip 已在主线可达集，不置 hidden
    /// 但仍记录 tip）。
    #[test]
    fn property_collapsing_only_marks_rows_and_never_moves_them() {
        for seed in 1..=30_u64 {
            let commits = random_history(seed * 23, 80, 3);
            let plain = layout(&commits, LayoutOptions::default());
            let marked = layout(
                &commits,
                LayoutOptions {
                    collapse_merged_branches: true,
                    ..LayoutOptions::default()
                },
            );

            assert_eq!(marked.rows.len(), plain.rows.len(), "seed {seed}：行数不变");
            for (index, marked_row) in marked.rows.iter().enumerate() {
                let plain_row = &plain.rows[index];
                // 关闭折叠时永远没有折叠标记
                assert!(
                    !plain_row.hidden && plain_row.collapsed.is_empty(),
                    "seed {seed}：第 {index} 行"
                );
                // 位置字段不变：折叠只做标记，不重排
                assert_eq!(marked_row.oid, plain_row.oid, "seed {seed}：第 {index} 行");
                assert_eq!(
                    marked_row.lane, plain_row.lane,
                    "seed {seed}：第 {index} 行"
                );
                assert_eq!(marked_row.row, plain_row.row, "seed {seed}：第 {index} 行");
                assert_eq!(
                    marked_row.color_index, plain_row.color_index,
                    "seed {seed}：第 {index} 行"
                );
                assert_eq!(
                    marked_row.is_merge, plain_row.is_merge,
                    "seed {seed}：第 {index} 行"
                );
                // hidden 只有折叠开启才可能置位
                if !marked_row.hidden {
                    assert_eq!(
                        plain_row.hidden, marked_row.hidden,
                        "seed {seed}：第 {index} 行"
                    );
                }
            }
            assert_eq!(marked.edges, plain.edges, "seed {seed}：边不变");
            assert_eq!(marked.lane_count, plain.lane_count, "seed {seed}");
        }
    }

    /// P7（折叠）：确定性 —— 同输入两次折叠布局结果完全相同。
    #[test]
    fn property_collapsing_is_deterministic() {
        for seed in 1..=20_u64 {
            let commits = random_history(seed * 29, 80, 3);
            let options = LayoutOptions {
                collapse_merged_branches: true,
                ..LayoutOptions::default()
            };
            assert_eq!(
                layout(&commits, options),
                layout(&commits, options),
                "seed {seed}"
            );
        }
    }

    /// 测试版可达闭包（含起点自身）：沿父边向下收集直到没有新提交。
    fn reachable_from<'a>(
        parents_of: &HashMap<&'a str, Vec<&'a str>>,
        tip: &'a str,
    ) -> Vec<&'a str> {
        let mut seen = vec![tip];
        let mut queue = vec![tip];
        while let Some(oid) = queue.pop() {
            if let Some(parents) = parents_of.get(oid) {
                for parent in parents {
                    if !seen.contains(parent) {
                        seen.push(parent);
                        queue.push(parent);
                    }
                }
            }
        }
        seen
    }

    /// T2.1 的性能门槛：5000 个节点的布局 < 200ms。
    ///
    /// criterion 无法离线安装（同 proptest），因此用 release 模式下的计时断言代替：
    /// 断言的是任务书给的上限（200ms）而不是固定值，机器波动不会造成误报。
    /// 跑法：`cargo test --release -p forgedesk-domain -- --ignored --nocapture`
    #[test]
    #[allow(clippy::print_stdout)]
    #[ignore = "性能基准：--release --ignored"]
    fn layout_of_5000_nodes_stays_under_200ms() {
        let commits = random_history(0xDEAD_BEEF, 5_000, 3);
        let started = std::time::Instant::now();
        let graph = layout(&commits, LayoutMode::AllBranches.into());
        let elapsed = started.elapsed();

        println!(
            "布局 5000 节点：{elapsed:.1?}，边 {} 条，lane {} 条",
            graph.edges.len(),
            graph.lane_count
        );
        assert!(
            elapsed.as_millis() < 200,
            "布局耗时 {elapsed:?} 超过任务书要求的 200ms"
        );
    }
}
