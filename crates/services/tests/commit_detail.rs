//! 提交详情服务（T2.4）的集成测试。
//!
//! # 为什么跑在真实仓库上
//!
//! 详情是**编排**：`show` 的元数据、CLI diff 的统计、`remote_refs_containing`
//! 的推送判定、URL 推断，拼在一起才是面板需要的那一行。其中最容易写错的
//! 是合并提交的父选择——相对第一父与相对第二父的文件清单必须**不同**，
//! 且与 `git diff <父>..<提交>` 的口径一致（这是验收项）。
//!
//! # 夹具
//!
//! 与 `layout_dag.rs` 同一个 merge 仓库形状；时间戳互不相同保证 walk 顺序。
//! `is_pushed` 用 `update-ref refs/remotes/origin/main` 造远端跟踪引用
//! （不联网、不真正 push，只验证"包含判定"本身）。
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::path::Path;

use forgedesk_domain::ErrorCode;
use forgedesk_git_engine::engines::GitEngines;
use forgedesk_services::CommitDetailService;
use forgedesk_storage::{Database, RepositoryStore, RepositoryUpsert};
use support::{git, git_ok, init_repo, memory_database, write, TempDir};

/// 用固定时间戳提交（保证 walk 顺序可预测）。
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

/// 与 layout_dag.rs 同形状的 merge 仓库：
///
/// ```text
///           M (merge)     ← main（最新）
///          / \
///     B (main)  D (feature)
///     A          C
///          \   /
///           BASE
/// ```
///
/// 返回（目录、数据库、登记后的 repo id）。
fn build_merge_repo(label: &str) -> (TempDir, Database, i64) {
    let dir = TempDir::new(label);
    init_repo(dir.path());

    write(dir.path(), "base.txt", b"base\n");
    commit_at(dir.path(), "base", 1_700_000_000);

    write(dir.path(), "a.txt", b"a\n");
    commit_at(dir.path(), "A on main", 1_700_000_060);

    git_ok(dir.path(), &["checkout", "-q", "-b", "feature"]);
    write(dir.path(), "c.txt", b"c\n");
    commit_at(dir.path(), "C on feature", 1_700_000_120);

    git_ok(dir.path(), &["checkout", "-q", "main"]);
    write(dir.path(), "b.txt", b"b\n");
    commit_at(dir.path(), "B on main", 1_700_000_180);

    git_ok(dir.path(), &["checkout", "-q", "feature"]);
    write(dir.path(), "d.txt", b"d\n");
    commit_at(dir.path(), "D on feature", 1_700_000_240);

    git_ok(dir.path(), &["checkout", "-q", "main"]);
    let stamp = format!("{} +0000", 1_700_000_300);
    let output = std::process::Command::new("git")
        .current_dir(dir.path())
        .args([
            "merge",
            "--no-ff",
            "-q",
            "-m",
            "M merge feature",
            // 正文占一段：验证 show 的 %b 链路（列表查询不带正文）
            "-m",
            "brings C and D onto main",
            "feature",
        ])
        .env("GIT_AUTHOR_DATE", &stamp)
        .env("GIT_COMMITTER_DATE", &stamp)
        .output()
        .expect("git 运行失败");
    assert!(output.status.success(), "merge 失败");

    let database = memory_database();
    let store = RepositoryStore::new(&database);
    let repo_id = store
        .upsert(
            &RepositoryUpsert {
                name: label.to_owned(),
                path: dir.path().to_string_lossy().to_string(),
                default_branch: Some("main".to_owned()),
                provider_id: None,
                size_class: None,
            },
            0,
        )
        .expect("登记仓库失败");

    (dir, database, repo_id)
}

/// 取 merge 提交与其两个父提交的完整 oid。
fn merge_and_parents(dir: &Path) -> (String, String, String) {
    let output = git(dir, &["log", "--all", "--format=%H%x1f%P%x1f%s%x1e"]);
    let stdout = output.stdout_lossy();
    for record in stdout.split('\x1e') {
        let record = record.trim();
        if record.is_empty() {
            continue;
        }
        let fields: Vec<&str> = record.split('\x1f').collect();
        if fields.len() >= 3 && fields[2] == "M merge feature" {
            let parents: Vec<&str> = fields[1].split_whitespace().collect();
            assert_eq!(parents.len(), 2, "夹具的 merge 应有两个父提交");
            return (
                fields[0].to_owned(),
                parents[0].to_owned(),
                parents[1].to_owned(),
            );
        }
    }
    panic!("夹具里没有 merge 提交");
}

#[test]
fn the_merge_commit_lists_files_relative_to_the_chosen_parent() {
    let (dir, database, repo_id) = build_merge_repo("detail-merge");
    let (merge_oid, first_parent, second_parent) = merge_and_parents(dir.path());
    let engines = GitEngines::new().expect("创建引擎失败");
    let service = CommitDetailService::new(&engines, RepositoryStore::new(&database));

    // 第一父视角：相对 main 侧（B），merge 带来了 feature 上的 C、D
    let first = service.detail(repo_id, &merge_oid, None).expect("详情失败");
    assert!(first.is_merge);
    assert_eq!(first.parent_index, 0);
    let first_paths: Vec<&str> = first.files.iter().map(|file| file.path.as_str()).collect();
    assert!(
        first_paths.contains(&"c.txt"),
        "第一父视角应有 c.txt：{first_paths:?}"
    );
    assert!(first_paths.contains(&"d.txt"));

    // 第二父视角：相对 feature 侧（D），merge 带来了 main 上的 B（A 已在共同祖先里）
    let second = service
        .detail(repo_id, &merge_oid, Some(1))
        .expect("详情失败");
    assert_eq!(second.parent_index, 1);
    let second_paths: Vec<&str> = second.files.iter().map(|file| file.path.as_str()).collect();
    assert!(
        second_paths.contains(&"b.txt"),
        "第二父视角应有 b.txt：{second_paths:?}"
    );
    assert!(
        !second_paths.contains(&"d.txt"),
        "第二父视角不应把 D 自己算进来"
    );

    // 两个视角必须真的不同（验收项：双父 diff 都能看）
    assert_ne!(
        first.stats, second.stats,
        "双父视角的统计不应相同（否则父选择没生效）"
    );

    // 父 oid 确实来自夹具，且元数据带正文与短 oid
    assert_eq!(first.meta.parents, vec![first_parent, second_parent]);
    assert_eq!(first.meta.short_oid.len(), 7);
    assert!(first.meta.body.is_some(), "show 带 %b（与列表查询不同）");
}

#[test]
fn out_of_range_parent_index_is_a_validation_error() {
    let (dir, database, repo_id) = build_merge_repo("detail-range");
    let (merge_oid, _, _) = merge_and_parents(dir.path());
    let engines = GitEngines::new().expect("创建引擎失败");
    let service = CommitDetailService::new(&engines, RepositoryStore::new(&database));

    for bad in [2, 7] {
        let error = service
            .detail(repo_id, &merge_oid, Some(bad))
            .expect_err("越界父应报错");
        assert_eq!(error.code, ErrorCode::Validation, "parent_index={bad}");
    }

    // 非合并提交只有一个父：选第二父同样越界
    let output = git(
        dir.path(),
        &["log", "--format=%H%x1f%P%x1f%s%x1e", "main~1"],
    );
    let stdout = output.stdout_lossy();
    let single = stdout
        .split('\x1e')
        .map(str::trim)
        .filter(|record| !record.is_empty())
        .map(|record| record.split('\x1f').map(str::to_owned).collect::<Vec<_>>())
        .find(|fields| fields.len() >= 2 && fields[1].split_whitespace().count() == 1)
        .expect("应有单父提交");
    let error = service
        .detail(repo_id, &single[0], Some(1))
        .expect_err("单父提交选第二父应报错");
    assert_eq!(error.code, ErrorCode::Validation);
}

#[test]
fn a_malformed_oid_is_rejected_before_touching_the_engine() {
    let (_, database, repo_id) = build_merge_repo("detail-oid");
    let engines = GitEngines::new().expect("创建引擎失败");
    let service = CommitDetailService::new(&engines, RepositoryStore::new(&database));

    for bad in ["", "abc", "g".repeat(40).as_str()] {
        let error = service
            .detail(repo_id, bad, None)
            .expect_err("非法 oid 应报错");
        assert_eq!(error.code, ErrorCode::Validation, "oid={bad:?}");
    }
}

#[test]
fn a_root_commit_diffs_against_the_empty_tree() {
    let (dir, database, repo_id) = build_merge_repo("detail-root");
    let engines = GitEngines::new().expect("创建引擎失败");
    let service = CommitDetailService::new(&engines, RepositoryStore::new(&database));

    let output = git(dir.path(), &["log", "--format=%H%x1f%P%x1f%s%x1e"]);
    let stdout = output.stdout_lossy();
    let root = stdout
        .split('\x1e')
        .map(str::trim)
        .filter(|record| !record.is_empty())
        .map(|record| record.split('\x1f').map(str::to_owned).collect::<Vec<_>>())
        .find(|fields| fields.len() >= 2 && fields[1].trim().is_empty())
        .expect("应有根提交");

    let detail = service
        .detail(repo_id, &root[0], None)
        .expect("根提交详情失败");
    assert!(!detail.is_merge);
    assert!(detail.meta.parents.is_empty());
    assert_eq!(detail.stats.files_changed, 1, "根提交只带 base.txt");
    let paths: Vec<&str> = detail.files.iter().map(|file| file.path.as_str()).collect();
    assert!(paths.contains(&"base.txt"), "{paths:?}");
}

#[test]
fn head_and_pushed_flags_follow_the_repository_state() {
    let (dir, database, repo_id) = build_merge_repo("detail-flags");
    let (merge_oid, _, _) = merge_and_parents(dir.path());
    let engines = GitEngines::new().expect("创建引擎失败");
    let service = CommitDetailService::new(&engines, RepositoryStore::new(&database));

    // HEAD 上的 merge 提交：is_head = true，还没有远端 → is_pushed = false
    let detail = service.detail(repo_id, &merge_oid, None).expect("详情失败");
    assert!(detail.is_head);
    assert!(!detail.is_pushed);
    assert!(detail.web_url.is_none(), "没有远端时推不出网页 URL");
    assert!(
        detail.refs.iter().any(|refname| refname.contains("main")),
        "HEAD -> main 应出现在 refs 里"
    );

    // 造一个远端跟踪引用包含该提交（不联网）：is_pushed 翻真
    git_ok(
        dir.path(),
        &["update-ref", "refs/remotes/origin/main", &merge_oid],
    );
    let detail = service.detail(repo_id, &merge_oid, None).expect("详情失败");
    assert!(detail.is_pushed);

    // 老提交不再是 HEAD，也不被刚造的 origin/main 包含之前的判定已经覆盖；
    // 这里只验证 is_head 会随提交移动
    let output = git(dir.path(), &["rev-parse", "main~1"]);
    let parent_oid = output.stdout_lossy().trim().to_owned();
    let older = service
        .detail(repo_id, &parent_oid, None)
        .expect("详情失败");
    assert!(!older.is_head);
}

#[test]
fn an_unknown_record_id_is_not_found() {
    let (_, database, _) = build_merge_repo("detail-missing-repo");
    let engines = GitEngines::new().expect("创建引擎失败");
    let service = CommitDetailService::new(&engines, RepositoryStore::new(&database));
    let error = service
        .detail(99_999, &"a".repeat(40), None)
        .expect_err("未知仓库应报错");
    assert_eq!(error.code, ErrorCode::NotFound);
}
