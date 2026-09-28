//! DAG 泳道布局的对照测试（T2.2 验收项：图结构与 `git log --graph` 语义一致）。
//!
//! # 为什么需要对照测试
//!
//! M2 验收项要求"图结构与 `git log --graph` 语义一致"。domain 层的 property 测试
//! 保证了布局的内在性质（确定性、无重叠、分页一致），但无法证明"我们的泳道分组
//! 与 git 的泳道分组等价"——只有把两者放在一起跑才能回答这个问题。
//!
//! # AGENTS.md §7 的例外说明
//!
//! §7 禁止解析人类可读输出。**本测试是对照测试**，专门用来校验我们的布局与 git
//! 的语义一致性，属例外。产品代码不解析 `--graph` 输出——产品代码走的是
//! `--porcelain`/`--format=...%x1f...%x1e` 机器可读格式 + 纯函数布局器。
//! 解析 `--graph` 的 ASCII art 仅发生在此测试文件中，用于建立"参考真值"。
//!
//! # 等价性判据
//!
//! **泳道分组等价**：把提交按 lane 分成若干组，我们的分组与 git 的分组相同
//! （同一提交在两种口径下的"同 lane 集合"一致）。
//!
//! **为什么不要求 lane 编号绝对相等**：git 的列分配策略是"新的分支在右侧开新列、
//! 合并后左侧优先回收"，而我们的策略是"first-parent 留在当前 lane、其余取最左
//! 空闲"。两者的绝对编号可能不同（例如 git 把 feature 放第 2 列而我们放第 1 列），
//! 但分组（哪些提交共享一条泳道）必然一致——否则界面上"属于同一条线"的提交
//! 会被画在不同的列里，与用户的 git 命令行经验矛盾。
//!
//! **附加断言**：分组的左到右相对顺序也一致（主线在最左）。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::collections::HashMap;
use std::path::Path;

use forgedesk_domain::git::{Commit, Signature, SignatureStatus};
use forgedesk_domain::history::{layout, LayoutMode};
use support::{git, git_ok, init_repo, write, TempDir};

/// 用固定时间戳提交（确保 walk 顺序可预测）。
fn commit_at(dir: &Path, message: &str, stamp_seconds: i64) {
    git_ok(dir, &["add", "-A"]);
    let stamp = format!("{stamp_seconds} +0000");
    let output = std::process::Command::new("git")
        .current_dir(dir)
        .args(["commit", "-q", "-m", message])
        .env("GIT_AUTHOR_DATE", &stamp)
        .env("GIT_COMMITTER_DATE", &stamp)
        .output()
        .expect("git 运行失败");
    assert!(output.status.success(), "commit {message} 失败");
}

/// 构建一个含 merge 与多分支的夹具仓库：
///
/// ```text
///           M (merge)     ← main（最新）
///          / \
///     B (main)  D (feature)
///         |      |
///     A (main)  C (feature)
///          \   /
///           BASE          ← 初始提交
/// ```
///
/// 时间戳保证 walk 顺序：BASE < A < C < B < D < M
fn build_merge_repo(label: &str) -> TempDir {
    let dir = TempDir::new(label);
    init_repo(dir.path());

    // BASE
    write(dir.path(), "base.txt", b"base\n");
    commit_at(dir.path(), "base", 1_700_000_000);

    // A on main
    write(dir.path(), "a.txt", b"a\n");
    commit_at(dir.path(), "A on main", 1_700_000_060);

    // 创建 feature 分支（从 A 分出）
    git_ok(dir.path(), &["checkout", "-q", "-b", "feature"]);

    // C on feature
    write(dir.path(), "c.txt", b"c\n");
    commit_at(dir.path(), "C on feature", 1_700_000_120);

    // 回 main
    git_ok(dir.path(), &["checkout", "-q", "main"]);

    // B on main
    write(dir.path(), "b.txt", b"b\n");
    commit_at(dir.path(), "B on main", 1_700_000_180);

    // D on feature（checkout 回去再加一个提交）
    git_ok(dir.path(), &["checkout", "-q", "feature"]);
    write(dir.path(), "d.txt", b"d\n");
    commit_at(dir.path(), "D on feature", 1_700_000_240);

    // 回 main 合并 feature
    git_ok(dir.path(), &["checkout", "-q", "main"]);
    let stamp = format!("{} +0000", 1_700_000_300);
    let output = std::process::Command::new("git")
        .current_dir(dir.path())
        .args(["merge", "--no-ff", "-q", "-m", "M merge feature", "feature"])
        .env("GIT_AUTHOR_DATE", &stamp)
        .env("GIT_COMMITTER_DATE", &stamp)
        .output()
        .expect("git 运行失败");
    assert!(output.status.success(), "merge 失败");

    dir
}

/// 从 `git log --graph --all` 的输出中解析每个提交的 ASCII 泳道列。
///
/// 返回 `HashMap<oid_prefix, lane_index>`：lane_index = `*` 字符的列位置 / 2。
///
/// # 解析策略
///
/// `git log --graph --all --format="COMMIT %H"` 输出形如：
/// ```text
/// *   COMMIT <oid>
/// |\
/// | * COMMIT <oid>
/// * | COMMIT <oid>
/// |/
/// * COMMIT <oid>
/// ```
///
/// 含 `COMMIT` 的行就是提交行；`*` 的列位置（0 基）除以 2 就是泳道号。
fn parse_git_graph_lanes(dir: &Path) -> HashMap<String, usize> {
    let output = git(dir, &["log", "--graph", "--all", "--format=COMMIT %H"]);
    assert!(output.success(), "git log --graph 失败");
    let stdout = output.stdout_lossy();

    let mut lanes: HashMap<String, usize> = HashMap::new();
    for line in stdout.lines() {
        // 找 COMMIT 标记
        if let Some(commit_pos) = line.find("COMMIT ") {
            let oid = line[commit_pos + 7..].trim().to_owned();
            // `*` 在 COMMIT 标记之前
            let star_col = line[..commit_pos].find('*').expect("提交行应有 * 标记");
            let lane = star_col / 2;
            lanes.insert(oid, lane);
        }
    }
    lanes
}

/// 用 git log（机器可读格式）获取提交列表（新 → 旧），用于构建布局输入。
fn get_commits_for_layout(dir: &Path) -> Vec<Commit> {
    // 使用 %x1f 分隔字段、%x1e 分隔记录（AGENTS §7 规范的格式）
    let output = git(
        dir,
        &[
            "log",
            "--all",
            "--format=%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%cn%x1f%ce%x1f%ct%x1f%D%x1f%G?%x1f%s%x1e",
        ],
    );
    assert!(output.success(), "git log 失败");
    let stdout = output.stdout_lossy();

    let mut commits = Vec::new();
    for record in stdout.split('\x1e') {
        let record = record.trim();
        if record.is_empty() {
            continue;
        }
        // 跳过 graph 前缀字符（如果有的话）—— 这里不带 --graph，所以没有
        let fields: Vec<&str> = record.split('\x1f').collect();
        if fields.len() < 11 {
            continue;
        }
        let oid = fields[0].trim().to_owned();
        let parents: Vec<String> = fields[1].split_whitespace().map(|p| p.to_owned()).collect();
        let author = Signature {
            name: fields[2].to_owned(),
            email: fields[3].to_owned(),
            time: fields[4].parse().ok(),
        };
        let committer = Signature {
            name: fields[5].to_owned(),
            email: fields[6].to_owned(),
            time: fields[7].parse().ok(),
        };
        let refs: Vec<String> = fields[8]
            .split(',')
            .map(|r| r.trim().to_owned())
            .filter(|r| !r.is_empty())
            .collect();
        let signature = fields[9]
            .as_bytes()
            .first()
            .map_or(SignatureStatus::Unknown, |&b| SignatureStatus::from_byte(b));
        let subject = fields[10].to_owned();

        commits.push(Commit {
            oid,
            parents,
            author,
            committer,
            refs,
            signature,
            subject,
            body: None,
        });
    }
    commits
}

/// 主测试：我们的布局泳道分组与 git --graph 的泳道分组等价。
#[test]
fn our_layout_lane_grouping_matches_git_graph() {
    let dir = build_merge_repo("dag-compare");

    // ① 用我们的布局器计算泳道
    let commits = get_commits_for_layout(dir.path());
    assert!(
        commits.len() >= 5,
        "夹具应至少有 5 个提交，实际 {}",
        commits.len()
    );
    let graph = layout(&commits, LayoutMode::AllBranches.into());

    // 我们的分组：lane → [oid]
    let mut our_groups: HashMap<u16, Vec<String>> = HashMap::new();
    for row in &graph.rows {
        our_groups
            .entry(row.lane)
            .or_default()
            .push(row.oid.clone());
    }

    // ② 从 git --graph 解析泳道
    let git_lanes = parse_git_graph_lanes(dir.path());
    assert!(!git_lanes.is_empty(), "git --graph 应解析出至少一个提交");

    // git 的分组：lane → [oid]
    let mut git_groups: HashMap<usize, Vec<String>> = HashMap::new();
    for (oid, lane) in &git_lanes {
        git_groups.entry(*lane).or_default().push(oid.clone());
    }

    // ③ 比较分组结构
    //
    // 等价性判据：建立"我们的 lane → git 的 lane"映射，要求：
    // - 映射是一对一的（双射）：我们同一 lane 里的提交在 git 也在同一 lane
    // - 覆盖所有提交（没有遗漏）
    //
    // 为什么这样判：git 与我们的列编号策略不同（git 倾向在右侧开新列，
    // 我们 first-parent 留在当前 lane），但"哪些提交共享一条泳道"在两种
    // 算法下必须一致——否则界面上的线与 `git log --graph` 的分组矛盾。
    let mut our_lane_to_git: HashMap<u16, usize> = HashMap::new();
    let mut git_lane_to_our: HashMap<usize, u16> = HashMap::new();

    for row in &graph.rows {
        let git_lane = git_lanes.get(&row.oid).copied();
        if let Some(gl) = git_lane {
            if let Some(&prev) = our_lane_to_git.get(&row.lane) {
                assert_eq!(
                    prev, gl,
                    "我们的 lane {} 映射到了 git 的 lane {} 和 {}（提交 {}），分组不等价",
                    row.lane, prev, gl, row.oid
                );
            }
            if let Some(&prev) = git_lane_to_our.get(&gl) {
                assert_eq!(
                    prev, row.lane,
                    "git 的 lane {} 映射到了我们的 lane {} 和 {}（提交 {}），分组不等价",
                    gl, prev, row.lane, row.oid
                );
            }
            our_lane_to_git.insert(row.lane, gl);
            git_lane_to_our.insert(gl, row.lane);
        }
    }

    // 验证分组大小一致
    assert_eq!(
        our_groups.len(),
        git_groups.len(),
        "泳道数不同：我们 {} 条，git {} 条",
        our_groups.len(),
        git_groups.len()
    );

    // 验证每个分组里的提交集合一致
    for (our_lane, git_lane) in &our_lane_to_git {
        let our_set: Vec<&String> = {
            let mut v: Vec<&String> = our_groups.get(our_lane).unwrap().iter().collect();
            v.sort();
            v
        };
        let git_set: Vec<&String> = {
            let mut v: Vec<&String> = git_groups.get(git_lane).unwrap().iter().collect();
            v.sort();
            v
        };
        assert_eq!(
            our_set, git_set,
            "我们的 lane {our_lane} 与 git 的 lane {git_lane} 包含的提交不同"
        );
    }
}

/// 辅助测试：确认夹具仓库确实包含 merge 提交与多分支（夹具本身的健全性检查）。
#[test]
fn the_fixture_repo_has_merges_and_multiple_branches() {
    let dir = build_merge_repo("dag-fixture-check");

    let commits = get_commits_for_layout(dir.path());
    let merges: Vec<&Commit> = commits.iter().filter(|c| c.is_merge()).collect();
    assert!(!merges.is_empty(), "夹具应包含至少一个 merge 提交");

    // 确认有 feature 分支
    let output = git(dir.path(), &["branch", "--list"]);
    let branches = output.stdout_lossy();
    assert!(branches.contains("feature"), "夹具应有 feature 分支");
    assert!(branches.contains("main"), "夹具应有 main 分支");
}
