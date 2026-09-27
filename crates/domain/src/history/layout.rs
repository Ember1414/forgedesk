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

use std::collections::HashMap;

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
#[derive(Debug, Clone, PartialEq, Eq)]
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
pub fn layout(commits: &[Commit], mode: LayoutMode) -> GraphLayout {
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
        let graph = layout(&commits, LayoutMode::AllBranches);

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
        let graph = layout(&commits, LayoutMode::AllBranches);

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
        let graph = layout(&commits, LayoutMode::AllBranches);

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
        let graph = layout(&commits, LayoutMode::AllBranches);

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
        let graph = layout(&commits, LayoutMode::AllBranches);

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
        let graph = layout(&commits, LayoutMode::AllBranches);

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
        let graph = layout(&commits, LayoutMode::FirstParentOnly);

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
        let graph = layout(&commits, LayoutMode::AllBranches);

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
        let graph = layout(&commits, LayoutMode::AllBranches);

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

    // ---------------------------------------------------------------- 性质测试
    //
    // T2.1 要求用 proptest 做属性测试。本环境**无法离线引入 proptest**
    // （不在 Cargo.lock、本地 registry 也没有），因此这里用固定种子的
    // xorshift 生成随机 DAG 并断言同样的四条性质。
    // 差异记在 `docs/acceptance/M2.md`：性质没有打折，只是生成器是自带的。

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
            let graph = layout(&commits, LayoutMode::AllBranches);

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
                layout(&commits, LayoutMode::AllBranches),
                layout(&commits, LayoutMode::AllBranches),
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
            let graph = layout(&commits, LayoutMode::AllBranches);
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
            let short = layout(&commits[..100], LayoutMode::AllBranches);
            let long = layout(&commits, LayoutMode::AllBranches);

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
            let graph = layout(&commits, LayoutMode::AllBranches);
            assert!(graph.lane_count >= 1);
            assert!(
                usize::from(graph.lane_count) <= commits.len(),
                "seed {seed}"
            );
        }
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
        let graph = layout(&commits, LayoutMode::AllBranches);
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
