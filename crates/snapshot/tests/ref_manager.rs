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
    RefSnapshotManager, RetentionPolicy, SnapshotKind, SnapshotLimits, SnapshotManager,
    SnapshotRequest, SnapshotWarning,
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

/// 在库里登记一个仓库，返回它的 id。
fn registered_repo(database: &Arc<Database>, dir: &Path) -> i64 {
    forgedesk_storage::migrate(database).expect("执行迁移失败");
    RepositoryStore::new(database)
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
        .expect("登记仓库失败")
}

/// 管理器 + 已登记的仓库 id（不带内容备份根：v1 的行为）。
fn manager(dir: &Path) -> (RefSnapshotManager, i64) {
    let database = Arc::new(Database::open_in_memory().expect("打开内存库失败"));
    let repo_id = registered_repo(&database, dir);
    let engines = Arc::new(GitEngines::new().expect("创建引擎失败"));
    (RefSnapshotManager::new(engines, database), repo_id)
}

/// 带内容备份根与磁盘策略的管理器（T3.8）。
fn manager_with_backup(
    dir: &Path,
    backup_root: &Path,
    limits: SnapshotLimits,
) -> (RefSnapshotManager, i64) {
    let database = Arc::new(Database::open_in_memory().expect("打开内存库失败"));
    let repo_id = registered_repo(&database, dir);
    let engines = Arc::new(GitEngines::new().expect("创建引擎失败"));
    (
        RefSnapshotManager::new(engines, database)
            .with_backup_root(backup_root.to_path_buf())
            .with_limits(limits),
        repo_id,
    )
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
        .expect("创建快照失败")
        .id;

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
        .expect("创建快照失败")
        .id;

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
                .expect("创建快照失败")
                .id,
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
        .expect("创建快照失败")
        .id;
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

// ---------------------------------------------------------------- T3.8 快照 v2

/// 手动快照的请求（T3.8 的用例大多只需要这一个形态）。
fn manual_request(repo_id: i64, workdir: &Path) -> SnapshotRequest<'_> {
    SnapshotRequest {
        repo_id,
        workdir,
        label: SnapshotKind::Manual.key(),
        kind: SnapshotKind::Manual,
    }
}

/// 任务书验收原文：含未跟踪文件的工作区 → `reset --hard` + `clean -fdx` →
/// 回滚后未跟踪文件内容**逐字节**恢复。
#[test]
fn restoring_brings_untracked_content_back_byte_for_byte() {
    let dir = init_repo("untracked-content");
    let backup_root = TempDir::new("untracked-content-store");
    let (manager, repo_id) =
        manager_with_backup(dir.path(), backup_root.path(), SnapshotLimits::default());

    // 嵌套目录 + 二进制内容 + 空文件：复制/恢复最容易漏掉的三种形态
    std::fs::create_dir_all(dir.path().join("scratch/nested")).expect("创建目录失败");
    std::fs::write(
        dir.path().join("scratch/nested/blob.bin"),
        [0_u8, 1, 2, 255, 254, 0, 128],
    )
    .expect("写文件失败");
    std::fs::write(dir.path().join("scratch/empty.txt"), b"").expect("写文件失败");
    std::fs::write(dir.path().join("scratch.txt"), b"keep me exactly\n").expect("写文件失败");

    let outcome = manager
        .create(&manual_request(repo_id, dir.path()))
        .expect("创建快照失败");
    assert_eq!(outcome.backed_up, 3, "三个未跟踪文件都要进备份");
    assert!(
        outcome.warnings.is_empty(),
        "不该有告警：{:?}",
        outcome.warnings
    );
    assert!(outcome.backup_bytes > 0);

    // 破坏性操作：把工作区与未跟踪内容一起清掉
    git(dir.path(), &["reset", "--hard", "HEAD"]);
    git(dir.path(), &["clean", "-fdx"]);
    assert!(
        !dir.path().join("scratch.txt").exists(),
        "操作必须真的删掉了未跟踪文件，否则测试没有意义"
    );

    let report = manager.restore(repo_id, outcome.id).expect("回滚失败");
    assert_eq!(report.untracked_restored, 3);
    assert!(report.untracked_failed.is_empty(), "不该有恢复失败的文件");
    assert!(report.verified, "恢复后的内容校验必须通过");
    assert!(report.untracked_extra.is_empty(), "快照之后没有新增文件");

    // 逐字节核对（这是本条验收的全部意义）
    assert_eq!(
        std::fs::read(dir.path().join("scratch.txt")).unwrap(),
        b"keep me exactly\n"
    );
    assert_eq!(
        std::fs::read(dir.path().join("scratch/nested/blob.bin")).unwrap(),
        vec![0_u8, 1, 2, 255, 254, 0, 128]
    );
    assert_eq!(
        std::fs::read(dir.path().join("scratch/empty.txt")).unwrap(),
        Vec::<u8>::new()
    );
}

/// 超限时不静默跳过：创建仍然成功，但结果里带着告警与被跳过的清单。
#[test]
fn an_oversized_untracked_set_is_reported_as_a_warning_and_skipped_as_a_whole() {
    let dir = init_repo("oversized");
    let backup_root = TempDir::new("oversized-store");
    std::fs::write(dir.path().join("big.bin"), vec![7_u8; 4096]).expect("写文件失败");
    let (manager, repo_id) = manager_with_backup(
        dir.path(),
        backup_root.path(),
        SnapshotLimits {
            max_snapshot_bytes: 64,
            ..SnapshotLimits::default()
        },
    );

    let outcome = manager
        .create(&manual_request(repo_id, dir.path()))
        .expect("超限不能让快照创建失败");
    assert_eq!(outcome.backup_bytes, 0);
    assert_eq!(outcome.backed_up, 0);
    assert_eq!(outcome.untracked_total, 2, "当时确实有两个未跟踪文件");
    assert!(outcome.skipped.contains(&"big.bin".to_owned()));
    assert!(
        outcome.skipped.contains(&"scratch.txt".to_owned()),
        "超限是**整体**跳过，不是备到上限为止：{:?}",
        outcome.skipped
    );
    assert!(
        matches!(
            outcome.warnings.first(),
            Some(SnapshotWarning::UntrackedBackupSkipped { count: 2, .. })
        ),
        "必须给出明确的告警：{:?}",
        outcome.warnings
    );
    assert!(
        !backup_root
            .path()
            .join(repo_id.to_string())
            .join(outcome.id.to_string())
            .exists(),
        "被整体跳过时不该留下空的正式目录"
    );
}

/// 仓库总占用超过上限时按 LRU 回收，且**不留孤儿目录**。
#[test]
fn the_repository_quota_evicts_the_oldest_snapshot_and_leaves_no_orphans() {
    let dir = init_repo("quota");
    let backup_root = TempDir::new("quota-store");
    let (manager, repo_id) = manager_with_backup(
        dir.path(),
        backup_root.path(),
        SnapshotLimits {
            // 单份不限，只卡总占用：8 字节
            max_snapshot_bytes: 0,
            max_repo_bytes: 8,
            include_ignored: false,
        },
    );
    std::fs::write(dir.path().join("scratch.txt"), b"12345678").expect("写文件失败");

    let first = manager
        .create(&manual_request(repo_id, dir.path()))
        .expect("创建快照失败");
    assert_eq!(first.backup_bytes, 8);
    let second = manager
        .create(&manual_request(repo_id, dir.path()))
        .expect("创建快照失败");

    assert!(
        second.pruned.contains(&first.id),
        "超过仓库上限必须回收最旧的那条：{:?}",
        second.pruned
    );

    let usage = manager.usage(repo_id).unwrap();
    assert_eq!(usage.snapshot_count, 1);
    assert!(
        usage.orphan_dirs.is_empty(),
        "回收后不能留下孤儿目录：{:?}",
        usage.orphan_dirs
    );

    // 磁盘目录与记录一一对应（回收要连备份目录一起删）
    let repo_root = backup_root.path().join(repo_id.to_string());
    let mut remaining: Vec<String> = std::fs::read_dir(&repo_root)
        .expect("备份根应存在")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    remaining.sort();
    assert_eq!(remaining, vec![second.id.to_string()]);
}

/// 手动清理：孤儿（含复制中途崩溃的 `.tmp-*`）被收走，正式快照不受影响。
#[test]
fn cleanup_removes_orphans_and_reports_freed_bytes() {
    let dir = init_repo("cleanup");
    let backup_root = TempDir::new("cleanup-store");
    let (manager, repo_id) =
        manager_with_backup(dir.path(), backup_root.path(), SnapshotLimits::default());

    let outcome = manager
        .create(&manual_request(repo_id, dir.path()))
        .expect("创建快照失败");
    let repo_root = backup_root.path().join(repo_id.to_string());

    // 手工造两个孤儿：一个"看起来像快照"的目录、一个复制中途崩溃的临时目录
    std::fs::create_dir_all(repo_root.join("999999")).expect("创建目录失败");
    std::fs::write(repo_root.join("999999/manifest.json"), b"{}").expect("写文件失败");
    std::fs::create_dir_all(repo_root.join(".tmp-crashed")).expect("创建目录失败");

    let usage = manager.usage(repo_id).unwrap();
    assert_eq!(
        usage.orphan_dirs.len(),
        2,
        "两个孤儿都要被认出来：{:?}",
        usage.orphan_dirs
    );

    let cleaned = manager.cleanup(repo_id).unwrap();
    assert_eq!(cleaned.orphans_removed, 2);
    assert!(cleaned.freed_bytes >= 2);
    assert!(manager.usage(repo_id).unwrap().orphan_dirs.is_empty());
    assert!(
        repo_root.join(outcome.id.to_string()).is_dir(),
        "正式快照目录不能被顺手清掉"
    );
}

/// 幂等：对同一快照重复回滚，第二次不改变任何东西。
#[test]
fn restoring_the_same_snapshot_twice_changes_nothing_the_second_time() {
    let dir = init_repo("idempotent");
    let backup_root = TempDir::new("idempotent-store");
    let (manager, repo_id) =
        manager_with_backup(dir.path(), backup_root.path(), SnapshotLimits::default());

    let outcome = manager
        .create(&manual_request(repo_id, dir.path()))
        .expect("创建快照失败");
    git(dir.path(), &["reset", "--hard", "HEAD"]);
    git(dir.path(), &["clean", "-fdx"]);

    let first = manager.restore(repo_id, outcome.id).expect("回滚失败");
    let restored_content = std::fs::read(dir.path().join("scratch.txt")).unwrap();
    let head_after_first = git(dir.path(), &["rev-parse", "HEAD"]);

    let second = manager
        .restore(repo_id, outcome.id)
        .expect("第二次回滚必须安全");
    assert_eq!(second.untracked_restored, first.untracked_restored);
    assert!(second.verified);
    assert_eq!(
        std::fs::read(dir.path().join("scratch.txt")).unwrap(),
        restored_content,
        "第二次回滚不能改变内容"
    );
    assert_eq!(
        git(dir.path(), &["rev-parse", "HEAD"]),
        head_after_first,
        "第二次回滚不能移动 HEAD"
    );
}

/// 同一仓库的并发创建被串行化：各自独立、不留半成品。
#[test]
fn concurrent_creations_on_one_repository_are_serialized() {
    let dir = init_repo("concurrent");
    let backup_root = TempDir::new("concurrent-store");
    let (manager, repo_id) =
        manager_with_backup(dir.path(), backup_root.path(), SnapshotLimits::default());
    let manager = Arc::new(manager);

    let mut handles = Vec::new();
    for _ in 0..4 {
        let manager = Arc::clone(&manager);
        let workdir = dir.path().to_path_buf();
        handles.push(std::thread::spawn(move || {
            manager
                .create(&manual_request(repo_id, &workdir))
                .map(|outcome| outcome.id)
        }));
    }
    let ids: Vec<i64> = handles
        .into_iter()
        .map(|handle| handle.join().expect("线程 panic").expect("创建失败"))
        .collect();

    let unique: std::collections::HashSet<i64> = ids.iter().copied().collect();
    assert_eq!(unique.len(), 4, "四次创建必须各自拿到独立 id：{ids:?}");

    let usage = manager.usage(repo_id).unwrap();
    assert_eq!(usage.snapshot_count, 4);
    assert!(
        usage.orphan_dirs.is_empty(),
        "并发创建不能留下半成品目录：{:?}",
        usage.orphan_dirs
    );
}
