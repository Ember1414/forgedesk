//! 双实现差分一致性测试：`CliGitEngine` vs `Libgit2Engine`。
//!
//! # 为什么必须做差分测试
//!
//! 两个引擎会同时存在于产品里（读走 libgit2、写走 CLI），因此**同一份状态
//! 经两条路径必须得到同一个结论**。而它们的差异恰恰藏在最不容易被发现的地方：
//! 重命名检测阈值、`core.autocrlf`、子模块、空文件、二进制判定。
//! 这些差异不会报错，只会让界面时而显示 A、时而显示 B。
//!
//! # "一致"的规范化规则
//!
//! 两个引擎的信息量**本来就不一样**（libgit2 的状态 API 不暴露文件模式与 oid、
//! 不做 GPG 校验、没有 `%D` 等价物）。因此这里不是"逐字段相等"，而是
//! "在两者都能给出的语义上相等"，规则如下（同时登记在 `docs/GIT-ENGINE-DIFF.md`）：
//!
//! 1. **状态**：比较 `(路径, 标记, 来源路径)` 的集合。未跟踪统一记为 `??`
//!    （porcelain 用 `?`，libgit2 用 `WT_NEW` 位）；冲突统一记为 `conflicted`
//!    （porcelain 能区分 `UU`/`AA`/`DU`，libgit2 只有 `CONFLICTED` 位）。
//! 2. **diff**：比较 `(路径, 来源路径, 新增行, 删除行, 是否二进制)` 的集合。
//! 3. **log**：比较 oid 序列，以及每个 oid 的父提交、subject、提交时间。
//!    `signature` 不在比较范围内（见上）。`refs` 在 log 里同样不比，但
//!    `show` 侧逐提交比较排序后的 token 序列（详见
//!    `commit_detail_inputs_are_consistent_across_engines`）。
//!
//! # 夹具的确定性
//!
//! 所有提交的时间戳逐个递增、`core.autocrlf=false`、身份固定——否则
//! 同一秒内的多个提交会因两个引擎的排序平局规则不同而产生假阳性。
//!
//! 集成测试是独立 crate，测试代码里允许 panic 式写法（失败即断言失败）。
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::collections::BTreeSet;
use std::path::Path;

use forgedesk_domain::git::{
    Commit, CommitSpec, DiffReport, DiffSpec, DiffTarget, EntryKind, LogQuery, Page, RepoId,
    RepoPath, StageSpec, StatusQuery, StatusReport,
};
use forgedesk_git_engine::engine::{CliGitEngine, GitEngine, Libgit2Engine, ProgressSink};
use support::{
    commit_all, commit_all_with_author, git_ok, git_with_env, init_repo, write, TempDir,
};

/// 两个引擎。
fn engines() -> (CliGitEngine, Libgit2Engine) {
    (
        CliGitEngine::new().expect("创建 CLI 引擎失败"),
        Libgit2Engine::new(),
    )
}

// ---------------------------------------------------------------- 规范化

/// 规范化后的状态条目。
type NormStatus = BTreeSet<(String, String, String)>;

fn normalize_status(report: &StatusReport) -> NormStatus {
    report
        .entries
        .iter()
        .map(|entry| {
            let marker = match entry.kind {
                EntryKind::Untracked => "??".to_owned(),
                EntryKind::Ignored => "!!".to_owned(),
                EntryKind::Unmerged => "conflicted".to_owned(),
                EntryKind::Ordinary | EntryKind::RenamedOrCopied => format!(
                    "{}{}",
                    entry.index_status.as_char(),
                    entry.worktree_status.as_char()
                ),
            };
            (
                entry.path.to_string(),
                marker,
                entry
                    .original_path
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_default(),
            )
        })
        .collect()
}

/// 规范化后的 diff 条目。
type NormDiff = BTreeSet<(String, String, u64, u64, bool)>;

fn normalize_diff(report: &DiffReport) -> NormDiff {
    report
        .files
        .iter()
        .map(|file| {
            (
                file.path.to_string(),
                file.original_path
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_default(),
                file.additions,
                file.deletions,
                file.binary,
            )
        })
        .collect()
}

/// 规范化后的提交：`(oid, 父提交, subject, 提交时间)`。
type NormLog = Vec<(String, Vec<String>, String, Option<i64>)>;

fn normalize_log(page: &Page<Commit>) -> NormLog {
    page.items
        .iter()
        .map(|commit| {
            (
                commit.oid.clone(),
                commit.parents.clone(),
                commit.subject.clone(),
                commit.committer.time,
            )
        })
        .collect()
}

// ---------------------------------------------------------------- 对比

fn compare_status(cli: &CliGitEngine, libgit2: &Libgit2Engine, repo: &RepoId, label: &str) {
    let from_cli = normalize_status(
        &cli.status(repo, &StatusQuery::default())
            .expect("CLI status 失败"),
    );
    let from_libgit2 = normalize_status(
        &libgit2
            .status(repo, &StatusQuery::default())
            .expect("libgit2 status 失败"),
    );

    assert_eq!(
        from_cli, from_libgit2,
        "[{label}] 两个引擎的状态不一致\nCLI:     {from_cli:?}\nlibgit2: {from_libgit2:?}"
    );
}

fn compare_diff(
    cli: &CliGitEngine,
    libgit2: &Libgit2Engine,
    repo: &RepoId,
    target: DiffTarget,
    label: &str,
) {
    let spec = DiffSpec::new(target.clone());
    let from_cli = normalize_diff(&cli.diff(repo, spec.clone()).expect("CLI diff 失败"));
    let from_libgit2 = normalize_diff(&libgit2.diff(repo, spec).expect("libgit2 diff 失败"));

    assert_eq!(
        from_cli, from_libgit2,
        "[{label}] 两个引擎的 diff（{target:?}）不一致\nCLI:     {from_cli:?}\nlibgit2: {from_libgit2:?}"
    );
}

fn compare_log(cli: &CliGitEngine, libgit2: &Libgit2Engine, repo: &RepoId, label: &str) {
    let query = LogQuery::new().with_limit(100);
    let from_cli = normalize_log(&cli.log(repo, query.clone()).expect("CLI log 失败"));
    let from_libgit2 = normalize_log(&libgit2.log(repo, query).expect("libgit2 log 失败"));

    assert_eq!(
        from_cli, from_libgit2,
        "[{label}] 两个引擎的 log 不一致\nCLI:     {from_cli:?}\nlibgit2: {from_libgit2:?}"
    );
}

/// 对一份仓库跑完三项对比。
fn compare_all(cli: &CliGitEngine, libgit2: &Libgit2Engine, root: &Path, label: &str) {
    let repo = RepoId::new(root);

    compare_status(cli, libgit2, &repo, label);
    compare_log(cli, libgit2, &repo, label);
    compare_diff(cli, libgit2, &repo, DiffTarget::Staged, label);
    compare_diff(cli, libgit2, &repo, DiffTarget::Unstaged, label);
}

// ---------------------------------------------------------------- 六类仓库

/// ① 简单线性历史。
fn shape_linear(dir: &Path) {
    init_repo(dir);
    for (sequence, name) in [(1_u32, "a.txt"), (2, "b.txt"), (3, "c.txt")] {
        write(dir, name, format!("{name}\n").as_bytes());
        commit_all(dir, &format!("add {name}"), sequence);
    }
    // 制造工作区与索引的差异
    write(dir, "a.txt", b"a.txt\nchanged\n");
    write(dir, "untracked.txt", b"u\n");
}

/// ② 多分叉 + 合并。
fn shape_forked(dir: &Path) {
    init_repo(dir);
    write(dir, "base.txt", b"base\n");
    commit_all(dir, "base", 1);

    git_ok(dir, &["checkout", "-q", "-b", "feature"]);
    write(dir, "feature.txt", b"feature\n");
    commit_all(dir, "feature work", 2);

    git_ok(dir, &["checkout", "-q", "main"]);
    write(dir, "main.txt", b"main\n");
    commit_all(dir, "main work", 3);

    git_ok(
        dir,
        &["merge", "-q", "--no-ff", "-m", "merge feature", "feature"],
    );
}

/// ③ 重命名 + 删除。
fn shape_rename_delete(dir: &Path) {
    init_repo(dir);
    write(
        dir,
        "old.txt",
        b"one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\n",
    );
    write(dir, "doomed.txt", b"bye\n");
    commit_all(dir, "base", 1);

    std::fs::rename(dir.join("old.txt"), dir.join("new.txt")).unwrap();
    std::fs::remove_file(dir.join("doomed.txt")).unwrap();
    git_ok(dir, &["add", "--all"]);
}

/// ④ 二进制文件。
fn shape_binary(dir: &Path) {
    init_repo(dir);
    write(dir, "text.txt", b"one\n");
    commit_all(dir, "base", 1);

    write(dir, "bin.dat", &[0x00, 0x01, 0x02, 0xFF, 0xFE, 0x00]);
    write(dir, "text.txt", b"one\ntwo\n");
    git_ok(dir, &["add", "--all"]);
}

/// ⑤ 子模块。
fn shape_submodule(dir: &Path) {
    let source = dir.join("subsource");
    std::fs::create_dir_all(&source).unwrap();
    init_repo(&source);
    write(&source, "s.txt", b"sub\n");
    commit_all(&source, "sub initial", 1);

    init_repo(dir);
    write(dir, "a.txt", b"one\n");
    commit_all(dir, "base", 2);

    let url = source.to_string_lossy().replace('\\', "/");
    git_ok(
        dir,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "-q",
            &url,
            "vendor/sub",
        ],
    );
    commit_all(dir, "add submodule", 3);

    // 子模块工作区变脏
    write(&dir.join("vendor/sub"), "s.txt", b"sub\ndirty\n");
    write(&dir.join("vendor/sub"), "untracked.txt", b"u\n");
}

/// ⑥ 大量文件（1000+）。
fn shape_many_files(dir: &Path) {
    init_repo(dir);
    for index in 0..1000 {
        write(
            dir,
            &format!("many/dir{}/file{index:04}.txt", index % 10),
            b"content\n",
        );
    }
    commit_all(dir, "bulk", 1);

    for index in 0..200 {
        write(
            dir,
            &format!("many/dir{}/file{index:04}.txt", index % 10),
            b"content\nchanged\n",
        );
    }
    git_ok(dir, &["add", "--all"]);
}

// ---------------------------------------------------------------- 测试

#[test]
fn linear_history_is_consistent_across_engines() {
    let dir = TempDir::new("diff-linear");
    shape_linear(dir.path());
    let (cli, libgit2) = engines();

    compare_all(&cli, &libgit2, dir.path(), "linear");
}

#[test]
fn forked_history_with_merge_is_consistent_across_engines() {
    let dir = TempDir::new("diff-forked");
    shape_forked(dir.path());
    let (cli, libgit2) = engines();

    compare_all(&cli, &libgit2, dir.path(), "forked");
}

#[test]
fn renames_and_deletions_are_consistent_across_engines() {
    let dir = TempDir::new("diff-rename");
    shape_rename_delete(dir.path());
    let (cli, libgit2) = engines();

    compare_all(&cli, &libgit2, dir.path(), "rename+delete");
}

#[test]
fn binary_files_are_consistent_across_engines() {
    let dir = TempDir::new("diff-binary");
    shape_binary(dir.path());
    let (cli, libgit2) = engines();

    compare_all(&cli, &libgit2, dir.path(), "binary");
}

#[test]
fn submodules_are_consistent_across_engines() {
    let dir = TempDir::new("diff-submodule");
    shape_submodule(dir.path());
    let (cli, libgit2) = engines();
    let repo = RepoId::new(dir.path());

    // 状态与历史一致；**diff 的行数统计不一致**，因此不在这里比较，
    // 而是由下面那条 `#[ignore]` 的测试把差异钉住（见 docs/GIT-ENGINE-DIFF.md §3）
    compare_status(&cli, &libgit2, &repo, "submodule");
    compare_log(&cli, &libgit2, &repo, "submodule");
}

#[test]
#[ignore = "已知差异：gitlink（子模块）的增删行数两个引擎不同，见 docs/GIT-ENGINE-DIFF.md §3"]
fn submodule_diff_line_counts_differ_between_engines() {
    let dir = TempDir::new("diff-submodule-diff");
    shape_submodule(dir.path());
    let (cli, libgit2) = engines();
    let repo = RepoId::new(dir.path());

    let from_cli = normalize_diff(
        &cli.diff(&repo, DiffSpec::new(DiffTarget::Unstaged))
            .expect("CLI diff 失败"),
    );
    let from_libgit2 = normalize_diff(
        &libgit2
            .diff(&repo, DiffSpec::new(DiffTarget::Unstaged))
            .expect("libgit2 diff 失败"),
    );

    assert_eq!(
        from_cli, from_libgit2,
        "两个引擎对子模块的行数统计仍然不同（这是被记录的已知差异）"
    );
}

#[test]
fn a_thousand_files_are_consistent_across_engines() {
    let dir = TempDir::new("diff-many");
    shape_many_files(dir.path());
    let (cli, libgit2) = engines();

    compare_all(&cli, &libgit2, dir.path(), "many-files");
}

#[test]
fn an_empty_repository_is_consistent_across_engines() {
    let dir = TempDir::new("diff-empty");
    init_repo(dir.path());
    let (cli, libgit2) = engines();

    compare_all(&cli, &libgit2, dir.path(), "empty");
}

// ------------------------------------------------- LogQuery 过滤器（T2.1）

/// 在指定时间戳上提交（`commit_all` 的日期只有"一分钟内的秒序号"一个维度，
/// 跨时段的夹具需要完整的日期字符串）。日期必须彼此不同：同秒提交在两个
/// 引擎里的排序平局规则不同（见文件头"夹具的确定性"）。
fn commit_all_at(dir: &Path, message: &str, date: &str) {
    git_ok(dir, &["add", "--all"]);
    let output = git_with_env(
        dir,
        &["commit", "-q", "-m", message],
        &[("GIT_AUTHOR_DATE", date), ("GIT_COMMITTER_DATE", date)],
    );
    assert!(output.success(), "commit 失败: {}", output.stderr_lossy());
}

/// 带正文的提交（`commit_all` 只能传单行 message，而 `message_contains`
/// 的语义是"全文含正文"，需要正文命中的样本来钉住它）。
fn commit_all_with_body(dir: &Path, subject: &str, body: &str, sequence: u32) {
    git_ok(dir, &["add", "--all"]);
    let date = format!("2024-01-02T03:04:{sequence:02}+00:00");
    let output = git_with_env(
        dir,
        &["commit", "-q", "-m", subject, "-m", body],
        &[
            ("GIT_AUTHOR_DATE", date.as_str()),
            ("GIT_COMMITTER_DATE", date.as_str()),
        ],
    );
    assert!(output.success(), "commit 失败: {}", output.stderr_lossy());
}

/// 重命名**已提交**的仓库。`shape_rename_delete` 只把重命名留在索引里，
/// 历史上没有 new.txt，`--follow` 无事可做。
fn shape_committed_rename(dir: &Path) {
    init_repo(dir);
    write(
        dir,
        "old.txt",
        b"one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\n",
    );
    commit_all(dir, "add old.txt", 1);

    std::fs::rename(dir.join("old.txt"), dir.join("new.txt")).unwrap();
    git_ok(dir, &["add", "--all"]);
    commit_all(dir, "rename old.txt to new.txt", 2);

    write(
        dir,
        "new.txt",
        b"one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\n",
    );
    commit_all(dir, "edit new.txt", 3);
}

/// 多分支 + merge + 一条**未合并**的旁支。`--all` 相对 HEAD 遍历的增量只有
/// 旁支可见（merge 会把 feature 分支带回 HEAD 历史，体现不出 `--all`）。
/// 全部日期固定且严格递增，排序没有平局。
fn shape_all_branches(dir: &Path) {
    init_repo(dir);
    write(dir, "base.txt", b"base\n");
    commit_all(dir, "base", 1);

    git_ok(dir, &["checkout", "-q", "-b", "feature"]);
    write(dir, "feature.txt", b"feature\n");
    commit_all(dir, "feature work", 2);

    git_ok(dir, &["checkout", "-q", "main"]);
    write(dir, "main.txt", b"main\n");
    commit_all(dir, "main work", 3);

    // merge 的提交日期也要固定：默认取"现在"会让夹具依赖测试运行的时刻
    let merge_date = "2024-01-02T03:04:05+00:00";
    let output = git_with_env(
        dir,
        &["merge", "-q", "--no-ff", "-m", "merge feature", "feature"],
        &[
            ("GIT_AUTHOR_DATE", merge_date),
            ("GIT_COMMITTER_DATE", merge_date),
        ],
    );
    assert!(output.success(), "merge 失败: {}", output.stderr_lossy());

    // 未合并的旁支从 merge 前的 main 长出来，日期排在 merge 之前
    git_ok(dir, &["checkout", "-q", "-b", "side", "main~1"]);
    write(dir, "side.txt", b"side\n");
    commit_all(dir, "side work", 4);
    git_ok(dir, &["checkout", "-q", "main"]);
}

#[test]
fn log_message_contains_is_consistent_across_engines() {
    let dir = TempDir::new("diff-msg-contains");
    init_repo(dir.path());
    write(dir.path(), "base.txt", b"base\n");
    commit_all(dir.path(), "base setup", 1);
    write(dir.path(), "readme.md", b"# ReadmePipeline\n");
    commit_all(dir.path(), "Add ReadmePipeline", 2);
    write(dir.path(), "build.txt", b"build\n");
    commit_all_with_body(
        dir.path(),
        "tweak build script",
        "Refs ReadmePipeline for details",
        3,
    );
    let (cli, libgit2) = engines();
    let repo = RepoId::new(dir.path());

    // 命中词：subject 与正文里的出现都算数（--grep 是全文匹配）
    let hit = LogQuery {
        message_contains: Some("ReadmePipeline".to_owned()),
        ..LogQuery::new().with_limit(100)
    };
    let from_cli = normalize_log(&cli.log(&repo, hit.clone()).expect("CLI log 失败"));
    let from_libgit2 = normalize_log(&libgit2.log(&repo, hit).expect("libgit2 log 失败"));
    assert_eq!(
        from_cli, from_libgit2,
        "[message_contains] 命中查询两侧不一致\nCLI:     {from_cli:?}\nlibgit2: {from_libgit2:?}"
    );
    assert_eq!(from_cli.len(), 2, "subject 与正文中的命中都应算数");
    assert!(
        !from_cli
            .iter()
            .any(|(_, _, subject, _)| subject == "base setup"),
        "没有命中的提交必须被过滤掉"
    );

    // 大小写不同的变体：两侧都必须零命中（语义是区分大小写）
    let miss = LogQuery {
        message_contains: Some("readmepipeline".to_owned()),
        ..LogQuery::new().with_limit(100)
    };
    let from_cli = normalize_log(&cli.log(&repo, miss.clone()).expect("CLI log 失败"));
    let from_libgit2 = normalize_log(&libgit2.log(&repo, miss).expect("libgit2 log 失败"));
    assert_eq!(
        from_cli, from_libgit2,
        "[message_contains] 大小写变体两侧不一致\nCLI:     {from_cli:?}\nlibgit2: {from_libgit2:?}"
    );
    assert!(
        from_cli.is_empty(),
        "message_contains 区分大小写，全小写变体不应命中：{from_cli:?}"
    );
}

#[test]
fn log_since_and_until_bounds_are_inclusive_and_consistent_across_engines() {
    let dir = TempDir::new("diff-since-until");
    init_repo(dir.path());
    for (name, date) in [
        ("a.txt", "2024-01-02T01:00:00+00:00"),
        ("b.txt", "2024-01-02T06:00:00+00:00"),
        ("c.txt", "2024-01-02T12:00:00+00:00"),
        ("d.txt", "2024-01-02T18:00:00+00:00"),
        ("e.txt", "2024-01-03T00:00:00+00:00"),
    ] {
        write(dir.path(), name, format!("{name}\n").as_bytes());
        commit_all_at(dir.path(), &format!("add {name}"), date);
    }
    let (cli, libgit2) = engines();
    let repo = RepoId::new(dir.path());

    // 先不带过滤器读一遍：拿到 oid↔时间的对应关系，也自检夹具日期严格递增
    let all = cli
        .log(&repo, LogQuery::new().with_limit(100))
        .expect("CLI log 失败");
    let items = &all.items;
    assert_eq!(items.len(), 5, "夹具应有 5 个提交");
    for pair in items.windows(2) {
        assert!(
            pair[0].committer.time.unwrap() > pair[1].committer.time.unwrap(),
            "夹具日期必须严格递增（此处按新→旧遍历），否则排序有平局"
        );
    }
    // 线性历史、新→旧：items[0] 最新、items[4] 最旧
    let newest = &items[0];
    let until_commit = &items[1]; // == until 的边界提交
    let since_commit = &items[3]; // == since 的边界提交
    let oldest = &items[4];

    let window = LogQuery {
        since: Some(since_commit.committer.time.unwrap()),
        until: Some(until_commit.committer.time.unwrap()),
        ..LogQuery::new().with_limit(100)
    };
    let from_cli = normalize_log(&cli.log(&repo, window.clone()).expect("CLI log 失败"));
    let from_libgit2 = normalize_log(&libgit2.log(&repo, window).expect("libgit2 log 失败"));
    assert_eq!(
        from_cli, from_libgit2,
        "[since/until] 窗口查询两侧不一致\nCLI:     {from_cli:?}\nlibgit2: {from_libgit2:?}"
    );

    // 闭区间：恰好落在边界上的两个提交都必须出现，窗口外的两个必须缺席
    assert_eq!(from_cli.len(), 3, "窗口应恰好框住 3 个提交");
    assert!(
        from_cli.iter().any(|(oid, ..)| oid == &until_commit.oid),
        "== until 的提交必须包含（闭区间）"
    );
    assert!(
        from_cli.iter().any(|(oid, ..)| oid == &since_commit.oid),
        "== since 的提交必须包含（闭区间）"
    );
    assert!(!from_cli.iter().any(|(oid, ..)| oid == &newest.oid));
    assert!(!from_cli.iter().any(|(oid, ..)| oid == &oldest.oid));
}

#[test]
fn log_first_parent_only_is_consistent_across_engines() {
    let dir = TempDir::new("diff-first-parent");
    shape_forked(dir.path());
    let (cli, libgit2) = engines();
    let repo = RepoId::new(dir.path());

    // 基线：不裁剪时 merge 把 feature 分支的提交也带进 HEAD 历史
    let plain = LogQuery::new().with_limit(100);
    let plain_from_cli = normalize_log(&cli.log(&repo, plain.clone()).expect("CLI log 失败"));
    let plain_from_libgit2 = normalize_log(&libgit2.log(&repo, plain).expect("libgit2 log 失败"));
    assert_eq!(
        plain_from_cli, plain_from_libgit2,
        "[first_parent] 基线（不裁剪）两侧就不一致"
    );
    assert_eq!(plain_from_cli.len(), 4, "shape_forked 应有 4 个提交");
    let feature_oid = plain_from_cli
        .iter()
        .find(|(_, _, subject, _)| subject == "feature work")
        .map(|(oid, ..)| oid.clone())
        .expect("夹具缺少 feature work 提交");

    let trimmed = LogQuery {
        first_parent_only: true,
        ..LogQuery::new().with_limit(100)
    };
    let from_cli = normalize_log(&cli.log(&repo, trimmed.clone()).expect("CLI log 失败"));
    let from_libgit2 = normalize_log(&libgit2.log(&repo, trimmed).expect("libgit2 log 失败"));
    assert_eq!(
        from_cli, from_libgit2,
        "[first_parent] 裁剪后两侧不一致\nCLI:     {from_cli:?}\nlibgit2: {from_libgit2:?}"
    );

    // 裁剪必须真的改变了序列，且只留下 merge → main work → base
    assert_ne!(from_cli, plain_from_cli, "first-parent 必须真的改变序列");
    assert_eq!(
        from_cli.len(),
        3,
        "first-parent 链应为 merge → main work → base"
    );
    assert!(
        !from_cli.iter().any(|(oid, ..)| oid == &feature_oid),
        "支线提交不得出现在 first-parent 链上"
    );
}

#[test]
fn log_all_branches_is_consistent_across_engines() {
    let dir = TempDir::new("diff-all-branches");
    shape_all_branches(dir.path());
    let (cli, libgit2) = engines();
    let repo = RepoId::new(dir.path());

    let all_query = LogQuery {
        all_branches: true,
        ..LogQuery::new().with_limit(100)
    };
    let from_cli = normalize_log(&cli.log(&repo, all_query.clone()).expect("CLI log 失败"));
    let from_libgit2 = normalize_log(&libgit2.log(&repo, all_query).expect("libgit2 log 失败"));
    assert_eq!(
        from_cli, from_libgit2,
        "[all_branches] --all 两侧不一致\nCLI:     {from_cli:?}\nlibgit2: {from_libgit2:?}"
    );

    // 夹具自检：`--all` 必须真的比 HEAD 遍历多出未合并的旁支，
    // 否则这个测试退化成"两边都没看到 side work"的空对比
    let head_only = LogQuery::new().with_limit(100);
    let head_from_cli = normalize_log(&cli.log(&repo, head_only.clone()).expect("CLI log 失败"));
    let head_from_libgit2 =
        normalize_log(&libgit2.log(&repo, head_only).expect("libgit2 log 失败"));
    assert_eq!(
        head_from_cli, head_from_libgit2,
        "HEAD 遍历的基线两侧不一致"
    );
    assert!(
        from_cli.len() > head_from_cli.len(),
        "--all 必须比 HEAD 遍历多出旁支提交，夹具可能坏了"
    );
    assert!(
        from_cli
            .iter()
            .any(|(_, _, subject, _)| subject == "side work"),
        "--all 应能看到未合并的 side work"
    );
    assert!(
        !head_from_cli
            .iter()
            .any(|(_, _, subject, _)| subject == "side work"),
        "HEAD 遍历不应看到未合并的 side work"
    );
}

#[test]
fn log_follow_renames_is_unsupported_by_libgit2() {
    let dir = TempDir::new("diff-follow");
    shape_committed_rename(dir.path());
    let (cli, libgit2) = engines();
    let repo = RepoId::new(dir.path());

    // 对照组：不带 --follow 时，重命名前的历史（old.txt 的诞生）不可见
    let plain = LogQuery {
        paths: vec![RepoPath::from("new.txt")],
        ..LogQuery::new().with_limit(100)
    };
    let plain_from_cli = cli.log(&repo, plain).expect("CLI log 失败");
    assert_eq!(
        plain_from_cli.items.len(),
        2,
        "new.txt 的直接历史只有 rename 与 edit 两个提交"
    );

    let follow = LogQuery {
        paths: vec![RepoPath::from("new.txt")],
        follow_renames: true,
        ..LogQuery::new().with_limit(100)
    };

    // CLI 正常返回，并把重命名前的历史也带了回来（--follow 的存在意义）
    let from_cli = cli
        .log(&repo, follow.clone())
        .expect("CLI log --follow 失败");
    assert!(!from_cli.items.is_empty(), "--follow 必须返回非空历史");
    assert_eq!(
        from_cli.items.len(),
        3,
        "--follow 应带回 old.txt 的诞生提交"
    );
    assert!(
        from_cli
            .items
            .iter()
            .any(|commit| commit.subject == "add old.txt"),
        "--follow 必须带回重命名前的历史"
    );

    // libgit2 明确拒绝：装作支持等于悄悄漏掉重命名前的历史（宁报错不给错）
    let error = libgit2
        .log(&repo, follow)
        .expect_err("libgit2 必须拒绝 --follow");
    assert_eq!(
        error.code,
        forgedesk_domain::ErrorCode::UnsupportedByEngine,
        "必须报 UNSUPPORTED_BY_ENGINE 而不是静默给出不完整结果：{error:?}"
    );
}

// ---------------------------------------------------------------- CLI 生命周期

#[test]
fn cli_engine_drives_a_full_read_write_lifecycle() {
    let dir = TempDir::new("lifecycle");
    let (cli, _) = engines();
    let repo = RepoId::new(dir.path());

    // init
    let info = cli.init(dir.path(), Default::default()).expect("init 失败");
    assert!(info.is_empty, "刚初始化的仓库应当是空的");
    assert!(!info.is_bare);
    // 引擎的 init 不会配置身份（那是用户环境的事）；commit 依赖身份，
    // CI runner 没有全局 user.name/email——本机有全局配置所以本地一直绿、
    // CI 一直红（2026-09-28 起的长期红灯根因）。仓库级配置让测试自足。
    git_ok(dir.path(), &["config", "user.name", "Fixture Author"]);
    git_ok(dir.path(), &["config", "user.email", "author@example.com"]);

    // 首次提交
    write(dir.path(), "a.txt", b"one\n");
    cli.stage(&repo, forgedesk_domain::git::StageSpec::All)
        .expect("stage 失败");
    let oid = cli
        .commit(
            &repo,
            forgedesk_domain::git::CommitSpec::new("first commit"),
        )
        .expect("commit 失败");
    assert_eq!(oid.len(), 40, "提交 oid 应当是完整哈希");

    // 读回
    let status = cli
        .status(&repo, &StatusQuery::default())
        .expect("status 失败");
    assert!(status.is_clean());
    let log = cli.log(&repo, LogQuery::new()).expect("log 失败");
    assert_eq!(log.items.len(), 1);
    assert_eq!(log.items[0].subject, "first commit");
    assert!(log.items[0].is_root());

    // 分支与标签
    let branches = cli.branch_list(&repo).expect("branch_list 失败");
    assert_eq!(branches.len(), 1);
    assert!(branches[0].is_head);

    // 改动 → 未暂存 → 暂存
    write(dir.path(), "a.txt", b"one\ntwo\n");
    let unstaged = cli
        .diff(&repo, DiffSpec::new(DiffTarget::Unstaged))
        .expect("diff 失败");
    assert_eq!(unstaged.file_count(), 1);
    assert_eq!(unstaged.files[0].additions, 1);

    cli.stage(&repo, forgedesk_domain::git::StageSpec::All)
        .expect("stage 失败");
    let staged = cli
        .diff(&repo, DiffSpec::new(DiffTarget::Staged))
        .expect("diff 失败");
    assert_eq!(
        staged.files[0].change,
        forgedesk_domain::git::DiffChangeKind::Modified
    );

    // 取消暂存
    cli.unstage(&repo, forgedesk_domain::git::StageSpec::All)
        .expect("unstage 失败");
    assert_eq!(
        cli.diff(&repo, DiffSpec::new(DiffTarget::Staged))
            .expect("diff 失败")
            .file_count(),
        0
    );

    // stash
    cli.stash(
        &repo,
        forgedesk_domain::git::StashSpec::push(Some("wip".to_owned())),
    )
    .expect("stash 失败");
    let stashes = cli.stash_list(&repo).expect("stash_list 失败");
    assert_eq!(stashes.len(), 1);
    assert_eq!(stashes[0].index, 0);
    assert!(cli
        .status(&repo, &StatusQuery::default())
        .expect("status 失败")
        .is_clean());

    // reflog
    let reflog = cli.reflog(&repo, 10).expect("reflog 失败");
    assert!(!reflog.is_empty());
    assert_eq!(reflog[0].reference, "HEAD");

    // show 带正文
    let shown = cli.show(&repo, "HEAD").expect("show 失败");
    assert_eq!(shown.subject, "first commit");
    assert_eq!(shown.oid, oid);
}

#[test]
fn cli_engine_reports_unsupported_for_rebase_until_m3() {
    let dir = TempDir::new("rebase-stub");
    let (cli, libgit2) = engines();
    let repo = RepoId::new(dir.path());
    let plan = forgedesk_domain::git::ReorderSpec {
        onto: "HEAD~1".to_owned(),
        steps: Vec::new(),
    };

    let from_cli = cli
        .rebase(&repo, plan.clone(), &ProgressSink::none())
        .expect_err("rebase 在 M3 之前必须明确失败");
    let from_libgit2 = libgit2
        .rebase(&repo, plan, &ProgressSink::none())
        .expect_err("libgit2 不支持 rebase");

    assert!(
        from_cli.message.contains("not implemented"),
        "CLI 的 rebase 应当是「尚未实现」而不是「不支持」：{}",
        from_cli.message
    );
    assert_eq!(
        from_libgit2.code,
        forgedesk_domain::ErrorCode::UnsupportedByEngine
    );
}

#[test]
fn libgit2_engine_reports_unsupported_for_the_whole_conflict_state_machine() {
    // T3.1 的取舍：冲突状态与 continue/abort/skip 唯一数据源是 git CLI
    // （stage 三方内容 + 2 MiB 阈值 + 二进制判定的语义以 CLI 为准，见
    // docs/GIT-ENGINE-DIFF.md §4）。这里钉住 libgit2 侧必须**明确拒绝**
    // 而不是静默给出另一套语义。
    let dir = TempDir::new("conflict-unsupported");
    init_repo(dir.path());
    let (_, libgit2) = engines();
    let repo = RepoId::new(dir.path());
    let op = forgedesk_domain::git::ConflictOpKind::Merge;

    let state_error = libgit2
        .conflict_state(&repo)
        .expect_err("libgit2 不支持冲突状态查询");
    let continue_error = libgit2
        .conflict_continue(&repo, op)
        .expect_err("libgit2 不支持冲突 continue");
    let abort_error = libgit2
        .conflict_abort(&repo, op)
        .expect_err("libgit2 不支持冲突 abort");
    let skip_error = libgit2
        .conflict_skip(&repo, op)
        .expect_err("libgit2 不支持冲突 skip");

    for error in [state_error, continue_error, abort_error, skip_error] {
        assert_eq!(
            error.code,
            forgedesk_domain::ErrorCode::UnsupportedByEngine,
            "必须报 UNSUPPORTED_BY_ENGINE：{error:?}"
        );
    }
}

#[test]
fn conflict_state_is_an_empty_probe_on_a_clean_repository() {
    let dir = TempDir::new("conflict-clean");
    init_repo(dir.path());
    let (cli, _) = engines();
    let repo = RepoId::new(dir.path());

    let state = cli.conflict_state(&repo).expect("conflict_state 失败");
    assert_eq!(state.op_kind, None);
    assert!(!state.op_in_progress);
    assert_eq!(state.current_step, None);
    assert_eq!(state.total_steps, None);
    assert!(state.files.is_empty());
    assert!(!state.can_continue);
    assert!(!state.can_abort);
    assert!(!state.can_skip);
}

#[test]
fn libgit2_engine_refuses_every_write_operation_explicitly() {
    let dir = TempDir::new("libgit2-writes");
    init_repo(dir.path());
    let (_, libgit2) = engines();
    let repo = RepoId::new(dir.path());

    let errors = [
        libgit2
            .stage(&repo, forgedesk_domain::git::StageSpec::All)
            .map(|_| ()),
        libgit2
            .unstage(&repo, forgedesk_domain::git::StageSpec::All)
            .map(|_| ()),
        libgit2
            .commit(&repo, forgedesk_domain::git::CommitSpec::new("x"))
            .map(|_| ()),
        libgit2
            .reset(
                &repo,
                forgedesk_domain::git::ResetSpec::to(
                    "HEAD",
                    forgedesk_domain::git::ResetMode::Soft,
                ),
            )
            .map(|_| ()),
        libgit2
            .stash(&repo, forgedesk_domain::git::StashSpec::push(None))
            .map(|_| ()),
    ];

    for error in errors {
        let error = error.expect_err("libgit2 的写操作必须明确失败");
        assert_eq!(
            error.code,
            forgedesk_domain::ErrorCode::UnsupportedByEngine,
            "写操作必须报 UNSUPPORTED_BY_ENGINE 而不是静默成功：{error:?}"
        );
    }
}

// ---------------------------------------------------------------- 索引指纹（T1.7）

#[test]
fn the_index_and_head_tree_oids_agree_across_engines() {
    let dir = TempDir::new("index-tree");
    let (cli, libgit2) = engines();
    init_repo(dir.path());
    let repo = RepoId::new(dir.path());

    // 空索引：两个引擎都要给出 git 的空树 oid，而不是各自报错或给出空串。
    // 服务层就是靠这个值判定"没有暂存内容"的。
    let empty = cli.index_tree(&repo).expect("CLI write-tree 失败");
    assert_eq!(empty, forgedesk_domain::git::EMPTY_TREE_OID);
    assert_eq!(
        libgit2.index_tree(&repo).expect("libgit2 write-tree 失败"),
        empty,
        "两个引擎对同一份空索引必须给出同一个树 oid"
    );
    assert_eq!(cli.head_tree(&repo).expect("CLI head tree 失败"), None);
    assert_eq!(
        libgit2.head_tree(&repo).expect("libgit2 head tree 失败"),
        None
    );

    // 暂存内容之后：指纹必须随索引变化
    write(dir.path(), "a.txt", b"one\n");
    cli.stage(&repo, StageSpec::All).expect("stage 失败");
    let staged = cli.index_tree(&repo).expect("CLI write-tree 失败");
    assert_ne!(staged, empty);
    assert_eq!(
        libgit2.index_tree(&repo).expect("libgit2 write-tree 失败"),
        staged
    );

    // 提交之后：HEAD 的树就等于刚才索引的树（这正是"索引 == HEAD ⇒ 没东西可提交"）
    cli.commit(&repo, CommitSpec::new("first"))
        .expect("commit 失败");
    let head_tree = cli.head_tree(&repo).expect("CLI head tree 失败");
    assert_eq!(head_tree.as_deref(), Some(staged.as_str()));
    assert_eq!(
        libgit2.head_tree(&repo).expect("libgit2 head tree 失败"),
        head_tree
    );

    // 再改一次内容：指纹又变了——这是"索引被外部改过"能被检出的依据
    write(dir.path(), "a.txt", b"two\n");
    cli.stage(&repo, StageSpec::All).expect("stage 失败");
    assert_ne!(cli.index_tree(&repo).expect("CLI write-tree 失败"), staged);
}

// ---------------------------------------------------------------- T2.4 提交详情

/// 详情面板消费的三类数据（元数据 / 合并双父 diff / 根提交 diff）两侧一致（T2.4）。
///
/// # 契约的不对称部分也要钉住
///
/// `show` 的 signature **只有 CLI 给得出**（libgit2 不做 GPG 校验，见引擎侧
/// 注释）——详情服务的签名徽标因此以 CLI 为准。这条不对称写成断言：如果哪天
/// libgit2 补了校验或 CLI 侧丢了它，这条测试会先红。
///
/// `refs` 曾在同一个不对称表里（T2.10 之前 libgit2 恒为空，历史图 ref 胶囊
/// 因此全空）；现在 libgit2 用 `RefDecorations` 模仿 `%D`，两侧按"排序后的
/// token 序列相等"断言——形状或顺序的模仿走样都会在这里现形。
#[test]
fn commit_detail_inputs_are_consistent_across_engines() {
    let dir = TempDir::new("diff-detail");
    shape_forked(dir.path());
    let (cli, libgit2) = engines();
    let repo = RepoId::new(dir.path());

    // 全量提交（含 feature 分支）：从 CLI log 拿 oid 清单
    let query = LogQuery::new().with_all_branches(true).with_limit(100);
    let commits = cli.log(&repo, query).expect("CLI log 失败").items;
    assert!(commits.len() >= 4, "夹具应有 base/C/B/merge 至少 4 条");
    let merge = commits
        .iter()
        .find(|commit| commit.parents.len() == 2)
        .expect("夹具应有 merge 提交");
    let root = commits
        .iter()
        .find(|commit| commit.parents.is_empty())
        .expect("夹具应有根提交");

    // ① 元数据：oid / 父提交 / 作者 / 提交者 / subject / body 两侧一致
    for commit in &commits {
        let from_cli = cli.show(&repo, &commit.oid).expect("CLI show 失败");
        let from_libgit2 = libgit2.show(&repo, &commit.oid).expect("libgit2 show 失败");

        assert_eq!(from_cli.oid, from_libgit2.oid);
        assert_eq!(from_cli.parents, from_libgit2.parents, "oid {}", commit.oid);
        assert_eq!(from_cli.author, from_libgit2.author, "oid {}", commit.oid);
        assert_eq!(
            from_cli.committer, from_libgit2.committer,
            "oid {}",
            commit.oid
        );
        assert_eq!(from_cli.subject, from_libgit2.subject, "oid {}", commit.oid);
        assert_eq!(from_cli.body, from_libgit2.body, "oid {}", commit.oid);

        // 签名状态的契约不对称仍然成立：只有 CLI 做 GPG 校验
        assert_eq!(
            from_libgit2.signature,
            forgedesk_domain::git::SignatureStatus::Unknown
        );

        // refs 自 T2.10 起两侧都有（libgit2 用 RefDecorations 模仿 %D）：
        // 排序后的 token 序列必须相等——这是历史图 ref 胶囊的数据源，
        // 两侧不一致会让同一提交在详情面板和图页显示不同的 ref 集合
        let mut sorted_cli = from_cli.refs.clone();
        sorted_cli.sort();
        let mut sorted_libgit2 = from_libgit2.refs.clone();
        sorted_libgit2.sort();
        assert_eq!(
            sorted_cli, sorted_libgit2,
            "oid {} 的 ref 装饰两侧不一致",
            commit.oid
        );
    }
    // CLI 侧的 merge 提交带 HEAD -> main 装饰（详情面板的 ref 胶囊数据源）
    let merge_from_cli = cli.show(&repo, &merge.oid).expect("CLI show 失败");
    assert!(
        merge_from_cli
            .refs
            .iter()
            .any(|refname| refname.contains("main")),
        "merge 提交的 %D 应含 main，实际 {:?}",
        merge_from_cli.refs
    );

    // ② 合并双父 diff：相对第一父与相对第二父，两侧逐文件一致
    for (index, parent) in merge.parents.iter().enumerate() {
        compare_diff(
            &cli,
            &libgit2,
            &repo,
            DiffTarget::between(parent.clone(), &merge.oid),
            &format!("merge parent {index}"),
        );
    }

    // ③ 非合并提交与根提交的 `Commit` 目标（详情对单父/根提交的口径）
    let normal = commits
        .iter()
        .find(|commit| commit.parents.len() == 1)
        .expect("夹具应有单父提交");
    compare_diff(
        &cli,
        &libgit2,
        &repo,
        DiffTarget::Commit(normal.oid.clone()),
        "single parent",
    );
    compare_diff(
        &cli,
        &libgit2,
        &repo,
        DiffTarget::Commit(root.oid.clone()),
        "root commit",
    );
}

// ---------------------------------------------------------------- T2.3 筛选

/// 作者过滤 × 跳页：`--skip` 数的是**过滤后**的第 N 条（CLI），libgit2 侧
/// 同样必须"先过滤再跳"。这条测试是 T2.3 修掉的 skip-前置缺陷的回归护栏
/// （此前 `revwalk.skip()` 在过滤前跳行，翻第二页时与 CLI 给出不同的窗口）。
#[test]
fn log_author_filter_with_skip_is_consistent_across_engines() {
    let dir = TempDir::new("diff-author-skip");
    init_repo(dir.path());
    write(dir.path(), "a.txt", b"one\n");
    commit_all_with_author(dir.path(), "alice one", "Alice", "alice@example.com", 1);
    write(dir.path(), "b.txt", b"two\n");
    commit_all_with_author(dir.path(), "bob one", "Bob", "bob@example.com", 2);
    write(dir.path(), "c.txt", b"three\n");
    commit_all_with_author(dir.path(), "alice two", "Alice", "alice@example.com", 3);
    write(dir.path(), "d.txt", b"four\n");
    commit_all_with_author(dir.path(), "alice three", "Alice", "alice@example.com", 4);
    let (cli, libgit2) = engines();
    let repo = RepoId::new(dir.path());

    // 第 3 页（skip=2）：Alice 有 3 条，walk 新→旧，跳过最新两条后只剩最旧一条
    let query = LogQuery {
        author: Some("alice".to_owned()),
        skip: 2,
        ..LogQuery::new().with_limit(2)
    };
    let from_cli = normalize_log(&cli.log(&repo, query.clone()).expect("CLI log 失败"));
    let from_libgit2 = normalize_log(&libgit2.log(&repo, query).expect("libgit2 log 失败"));
    assert_eq!(
        from_cli, from_libgit2,
        "作者过滤 × skip 两侧不一致\nCLI:     {from_cli:?}\nlibgit2: {from_libgit2:?}"
    );
    assert_eq!(from_cli.len(), 1, "skip=2 后第三页只剩 alice three");
    assert_eq!(
        from_cli[0].2, "alice one",
        "walk 新→旧：skip=2 跳过最新两条，剩最旧的 alice one"
    );
}

/// 仅显示合并提交：两侧一致，且非合并提交必须被过滤掉。
#[test]
fn log_merges_only_is_consistent_across_engines() {
    let dir = TempDir::new("diff-merges");
    shape_forked(dir.path());
    let (cli, libgit2) = engines();
    let repo = RepoId::new(dir.path());

    let query = LogQuery {
        merges_only: true,
        ..LogQuery::new().with_limit(100)
    };
    let from_cli = normalize_log(&cli.log(&repo, query.clone()).expect("CLI log 失败"));
    let from_libgit2 = normalize_log(&libgit2.log(&repo, query).expect("libgit2 log 失败"));
    assert_eq!(
        from_cli, from_libgit2,
        "[merges_only] 两侧不一致\nCLI:     {from_cli:?}\nlibgit2: {from_libgit2:?}"
    );
    assert_eq!(from_cli.len(), 1, "夹具只有一个合并提交");
}

/// 关键词忽略大小写（`-i` 口径）：大小写变体也要命中，且两侧一致。
#[test]
fn log_case_insensitive_search_is_consistent_across_engines() {
    let dir = TempDir::new("diff-grep-i");
    init_repo(dir.path());
    write(dir.path(), "a.txt", b"one\n");
    commit_all(dir.path(), "Add ReadmePipeline", 1);
    write(dir.path(), "b.txt", b"two\n");
    commit_all(dir.path(), "unrelated", 2);
    let (cli, libgit2) = engines();
    let repo = RepoId::new(dir.path());

    let query = LogQuery {
        message_contains: Some("readmepipeline".to_owned()),
        case_insensitive: true,
        ..LogQuery::new().with_limit(100)
    };
    let from_cli = normalize_log(&cli.log(&repo, query.clone()).expect("CLI log 失败"));
    let from_libgit2 = normalize_log(&libgit2.log(&repo, query).expect("libgit2 log 失败"));
    assert_eq!(
        from_cli, from_libgit2,
        "[case_insensitive] 两侧不一致\nCLI:     {from_cli:?}\nlibgit2: {from_libgit2:?}"
    );
    assert_eq!(from_cli.len(), 1, "忽略大小写后小写变体应命中");
}

/// 分支多选：并集语义（两个 tip 的提交都在结果里），且两侧一致。
///
/// 夹具刻意**不合并**：两个分支各有一个对方拿不到的提交，并集才会真的
/// 比单分支多（forked+merge 夹具里 feature 的提交已经是 main 的祖先，
/// 证明不了并集）。
#[test]
fn log_multi_revision_is_consistent_across_engines() {
    let dir = TempDir::new("diff-multi-rev");
    init_repo(dir.path());
    write(
        dir.path(),
        "base.txt",
        b"base
",
    );
    commit_all(dir.path(), "base", 1);
    git_ok(dir.path(), &["checkout", "-q", "-b", "feature"]);
    write(
        dir.path(),
        "feature.txt",
        b"feature
",
    );
    commit_all(dir.path(), "feature work", 2);
    git_ok(dir.path(), &["checkout", "-q", "main"]);
    write(
        dir.path(),
        "main.txt",
        b"main
",
    );
    commit_all(dir.path(), "main work", 3);
    let (cli, libgit2) = engines();
    let repo = RepoId::new(dir.path());

    let query = LogQuery {
        revisions: vec!["main".to_owned(), "feature".to_owned()],
        ..LogQuery::new().with_limit(100)
    };
    let from_cli = normalize_log(&cli.log(&repo, query.clone()).expect("CLI log 失败"));
    let from_libgit2 = normalize_log(&libgit2.log(&repo, query).expect("libgit2 log 失败"));
    assert_eq!(
        from_cli, from_libgit2,
        "[revisions] 两侧不一致
CLI:     {from_cli:?}
libgit2: {from_libgit2:?}"
    );
    assert_eq!(from_cli.len(), 3, "并集应覆盖 base/C/B 三条提交");

    // 对照：只看 main 时 feature 侧的提交不在结果里（两侧都要过——
    // libgit2 的单 revision 路径历史上只认完整引用名，短名会报 not valid）
    let single = LogQuery::new().with_revision("main").with_limit(100);
    let from_cli = normalize_log(&cli.log(&repo, single.clone()).expect("CLI log 失败"));
    let from_libgit2 = normalize_log(&libgit2.log(&repo, single).expect("libgit2 log 失败"));
    assert_eq!(from_cli, from_libgit2, "单分支查询两侧不一致");
    assert_eq!(from_cli.len(), 2, "单分支只应有 base/B 两条");
}

/// 作者列表：CLI 按（邮箱）去重并计数排序；libgit2 侧如实拒绝（先例：
/// `remote_refs_containing`）。
#[test]
fn authors_are_summarized_by_cli_and_refused_by_libgit2() {
    let dir = TempDir::new("diff-authors");
    init_repo(dir.path());
    write(dir.path(), "a.txt", b"one\n");
    commit_all_with_author(dir.path(), "one", "Alice", "alice@example.com", 1);
    write(dir.path(), "b.txt", b"two\n");
    commit_all_with_author(dir.path(), "two", "Bob", "bob@example.com", 2);
    write(dir.path(), "c.txt", b"three\n");
    // 同一邮箱换名字：按邮箱去重后应归并到出现最多（这里并列，取字典序）的名字
    commit_all_with_author(dir.path(), "three", "Alicia", "alice@example.com", 3);
    let (cli, libgit2) = engines();
    let repo = RepoId::new(dir.path());

    let authors = cli.authors(&repo).expect("CLI authors 失败");
    assert_eq!(authors.len(), 2, "两个邮箱 → 两个作者");
    assert_eq!(authors[0].email, "alice@example.com", "提交数多者排前");
    assert_eq!(authors[0].commit_count, 2);
    assert_eq!(authors[1].email, "bob@example.com");
    assert_eq!(authors[1].name, "Bob");

    let error = libgit2
        .authors(&repo)
        .expect_err("libgit2 未实现 authors，应返回 Unsupported");
    assert_eq!(error.code, forgedesk_domain::ErrorCode::UnsupportedByEngine);
}
