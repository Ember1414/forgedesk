//! 写操作的安全网关联断言（T2.10 第 3 条 / M2 验收第 6 条）。
//!
//! # 断言什么
//!
//! M2 验收要求"所有写操作（reset / cherry-pick / revert / stash drop）均有
//! 快照与审计记录"。这个测试把每个写操作按**命令层的真实接线**执行
//! （`record_with` 包住服务调用，与 `crates/commands/src` 里的命令体一致），
//! 然后遍历 `operation_records` 与 `snapshots` 两张表，逐条核对关联性：
//!
//! 1. 每个操作留下一条**已收尾**的审计记录（成功、退出码 0）；
//! 2. 结果里带 `snapshotId` 的操作（reset / cherry-pick / revert / stash
//!    save / apply / pop），审计记录的 `snapshot_id` 指向 snapshots 表里
//!    **真实存在**的一行，且 `reversible = true`——T2.10 修复前这一列恒为
//!    NULL，`reversible` 恒为 false，"能不能回滚"在操作历史里全是谎言；
//! 3. `stash_drop` / `stash_clear` **按设计不打快照**（工作区快照找不回
//!    stash 内容），它们的审计记录 `snapshot_id` 为 `None` 且 `reversible`
//!    为 `false`——但 args 里必须带被丢 oid（gc 前的自救线索）。
//!
//! # 为什么在命令层测
//!
//! 快照在 services 层打、审计在 commands 层记：两层各对一半，
//! 只有 `record_with` 这条接缝是"关联性"真正诞生的地方。
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::sync::Arc;

use forgedesk_commands::{record_with, ResetOutcomeDto, StashDiscardDto, StashSaveDto};
use forgedesk_domain::git::{
    CherryPickSpec, ResetMode, ResetSpec, RevertSpec, StashAction, StashSpec,
};
use forgedesk_services::audit::op_type;
use forgedesk_services::{
    AuditArgs, AuditEntry, AuditLog, GitEngines, HistoryOpsService, RepositoryService,
    ResetPlanRegistry, StashService,
};
use forgedesk_snapshot::RefSnapshotManager;
use forgedesk_storage::{
    migrate, Database, OperationQuery, OperationStore, RepositoryStore, SnapshotStore,
};

// ---------------------------------------------------------------- 工具

/// 进程内唯一的临时目录，`Drop` 时尽力删除（与其他 crate 的 support 同形）。
struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(prefix: &str) -> Self {
        static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let unique = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "forgedesk-cmd-{prefix}-{}-{unique}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("创建临时目录失败");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn init_repo(dir: &Path) {
    for args in [
        vec!["init", "-q", "-b", "main", "."],
        vec!["config", "user.name", "Fixture Author"],
        vec!["config", "user.email", "author@example.com"],
        vec!["config", "core.autocrlf", "false"],
    ] {
        let output = std::process::Command::new("git")
            .current_dir(dir)
            .args(&args)
            .output()
            .expect("git 运行失败");
        assert!(output.status.success(), "git {args:?} 失败");
    }
}

fn write(dir: &Path, relative: &str, content: &[u8]) {
    let path = dir.join(relative);
    std::fs::write(path, content).expect("写文件失败");
}

fn git_ok(dir: &Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("git 运行失败");
    assert!(output.status.success(), "git {args:?} 失败");
}

fn commit_at(dir: &Path, file: &str, content: &str, message: &str, stamp_seconds: i64) {
    write(dir, file, content.as_bytes());
    let stamp = format!("{stamp_seconds} +0000");
    let output = std::process::Command::new("git")
        .current_dir(dir)
        .args(["add", "-A"])
        .output()
        .expect("git 运行失败");
    assert!(output.status.success(), "git add 失败");
    let output = std::process::Command::new("git")
        .current_dir(dir)
        .args(["commit", "-q", "-m", message])
        .env("GIT_AUTHOR_DATE", &stamp)
        .env("GIT_COMMITTER_DATE", &stamp)
        .output()
        .expect("git 运行失败");
    assert!(output.status.success(), "commit {message} 失败");
}

struct Harness {
    _dir: TempDir,
    repo_id: i64,
    audit: AuditLog<'static>,
    history_ops: HistoryOpsService<'static>,
    stash: StashService<'static>,
    snapshots: &'static SnapshotStore<'static>,
    operations: &'static OperationStore<'static>,
    dir: std::path::PathBuf,
}

fn harness(label: &str) -> Harness {
    let dir = TempDir::new(label);
    let dir_path = dir.path().to_path_buf();
    init_repo(dir.path());

    // 内存库是**单连接**的：快照管理器与服务必须共享同一个实例，
    // 否则审计/快照各写各的库，"关联性"从根上不成立。
    // 先泄漏 Arc 本体（'static），再从它派生 &'static 引用给服务用。
    let database_arc: &'static Arc<Database> = Box::leak(Box::new(Arc::new({
        let database = Database::open_in_memory().expect("打开内存库失败");
        migrate(&database).expect("迁移失败");
        database
    })));
    let engines_arc: &'static Arc<GitEngines> =
        Box::leak(Box::new(Arc::new(GitEngines::new().expect("创建引擎失败"))));
    let engines_static: &'static GitEngines = engines_arc;
    let database_static: &'static Database = database_arc;

    let manager = RefSnapshotManager::new(Arc::clone(engines_arc), Arc::clone(database_arc));
    let snapshots_manager: &'static RefSnapshotManager = Box::leak(Box::new(manager));

    let plans: &'static ResetPlanRegistry = Box::leak(Box::new(ResetPlanRegistry::new()));

    {
        use forgedesk_services::repository::OpenRepoRegistry;
        let open = OpenRepoRegistry::new();
        let repository =
            RepositoryService::new(engines_static, RepositoryStore::new(database_static), &open);
        let opened = repository.open(&dir_path).expect("打开仓库失败");
        let repo_id = opened.record_id;

        let audit = AuditLog::new(OperationStore::new(database_static));
        let history_ops = HistoryOpsService::new(
            engines_static,
            RepositoryStore::new(database_static),
            snapshots_manager,
            plans,
        );
        let stash = StashService::new(
            engines_static,
            RepositoryStore::new(database_static),
            snapshots_manager,
        );

        Harness {
            _dir: dir,
            repo_id,
            audit,
            history_ops,
            stash,
            snapshots: Box::leak(Box::new(SnapshotStore::new(database_static))),
            operations: Box::leak(Box::new(OperationStore::new(database_static))),
            dir: dir_path,
        }
    }
}

impl Harness {
    /// 按命令层的接线执行一个操作，返回审计表里的主键。
    fn record<T: serde::Serialize>(
        &self,
        op_type: &str,
        args: AuditArgs,
        run: impl FnOnce() -> forgedesk_domain::AppResult<T>,
    ) -> i64 {
        let entry = AuditEntry::new(self.repo_id, op_type).with_args(args);
        let _ = record_with(&self.audit, entry, run).expect("操作应成功");
        let records = self.operations.recent(self.repo_id, 1).expect("读审计失败");
        records[0].id
    }

    fn latest_record(&self, op_type: &str) -> forgedesk_storage::OperationRecord {
        let page = self
            .operations
            .query_all(&OperationQuery {
                repo_id: Some(self.repo_id),
                op_type: Some(op_type.to_owned()),
                from_ms: None,
                to_ms: None,
                limit: 50,
                offset: 0,
            })
            .expect("读审计失败");
        assert!(!page.is_empty(), "op_type={op_type} 应至少有一条审计记录");
        page[0].clone()
    }

    fn snapshot_exists(&self, id: Option<i64>) -> bool {
        id.is_some_and(|id| self.snapshots.find(id).expect("查快照失败").is_some())
    }

    fn snapshot_count(&self) -> usize {
        self.snapshots
            .list(self.repo_id, 1000)
            .expect("列快照失败")
            .len()
    }
}

// ---------------------------------------------------------------- 断言

#[test]
fn every_snapshot_carrying_write_operation_links_to_a_real_snapshot_row() {
    let h = harness("safety-net-links");
    let dir = &h.dir;

    // 底座：两个提交 + 一条分支（cherry-pick 的素材）。
    // stash 的素材必须是**已跟踪**文件的改动（默认不带 -u）
    commit_at(dir, "base.txt", "base\n", "base", 1_700_000_000);
    commit_at(dir, "a.txt", "one\n", "c1", 1_700_000_060);
    commit_at(dir, "b.txt", "b\n", "c2", 1_700_000_090);
    git_ok(dir, &["checkout", "-q", "-b", "feature"]);
    commit_at(dir, "feature.txt", "f\n", "f1 on feature", 1_700_000_120);
    git_ok(dir, &["checkout", "-q", "main"]);

    // ① stash save：结果带 snapshotId → 审计关联到真实快照行
    write(dir, "a.txt", b"one-v2\n");
    let save_outcome = h
        .stash
        .save(h.repo_id, &stash_push_spec(None))
        .expect("stash save 失败");
    assert!(save_outcome.stashed);
    assert!(save_outcome.snapshot_id.is_some(), "stash save 应带快照 id");
    let record_id = h.record(
        op_type::STASH_SAVE,
        AuditArgs::new().text("scope", "test"),
        || Ok(StashSaveDto::from(save_outcome)),
    );
    let record = h.latest_record(op_type::STASH_SAVE);
    assert_eq!(record.id, record_id);
    assert!(
        h.snapshot_exists(record.snapshot_id),
        "save 的快照应真实存在"
    );
    assert!(record.reversible, "有快照即可回滚");

    // ② stash apply / pop：同上
    let apply_outcome = h
        .stash
        .apply(h.repo_id, &stash_apply_spec(0))
        .expect("stash apply 失败");
    assert!(apply_outcome.snapshot_id.is_some(), "apply 应带快照 id");
    let _ = h.record(op_type::STASH_APPLY, AuditArgs::new(), || Ok(apply_outcome));
    let record = h.latest_record(op_type::STASH_APPLY);
    assert!(h.snapshot_exists(record.snapshot_id));
    assert!(record.reversible);

    // 再存一条用于 pop：改**另一个**文件（b.txt 已跟踪），apply/pop 才不会与
    // 工作区里 a.txt 的改动相互纠缠
    write(dir, "b.txt", b"b-v2\n");
    let second = h
        .stash
        .save(h.repo_id, &stash_push_spec(None))
        .expect("stash save 失败");
    let _ = h.record(op_type::STASH_SAVE, AuditArgs::new(), || {
        Ok(StashSaveDto::from(second))
    });
    let pop_outcome = h
        .stash
        .pop(h.repo_id, &stash_pop_spec(0))
        .expect("stash pop 失败");
    assert!(pop_outcome.snapshot_id.is_some(), "pop 应带快照 id");
    let _ = h.record(
        op_type::STASH_APPLY,
        AuditArgs::new().text("action", "pop"),
        || Ok(pop_outcome),
    );

    // ③ cherry-pick：MergeOutcome 带快照 id
    let feature_tip = {
        let output = std::process::Command::new("git")
            .current_dir(dir)
            .args(["rev-parse", "feature"])
            .output()
            .expect("git 运行失败");
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    };
    let pick = h
        .history_ops
        .cherry_pick(
            h.repo_id,
            &CherryPickSpec {
                revision: feature_tip,
                record_source: false,
                no_commit: false,
            },
        )
        .expect("cherry-pick 失败");
    assert!(!pick.has_conflicts());
    assert!(pick.snapshot_id.is_some(), "cherry-pick 应带快照 id");
    let _ = h.record(op_type::CHERRY_PICK, AuditArgs::new(), || Ok(pick));
    let record = h.latest_record(op_type::CHERRY_PICK);
    assert!(h.snapshot_exists(record.snapshot_id));
    assert!(record.reversible);

    // ④ revert：反转刚拣选上来的提交
    let main_tip = {
        let output = std::process::Command::new("git")
            .current_dir(dir)
            .args(["rev-parse", "main"])
            .output()
            .expect("git 运行失败");
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    };
    let revert = h
        .history_ops
        .revert(
            h.repo_id,
            &RevertSpec {
                revision: main_tip,
                mainline: None,
                no_commit: false,
            },
        )
        .expect("revert 失败");
    assert!(revert.snapshot_id.is_some(), "revert 应带快照 id");
    let _ = h.record(op_type::REVERT, AuditArgs::new(), || Ok(revert));
    let record = h.latest_record(op_type::REVERT);
    assert!(h.snapshot_exists(record.snapshot_id));
    assert!(record.reversible);

    // ⑤ reset（mixed，无需确认词）：两段式
    let plan = h
        .history_ops
        .reset_prepare(
            h.repo_id,
            &ResetSpec {
                revision: "HEAD~1".to_owned(),
                mode: ResetMode::Mixed,
                paths: Vec::new(),
            },
        )
        .expect("reset prepare 失败");
    let outcome = h
        .history_ops
        .reset_execute(h.repo_id, &plan.plan_id, None)
        .expect("reset execute 失败");
    assert!(outcome.snapshot_id.is_some(), "reset 应带快照 id");
    let _ = h.record(op_type::RESET, AuditArgs::new(), || {
        Ok(ResetOutcomeDto::from(outcome))
    });
    let record = h.latest_record(op_type::RESET);
    assert!(h.snapshot_exists(record.snapshot_id));
    assert!(record.reversible);

    // 总账：本仓库所有快照型操作的关联列都能在 snapshots 表里对上号
    let all = h.operations.recent(h.repo_id, 100).expect("读审计失败");
    assert!(all.len() >= 6);
    for record in &all {
        if record.snapshot_id.is_some() {
            assert!(
                h.snapshot_exists(record.snapshot_id),
                "审计记录 {} 指向的快照不存在",
                record.id
            );
        }
    }
}

#[test]
fn stash_drop_and_clear_have_audit_trail_but_deliberately_no_snapshot() {
    let h = harness("safety-net-drop");
    let dir = &h.dir;

    commit_at(dir, "base.txt", "base\n", "base", 1_700_000_000);

    // 存两条，然后逐个丢弃（改**已跟踪**的 base.txt：默认不带 -u 的 stash
    // 只储藏已跟踪改动）
    write(dir, "base.txt", b"one\n");
    let first = h
        .stash
        .save(h.repo_id, &stash_push_spec(None))
        .expect("save 失败");
    let _ = h.record(op_type::STASH_SAVE, AuditArgs::new(), || {
        Ok(StashSaveDto::from(first))
    });
    write(dir, "base.txt", b"two\n");
    let second = h
        .stash
        .save(h.repo_id, &stash_push_spec(None))
        .expect("save 失败");
    let _ = h.record(op_type::STASH_SAVE, AuditArgs::new(), || {
        Ok(StashSaveDto::from(second))
    });
    let snapshots_before = h.snapshot_count();

    // drop：不打快照（设计如此），但 args 里带被丢 oid
    let dropped = h.stash.drop_one(h.repo_id, 0).expect("drop 失败");
    assert_eq!(dropped.dropped.len(), 1);
    let dropped_oid = dropped.dropped[0].oid.clone();
    let _ = h.record(
        op_type::STASH_DROP,
        {
            let mut args = AuditArgs::new().number("index", 0).text("scope", "one");
            args = args.text("oid", &dropped_oid);
            args
        },
        || Ok(StashDiscardDto::from(dropped)),
    );
    let record = h.latest_record(op_type::STASH_DROP);
    assert_eq!(
        record.snapshot_id, None,
        "drop 按设计不打快照：工作区快照找不回 stash 内容"
    );
    assert!(!record.reversible);
    assert!(
        record
            .args_json
            .as_deref()
            .is_some_and(|args| args.contains(&dropped_oid)),
        "drop 的审计 args 必须带被丢 oid（gc 前的自救线索），实际 {:?}",
        record.args_json
    );
    assert_eq!(h.snapshot_count(), snapshots_before, "drop 不应产生新快照");

    // clear：同 drop，args 带被丢 oid 清单
    write(dir, "base.txt", b"three\n");
    let third = h
        .stash
        .save(h.repo_id, &stash_push_spec(None))
        .expect("save 失败");
    let _ = h.record(op_type::STASH_SAVE, AuditArgs::new(), || {
        Ok(StashSaveDto::from(third))
    });
    let cleared = h.stash.clear(h.repo_id).expect("clear 失败");
    // 2 次 save − 1 次 drop + 1 次再 save = clear 时还剩 2 条
    assert_eq!(cleared.dropped.len(), 2);
    let cleared_oid = cleared.dropped[0].oid.clone();
    let _ = h.record(
        op_type::STASH_DROP,
        {
            let mut args = AuditArgs::new().text("scope", "all");
            args = args.text("oid", &cleared_oid);
            args
        },
        || Ok(StashDiscardDto::from(cleared)),
    );
    let record = h.latest_record(op_type::STASH_DROP);
    assert_eq!(record.snapshot_id, None);
    assert!(
        record
            .args_json
            .as_deref()
            .is_some_and(|args| args.contains(&cleared_oid)),
        "clear 的审计 args 必须带被丢 oid，实际 {:?}",
        record.args_json
    );
}

#[test]
fn a_failed_operation_records_no_snapshot_link() {
    // 失败的操作不应留下快照关联：结果为 Err 时提取不到 snapshotId，
    // 审计记录如实地以 reversible=false 收尾
    let h = harness("safety-net-failure");
    let dir = &h.dir;
    commit_at(dir, "base.txt", "base\n", "base", 1_700_000_000);

    let _ = record_with(
        &h.audit,
        AuditEntry::new(h.repo_id, op_type::STASH_SAVE),
        || -> forgedesk_domain::AppResult<StashSaveDto> {
            // 对不存在的仓库执行 stash save：直接得到一个 Err
            h.stash
                .save(h.repo_id + 999, &stash_push_spec(None))
                .map(StashSaveDto::from)
        },
    );
    let record = h.latest_record(op_type::STASH_SAVE);
    assert_eq!(record.snapshot_id, None);
    assert!(!record.reversible);
    assert_eq!(record.exit_code, Some(1), "失败的操作应以非零退出码收尾");
}

// ---------------------------------------------------------------- spec 助手

fn stash_push_spec(message: Option<&str>) -> StashSpec {
    StashSpec {
        action: StashAction::Push,
        message: message.map(str::to_owned),
        include_untracked: false,
        keep_index: false,
        paths: Vec::new(),
        restore_index: false,
    }
}

fn stash_apply_spec(index: usize) -> StashSpec {
    StashSpec {
        action: StashAction::Apply { index },
        message: None,
        include_untracked: false,
        keep_index: false,
        paths: Vec::new(),
        restore_index: false,
    }
}

fn stash_pop_spec(index: usize) -> StashSpec {
    StashSpec {
        action: StashAction::Pop { index },
        message: None,
        include_untracked: false,
        keep_index: false,
        paths: Vec::new(),
        restore_index: false,
    }
}
