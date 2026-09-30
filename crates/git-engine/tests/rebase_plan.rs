//! rebase 计划模型（T3.5）与真实 git 的对拍。
//!
//! # 任务书验收项的落地
//!
//! - **① todo 可被真实 git 解析**：把 domain 生成的 todo 通过
//!   `GIT_SEQUENCE_EDITOR="cp <file>"` 注入真实 `git rebase -i`——editor 收到
//!   的第二个参数是 rebase 自己的 todo 路径，`cp src dst` 恰好完成"替换"。
//!   解析失败（格式错）会让 rebase 非零退出——断言成功即证明可解析。
//! - **② preview 数量与真实执行一致**：对 20 组**确定性伪随机**的合法计划，
//!   分别用 domain 的 `preview` 预测存活提交数，再真实执行，与执行后的
//!   `rev-list --count` 对比。proptest 在本机装不上（离线沙箱），用固定
//!   种子的 LCG 替代——种子固定，失败可复现（取舍记录在 domain/rebase.rs）。
//!
//! 动作集合刻意排除 `Edit`（会停下等用户改内容，无法干跑）；
//! `Reword` 用 `GIT_EDITOR=true` 原样通过（新信息的注入验证归 T3.7 执行引擎）。
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::collections::{HashMap, HashSet};

use forgedesk_domain::git::{GraphCommit, GraphView, RebasePlan, ReorderAction, ReorderStep};
use support::{commit_all, git, git_ok, git_with_env, init_repo, write, TempDir};

// ---------------------------------------------------------------- 夹具

/// 线性仓库：base + `commit_count` 个提交；返回 (目录, 旧到新的 oid 表)。
fn linear_repo(prefix: &str, commit_count: usize) -> (TempDir, Vec<String>) {
    let dir = TempDir::new(prefix);
    init_repo(dir.path());
    write(dir.path(), "a.txt", b"base\n");
    commit_all(dir.path(), "base", 0);
    let mut oids = Vec::new();
    for index in 0..commit_count {
        write(
            dir.path(),
            &format!("f{index}.txt"),
            format!("content {index}\n").as_bytes(),
        );
        commit_all(dir.path(), &format!("commit {index}"), index as u32 + 1);
        let oid = git(dir.path(), &["rev-parse", "HEAD"])
            .stdout_lossy()
            .trim()
            .to_owned();
        oids.push(oid);
    }
    (dir, oids)
}

/// 从 git 输出装配 GraphView。
///
/// 用 NUL 分隔的记录格式（`%x00`）：一条 = "oid 父… 换行 主题 NUL"。
/// 不能用行式解析：测试提交的信息本身以 "commit " 开头，会与 log 的记录头
/// 混淆（真实事故：图被解析成空，所有 oid 误报越界）。
fn load_graph(dir: &std::path::Path, base: &str, head: &str) -> GraphView {
    let output = git(
        dir,
        &[
            "log",
            "--format=%H %P%x00%s%x00",
            &format!("{base}..{head}"),
        ],
    );
    let mut commits = HashMap::new();
    // %x00 把每条提交切成 header（oid+父）与 subject 两个**交替**的段；
    // 段与段之间有记录分隔的 \n（在下一 header 的开头）
    let stdout = output.stdout_lossy();
    let mut chunks = stdout.split('\u{0}');
    while let (Some(header), Some(subject)) = (chunks.next(), chunks.next()) {
        let header = header.trim_start_matches('\n');
        if header.is_empty() {
            continue;
        }
        let mut ids = header.split_whitespace();
        let Some(oid) = ids.next() else { continue };
        commits.insert(
            oid.to_owned(),
            GraphCommit {
                parents: ids.map(str::to_owned).collect(),
                subject: subject.trim_start_matches('\n').to_owned(),
            },
        );
    }
    GraphView {
        commits,
        pushed_oids: HashSet::new(),
    }
}

/// 确定性 LCG（固定种子；换 proptest 时只替换这一段）。
struct Lcg(u64);

impl Lcg {
    fn next(&mut self, bound: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) % bound
    }
}

/// 从 oid 列表生成一组随机**合法**计划：
/// - 保持从旧到新的拓扑顺序；
/// - 第一条不是 Squash / Fixup；
/// - 至少保留一个非 Drop 提交；
/// - Squash / Fixup 需要前面有存活的提交可并入（空档降级为 Pick）。
fn random_plan(seed: u64, base: &str, head: &str, oids: &[String]) -> RebasePlan {
    let mut rng = Lcg(seed);
    let mut steps: Vec<ReorderStep> = Vec::new();
    let actions = [
        ReorderAction::Pick,
        ReorderAction::Reword,
        ReorderAction::Drop,
        ReorderAction::Squash,
        ReorderAction::Fixup,
    ];
    let mut kept = 0_usize;
    for oid in oids {
        let action = actions[rng.next(actions.len() as u64) as usize];
        let squash_first =
            steps.is_empty() && matches!(action, ReorderAction::Squash | ReorderAction::Fixup);
        let no_merge_target =
            matches!(action, ReorderAction::Squash | ReorderAction::Fixup) && kept == 0;
        let drops_everything =
            action == ReorderAction::Drop && kept == 0 && steps.len() + 1 == oids.len();
        let step = if squash_first || no_merge_target || drops_everything {
            ReorderStep {
                oid: oid.clone(),
                action: ReorderAction::Pick,
                new_message: None,
            }
        } else {
            if action != ReorderAction::Drop {
                kept += 1;
            }
            ReorderStep {
                oid: oid.clone(),
                action,
                new_message: None,
            }
        };
        steps.push(step);
    }
    RebasePlan {
        base: base.to_owned(),
        head: head.to_owned(),
        steps,
        allow_flatten_merges: false,
        autosquash: false,
    }
}

#[test]
fn twenty_random_plans_execute_exactly_as_previewed() {
    let (dir, oids) = linear_repo("rebase-prop", 6);
    // rebase 的 base = 第一个提交；待重排区间 = 其后的 5 个提交
    let base = oids[0].clone();
    let head = oids[5].clone();
    let chain: Vec<String> = oids[1..].to_vec();
    let graph = load_graph(dir.path(), &base, &head);

    let mut mismatches = Vec::new();
    for seed in 1..=20_u64 {
        let plan = random_plan(seed, &base, &head, &chain);
        // 合法性闸门（生成器保证，validate 兜底）
        plan.validate(&graph)
            .unwrap_or_else(|errors| panic!("seed {seed} 生成了非法计划：{errors:?}"));
        let expected_count = plan.preview(&graph).surviving.len();

        // 注入 todo 并真实执行
        let todo = plan.to_todo_file(&graph);
        let todo_path = dir.path().join(".git").join("forgedesk-todo");
        std::fs::write(&todo_path, todo.as_bytes()).unwrap();
        let editor = format!("cp {}", todo_path.to_string_lossy().replace('\\', "/"));

        git_ok(dir.path(), &["reset", "--hard", &head]);
        let output = git_with_env(
            dir.path(),
            &["rebase", "-i", &base],
            &[
                ("GIT_SEQUENCE_EDITOR", editor.as_str()),
                ("GIT_EDITOR", "true"),
            ],
        );
        assert!(
            output.success(),
            "seed {seed} 的 todo 被 git 拒绝：{}",
            output.stderr_lossy()
        );

        let after = git(
            dir.path(),
            &["rev-list", "--count", &format!("{base}..HEAD")],
        )
        .stdout_lossy()
        .trim()
        .to_owned();
        let after_count: usize = after.parse().unwrap();
        if after_count != expected_count {
            mismatches.push(format!(
                "seed {seed}: preview {expected_count} != actual {after_count}"
            ));
        }
    }
    assert!(
        mismatches.is_empty(),
        "preview 与真实执行不一致：{mismatches:?}"
    );
}
