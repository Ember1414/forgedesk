//! 仓库发现的集成测试（T1.3）。
//!
//! 覆盖 `RepositoryInfo` 在 T1.3 新增的四个字段，以及 `CliGitEngine` 上
//! 两个只属于 CLI 侧的能力（`version` 与 `repository_config`）。
//!
//! 这些用例都**必须**用真实仓库：`is_shallow` 来自 `.git/shallow` 文件、
//! `worktrees` 来自 `git worktree` 的元数据目录、默认分支来自 `origin/HEAD`
//! 这个符号引用——用假数据构造它们等于在测试自己的假设。
//!
//! 集成测试是独立 crate，测试代码里允许 panic 式写法（失败即断言失败）。
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::path::Path;

use forgedesk_domain::git::{CloneSpec, RepoId};
use forgedesk_domain::ErrorCode;
use forgedesk_git_engine::engine::{CliGitEngine, GitEngine, Libgit2Engine, ProgressSink};
// 克隆的凭据方案：`file://` 远端不需要凭据，因此这些用例一律传匿名方案
use forgedesk_git_engine::process::NetworkAuth;
use support::{commit_all, git, git_ok, init_repo, write, TempDir};

fn cli() -> CliGitEngine {
    CliGitEngine::new().expect("创建 CLI 引擎失败")
}

/// 把一个本地目录变成 `file://` URL。
///
/// 必须走 `file://`：`git clone --depth` 对**本地路径**克隆会明确忽略深度
/// （"--depth is ignored in local clones"），那样测出来的 `is_shallow`
/// 永远是 false，而测试会"通过"。
fn file_url(path: &Path) -> String {
    let normalized = path.to_string_lossy().replace('\\', "/");
    if normalized.starts_with('/') {
        format!("file://{normalized}")
    } else {
        format!("file:///{normalized}")
    }
}

// ------------------------------------------------------------ 默认分支

#[test]
fn default_branch_comes_from_origin_head_when_there_is_a_remote() {
    let source_dir = TempDir::new("discover-remote-src");
    init_repo(source_dir.path());
    write(source_dir.path(), "a.txt", b"a\n");
    commit_all(source_dir.path(), "first", 0);

    let clone_dir = TempDir::new("discover-remote-dst");
    let clone_path = clone_dir.path().join("work");
    let engine = cli();
    engine
        .clone(
            CloneSpec::new(file_url(source_dir.path()), &clone_path),
            &ProgressSink::none(),
            &NetworkAuth::none(),
        )
        .expect("克隆失败");

    // 切到一个特性分支：此时"当前分支"是 feature，而"默认分支"仍应是 main。
    // 只有真的读了 `origin/HEAD` 才能区分这两者。
    git_ok(&clone_path, &["checkout", "-q", "-b", "feature"]);

    let info = engine.discover(&clone_path).expect("发现失败");
    assert_eq!(info.head.as_deref(), Some("feature"));
    assert_eq!(
        info.default_branch.as_deref(),
        Some("main"),
        "默认分支应来自 origin/HEAD，而不是当前分支"
    );
    assert!(!info.is_shallow);
    assert!(!info.is_lfs);
}

#[test]
fn default_branch_falls_back_to_the_current_branch_without_a_remote() {
    let dir = TempDir::new("discover-local");
    init_repo(dir.path());
    write(dir.path(), "a.txt", b"a\n");
    commit_all(dir.path(), "first", 0);
    git_ok(dir.path(), &["branch", "-m", "trunk"]);

    let info = cli().discover(dir.path()).expect("发现失败");
    assert_eq!(info.default_branch.as_deref(), Some("trunk"));
    assert_eq!(info.head.as_deref(), Some("trunk"));
}

// ------------------------------------------------------------ 浅克隆

#[test]
fn a_shallow_clone_is_reported_as_shallow() {
    let source_dir = TempDir::new("discover-shallow-src");
    init_repo(source_dir.path());
    for sequence in 0..3 {
        write(
            source_dir.path(),
            "a.txt",
            format!("line {sequence}\n").as_bytes(),
        );
        commit_all(source_dir.path(), &format!("commit {sequence}"), sequence);
    }

    let target_dir = TempDir::new("discover-shallow-dst");
    let target = target_dir.path().join("shallow");
    let engine = cli();
    engine
        .clone(
            CloneSpec::new(file_url(source_dir.path()), &target).with_depth(1),
            &ProgressSink::none(),
            &NetworkAuth::none(),
        )
        .expect("浅克隆失败");

    let info = engine.discover(&target).expect("发现失败");
    assert!(info.is_shallow, "depth=1 的克隆必须被识别为浅克隆");
}

// ------------------------------------------------------------ LFS

#[test]
fn lfs_usage_is_detected_from_gitattributes() {
    let dir = TempDir::new("discover-lfs");
    init_repo(dir.path());
    write(
        dir.path(),
        ".gitattributes",
        b"*.psd filter=lfs diff=lfs merge=lfs -text\n",
    );
    commit_all(dir.path(), "declare lfs", 0);

    let info = cli().discover(dir.path()).expect("发现失败");
    assert!(info.is_lfs, "声明了 filter=lfs 的仓库应被识别为使用 LFS");
}

#[test]
fn a_repository_without_lfs_is_not_flagged() {
    let dir = TempDir::new("discover-no-lfs");
    init_repo(dir.path());
    write(dir.path(), ".gitattributes", b"*.txt text=auto\n");
    commit_all(dir.path(), "plain attributes", 0);

    assert!(!cli().discover(dir.path()).expect("发现失败").is_lfs);
}

// ------------------------------------------------------------ 工作区

#[test]
fn linked_worktrees_are_listed_with_the_main_one_first() {
    let dir = TempDir::new("discover-worktree");
    init_repo(dir.path());
    write(dir.path(), "a.txt", b"a\n");
    commit_all(dir.path(), "first", 0);

    let linked = dir
        .path()
        .parent()
        .unwrap()
        .join(format!("forgedesk-linked-{}-wt", std::process::id()));
    let _ = std::fs::remove_dir_all(&linked);
    git_ok(
        dir.path(),
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feature",
            linked.to_str().unwrap(),
        ],
    );

    let info = cli().discover(dir.path()).expect("发现失败");
    let cleanup = || {
        let _ = std::fs::remove_dir_all(&linked);
    };

    assert_eq!(info.worktrees.len(), 2, "应列出主工作区与一个关联工作区");
    assert_eq!(info.linked_worktree_count(), 1);
    assert_eq!(
        info.main_worktree().map(|tree| tree.path.as_path()),
        Some(dir.path()),
        "主工作区必须在首位"
    );

    let feature = &info.worktrees[1];
    assert_eq!(feature.path, linked);
    assert_eq!(feature.branch.as_deref(), Some("feature"));
    assert!(!feature.locked);
    assert!(!feature.detached);

    cleanup();
}

#[test]
fn a_worktree_locked_with_a_reason_is_reported_as_locked() {
    let dir = TempDir::new("discover-locked");
    init_repo(dir.path());
    write(dir.path(), "a.txt", b"a\n");
    commit_all(dir.path(), "first", 0);

    let linked = dir
        .path()
        .parent()
        .unwrap()
        .join(format!("forgedesk-locked-{}-wt", std::process::id()));
    let _ = std::fs::remove_dir_all(&linked);
    git_ok(
        dir.path(),
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "locked-branch",
            linked.to_str().unwrap(),
        ],
    );
    git_ok(
        dir.path(),
        &[
            "worktree",
            "lock",
            "--reason",
            "don't touch",
            linked.to_str().unwrap(),
        ],
    );

    let info = cli().discover(dir.path()).expect("发现失败");
    assert!(info.worktrees[1].locked, "被锁定的工作区必须报告 locked");
    assert_eq!(info.worktrees[1].branch.as_deref(), Some("locked-branch"));

    let _ = std::fs::remove_dir_all(&linked);
}

// ------------------------------------------------------------ 裸仓库与错误

#[test]
fn a_bare_repository_has_no_workdir_but_still_has_a_worktree_entry() {
    let dir = TempDir::new("discover-bare");
    git_ok(dir.path(), &["init", "-q", "--bare", "-b", "main", "."]);

    let info = cli().discover(dir.path()).expect("发现失败");
    assert!(info.is_bare);
    assert_eq!(info.workdir, None);
    assert_eq!(info.worktrees.len(), 1);
    assert!(info.worktrees[0].is_bare);
}

#[test]
fn a_directory_outside_any_repository_reports_path_not_repo() {
    let dir = TempDir::new("discover-none");
    let expected = ErrorCode::PathNotRepo;

    let cli_error = cli().discover(dir.path()).unwrap_err();
    assert_eq!(cli_error.code, expected);

    let libgit2_error = Libgit2Engine::new().discover(dir.path()).unwrap_err();
    assert_eq!(libgit2_error.code, expected);
}

// ------------------------------------------------------------ 双实现的已知差异

#[test]
fn libgit2_reports_only_the_main_worktree() {
    // libgit2 的 `Repository::worktrees` 只返回工作区**名称**，没有路径与 HEAD，
    // 因此读实现只报主工作区；完整列表由 CLI 侧提供（见 docs/GIT-ENGINE-DIFF.md）。
    let dir = TempDir::new("discover-libgit2-worktree");
    init_repo(dir.path());
    write(dir.path(), "a.txt", b"a\n");
    commit_all(dir.path(), "first", 0);

    let linked = dir
        .path()
        .parent()
        .unwrap()
        .join(format!("forgedesk-libgit2-{}-wt", std::process::id()));
    let _ = std::fs::remove_dir_all(&linked);
    git_ok(
        dir.path(),
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feature",
            linked.to_str().unwrap(),
        ],
    );

    let cli_info = cli().discover(dir.path()).expect("发现失败");
    let libgit2_info = Libgit2Engine::new().discover(dir.path()).expect("发现失败");

    assert_eq!(cli_info.worktrees.len(), 2);
    assert_eq!(libgit2_info.worktrees.len(), 1);

    // 两边都必须一致的那部分：主工作区路径与浅克隆/LFS 判定
    assert_eq!(cli_info.worktrees[0].path, libgit2_info.worktrees[0].path);
    assert_eq!(cli_info.is_shallow, libgit2_info.is_shallow);
    assert_eq!(cli_info.is_lfs, libgit2_info.is_lfs);

    let _ = std::fs::remove_dir_all(&linked);
}

// ------------------------------------------------------------ CLI 侧独有能力

#[test]
fn cli_reports_a_parseable_git_version_that_meets_the_minimum() {
    let version = cli().version().expect("读取 git 版本失败");

    assert!(
        version.is_supported(),
        "测试环境里的 git 低于最低版本 {}：{version}",
        forgedesk_domain::git::MINIMUM_GIT_VERSION
    );
    assert_eq!(version.major, 2);
}

#[test]
fn repository_config_reads_local_entries_and_redacts_secrets() {
    let dir = TempDir::new("discover-config");
    init_repo(dir.path());
    let secret = "ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    git_ok(
        dir.path(),
        &[
            "config",
            "--local",
            "remote.origin.url",
            &format!("https://{secret}@example.com/a.git"),
        ],
    );
    git_ok(
        dir.path(),
        &["config", "--local", "core.sshCommand", "ssh -i /tmp/key"],
    );

    let repo = RepoId::new(dir.path());
    let entries = cli().repository_config(&repo).expect("读取配置失败");

    let url = entries
        .iter()
        .find(|entry| entry.key == "remote.origin.url")
        .expect("应读到 remote.origin.url");
    assert!(
        !url.value.contains(secret),
        "配置值必须在读入时脱敏，实际：{}",
        url.value
    );

    assert!(
        entries
            .iter()
            .any(|entry| entry.key == "core.sshcommand" || entry.key == "core.sshCommand"),
        "应读到 core.sshCommand，实际键：{:?}",
        entries.iter().map(|entry| &entry.key).collect::<Vec<_>>()
    );
}

#[test]
fn repository_config_returns_empty_outside_a_repository() {
    let dir = TempDir::new("discover-config-none");
    let entries = cli()
        .repository_config(&RepoId::new(dir.path()))
        .expect("非仓库不应报错");

    assert!(entries.is_empty());
}

// ------------------------------------------------------------ 子目录探测

#[test]
fn discovery_walks_up_from_a_subdirectory_to_the_repository_root() {
    let dir = TempDir::new("discover-subdir");
    init_repo(dir.path());
    write(dir.path(), "nested/deep/a.txt", b"a\n");
    commit_all(dir.path(), "first", 0);

    let info = cli()
        .discover(&dir.path().join("nested/deep"))
        .expect("发现失败");

    assert_eq!(
        info.workdir.as_deref(),
        Some(dir.path()),
        "从子目录打开应落到仓库根"
    );
    assert_eq!(info.id.root(), dir.path());
}

// ------------------------------------------------------------ 辅助

/// 确保 `git` 本身可用（否则上面所有用例的失败原因都会指向错误的方向）。
#[test]
fn the_test_environment_has_a_working_git() {
    let output = git(Path::new("."), &["--version"]);
    assert!(output.success(), "测试环境没有可用的 git");
    assert!(!output.stdout_lossy().trim().is_empty());
}
