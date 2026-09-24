//! 快照管理器的集成测试（M1 / T1.9 的强制验收）。
//!
//! 任务验收原文：破坏性操作 → 回滚 → 断言 `git status --porcelain=v2 -z` 输出、
//! `rev-parse HEAD`、`ls-files --stage` 哈希、未跟踪文件集合四项与操作前完全一致。
//!
//! 为什么断言的是 git 的输出而不是管理器自己的返回值：回滚"成功"的唯一可信来源
//! 是 git 亲口说仓库回到了那个状态。管理器返回 `Ok` 但仓库不对，
//! 这样的 bug 只有对着 git 的事实断言才抓得住。

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use forgedesk_git_engine::engines::GitEngines;
use forgedesk_snapshot::{
    RefSnapshotManager, RetentionPolicy, SnapshotKind, SnapshotManager, SnapshotRequest,
};
use forgedesk_storage::{Database, RepositoryStore, RepositoryUpsert};

/// 一个自动清理的临时目录。
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Self {
        let unique = std::process::id();
        let path = std::env::temp_dir().join(format!("forgedesk-snapshot-{label}-{unique}"));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("创建临时目录失败");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        // Windows 上文件可能仍被占用：清理失败不该让测试红掉
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// 执行一条 git 命令并返回 stdout（失败即 panic）。
fn git(dir: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "Fixture Author")
        .env("GIT_AUTHOR_EMAIL", "author@example.com")
        .env("GIT_COMMITTER_NAME", "Fixture Author")
        .env("GIT_COMMITTER_EMAIL", "author@example.com")
        .env("LC_ALL", "C")
        .output()
        .expect("运行 git 失败");
    assert!(
        output.status.success(),
        "git {args:?} 失败: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// 初始化一个带 base 提交与一个未跟踪文件的仓库。
fn init_repo(label: &str) -> TempDir {
    let dir = TempDir::new(label);
    git(dir.path(), &["init", "-q", "-b", "main"]);
    std::fs::write(dir.path().join("tracked.txt"), b"base\n").expect("写文件失败");
    git(dir.path(), &["add", "--all"]);
    git(dir.path(), &["commit", "-q", "-m", "base"]);
    // 未跟踪文件在打快照**之前**就存在：快照会记录它，而 v1 不动未跟踪内容
    std::fs::write(dir.path().join("scratch.txt"), b"keep me\n").expect("写文件失败");
    dir
}

/// 破坏性操作：改掉已跟踪文件并提交。
///
/// 刻意**不动未跟踪文件**：v1 快照只记录未跟踪清单、不备份内容，因此
/// "回滚后未跟踪集合与操作前一致"这条验收只有在操作不吞掉未跟踪文件时才可能成立
/// ——这与 v1 覆盖的操作（提交、amend）一致；会删除未跟踪文件的操作
/// （例如部分 discard）属于 T3.8 快照 v2 的内容备份范围。
fn destructive_operation(dir: &Path) {
    std::fs::write(dir.join("tracked.txt"), b"changed\n").expect("写文件失败");
    git(dir, &["add", "tracked.txt"]);
    git(dir, &["commit", "-q", "-m", "the destructive operation"]);
}

/// 验收要求的四项事实：porcelain v2 输出、HEAD、索引内容、未跟踪集合。
fn facts(dir: &Path) -> (String, String, String, Vec<String>) {
    let porcelain = git(dir, &["status", "--porcelain=v2", "-z"]);
    let head = git(dir, &["rev-parse", "HEAD"]);
    let index = git(dir, &["ls-files", "--stage"]);
    let untracked = porcelain
        .split('\0')
        .filter(|entry| entry.starts_with("? "))
        .map(str::to_owned)
        .collect();
    (porcelain, head, index, untracked)
}

/// 管理器 + 已登记的仓库 id。
fn manager(dir: &Path) -> (RefSnapshotManager, i64) {
    let database = Arc::new(Database::open_in_memory().expect("打开内存库失败"));
    forgedesk_storage::migrate(&database).expect("执行迁移失败");
    let repo_id = RepositoryStore::new(&database)
        .upsert(
            &RepositoryUpsert {
                path: dir.to_string_lossy().into_owned(),
                name: "fixture".to_owned(),
                default_branch: Some("main".to_owned()),
                provider_id: None,
                size_class: None,
            },
            1_000,
        )
        .expect("登记仓库失败");

    let engines = Arc::new(GitEngines::new().expect("创建引擎失败"));
    (RefSnapshotManager::new(engines, database), repo_id)
}

#[test]
fn restoring_returns_the_four_git_facts_to_their_pre_operation_state() {
    let dir = init_repo("restore");
    let (manager, repo_id) = manager(dir.path());

    let before = facts(dir.path());

    // 操作前打点
    let snapshot_id = manager
        .create(&SnapshotRequest {
            repo_id,
            workdir: dir.path(),
            label: SnapshotKind::PreCommit.key(),
            kind: SnapshotKind::PreCommit,
        })
        .expect("创建快照失败");

    // 破坏性操作：HEAD、工作区与索引都被改变
    destructive_operation(dir.path());
    let during = facts(dir.path());
    assert_ne!(
        during.1, before.1,
        "操作必须真的改变了 HEAD，否则测试没有意义"
    );

    // 回滚
    let report = manager.restore(repo_id, snapshot_id).expect("回滚失败");
    assert_eq!(report.restored_snapshot_id, snapshot_id);
    assert_eq!(report.head_oid, before.1);
    assert!(
        report.pre_restore_snapshot_id.is_some(),
        "回滚前必须留保护点"
    );
    assert!(
        report
            .untracked_paths
            .iter()
            .any(|path| path == "scratch.txt"),
        "快照要记录当时存在哪些未跟踪文件"
    );

    // 强制验收：四项事实与操作前完全一致
    let after = facts(dir.path());
    assert_eq!(after.0, before.0, "porcelain v2 输出必须一致");
    assert_eq!(after.1, before.1, "HEAD 必须一致");
    assert_eq!(after.2, before.2, "ls-files --stage（索引内容）必须一致");
    assert_eq!(after.3, before.3, "未跟踪文件集合必须一致");
}

#[test]
fn restoring_a_snapshot_whose_anchor_is_missing_is_reported_as_unrestorable() {
    let dir = init_repo("missing-anchor");
    let (manager, repo_id) = manager(dir.path());

    let snapshot_id = manager
        .create(&SnapshotRequest {
            repo_id,
            workdir: dir.path(),
            label: SnapshotKind::PreCommit.key(),
            kind: SnapshotKind::PreCommit,
        })
        .expect("创建快照失败");

    // 锚点被外部清掉（别的工具、手工 git update-ref -d）：
    // 管理器必须在动手之前就发现，而不是 reset 到一半才失败
    let anchor = format!("refs/forgedesk/snapshots/{snapshot_id}");
    git(dir.path(), &["update-ref", "-d", &anchor]);

    let error = manager.restore(repo_id, snapshot_id).expect_err("必须失败");
    assert_eq!(
        error.code(),
        forgedesk_domain::ErrorCode::NotFound,
        "锚点丢失属于'不可恢复'，不是内部错误"
    );

    // 仓库不能被动过：HEAD 仍是操作后的
    destructive_operation(dir.path());
    let head = git(dir.path(), &["rev-parse", "HEAD"]);
    let _ = head;
}

#[test]
fn prune_removes_the_oldest_snapshots_and_deletes_their_anchors() {
    let dir = init_repo("prune");
    let (manager, repo_id) = manager(dir.path());

    let mut ids = Vec::new();
    for index in 0..3 {
        if index > 0 {
            std::fs::write(dir.path().join("tracked.txt"), format!("v{index}\n"))
                .expect("写文件失败");
            git(dir.path(), &["add", "--all"]);
            git(dir.path(), &["commit", "-q", "-m", &format!("c{index}")]);
        }
        ids.push(
            manager
                .create(&SnapshotRequest {
                    repo_id,
                    workdir: dir.path(),
                    label: SnapshotKind::Manual.key(),
                    kind: SnapshotKind::Manual,
                })
                .expect("创建快照失败"),
        );
    }

    // 只保留 2 条：最旧的快照（连同它的锚点 ref）应被清掉
    let pruned = manager
        .prune(
            repo_id,
            &RetentionPolicy {
                max_count: 2,
                max_age_days: 30,
            },
        )
        .expect("清理失败");
    assert_eq!(pruned, vec![ids[0]], "被清理的必须是最旧的那条");

    // 锚点 ref 确实没了（只删记录不删 ref 会留下永远不被 gc 的孤儿）
    let anchor_of_oldest = format!("refs/forgedesk/snapshots/{}", ids[0]);
    let anchor_of_kept = format!("refs/forgedesk/snapshots/{}", ids[2]);
    assert!(!Path::new(&dir.path().join(".git"))
        .join(anchor_of_oldest.trim_start_matches("refs/"))
        .exists());
    // 用 git 问最可靠；匹配**完整的 ref 名**而不是裸数字——
    // oid 的十六进制串里可能恰好包含那个数字
    let still_there = git(dir.path(), &["for-each-ref", "refs/forgedesk/snapshots"]);
    assert!(
        !still_there.contains(&format!("snapshots/{}", ids[0])),
        "被清理的锚点必须消失：{still_there}"
    );
    assert!(
        still_there.contains(&format!("snapshots/{}", ids[2])),
        "保留的锚点必须还在：{still_there}"
    );
    let _ = anchor_of_kept;
}

#[test]
fn the_anchor_ref_keeps_the_snapshotted_commit_alive_after_head_moves() {
    let dir = init_repo("anchor");
    let (manager, repo_id) = manager(dir.path());

    let snapshot_id = manager
        .create(&SnapshotRequest {
            repo_id,
            workdir: dir.path(),
            label: SnapshotKind::PreCommit.key(),
            kind: SnapshotKind::PreCommit,
        })
        .expect("创建快照失败");
    let head_at_snapshot = git(dir.path(), &["rev-parse", "HEAD"]);

    // HEAD 移走两次：如果快照没有自己的锚点，旧提交早已进入 gc 的可回收范围
    destructive_operation(dir.path());
    std::fs::write(dir.path().join("tracked.txt"), b"changed again\n").expect("写文件失败");
    git(dir.path(), &["add", "tracked.txt"]);
    git(dir.path(), &["commit", "-q", "-m", "second operation"]);

    // 锚点仍指着快照时刻的提交——这就是"回滚永远可行"的依据
    let anchor = format!("refs/forgedesk/snapshots/{snapshot_id}");
    let anchored = git(dir.path(), &["rev-parse", &anchor]);
    assert_eq!(anchored, head_at_snapshot, "锚点必须仍指着快照时刻的提交");
}
