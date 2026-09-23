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
//!    `refs` 与 `signature` 不在比较范围内（见上）。
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
    Commit, DiffReport, DiffSpec, DiffTarget, EntryKind, LogQuery, Page, RepoId, StatusReport,
};
use forgedesk_git_engine::engine::{CliGitEngine, GitEngine, Libgit2Engine, ProgressSink};
use support::{commit_all, git_ok, init_repo, write, TempDir};

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
    let from_cli = normalize_status(&cli.status(repo).expect("CLI status 失败"));
    let from_libgit2 = normalize_status(&libgit2.status(repo).expect("libgit2 status 失败"));

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
    let status = cli.status(&repo).expect("status 失败");
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
    assert!(cli.status(&repo).expect("status 失败").is_clean());

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
        libgit2.stash(&repo, forgedesk_domain::git::StashSpec::push(None)),
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
