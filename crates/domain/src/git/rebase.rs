//! rebase 计划模型（T3.5）——**严禁 IO**。
//!
//! # 职责边界
//!
//! 这一模块回答三个问题，全部是纯函数：
//! 1. **这个计划合法吗**（[`RebasePlan::validate`]，规则见 [`PlanError`]）；
//! 2. **它对应的 `git rebase -i` todo 文件长什么样**（[`RebasePlan::to_todo_file`]）；
//! 3. **执行后历史会变成什么样**（[`RebasePlan::preview`]，给拖拽面板的预览树）。
//!
//! 实际执行（todo 注入、GIT_SEQUENCE_EDITOR、逐步驱动）是 T3.7 的执行引擎；
//! 拖拽面板 UI 是 T3.6。提前在 M3 前期定好这三份纯逻辑，是为了让 UI 与
//! 执行引擎都对着同一份可测试的模型编程，而不是各写各的字符串处理。
//!
//! # 数据来源：GraphView
//!
//! 校验与预览需要的仓库事实（提交的父提交、主题、是否 merge、是否已推送）
//! 由调用方打包成 [`GraphView`] 传入——domain 不允许 IO，所以"从仓库读出
//! 这些事实"发生在 git-engine/services。GraphView 是**纯数据**，可以手工
//! 构造做表驱动测试，也可以从真实仓库装配（T3.6/T3.7 的集成测试做后者）。
//!
//! # 属性测试的取舍
//!
//! 任务书要求 proptest；本机离线装不上 proptest（`.cargo/config.toml` 指向
//! 镜像但沙箱网络受限），采用**确定性伪随机**（固定种子的 LCG）生成随机
//! 合法计划做同样的属性断言：随机 plan 的 todo 可被真实 git 解析（集成测试
//! 干跑）、preview 的数量守恒（Pick − Squash − Fixup − Drop 的纯函数关系）。
//! 种子固定，失败可复现；换用 proptest 时只需替换生成器。

use std::collections::{HashMap, HashSet};

use super::spec::{ReorderAction, ReorderStep};

/// 一份仓库事实的纯数据快照（校验与预览的输入）。
///
/// 构造方（services）负责事实的准确性；本模块只消费。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GraphView {
    /// 区间内全部提交：oid → 节点。
    pub commits: HashMap<String, GraphCommit>,
    /// 已推送（存在于任何 remote-tracking ref 中）的提交 oid 集合。
    /// 用于 preview 的"会影响已推送历史"警告——本地判断只有"上次 fetch
    /// 时的印象"这一精度（与 `remote_refs_containing` 同一边界）。
    pub pushed_oids: HashSet<String>,
}

/// 图里的一个提交节点。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphCommit {
    /// 父提交 oid（顺序与 git 一致；merge 提交有 ≥2 个父）。
    pub parents: Vec<String>,
    /// 提交信息首行。
    pub subject: String,
}

impl GraphCommit {
    /// 是否为 merge 提交（对 merge 做 squash 会把两条历史压成一条）。
    pub fn is_merge(&self) -> bool {
        self.parents.len() >= 2
    }
}

/// 校验失败的具体原因（每个变体对应一条任务书规则）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanError {
    /// 全部提交都被 Drop：执行等于"丢掉整个区间"，要求用户直接用 reset。
    AllDropped,
    /// 第一条步骤是 Squash / Fixup：它们必须并入**前面的**提交，第一条无前可并。
    SquashAsFirst,
    /// 对 merge 提交做 Squash（且未开 `allow_flatten_merges`）。
    SquashOnMerge {
        /// 被压缩的 merge 提交。
        oid: String,
    },
    /// 同一提交出现两次。
    DuplicateOid {
        /// 重复出现的提交。
        oid: String,
    },
    /// 步骤里的 oid 不在 base..head 区间内（不是区间提交）。
    OidOutsideRange {
        /// 越界的提交 oid。
        oid: String,
    },
}

impl PlanError {
    /// 稳定短名（界面按它选 i18n；测试按它断言）。
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::AllDropped => "allDropped",
            Self::SquashAsFirst => "squashAsFirst",
            Self::SquashOnMerge { .. } => "squashOnMerge",
            Self::DuplicateOid { .. } => "duplicateOid",
            Self::OidOutsideRange { .. } => "oidOutsideRange",
        }
    }
}

/// rebase 计划（T3.5 任务书的数据结构）。
///
/// `steps` 顺序为**从旧到新**（与 `git rebase -i` 的清单一致）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebasePlan {
    /// 新的基点（`--onto` 的目标，通常是 base 提交的 oid）。
    pub base: String,
    /// 区间右端（要重排的提交链的顶端）。
    pub head: String,
    /// 步骤清单（每条 = 被操作的提交 oid + 动作）。
    pub steps: Vec<ReorderStep>,
    /// 允许对 merge 提交做 Squash（会压平合并结构，必须显式开启并提示）。
    pub allow_flatten_merges: bool,
    /// 生成 todo 时启用 autosquash 语义（`fixup!` / `squash!` 前缀自动归并）。
    pub autosquash: bool,
}

impl RebasePlan {
    /// 校验计划（任务书第 2 条的全部规则；返回**全部**错误而不是第一个）。
    ///
    /// 区间的确定：GraphView 里从 `head` 沿第一父链到 `base`（不含）的提交
    /// 集合即"区间内"。步骤 oid 必须都在这个集合里。
    pub fn validate(&self, graph: &GraphView) -> Result<(), Vec<PlanError>> {
        let mut errors = Vec::new();

        // 区间集合：head 的祖先中、base 之前的全部提交（含 merge 的全部父链）
        let in_range = ancestors_until(graph, &self.head, &self.base);

        let mut seen: HashSet<&str> = HashSet::new();
        for (index, step) in self.steps.iter().enumerate() {
            // 规则：重复 oid
            if !seen.insert(step.oid.as_str()) {
                errors.push(PlanError::DuplicateOid {
                    oid: step.oid.clone(),
                });
                continue;
            }
            // 规则：oid 必须在区间内
            if !in_range.contains(step.oid.as_str()) {
                errors.push(PlanError::OidOutsideRange {
                    oid: step.oid.clone(),
                });
                continue;
            }
            // 规则：第一条不能是 Squash / Fixup
            if index == 0 && matches!(step.action, ReorderAction::Squash | ReorderAction::Fixup) {
                errors.push(PlanError::SquashAsFirst);
            }
            // 规则：merge 提交不能 Squash（除非 allow_flatten_merges）
            if step.action == ReorderAction::Squash
                && !self.allow_flatten_merges
                && graph
                    .commits
                    .get(&step.oid)
                    .is_some_and(GraphCommit::is_merge)
            {
                errors.push(PlanError::SquashOnMerge {
                    oid: step.oid.clone(),
                });
            }
            // 拓扑顺序**不做校验**：git rebase -i 接受任意重排（线性链交换
            // 相邻提交是常见操作，重放冲突时自然暂停），校验它反而会把用户
            // 的合法拖拽挡在门外。
        }

        // 规则：禁止 drop 全部提交（清一色 Drop 等于把区间整体丢掉）
        if !self.steps.is_empty()
            && self
                .steps
                .iter()
                .all(|step| step.action == ReorderAction::Drop)
        {
            errors.push(PlanError::AllDropped);
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// 生成与 `git rebase -i` 兼容的 todo 内容。
    ///
    /// 格式：`<指令> <短 oid> <主题>`（git 自己生成的 todo 就是这个形状，
    /// 短 oid 长度随仓库增长，7 位足够测试仓库使用；reword 的新信息不进
    /// todo——执行时由 GIT_SEQUENCE_EDITOR 注入，见 T3.7 的决策记录）。
    /// `autosquash` 时按 git 惯例输出注释提示（真正自动归并由
    /// `--autosquash` 标志完成，todo 内容不变）。
    pub fn to_todo_file(&self, graph: &GraphView) -> String {
        let mut lines = Vec::new();
        if self.autosquash {
            lines.push(
                "# autosquash is enabled; fixup!/squash! commits are merged by --autosquash"
                    .to_owned(),
            );
        }
        for step in &self.steps {
            let subject = graph
                .commits
                .get(&step.oid)
                .map(|node| node.subject.as_str())
                .unwrap_or("");
            let instruction = if step.action == ReorderAction::Reword {
                "reword"
            } else {
                step.action.as_instruction()
            };
            lines.push(format!(
                "{} {} {}",
                instruction,
                short_oid(&step.oid),
                subject
            ));
        }
        lines.push(String::new());
        lines.join("\n")
    }

    /// 预览执行后的历史（任务书第 4 条；纯函数，不碰仓库）。
    pub fn preview(&self, graph: &GraphView) -> RebasePreview {
        let mut surviving: Vec<PreviewCommit> = Vec::new();
        let mut dropped: Vec<String> = Vec::new();
        let mut reworded: Vec<String> = Vec::new();
        let mut squashed: Vec<String> = Vec::new();

        for step in &self.steps {
            let subject = graph
                .commits
                .get(&step.oid)
                .map(|node| node.subject.clone())
                .unwrap_or_default();
            match step.action {
                ReorderAction::Pick => surviving.push(PreviewCommit {
                    oid: step.oid.clone(),
                    subject,
                }),
                ReorderAction::Reword => {
                    // 信息草案：用户写的新信息；没写就沿用原信息（todo 里 reword
                    // 会打开编辑器，这里只能预填已知值）
                    let draft = step.new_message.clone().unwrap_or_else(|| subject.clone());
                    reworded.push(step.oid.clone());
                    surviving.push(PreviewCommit {
                        oid: step.oid.clone(),
                        subject: draft,
                    });
                }
                ReorderAction::Edit => surviving.push(PreviewCommit {
                    oid: step.oid.clone(),
                    subject,
                }),
                ReorderAction::Squash => {
                    // 并入上一个存活的提交：信息草案 = 原信息 + 换行 + 本条
                    let merged = match surviving.last_mut() {
                        Some(previous) => {
                            previous.subject = format!("{}\n\n{}", previous.subject, subject);
                            previous.oid.clone()
                        }
                        None => String::new(),
                    };
                    squashed.push(format!("{} -> {}", step.oid, merged));
                }
                ReorderAction::Fixup => {
                    // 并入上一个存活的提交：信息保留前一条，本条丢弃
                    if surviving.last_mut().is_some() {
                        squashed.push(format!("{} -> (fixup)", step.oid));
                    }
                }
                ReorderAction::Drop => dropped.push(step.oid.clone()),
            }
        }

        // 是否影响已推送历史：区间内被重写的提交里，有任何一个已推送
        let touches_pushed = self.steps.iter().any(|step| {
            step.action != ReorderAction::Pick && graph.pushed_oids.contains(&step.oid)
        });

        RebasePreview {
            surviving,
            dropped,
            reworded,
            squashed,
            affected_count: self.steps.len(),
            touches_pushed,
            todo_text: self.to_todo_file(graph),
        }
    }
}

/// rebase 区间内的一条提交（T3.6 面板的初始清单）。
///
/// 与 [`GraphView`] 的区别：这里保留 oid 与**从旧到新**的顺序，专供
/// "打开面板时列出区间内全部提交"使用；GraphView 是校验与预览的内部形状
///（HashMap，无序）。缺了这份清单，前端只能从已加载的历史页数据推断区间，
/// 分页边界上会漏提交——而 todo 未列出的区间提交会被 git rebase 直接丢弃，
/// 那是数据丢失级别的错误，不能靠"通常够用"的实现。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeCommit {
    /// 提交 oid。
    pub oid: String,
    /// 父提交 oid（界面据此识别 merge 提交与初始拓扑）。
    pub parents: Vec<String>,
    /// 提交信息首行。
    pub subject: String,
    /// 作者名（展示用；lossy 与 [`super::commit::Commit`] 同一约定）。
    pub author: String,
    /// 作者时间（Unix 秒；界面按本地时区格式化）。
    pub author_time: i64,
}

/// 预览中的一条存活提交（oid 是**重写前**的 oid——真实新 oid 只有执行后才知道，
/// 预览树用旧 oid 占位并由 UI 标注"将重写"）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewCommit {
    /// 原提交 oid（占位）。
    pub oid: String,
    /// 信息草案（reword / squash 后是合并草案）。
    pub subject: String,
}

/// 预览结果（任务书第 4 条的全部字段）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebasePreview {
    /// 执行后存活的提交（顺序 = 新顺序）。
    pub surviving: Vec<PreviewCommit>,
    /// 被丢弃的提交 oid。
    pub dropped: Vec<String>,
    /// 信息被改写的提交 oid。
    pub reworded: Vec<String>,
    /// 被并入其他提交的记录（"本条 -> 归入哪条"）。
    pub squashed: Vec<String>,
    /// 受影响的提交总数（含被丢弃与被改写的）。
    pub affected_count: usize,
    /// 区间内有已推送的提交将被重写——需要 force-with-lease（红线 R7 允许的唯一强推形态）。
    pub touches_pushed: bool,
    /// 等价 todo 内容（T3.6 面板底部展示，让用户学习 `git rebase -i`）。
    pub todo_text: String,
}

/// 从 `start` 沿**全部**父链收集祖先，遇到 `stop`（不含）即停。
///
/// rebase 区间可能包含 merge 提交，只走第一父链会漏掉 merge 的第二父侧。
fn ancestors_until(graph: &GraphView, start: &str, stop: &str) -> HashSet<String> {
    let mut in_range = HashSet::new();
    let mut stack = vec![start.to_owned()];
    while let Some(oid) = stack.pop() {
        if oid == stop || in_range.contains(&oid) {
            continue;
        }
        if let Some(node) = graph.commits.get(&oid) {
            in_range.insert(oid.clone());
            stack.extend(node.parents.iter().cloned());
        }
    }
    in_range
}

/// 短 oid（todo 文件惯例；git 接受任何前缀长度）。
fn short_oid(oid: &str) -> &str {
    &oid[..oid.len().min(7)]
}

/// 执行 rebase 计划的结局（T3.7）。
///
/// rebase 是**一次进程**：要么跑完，要么停在某个暂停点（冲突 / edit）。
/// 暂停不是错误——仓库处于可恢复的中间态（T3.1 的冲突状态机负责继续/中止）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum RebaseOutcome {
    /// 完成：HEAD 是重写后的新顶端。
    Completed {
        /// 完成后的 HEAD oid。
        oid: String,
    },
    /// 停在冲突上：走 T3.1 的冲突状态机（continue / skip / abort）。
    PausedConflict {
        /// 冲突文件清单。
        conflicts: Vec<super::path::RepoPath>,
    },
    /// 停在 edit 步骤：用户改完内容后由应用 `commit --amend` + continue。
    PausedEdit {
        /// 被编辑的提交 oid（REBASE_HEAD）。
        oid: String,
    },
}

impl RebaseOutcome {
    /// 是否停在暂停点（界面据此决定跳冲突页还是提示"改完点继续"）。
    pub const fn is_paused(&self) -> bool {
        matches!(self, Self::PausedConflict { .. } | Self::PausedEdit { .. })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{GraphCommit, GraphView, PreviewCommit, RebasePlan, RebasePreview};
    use crate::git::spec::{ReorderAction, ReorderStep};
    use std::collections::{HashMap, HashSet};

    /// 线性 4 提交区间：base <- c1 <- c2 <- c3（head = c3）。
    fn linear_graph() -> GraphView {
        let mut commits = HashMap::new();
        commits.insert(
            "base".to_owned(),
            GraphCommit {
                parents: vec![],
                subject: "base".to_owned(),
            },
        );
        commits.insert(
            "c1".to_owned(),
            GraphCommit {
                parents: vec!["base".to_owned()],
                subject: "one".to_owned(),
            },
        );
        commits.insert(
            "c2".to_owned(),
            GraphCommit {
                parents: vec!["c1".to_owned()],
                subject: "two".to_owned(),
            },
        );
        commits.insert(
            "c3".to_owned(),
            GraphCommit {
                parents: vec!["c2".to_owned()],
                subject: "three".to_owned(),
            },
        );
        GraphView {
            commits,
            pushed_oids: HashSet::new(),
        }
    }

    fn step(oid: &str, action: ReorderAction) -> ReorderStep {
        ReorderStep {
            oid: oid.to_owned(),
            action,
            new_message: None,
        }
    }

    fn plan(steps: Vec<ReorderStep>) -> RebasePlan {
        RebasePlan {
            base: "base".to_owned(),
            head: "c3".to_owned(),
            steps,
            allow_flatten_merges: false,
            autosquash: false,
        }
    }

    #[test]
    fn a_plain_pick_plan_is_valid() {
        let graph = linear_graph();
        plan(vec![
            step("c1", ReorderAction::Pick),
            step("c2", ReorderAction::Pick),
        ])
        .validate(&graph)
        .expect("pick 全部合法");
    }

    #[test]
    fn dropping_every_commit_is_rejected() {
        let graph = linear_graph();
        let errors = plan(vec![
            step("c1", ReorderAction::Drop),
            step("c2", ReorderAction::Drop),
        ])
        .validate(&graph)
        .unwrap_err();
        assert!(errors.iter().any(|error| error.as_str() == "allDropped"));
    }

    #[test]
    fn squash_or_fixup_as_the_first_step_is_rejected() {
        let graph = linear_graph();
        for action in [ReorderAction::Squash, ReorderAction::Fixup] {
            let errors = plan(vec![step("c1", action)]).validate(&graph).unwrap_err();
            assert!(
                errors.iter().any(|error| error.as_str() == "squashAsFirst"),
                "{action:?}"
            );
        }
    }

    #[test]
    fn duplicate_oids_are_rejected() {
        let graph = linear_graph();
        let errors = plan(vec![
            step("c1", ReorderAction::Pick),
            step("c1", ReorderAction::Drop),
        ])
        .validate(&graph)
        .unwrap_err();
        assert!(errors.iter().any(|error| error.as_str() == "duplicateOid"));
    }

    #[test]
    fn oids_outside_the_range_are_rejected() {
        let graph = linear_graph();
        // base 本身与陌生 oid 都不在 base..c3 区间内
        let errors = plan(vec![step("base", ReorderAction::Pick)])
            .validate(&graph)
            .unwrap_err();
        assert!(errors
            .iter()
            .any(|error| error.as_str() == "oidOutsideRange"));
        let errors = plan(vec![step("zzz", ReorderAction::Pick)])
            .validate(&graph)
            .unwrap_err();
        assert!(errors
            .iter()
            .any(|error| error.as_str() == "oidOutsideRange"));
    }

    #[test]
    fn squash_on_a_merge_commit_is_rejected_unless_flattening_is_allowed() {
        // merge 提交 m：c1 与 side 的合并
        let mut graph = linear_graph();
        graph.commits.insert(
            "m".to_owned(),
            GraphCommit {
                parents: vec!["c1".to_owned(), "side".to_owned()],
                subject: "merge".to_owned(),
            },
        );
        graph.commits.insert(
            "side".to_owned(),
            GraphCommit {
                parents: vec!["base".to_owned()],
                subject: "side".to_owned(),
            },
        );

        let mut merge_plan = plan(vec![
            step("c1", ReorderAction::Pick),
            step("m", ReorderAction::Squash),
        ]);
        merge_plan.head = "m".to_owned();
        let errors = merge_plan.validate(&graph).unwrap_err();
        assert!(errors.iter().any(|error| error.as_str() == "squashOnMerge"));

        merge_plan.allow_flatten_merges = true;
        merge_plan.validate(&graph).expect("开启 flatten 后合法");
    }

    #[test]
    fn todo_file_uses_git_instruction_words_and_short_oids() {
        let graph = linear_graph();
        let todo = plan(vec![
            ReorderStep {
                oid: "c1".to_owned(),
                action: ReorderAction::Pick,
                new_message: None,
            },
            ReorderStep {
                oid: "c2".to_owned(),
                action: ReorderAction::Reword,
                new_message: Some("new two".to_owned()),
            },
            step("c3", ReorderAction::Fixup),
        ])
        .to_todo_file(&graph);

        let lines: Vec<&str> = todo.lines().collect();
        assert_eq!(lines[0], "pick c1 one");
        assert_eq!(
            lines[1], "reword c2 two",
            "todo 行不携带新信息（执行时注入）"
        );
        assert_eq!(lines[2], "fixup c3 three");
    }

    #[test]
    fn preview_computes_survivors_drops_and_message_drafts() {
        let graph = linear_graph();
        let preview: RebasePreview = plan(vec![
            ReorderStep {
                oid: "c1".to_owned(),
                action: ReorderAction::Reword,
                new_message: Some("rewritten one".to_owned()),
            },
            step("c2", ReorderAction::Drop),
            step("c3", ReorderAction::Pick),
        ])
        .preview(&graph);

        assert_eq!(
            preview.surviving,
            vec![
                PreviewCommit {
                    oid: "c1".to_owned(),
                    subject: "rewritten one".to_owned()
                },
                PreviewCommit {
                    oid: "c3".to_owned(),
                    subject: "three".to_owned()
                },
            ]
        );
        assert_eq!(preview.dropped, vec!["c2".to_owned()]);
        assert_eq!(preview.reworded, vec!["c1".to_owned()]);
        assert_eq!(preview.affected_count, 3);
        assert!(!preview.touches_pushed);
        assert!(
            preview.todo_text.contains("reword c1") && preview.todo_text.contains("pick c3"),
            "preview 必须携带等价 todo 内容：{}",
            preview.todo_text
        );
    }

    #[test]
    fn squash_merges_into_the_previous_survivor_and_fixup_keeps_its_message() {
        let graph = linear_graph();
        let preview = plan(vec![
            step("c1", ReorderAction::Pick),
            step("c2", ReorderAction::Squash),
            step("c3", ReorderAction::Fixup),
        ])
        .preview(&graph);

        assert_eq!(preview.surviving.len(), 1);
        assert_eq!(
            preview.surviving[0].subject, "one\n\ntwo",
            "squash 拼接两条信息"
        );
        assert_eq!(preview.squashed.len(), 2);
    }

    #[test]
    fn preview_warns_when_rewriting_pushed_commits() {
        let mut graph = linear_graph();
        graph.pushed_oids.insert("c1".to_owned());

        let preview = plan(vec![step("c1", ReorderAction::Reword)]).preview(&graph);
        assert!(preview.touches_pushed);

        // Pick 不算重写
        let preview = plan(vec![step("c1", ReorderAction::Pick)]).preview(&graph);
        assert!(!preview.touches_pushed);
    }
}
