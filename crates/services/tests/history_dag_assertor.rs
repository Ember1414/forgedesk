//! 独立的 DAG 断言器（T2.10 第 1 条 / M2 验收第 2 条）。
//!
//! # 断言什么、怎么断
//!
//! 布局的**语义契约**是：边集合 = 窗口内每个提交指向它的每个（去重后的）
//! 父提交的有向边；`FirstParentOnly` 模式下只保留第一父。这份真值**不来自
//! 我们的任何代码**，而是直接从 git 的祖先关系（`git rev-list --parents`，
//! 每行 `oid parent…` 的机器可读形状）独立推导，再与布局器产出的边集合
//! 做**集合等价**断言——不比对 `--graph` 的 ASCII art，也不依赖泳道编号。
//!
//! # 为什么泳道编号不参与断言
//!
//! 编号是布局器的实现自由（`layout_dag.rs` 的泳道**分组**对照测试钉过它与
//! git 的分组等价）；只要"谁连到谁"与祖先关系一致，图就画不出假历史。
//!
//! # 六种形状（M2 验收清单）
//!
//! 线性 / 分叉合并 / octopus merge（4 父）/ 游离 HEAD / 空提交 /
//! 同一分支被合并两次（重复合并）。每种形状都跑 AllBranches 与
//! FirstParentOnly 两种模式。
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::path::Path;

use forgedesk_domain::history::LayoutMode;
use forgedesk_services::{GitEngines, HistoryQuery, HistoryService, RepositoryService};
use forgedesk_storage::{Database, RepositoryStore};

mod support;

use support::{init_repo, memory_database, write, TempDir};

// ---------------------------------------------------------------- 夹具助手

/// 用固定时间戳提交（walk 顺序可预测）。
fn commit_at(dir: &Path, message: &str, stamp_seconds: i64) {
    git_add_all(dir);
    run_git(dir, &["commit", "-q", "-m", message], stamp_seconds);
}

fn git_add_all(dir: &Path) {
    let output = std::process::Command::new("git")
        .current_dir(dir)
        .args(["add", "-A"])
        .output()
        .expect("git 运行失败");
    assert!(output.status.success(), "git add 失败");
}

/// 带固定时间戳跑一条 git 命令（merge / commit 等会写入提交的）。
fn run_git(dir: &Path, args: &[&str], stamp_seconds: i64) {
    let stamp = format!("{stamp_seconds} +0000");
    let output = std::process::Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_AUTHOR_DATE", &stamp)
        .env("GIT_COMMITTER_DATE", &stamp)
        .output()
        .expect("git 运行失败");
    assert!(output.status.success(), "git {args:?} 失败");
}

fn fixture(label: &str) -> TempDir {
    let dir = TempDir::new(label);
    init_repo(dir.path());
    dir
}

// 六种形状。时间戳全部严格递增（与 walk 的按时间排序无平局），oid 可复现。

fn shape_linear(label: &str) -> TempDir {
    let dir = fixture(label);
    write(dir.path(), "a.txt", b"a\n");
    commit_at(dir.path(), "c1", 1_700_000_000);
    write(dir.path(), "b.txt", b"b\n");
    commit_at(dir.path(), "c2", 1_700_000_060);
    dir
}

fn shape_forked_merge(label: &str) -> TempDir {
    let dir = fixture(label);
    write(dir.path(), "base.txt", b"base\n");
    commit_at(dir.path(), "base", 1_700_000_000);

    write(dir.path(), "a.txt", b"a\n");
    commit_at(dir.path(), "A on main", 1_700_000_060);

    std::process::Command::new("git")
        .current_dir(dir.path())
        .args(["checkout", "-q", "-b", "feature"])
        .output()
        .expect("git 运行失败");
    write(dir.path(), "f.txt", b"f\n");
    commit_at(dir.path(), "F on feature", 1_700_000_120);

    std::process::Command::new("git")
        .current_dir(dir.path())
        .args(["checkout", "-q", "main"])
        .output()
        .expect("git 运行失败");
    write(dir.path(), "b.txt", b"b\n");
    commit_at(dir.path(), "B on main", 1_700_000_180);

    run_git(
        dir.path(),
        &["merge", "--no-ff", "-q", "-m", "M merge feature", "feature"],
        1_700_000_240,
    );
    dir
}

fn shape_octopus(label: &str) -> TempDir {
    let dir = fixture(label);
    write(dir.path(), "base.txt", b"base\n");
    commit_at(dir.path(), "base", 1_700_000_000);

    write(dir.path(), "a.txt", b"a\n");
    commit_at(dir.path(), "A on main", 1_700_000_060);

    // 三条分支各自从 main 的同一提交分出，各带一个提交（时间戳递增）
    for (index, name) in ["b1", "b2", "b3"].iter().enumerate() {
        std::process::Command::new("git")
            .current_dir(dir.path())
            .args(["checkout", "-q", "-b", name])
            .output()
            .expect("git 运行失败");
        write(dir.path(), &format!("{name}.txt"), name.as_bytes());
        commit_at(
            dir.path(),
            &format!("commit on {name}"),
            1_700_000_300 + (index as i64) * 60,
        );
        std::process::Command::new("git")
            .current_dir(dir.path())
            .args(["checkout", "-q", "main"])
            .output()
            .expect("git 运行失败");
    }
    std::process::Command::new("git")
        .current_dir(dir.path())
        .args(["checkout", "-q", "main"])
        .output()
        .expect("git 运行失败");
    // octopus：一次合并三条分支 → 4 个父提交
    run_git(
        dir.path(),
        &["merge", "--no-ff", "-q", "-m", "octopus", "b1", "b2", "b3"],
        1_700_000_480,
    );
    dir
}

fn shape_detached(label: &str) -> TempDir {
    let dir = fixture(label);
    write(dir.path(), "a.txt", b"a\n");
    commit_at(dir.path(), "c1", 1_700_000_000);
    write(dir.path(), "b.txt", b"b\n");
    commit_at(dir.path(), "c2", 1_700_000_060);

    // 游离到 c1，再在上面提交：c3 只从 HEAD 可达，不在任何分支上
    std::process::Command::new("git")
        .current_dir(dir.path())
        .args(["checkout", "-q", "--detach", "HEAD~1"])
        .output()
        .expect("git 运行失败");
    write(dir.path(), "c.txt", b"c\n");
    commit_at(dir.path(), "c3 detached", 1_700_000_120);
    dir
}

fn shape_empty_commit(label: &str) -> TempDir {
    let dir = fixture(label);
    write(dir.path(), "a.txt", b"a\n");
    commit_at(dir.path(), "c1", 1_700_000_000);
    // 空提交：布局器必须照样给它一行、一条指向父的边
    let stamp = format!("{} +0000", 1_700_000_060);
    let output = std::process::Command::new("git")
        .current_dir(dir.path())
        .args(["commit", "-q", "--allow-empty", "-m", "empty"])
        .env("GIT_AUTHOR_DATE", &stamp)
        .env("GIT_COMMITTER_DATE", &stamp)
        .output()
        .expect("git 运行失败");
    assert!(output.status.success(), "空提交失败");
    dir
}

fn shape_duplicate_merge(label: &str) -> TempDir {
    let dir = fixture(label);
    write(dir.path(), "base.txt", b"base\n");
    commit_at(dir.path(), "base", 1_700_000_000);

    write(dir.path(), "a.txt", b"a\n");
    commit_at(dir.path(), "A on main", 1_700_000_060);

    // b1、b2 两条分支名指向**同一个**提交 x：x 这条线被合并两次
    std::process::Command::new("git")
        .current_dir(dir.path())
        .args(["checkout", "-q", "-b", "b1"])
        .output()
        .expect("git 运行失败");
    write(dir.path(), "x.txt", b"x\n");
    commit_at(dir.path(), "X on branch", 1_700_000_120);
    std::process::Command::new("git")
        .current_dir(dir.path())
        .args(["branch", "b2"])
        .output()
        .expect("git 运行失败");

    std::process::Command::new("git")
        .current_dir(dir.path())
        .args(["checkout", "-q", "main"])
        .output()
        .expect("git 运行失败");
    run_git(
        dir.path(),
        &["merge", "--no-ff", "-q", "-m", "merge b1", "b1"],
        1_700_000_180,
    );
    write(dir.path(), "c.txt", b"c\n");
    commit_at(dir.path(), "C on main", 1_700_000_240);
    run_git(
        dir.path(),
        &["merge", "--no-ff", "-q", "-m", "merge b2 (same line)", "b2"],
        1_700_000_300,
    );
    dir
}

// ---------------------------------------------------------------- 断言器

/// 祖先真值：`git rev-list --parents <spec>` 的有序输出（新 → 旧）。
///
/// 这是**独立真值**：不经过我们的任何引擎或布局代码。**必须保序**——
/// 行序断言依赖 rev-list 的时序，用按 oid 排序的容器会把它毁掉。
fn ancestry(dir: &Path, spec: &[&str]) -> Vec<(String, Vec<String>)> {
    let output = std::process::Command::new("git")
        .current_dir(dir)
        .args(["rev-list", "--parents"])
        .args(spec)
        .output()
        .expect("git rev-list 运行失败");
    assert!(output.status.success(), "rev-list 失败");
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .map(|line| {
            let mut parts = line.split_whitespace();
            let oid = parts.next().expect("rev-list 行非空").to_owned();
            (oid, parts.map(str::to_owned).collect())
        })
        .collect()
}

/// 期望边集：窗口内每个提交 → 它的（去重后的）父提交。
///
/// 父提交在窗口外（游离分支的分叉点等）也照算——布局器对窗口外的父
/// 同样发出边（用预留槽位续画），集合语义两边一致。
fn expected_edges(
    ancestry: &[(String, Vec<String>)],
    mode: LayoutMode,
) -> BTreeSet<(String, String)> {
    let mut set = BTreeSet::new();
    for (oid, parents) in ancestry {
        match mode {
            LayoutMode::AllBranches => {
                // 布局器对重复父提交去重（同一条父线只画一条边），
                // 集合语义下去重前后的边集相等
                let mut seen = Vec::new();
                for parent in parents {
                    if !seen.contains(parent) {
                        seen.push(parent.clone());
                    }
                    set.insert((oid.clone(), parent.clone()));
                }
            }
            LayoutMode::FirstParentOnly => {
                if let Some(first) = parents.first() {
                    set.insert((oid.clone(), first.clone()));
                }
            }
        }
    }
    set
}

fn layout_edges(
    label: &str,
    service: &HistoryService<'_>,
    repo_id: i64,
    all_branches: bool,
    mode: LayoutMode,
    ancestry: &[(String, Vec<String>)],
) -> BTreeSet<(String, String)> {
    // 布局模式与查询过滤必须同步（服务层的契约：first_parent_only 同时作用于
    // 查询与布局），FirstParentOnly 的窗口因此也按 first-parent 走
    let first_parent_only = mode == LayoutMode::FirstParentOnly;
    let query = HistoryQuery {
        all_branches,
        first_parent_only,
        page_size: 500,
        ..Default::default()
    };
    let page = service.page(repo_id, &query).expect("取页失败");
    let edges: BTreeSet<(String, String)> = page
        .layout
        .edges
        .iter()
        .map(|edge| (edge.from_oid.clone(), edge.to_oid.clone()))
        .collect();

    // 行覆盖（完整窗口 = AllBranches 模式）：每个提交恰好一行，行序 = rev-list 时序
    if !first_parent_only {
        let rows: Vec<&str> = {
            let mut ordered: Vec<&forgedesk_domain::history::GraphRow> =
                page.layout.rows.iter().collect();
            ordered.sort_by_key(|row| row.row);
            ordered.iter().map(|row| row.oid.as_str()).collect()
        };
        let ancestry_order: Vec<&String> = ancestry.iter().map(|(oid, _)| oid).collect();
        assert_eq!(
            rows.len(),
            ancestry_order.len(),
            "{label}: 行数必须覆盖窗口内全部提交"
        );
        for (row_oid, true_oid) in rows.iter().zip(ancestry_order.iter()) {
            assert_eq!(*row_oid, true_oid.as_str(), "行序应与 rev-list 的时序一致");
        }
    } else {
        // FirstParentOnly 的窗口是全窗口的子序列：行都在真值里，且相对时序保持
        let mut ordered: Vec<&forgedesk_domain::history::GraphRow> =
            page.layout.rows.iter().collect();
        ordered.sort_by_key(|row| row.row);
        let truth_positions: Vec<usize> = ordered
            .iter()
            .map(|row| {
                ancestry
                    .iter()
                    .position(|(oid, _)| *oid == row.oid)
                    .unwrap_or_else(|| panic!("FirstParentOnly 行 {} 不在真值窗口里", row.oid))
            })
            .collect();
        let mut sorted = truth_positions.clone();
        sorted.sort();
        assert_eq!(
            truth_positions, sorted,
            "FirstParentOnly 的行序应保持全窗口的相对时序"
        );
    }

    edges
}

fn assert_shape_matches_ancestry(
    label: &str,
    dir: &TempDir,
    all_branches: bool,
    service: &HistoryService<'_>,
    repo_id: i64,
) {
    for (mode_name, mode) in [
        ("AllBranches", LayoutMode::AllBranches),
        ("FirstParentOnly", LayoutMode::FirstParentOnly),
    ] {
        // 窗口与模式一致：FirstParentOnly 用 rev-list --first-parent 取真值
        let first_parent = mode == LayoutMode::FirstParentOnly;
        let mut spec: Vec<&str> = Vec::new();
        if first_parent {
            spec.push("--first-parent");
        }
        if all_branches {
            // 我们 all_branches 的语义 = 引用集合（heads/remotes/tags）。
            // 刻意**不用 `--all`**：git 的 `--all` 还会附带 HEAD——游离 HEAD
            // 的提交会因此出现在真值里，而 libgit2 的引用枚举推不到它
            //（这条引擎差异记录在 M2 报告里，不在本测试的契约内）
            spec.extend(["--branches", "--tags", "--remotes"]);
        } else {
            spec.push("HEAD");
        }
        let ancestry = ancestry(dir.path(), &spec);
        let expected = expected_edges(&ancestry, mode);
        let actual = layout_edges(label, service, repo_id, all_branches, mode, &ancestry);
        assert_eq!(
            actual, expected,
            "{label} / {mode_name}：布局边集与 git 祖先关系不等价"
        );
    }
}

#[test]
fn layout_edge_sets_are_equivalent_to_git_ancestry_across_all_shapes() {
    // 引擎/数据库只建一次（有状态的重资源，测试内共享）
    let engines: &'static GitEngines =
        Box::leak(Box::new(GitEngines::new().expect("创建引擎失败")));
    let database: &'static Database = Box::leak(Box::new(memory_database()));

    let service_for = |dir: &TempDir, repo_id_out: &mut i64| -> HistoryService<'static> {
        use forgedesk_services::repository::OpenRepoRegistry;
        let open = OpenRepoRegistry::new();
        let repository = RepositoryService::new(engines, RepositoryStore::new(database), &open);
        let opened = repository.open(dir.path()).expect("打开仓库失败");
        *repo_id_out = opened.record_id;
        HistoryService::new(engines, RepositoryStore::new(database))
    };

    let mut repo_id = 0;
    let linear = shape_linear("dag-linear");
    let linear_service = service_for(&linear, &mut repo_id);
    assert_shape_matches_ancestry("linear", &linear, false, &linear_service, repo_id);
    assert_shape_matches_ancestry("linear", &linear, true, &linear_service, repo_id);

    let forked = shape_forked_merge("dag-forked");
    let forked_service = service_for(&forked, &mut repo_id);
    assert_shape_matches_ancestry("forked_merge", &forked, false, &forked_service, repo_id);
    assert_shape_matches_ancestry("forked_merge", &forked, true, &forked_service, repo_id);

    let octopus = shape_octopus("dag-octopus");
    let octopus_service = service_for(&octopus, &mut repo_id);
    assert_shape_matches_ancestry("octopus", &octopus, false, &octopus_service, repo_id);
    assert_shape_matches_ancestry("octopus", &octopus, true, &octopus_service, repo_id);

    let detached = shape_detached("dag-detached");
    let detached_service = service_for(&detached, &mut repo_id);
    assert_shape_matches_ancestry("detached", &detached, false, &detached_service, repo_id);
    assert_shape_matches_ancestry("detached", &detached, true, &detached_service, repo_id);

    let empty = shape_empty_commit("dag-empty");
    let empty_service = service_for(&empty, &mut repo_id);
    assert_shape_matches_ancestry("empty_commit", &empty, false, &empty_service, repo_id);
    assert_shape_matches_ancestry("empty_commit", &empty, true, &empty_service, repo_id);

    let duplicate = shape_duplicate_merge("dag-duplicate");
    let duplicate_service = service_for(&duplicate, &mut repo_id);
    assert_shape_matches_ancestry(
        "duplicate_merge",
        &duplicate,
        false,
        &duplicate_service,
        repo_id,
    );
    assert_shape_matches_ancestry(
        "duplicate_merge",
        &duplicate,
        true,
        &duplicate_service,
        repo_id,
    );
}

#[test]
fn octopus_merge_really_has_four_parents() {
    // 断言器的前提检查：octopus 夹具真的造出了 4 父提交——
    // 如果 git/参数行为变了，上面的等价断言会"空转通过"，这里先拦住
    let dir = shape_octopus("dag-octopus-check");
    let ancestry = ancestry(dir.path(), &["--all"]);
    let four_parent = ancestry
        .iter()
        .find(|(_, parents)| parents.len() == 4)
        .expect("octopus 夹具应有一个 4 父提交");
    assert_eq!(four_parent.1.len(), 4);
}
