//! 破坏性操作安全测试矩阵（T3.11 / `docs/PLAN.md` §10.6 的 15 个场景）。
//!
//! # 这套测试钉住什么
//!
//! 每个破坏性操作都要回答同一个问题：**"动手之后还能不能回到动手之前？"**
//! 本文件逐条构造独立仓库，执行操作，然后断言"可回滚且回滚后逐字节一致"。
//! "逐字节"不是修辞——断言工具 [`Fixture::assert_repo_equals`] 同时比较
//! 仓库指纹（[`RepoFingerprint`]，任务书点名的判据）与**工作区全部文件的内容**
//! （路径 → 字节，含未跟踪文件）、索引 stage 清单、分支引用、stash 栈与
//! `git status`。任何一样对不上，测试就红。
//!
//! # 场景 → 用例 对照表
//!
//! | # | 场景（PLAN §10.6） | 用例 |
//! | --- | --- | --- |
//! | 1 | `reset --hard HEAD~1`（已提交+已暂存+未跟踪） | `reset_hard_brings_back_committed_staged_and_untracked_content` |
//! | 2 | `reset --mixed`（已暂存） | `reset_mixed_restores_the_index` |
//! | 3 | rebase 中途 abort | `aborting_a_conflicting_rebase_returns_to_the_pre_rebase_state` |
//! | 4 | rebase 中途失败（hook 拒绝）无残留 | `a_hook_rejected_rebase_leaves_no_residue_and_stays_rollbackable` |
//! | 5 | `checkout -f` 丢弃修改 | `force_checkout_discards_modifications_and_the_snapshot_brings_them_back` |
//! | 6 | `clean -fdx`（未跟踪删除） | `discarding_untracked_files_is_reversible_byte_for_byte` |
//! | 7 | `stash drop` | `dropping_a_stash_is_reversible_through_the_snapshot` |
//! | 8 | `push --force-with-lease` 拒绝 | `a_stale_force_with_lease_push_is_refused_and_the_remote_is_untouched` |
//! | 9 | `branch -D` 未合并分支 | `a_deleted_unmerged_branch_is_brought_back_by_the_snapshot` |
//! | 10 | cherry-pick 冲突后 abort | `aborting_a_conflicting_cherry_pick_restores_the_state` |
//! | 11 | 删除 worktree 的不可恢复边界 | `removing_a_linked_worktree_is_outside_the_snapshot_boundary` |
//! | 12 | 回滚中途崩溃 → 重启后仍可回滚 | `a_restore_killed_mid_flight_is_still_rollbackable_after_a_restart` |
//! | 13 | 磁盘空间不足 → 提前拒绝不产生半成品 | `an_oversized_snapshot_leaves_no_half_products_behind` |
//! | 14 | 仓库只读 → 明确错误且不损坏 | `a_read_only_worktree_fails_loudly_and_damages_nothing` |
//! | 15 | 外部进程并发写 → 指纹校验发现 | `an_external_write_is_detected_and_the_stale_plan_is_refused` |
//!
//! # 接线的说明（诚实条款）
//!
//! 快照的**调用点**分布在两层：服务层（reset / cherry-pick / rebase / 切换分支…）
//! 与命令层（`workspace_discard` / `git_stash_drop`——`crates/commands/src` 里
//! 动手前调 [`forgedesk_snapshot::SnapshotManager::create`]）。Tauri 命令体无法在
//! 集成测试里构造（`State<'_, AppState>` / `AppHandle`），因此命令层打点的场景
//! （第 6、7 条）在这里按**命令体的同一顺序**重建接线：先快照、后动手、id 进结果。
//! "命令体没忘打点"这一半由 `crates/commands/tests/write_ops_safety_net.rs`
//! 的关联断言与 E2E 兜底；本文件钉的是**回滚语义**。
//!
//! # gc 的边界（如实记录，不做假承诺）
//!
//! 第 7、9 条依赖"被 drop 的 stash 提交 / 被删分支的提交仍在对象库里"。
//! `git gc` 默认只回收 **2 周**前的不可达对象，而快照时刻它们刚被记录，
//! 所以正常窗口内必然可恢复；超过窗口（或 `gc --prune=now`）后恢复会失败，
//! 那时回滚报告的 `stash_failed` / `branches_failed` 会如实列出——不会假装成功。
//! 把这两类引用也锚进 `refs/forgedesk/` 是后续工作（见任务回报的"风险与后续"）。
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use forgedesk_domain::git::{
    compare_fingerprints, BranchDeleteSpec, CherryPickSpec, DiscardSpec, FetchSpec, PushSpec,
    RebaseOutcome, RebasePlan, ReorderAction, ReorderStep, RepoFingerprint, RepoPath, ResetMode,
    ResetPlan, ResetSpec, StashSpec, SwitchStrategy,
};
use forgedesk_domain::ErrorCode;
use forgedesk_git_engine::engine::progress::ProgressSink;
use forgedesk_git_engine::engines::GitEngines;
use forgedesk_services::{
    BranchService, ConflictService, HistoryOpsService, OpenRepoRegistry, RebaseService,
    RepositoryService, ResetPlanRegistry, StashService, SyncService, WorkspaceService,
};
use forgedesk_snapshot::{
    BackupManifest, BranchRef, RefSnapshotManager, RestoreOutcomeKind, RestoreReport, RestoreStage,
    RetentionPolicy, SnapshotKind, SnapshotLimits, SnapshotManager, SnapshotRequest,
    SnapshotWarning, StashedRef,
};
use forgedesk_storage::{migrate, Database, RepositoryStore};
use tokio_util::sync::CancellationToken;

use support::{file_url, git, git_ok, init_repo, write, TempDir};

// ================================================================ 断言工具

/// 仓库的**字节级**状态：回滚前后必须完全一致的东西，逐项摊开。
///
/// 指纹回答"变了没有"，这份快照回答"到底哪里不一样"——排查失败时后者省下的
/// 时间远多于构造它的成本。
#[derive(Debug, Clone, PartialEq, Eq)]
struct RepoBytes {
    /// [`RepoFingerprint`]（任务书点名的统一判据）。
    fingerprint: RepoFingerprint,
    /// HEAD oid。
    head: Option<String>,
    /// `git ls-files --stage`（含 stage 号：冲突态也在里面）。
    index: String,
    /// `git status --porcelain=v1`。
    status: String,
    /// 本地分支引用（`name oid`，按名字排序）。
    branches: Vec<String>,
    /// stash 栈（`git stash list` 原文，新的在前）。
    stash: Vec<String>,
    /// 工作区全部文件（`.git` 除外）的内容。
    files: BTreeMap<String, Vec<u8>>,
}

/// 采集一份字节级状态。
fn repo_bytes(dir: &Path, fingerprint: RepoFingerprint) -> RepoBytes {
    let head = {
        let output = git(dir, &["rev-parse", "--verify", "--quiet", "HEAD"]);
        output
            .success()
            .then(|| output.stdout_lossy().trim().to_owned())
    };
    let text = |args: &[&str]| git(dir, args).stdout_lossy().to_string();

    RepoBytes {
        fingerprint,
        head,
        index: text(&["ls-files", "--stage"]),
        status: text(&["status", "--porcelain=v1"]),
        branches: text(&[
            "for-each-ref",
            "refs/heads",
            "--format=%(refname:short) %(objectname)",
        ])
        .lines()
        .map(str::to_owned)
        .collect(),
        stash: text(&["stash", "list"])
            .lines()
            .map(str::to_owned)
            .collect(),
        files: walk_files(dir, ".git"),
    }
}

/// 递归收集目录下的全部文件（跳过 `skip`），路径用正斜杠、按名字排序。
fn walk_files(root: &Path, skip: &str) -> BTreeMap<String, Vec<u8>> {
    let mut files = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let relative = path
                .strip_prefix(root)
                .expect("子路径必在根下")
                .to_string_lossy()
                .replace('\\', "/");
            if relative == skip {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(content) = std::fs::read(&path) {
                files.insert(relative, content);
            }
        }
    }
    files
}

// ================================================================ 夹具

/// 一个场景一份夹具：独立仓库、独立备份根、独立数据库文件。
///
/// 数据库刻意用**文件**而不是内存库：第 12 条要跨进程读同一份库
/// （"崩溃后重启"在同一个进程里是演不出来的）。
struct Fixture {
    /// 仓库工作区（临时目录，`Drop` 时清理）。
    repo: TempDir,
    /// 应用侧目录：数据库与快照内容备份都放在这里（不能放进仓库，
    /// 否则它们会变成"未跟踪文件"，污染指纹与备份）。
    home: TempDir,
    /// 数据库文件路径（第 12 条的子进程要打开它）。
    db_path: PathBuf,
    repo_id: i64,
    engines: &'static GitEngines,
    database: &'static Database,
    manager: &'static RefSnapshotManager,
    plans: &'static ResetPlanRegistry,
    open: &'static OpenRepoRegistry,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let repo = TempDir::new(&format!("matrix-{label}"));
        init_repo(repo.path());
        let home = TempDir::new(&format!("matrix-{label}-home"));
        let db_path = home.path().join("app.sqlite");
        let backup_root = home.path().join("snapshots");

        // 与 write_ops_safety_net.rs 同一技巧：先泄漏 Arc（'static），
        // 再从它派生 &'static 引用给服务用（Arc 与引用指向同一份对象）。
        let engines_arc: &'static Arc<GitEngines> =
            Box::leak(Box::new(Arc::new(GitEngines::new().expect("创建引擎失败"))));
        let database_arc: &'static Arc<Database> =
            Box::leak(Box::new(Arc::new(open_database(&db_path))));
        let manager: &'static RefSnapshotManager = Box::leak(Box::new(
            RefSnapshotManager::new(Arc::clone(engines_arc), Arc::clone(database_arc))
                .with_backup_root(backup_root),
        ));

        let repo_id = RepositoryService::new(
            engines_arc,
            RepositoryStore::new(database_arc),
            &OpenRepoRegistry::new(),
        )
        .open(repo.path())
        .expect("登记仓库失败")
        .record_id;

        Self {
            repo,
            home,
            db_path,
            repo_id,
            engines: engines_arc,
            database: database_arc,
            manager,
            plans: Box::leak(Box::new(ResetPlanRegistry::new())),
            open: Box::leak(Box::new(OpenRepoRegistry::new())),
        }
    }

    fn workdir(&self) -> &Path {
        self.repo.path()
    }

    fn store(&self) -> RepositoryStore<'static> {
        RepositoryStore::new(self.database)
    }

    // ---------------------------------------------------------- 服务（与
    // commands/src 里的构造一致：engines + store + 快照管理器）

    fn history_ops(&self) -> HistoryOpsService<'static> {
        HistoryOpsService::new(self.engines, self.store(), self.manager, self.plans)
    }

    fn stash(&self) -> StashService<'static> {
        StashService::new(self.engines, self.store(), self.manager)
    }

    fn branch(&self) -> BranchService<'static> {
        BranchService::new(self.engines, self.store(), self.manager)
    }

    fn conflict(&self) -> ConflictService<'static> {
        ConflictService::new(self.engines, self.store(), self.manager)
    }

    fn rebase(&self) -> RebaseService<'static> {
        RebaseService::new(self.engines, self.store(), self.manager)
    }

    fn workspace(&self) -> WorkspaceService<'static> {
        WorkspaceService::new(self.engines, self.store(), self.open)
    }

    fn sync(&self) -> SyncService<'static> {
        SyncService::new(self.engines, self.store(), self.manager)
    }

    // ---------------------------------------------------------- 状态与断言

    fn fingerprint(&self) -> RepoFingerprint {
        self.manager.fingerprint(self.repo_id).expect("读指纹失败")
    }

    fn bytes(&self) -> RepoBytes {
        repo_bytes(self.workdir(), self.fingerprint())
    }

    /// 按命令层/服务层共用的形状打一个快照，返回 id。
    fn snapshot(&self, kind: SnapshotKind) -> i64 {
        self.manager
            .create(&SnapshotRequest {
                repo_id: self.repo_id,
                workdir: self.workdir(),
                label: kind.key(),
                kind,
            })
            .expect("创建快照失败")
            .id
    }

    /// 任务书点名的统一断言：`assert_repo_equals(fingerprint_before, …)`。
    ///
    /// 指纹先比（它就是为"同一性"设计的），再比字节级快照——后者把
    /// "哪儿不一样"直接摊在断言消息里。
    #[track_caller]
    fn assert_repo_equals(&self, before: &RepoBytes) {
        let after = self.bytes();
        let diff = compare_fingerprints(&before.fingerprint, &after.fingerprint);
        assert!(
            diff.is_identical(),
            "回滚后仓库指纹与操作前不一致：{:?}（HEAD {:?} → {:?}，status {:?} → {:?}）",
            diff.changed_fields(),
            before.head,
            after.head,
            before.status,
            after.status
        );
        assert_eq!(
            after.head, before.head,
            "HEAD oid 必须与操作前一致（快照记录 {:?}）",
            before.head
        );
        assert_eq!(
            after.index, before.index,
            "索引（stage 清单）必须逐字节一致"
        );
        assert_eq!(after.status, before.status, "git status 必须一致");
        assert_eq!(after.branches, before.branches, "本地分支引用必须一致");
        assert_eq!(after.stash, before.stash, "stash 栈必须一致");
        assert_eq!(
            after.files,
            before.files,
            "工作区文件必须逐字节一致（{} 个文件）",
            before.files.len()
        );
    }

    /// 恢复到快照并断言"结局是 Completed、校验通过"。
    #[track_caller]
    fn restore_verified(&self, snapshot_id: i64) -> RestoreReport {
        let report = self
            .manager
            .restore(self.repo_id, snapshot_id)
            .expect("回滚失败");
        assert_eq!(
            report.outcome,
            RestoreOutcomeKind::Completed,
            "回滚必须完整成功：{:?}",
            report.report_lines
        );
        assert!(report.verified, "回滚后的校验必须通过");
        report
    }

    /// `reset --hard` 的服务层接线（prepare → execute，硬模式要确认词）。
    fn reset_hard_to(&self, revision: &str) {
        let history = self.history_ops();
        let plan = history
            .reset_prepare(self.repo_id, &ResetSpec::to(revision, ResetMode::Hard))
            .expect("计划失败");
        history
            .reset_execute(
                self.repo_id,
                &plan.plan_id,
                Some(ResetPlan::CONFIRMATION_WORD),
            )
            .expect("执行失败");
    }
}

/// 打开（或重新打开）一个文件数据库并完成迁移。
fn open_database(path: &Path) -> Database {
    let database = Database::open(path).expect("打开数据库失败");
    migrate(&database).expect("迁移失败");
    database
}

/// 给"另一个连接/另一个进程"用的引擎组合（引擎本身无状态，随用随建）。
fn fresh_engines() -> GitEngines {
    GitEngines::new().expect("创建引擎失败")
}

// ================================================================ 场景 1

/// # 1 `reset --hard HEAD~1`（有已提交 + 已暂存 + 未跟踪）
///
/// 最经典的破坏性操作，也是快照安全网的**全量**考题：它一口气动 HEAD、索引、
/// 工作区，还会把未跟踪文件一起带走。断言：三者全部恢复，且逐字节一致。
#[test]
fn reset_hard_brings_back_committed_staged_and_untracked_content() {
    let fixture = Fixture::new("reset-hard");
    let dir = fixture.workdir();

    // 底座：base 提交 + 两个后续提交（这样 HEAD~1 才存在）
    write(dir, "a.txt", b"base\n");
    support::commit_all(dir, "base");
    write(dir, "a.txt", b"second\n");
    support::commit_all(dir, "second");

    // 三种状态各占一样：已提交（第三个提交）、已暂存、未跟踪
    write(dir, "a.txt", b"third\n");
    support::commit_all(dir, "third");
    write(dir, "staged.txt", b"staged new file\n");
    git_ok(dir, &["add", "staged.txt"]);
    write(dir, "untracked.txt", b"never committed\n");

    let before = fixture.bytes();
    let snapshot = fixture.snapshot(SnapshotKind::PreHeadMove);
    // 动手**之前**记下 HEAD~1：reset 之后 "HEAD~1" 这个名字就指向更早的提交了
    let head_parent = git(dir, &["rev-parse", "HEAD~1"])
        .stdout_lossy()
        .trim()
        .to_owned();

    fixture.reset_hard_to("HEAD~1");
    let head_after = git(dir, &["rev-parse", "HEAD"])
        .stdout_lossy()
        .trim()
        .to_owned();
    assert_ne!(head_after, before.head.clone().expect("快照时刻必有 HEAD"));
    assert_eq!(head_after, head_parent, "HEAD 应该已经回到原来的 HEAD~1");

    // 回滚：已提交、已暂存、未跟踪三者全部回来
    fixture.restore_verified(snapshot);
    fixture.assert_repo_equals(&before);
    assert_eq!(
        fixture.bytes().files["staged.txt"],
        b"staged new file\n".to_vec(),
        "已暂存的新文件必须回来（工作区内容）"
    );
}

// ================================================================ 场景 2

/// # 2 `reset --mixed`（有已暂存内容）
///
/// `--mixed` 只动索引不动工作区，最容易让人以为"没什么可丢的"——
/// 丢的恰恰是"我已经 add 了"这件事。断言：索引回到快照时刻。
#[test]
fn reset_mixed_restores_the_index() {
    let fixture = Fixture::new("reset-mixed");
    let dir = fixture.workdir();

    write(dir, "a.txt", b"base\n");
    support::commit_all(dir, "base");

    // 暂存一个新文件（工作区与索引同内容），这就是 --mixed 会丢的东西
    write(dir, "new.txt", b"staged new\n");
    git_ok(dir, &["add", "new.txt"]);

    let before = fixture.bytes();
    assert!(
        before.status.lines().any(|line| line.starts_with("A ")),
        "夹具必须真的有已暂存条目，否则这条断言是空转：{:?}",
        before.status
    );
    let snapshot = fixture.snapshot(SnapshotKind::PreHeadMove);

    let history = fixture.history_ops();
    let plan = history
        .reset_prepare(fixture.repo_id, &ResetSpec::to("HEAD", ResetMode::Mixed))
        .expect("计划失败");
    history
        .reset_execute(fixture.repo_id, &plan.plan_id, None)
        .expect("执行失败");
    assert!(
        !fixture
            .bytes()
            .status
            .lines()
            .any(|line| line.starts_with("A ")),
        "mixed 之后索引里不该再有已暂存条目"
    );

    fixture.restore_verified(snapshot);
    fixture.assert_repo_equals(&before);
}

// ================================================================ 场景 3

/// # 3 rebase 中途 abort（有冲突）
///
/// 冲突暂停是**结果不是错误**；用户点"中止"之后必须回到 rebase 之前。
#[test]
fn aborting_a_conflicting_rebase_returns_to_the_pre_rebase_state() {
    let fixture = Fixture::new("rebase-abort");
    let dir = fixture.workdir();

    // main：base → 改同一行；feature：从 base 分叉 → f1 → f2。
    // rebase 的目标是 **main 的顶端**：feature 的两条提交与 main 改同一行，必然冲突。
    write(dir, "a.txt", b"base\n");
    support::commit_all(dir, "base");
    let fork = git(dir, &["rev-parse", "HEAD"])
        .stdout_lossy()
        .trim()
        .to_owned();
    write(dir, "a.txt", b"main line\n");
    support::commit_all(dir, "main change");
    let base = git(dir, &["rev-parse", "HEAD"])
        .stdout_lossy()
        .trim()
        .to_owned();
    git_ok(dir, &["checkout", "-q", "-b", "feature", &fork]);
    write(dir, "a.txt", b"feature line one\n");
    support::commit_all(dir, "f1");
    write(dir, "a.txt", b"feature line two\n");
    support::commit_all(dir, "f2");
    let head = git(dir, &["rev-parse", "HEAD"])
        .stdout_lossy()
        .trim()
        .to_owned();
    let one = git(dir, &["rev-parse", "HEAD~1"])
        .stdout_lossy()
        .trim()
        .to_owned();

    let before = fixture.bytes();
    let snapshot = fixture.snapshot(SnapshotKind::PreHeadMove);

    let plan = RebasePlan {
        base: base.clone(),
        head: head.clone(),
        steps: vec![
            ReorderStep {
                oid: one,
                action: ReorderAction::Pick,
                new_message: None,
            },
            ReorderStep {
                oid: head.clone(),
                action: ReorderAction::Pick,
                new_message: None,
            },
        ],
        allow_flatten_merges: false,
        autosquash: false,
    };
    let (outcome, _snapshot) = fixture
        .rebase()
        .execute(fixture.repo_id, &plan)
        .expect("执行失败");
    assert!(
        matches!(outcome, RebaseOutcome::PausedConflict { .. }),
        "rebase 应停在冲突上，实际 {outcome:?}"
    );

    // 中止走产品的冲突服务（它自己会先打快照）
    fixture
        .conflict()
        .abort_operation(fixture.repo_id)
        .expect("中止失败");
    fixture.assert_repo_equals(&before);

    // 再走一遍快照回滚：两条退路都得通
    fixture.restore_verified(snapshot);
    fixture.assert_repo_equals(&before);
}

// ================================================================ 场景 4

/// # 4 rebase 中途失败（hook 拒绝）→ 无残留 `.git/rebase-merge`
///
/// 冲突暂停会留下 rebase-merge（那是给用户的现场），而**失败**不能：
/// 用户没有点过"暂停"，留下半套状态只会让人不知道仓库在什么情况里。
/// 产品的出口是冲突页的"中止并还原"。
#[test]
fn a_hook_rejected_rebase_leaves_no_residue_and_stays_rollbackable() {
    let fixture = Fixture::new("rebase-hook");
    let dir = fixture.workdir();

    write(dir, "a.txt", b"base\n");
    support::commit_all(dir, "base");
    let base = git(dir, &["rev-parse", "HEAD"])
        .stdout_lossy()
        .trim()
        .to_owned();
    git_ok(dir, &["checkout", "-q", "-b", "feature"]);
    write(dir, "a.txt", b"feature one\n");
    support::commit_all(dir, "f1");
    write(dir, "a.txt", b"feature two\n");
    support::commit_all(dir, "f2");
    let head = git(dir, &["rev-parse", "HEAD"])
        .stdout_lossy()
        .trim()
        .to_owned();
    let one = git(dir, &["rev-parse", "HEAD~1"])
        .stdout_lossy()
        .trim()
        .to_owned();

    // pre-rebase 钩子一律拒绝。
    // 为什么不是 pre-commit：git 的 merge 后端重放 `pick` 时走 cherry-pick 语义，
    // **不**经过用户的 commit 流程（pre-commit 根本不触发）；
    // pre-rebase 是 git 官方的"钩子拒绝变基"入口。本条钉的是
    // "钩子拒绝 → 明确错误 → 无残留 → 可回滚"这条链，
    // "中途停下留了现场"的清理由第 3 条的 abort 路径覆盖。
    install_rejecting_hook(dir);

    let before = fixture.bytes();
    let snapshot = fixture.snapshot(SnapshotKind::PreHeadMove);

    let plan = RebasePlan {
        base: base.clone(),
        head: head.clone(),
        steps: vec![
            ReorderStep {
                oid: one,
                action: ReorderAction::Pick,
                new_message: None,
            },
            ReorderStep {
                oid: head.clone(),
                action: ReorderAction::Pick,
                new_message: None,
            },
        ],
        allow_flatten_merges: false,
        autosquash: false,
    };
    let result = fixture.rebase().execute(fixture.repo_id, &plan);

    let rebase_merge = dir.join(".git").join("rebase-merge");
    if rebase_merge.exists() {
        // git 留了现场：产品必须能一键清掉（冲突服务的"中止并还原"）
        fixture
            .conflict()
            .abort_operation(fixture.repo_id)
            .expect("中止失败");
    } else {
        assert!(
            result.is_err(),
            "既没有现场也没有错误，说明 rebase 根本没跑起来：{result:?}"
        );
    }
    assert!(
        !rebase_merge.exists(),
        "rebase 失败之后不允许留下 .git/rebase-merge 残留"
    );
    fixture.assert_repo_equals(&before);

    // 可回滚：动手前的快照仍然能把仓库带回去
    fixture.restore_verified(snapshot);
    fixture.assert_repo_equals(&before);
}

/// 写一个"一律拒绝"的 pre-rebase 钩子（见用例内的说明：为什么不是 pre-commit）。
///
/// LF 换行 + shebang：Windows 上的 git 会用自带的 sh 跑它（不需要可执行位），
/// Linux/macOS 走权限位。两端都会让 `git rebase` 失败。
fn install_rejecting_hook(dir: &Path) {
    let hooks = dir.join(".git").join("hooks");
    std::fs::create_dir_all(&hooks).expect("创建 hooks 目录失败");
    std::fs::write(hooks.join("pre-rebase"), b"#!/bin/sh\nexit 1\n").expect("写钩子失败");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            hooks.join("pre-rebase"),
            std::fs::Permissions::from_mode(0o755),
        )
        .expect("设置钩子权限失败");
    }
}

// ================================================================ 场景 5

/// # 5 `checkout -f` 丢弃修改
///
/// 强制切换的目标就选**当前分支**：它的全部效果就是"把未提交修改丢掉"，
/// 与 `checkout -f` 的语义完全一致，也不把"换分支"这个无关变量引进来。
#[test]
fn force_checkout_discards_modifications_and_the_snapshot_brings_them_back() {
    let fixture = Fixture::new("checkout-force");
    let dir = fixture.workdir();

    write(dir, "a.txt", b"base\n");
    support::commit_all(dir, "base");
    let current = git(dir, &["rev-parse", "--abbrev-ref", "HEAD"])
        .stdout_lossy()
        .trim()
        .to_owned();

    write(dir, "a.txt", b"uncommitted work\n");
    write(dir, "untracked.txt", b"untracked\n");

    let before = fixture.bytes();
    let snapshot = fixture.snapshot(SnapshotKind::PreHeadMove);

    // Force 必须**显式确认**：不给确认就是明确的错误，而不是悄悄丢东西
    let refused =
        fixture
            .branch()
            .switch_by_id(fixture.repo_id, &current, SwitchStrategy::Force, false);
    assert!(refused.is_err(), "没有确认的 Force 切换必须被拒绝");
    assert_eq!(
        fixture.bytes().files["a.txt"],
        b"uncommitted work\n".to_vec(),
        "拒绝时不能丢东西"
    );

    fixture
        .branch()
        .switch_by_id(fixture.repo_id, &current, SwitchStrategy::Force, true)
        .expect("执行失败");
    assert_eq!(
        fixture.bytes().files["a.txt"],
        b"base\n".to_vec(),
        "checkout -f 之后修改应当已经消失"
    );

    fixture.restore_verified(snapshot);
    fixture.assert_repo_equals(&before);
}

// ================================================================ 场景 6

/// # 6 `clean -fdx`（未跟踪文件被删）
///
/// 产品的对应物是"放弃未跟踪文件"（`WorkspaceService::discard` 的 untracked 分支）：
/// git 从此再也找不回这些文件，快照的内容备份是**唯一**的退路。
/// 快照的调用点在命令层（`workspace_discard`），这里按命令体的顺序重建接线。
#[test]
fn discarding_untracked_files_is_reversible_byte_for_byte() {
    let fixture = Fixture::new("clean-fdx");
    let dir = fixture.workdir();

    write(dir, "a.txt", b"tracked\n");
    support::commit_all(dir, "base");
    write(dir, "a.txt", b"tracked modified\n");

    let untracked = b"build output that git cannot recover\n".to_vec();
    write(dir, "generated.log", &untracked);
    write(dir, "nested/thing.txt", b"untracked in a subdirectory\n");

    let before = fixture.bytes();
    // 命令层接线：workspace_discard 在动手前打 `PreWorktreeChange` 快照
    let snapshot = fixture.snapshot(SnapshotKind::PreWorktreeChange);

    fixture
        .workspace()
        .discard(
            fixture.repo_id,
            DiscardSpec {
                tracked: vec![RepoPath::from("a.txt")],
                untracked: vec![
                    RepoPath::from("generated.log"),
                    RepoPath::from("nested/thing.txt"),
                ],
            },
        )
        .expect("放弃失败");
    assert!(
        !dir.join("generated.log").exists(),
        "未跟踪文件应当已经被删掉"
    );

    let report = fixture.restore_verified(snapshot);
    assert_eq!(
        report.untracked_restored, 3,
        "三个文件都要写回来：两个未跟踪 + a.txt 的工作区修改（T3.11 起也进备份）：{:?}",
        report.report_lines
    );
    fixture.assert_repo_equals(&before);
    assert_eq!(
        std::fs::read(dir.join("generated.log")).expect("读回滚后的文件"),
        untracked,
        "未跟踪文件必须**逐字节**回来"
    );
}

// ================================================================ 场景 7

/// # 7 `stash drop`
///
/// drop 只删掉**栈里的条目**（reflog），stash 提交本身还在对象库里。
/// 快照从 T3.11 起记下栈上每一条的 oid，回滚用 `git stash store` 把它们
/// 重新登记——所以"丢掉的东西回不来"这件事不再成立。
#[test]
fn dropping_a_stash_is_reversible_through_the_snapshot() {
    let fixture = Fixture::new("stash-drop");
    let dir = fixture.workdir();

    write(dir, "a.txt", b"base\n");
    support::commit_all(dir, "base");

    // 存两条（信息不同，便于辨认顺序），然后 drop 掉栈顶
    write(dir, "a.txt", b"first\n");
    fixture
        .stash()
        .save(
            fixture.repo_id,
            &StashSpec::push(Some("first stash".to_owned())),
        )
        .expect("save 失败");
    write(dir, "a.txt", b"second\n");
    fixture
        .stash()
        .save(
            fixture.repo_id,
            &StashSpec::push(Some("second stash".to_owned())),
        )
        .expect("save 失败");

    let before = fixture.bytes();
    assert_eq!(before.stash.len(), 2, "夹具必须真的存了两条");
    let dropped_oid = git(dir, &["rev-parse", "stash@{0}"])
        .stdout_lossy()
        .trim()
        .to_owned();

    // 命令层接线：git_stash_drop 在动手前打 `PreWorktreeChange` 快照
    let snapshot = fixture.snapshot(SnapshotKind::PreWorktreeChange);
    let outcome = fixture
        .stash()
        .drop_one(fixture.repo_id, 0)
        .expect("drop 失败");
    assert_eq!(outcome.dropped.len(), 1);
    assert_eq!(outcome.dropped[0].oid, dropped_oid);
    assert_eq!(fixture.bytes().stash.len(), 1, "drop 之后栈上应该只剩一条");

    let report = fixture.restore_verified(snapshot);
    assert_eq!(
        report.stash_restored, 1,
        "回滚必须把被丢的那条放回栈：{:?}",
        report.report_lines
    );
    fixture.assert_repo_equals(&before);

    // 放回去的是"那条 stash"本身：oid 与信息必须都回来，顺序也不能乱
    let listed = git(dir, &["stash", "list", "--format=%H%x1f%gs"])
        .stdout_lossy()
        .to_string();
    let entries: Vec<(String, String)> = listed
        .lines()
        .filter_map(|line| {
            let (oid, message) = line.split_once('\u{1f}')?;
            Some((oid.to_owned(), message.to_owned()))
        })
        .collect();
    assert_eq!(entries.len(), 2, "栈上应该有两条");
    assert!(
        entries.iter().any(|(oid, _)| oid == &dropped_oid),
        "被丢的 stash oid 必须回到栈上，实际 {entries:?}"
    );
    assert!(
        entries[0].1.contains("second stash"),
        "栈顶应当还是 second stash（顺序不能颠倒），实际 {:?}",
        entries
    );
}

// ================================================================ 场景 8

/// # 8 `push --force-with-lease`：lease 不匹配时必须拒绝（不覆盖）
///
/// 红线 R7 只放行 `--force-with-lease`；而它的全部意义就在"lease 对不上就拒绝"。
/// 这里用真实 bare 远端 + 另一台"机器"（克隆）制造 lease 过期，断言：
/// 推送被拒、远端**一个字节都没变**；fetch 之后按用户意图覆盖是被允许的。
#[test]
fn a_stale_force_with_lease_push_is_refused_and_the_remote_is_untouched() {
    let fixture = Fixture::new("force-lease");
    let dir = fixture.workdir();

    // 远端：bare 仓库；本地 origin 指向它
    let remote = TempDir::new("matrix-force-lease-remote");
    git_ok(remote.path(), &["init", "--bare", "-q", "-b", "main", "."]);
    fixture
        .sync()
        .remote_add(fixture.repo_id, "origin", &file_url(remote.path()))
        .expect("添加远端失败");
    write(dir, "a.txt", b"base\n");
    support::commit_all(dir, "base");
    let pushed = fixture
        .sync()
        .push(
            fixture.repo_id,
            PushSpec::new().with_set_upstream(true),
            &ProgressSink::none(),
            &CancellationToken::new(),
        )
        .expect("首次 push 失败");
    assert!(
        pushed.rejections.is_empty(),
        "首次 push 不应被拒绝：{:?}",
        pushed.rejections
    );

    // "另一台机器"克隆远端并推了一个提交——本地的 origin/main 由此过期
    let other = TempDir::new("matrix-force-lease-other");
    git_ok(
        other.path(),
        &["clone", "-q", &file_url(remote.path()), "."],
    );
    git_ok(other.path(), &["config", "user.name", "Second Author"]);
    git_ok(
        other.path(),
        &["config", "user.email", "second@example.com"],
    );
    write(other.path(), "theirs.txt", b"someone else's commit\n");
    support::commit_all(other.path(), "theirs");
    git_ok(other.path(), &["push", "-q", "origin", "main"]);

    // 本地也提交一个，并且**不 fetch**（lease 因此是过期的）
    write(dir, "mine.txt", b"my rewrite\n");
    support::commit_all(dir, "mine");
    let remote_before = git(remote.path(), &["rev-parse", "main"])
        .stdout_lossy()
        .trim()
        .to_owned();

    let attempt = fixture.sync().push(
        fixture.repo_id,
        PushSpec::new().with_force_with_lease(true),
        &ProgressSink::none(),
        &CancellationToken::new(),
    );
    let rejected = match &attempt {
        Err(_) => true,
        Ok(outcome) => !outcome.rejections.is_empty(),
    };
    assert!(
        rejected,
        "lease 过期的 force-with-lease 必须被拒绝：{attempt:?}"
    );
    assert_eq!(
        git(remote.path(), &["rev-parse", "main"])
            .stdout_lossy()
            .trim(),
        remote_before,
        "被拒绝的推送不允许动远端一个字节"
    );

    // 先 fetch（租约刷新）之后，同一个推送才能按用户意图执行
    fixture
        .sync()
        .fetch(
            fixture.repo_id,
            FetchSpec::default(),
            &ProgressSink::none(),
            &CancellationToken::new(),
        )
        .expect("fetch 失败");
    let outcome = fixture
        .sync()
        .push(
            fixture.repo_id,
            PushSpec::new().with_force_with_lease(true),
            &ProgressSink::none(),
            &CancellationToken::new(),
        )
        .expect("刷新租约后的推送失败");
    assert!(
        outcome.rejections.is_empty(),
        "刷新租约后不应再被拒绝：{:?}",
        outcome.rejections
    );
}

// ================================================================ 场景 9

/// # 9 `branch -D` 未合并分支
///
/// 删分支**不动 HEAD、也不动工作区**，所以"reset --hard + read-tree"那套回滚
/// 对它什么都没做——快照从 T3.11 起记下本地分支引用，回滚把**缺失的**分支
/// 重新创建出来（已存在的分支一律不动）。
#[test]
fn a_deleted_unmerged_branch_is_brought_back_by_the_snapshot() {
    let fixture = Fixture::new("branch-delete");
    let dir = fixture.workdir();

    write(dir, "a.txt", b"base\n");
    support::commit_all(dir, "base");
    git_ok(dir, &["checkout", "-q", "-b", "feature"]);
    write(dir, "feature.txt", b"only on the branch\n");
    support::commit_all(dir, "feature work");
    let feature_tip = git(dir, &["rev-parse", "HEAD"])
        .stdout_lossy()
        .trim()
        .to_owned();
    git_ok(dir, &["checkout", "-q", "main"]);

    let before = fixture.bytes();
    assert!(
        before
            .branches
            .iter()
            .any(|line| line.starts_with("feature ")),
        "夹具必须真的有 feature 分支：{:?}",
        before.branches
    );
    let snapshot = fixture.snapshot(SnapshotKind::PreHeadMove);

    let outcome = fixture
        .branch()
        .branch_delete(
            fixture.repo_id,
            &BranchDeleteSpec {
                names: vec!["feature".to_owned()],
                force: true,
                also_delete_remote: false,
            },
            true,
        )
        .expect("删除失败");
    assert_eq!(outcome.deleted, vec!["feature".to_owned()]);
    assert!(
        fixture
            .bytes()
            .branches
            .iter()
            .all(|line| !line.starts_with("feature ")),
        "分支应当已经消失"
    );

    let report = fixture.restore_verified(snapshot);
    assert_eq!(
        report.branches_restored, 1,
        "回滚必须把被删的分支重新创建出来：{:?}",
        report.report_lines
    );
    fixture.assert_repo_equals(&before);
    // 分支指回原来的提交，内容也就回来了
    assert_eq!(
        git(dir, &["rev-parse", "feature"]).stdout_lossy().trim(),
        feature_tip,
        "恢复出来的分支必须指向原来的提交"
    );
    assert_eq!(
        git(dir, &["show", "feature:feature.txt"]).stdout_lossy(),
        "only on the branch\n",
        "分支上的提交内容必须原样"
    );
}

// ================================================================ 场景 10

/// # 10 cherry-pick 冲突后 abort
#[test]
fn aborting_a_conflicting_cherry_pick_restores_the_state() {
    let fixture = Fixture::new("cherry-abort");
    let dir = fixture.workdir();

    write(dir, "a.txt", b"base\n");
    support::commit_all(dir, "base");
    git_ok(dir, &["checkout", "-q", "-b", "side"]);
    write(dir, "a.txt", b"side line\n");
    support::commit_all(dir, "side change");
    let pick = git(dir, &["rev-parse", "HEAD"])
        .stdout_lossy()
        .trim()
        .to_owned();
    git_ok(dir, &["checkout", "-q", "main"]);
    write(dir, "a.txt", b"main line\n");
    support::commit_all(dir, "main change");
    let main_tip = git(dir, &["rev-parse", "HEAD"])
        .stdout_lossy()
        .trim()
        .to_owned();

    let before = fixture.bytes();
    let snapshot = fixture.snapshot(SnapshotKind::PreHeadMove);

    let outcome = fixture
        .history_ops()
        .cherry_pick(fixture.repo_id, &CherryPickSpec::new(pick))
        .expect("cherry-pick 失败");
    assert!(
        !outcome.conflicts.is_empty(),
        "夹具必须制造出冲突，实际 {outcome:?}"
    );
    assert_eq!(
        git(dir, &["rev-parse", "HEAD"]).stdout_lossy().trim(),
        main_tip,
        "冲突时 HEAD 不应已经移动（拣选停在冲突上）"
    );

    fixture
        .conflict()
        .abort_operation(fixture.repo_id)
        .expect("中止失败");
    fixture.assert_repo_equals(&before);
    fixture.restore_verified(snapshot);
    fixture.assert_repo_equals(&before);
}

// ================================================================ 场景 11

/// # 11 删除 worktree（明确提示不可恢复的边界）
///
/// ForgeDesk 不提供删除 worktree 的功能（M3 范围之外），但必须**知道并说清**
/// 这条边界：主仓库的快照**不覆盖**关联工作区——里面的未提交内容删了就是没了。
/// 本条断言的正是这条边界本身：
/// 1. 主仓库快照里没有关联工作区的任何文件；
/// 2. git 对"删一个脏的关联工作区"要求显式 `--force`（不可恢复的第一道闸）；
/// 3. `--force` 删掉之后，**已提交**内容可以从分支引用原样找回（目录可恢复），
///    而**未提交**内容没有任何退路（明确提示不可恢复的那一半）。
#[test]
fn removing_a_linked_worktree_is_outside_the_snapshot_boundary() {
    let fixture = Fixture::new("worktree");
    let dir = fixture.workdir();

    write(dir, "a.txt", b"base\n");
    support::commit_all(dir, "base");
    let linked = TempDir::new("matrix-worktree-linked");
    let linked_dir = linked.path().join("wt");
    git_ok(
        dir,
        &[
            "worktree",
            "add",
            "-q",
            linked_dir.to_string_lossy().as_ref(),
            "-b",
            "feature-wt",
        ],
    );

    // 关联工作区里的内容：已提交的 + 未提交的修改 + 未跟踪的
    write(&linked_dir, "committed.txt", b"committed in the worktree\n");
    support::commit_all(&linked_dir, "worktree commit");
    write(&linked_dir, "committed.txt", b"modified, never committed\n");
    write(
        &linked_dir,
        "untracked.txt",
        b"only lives in the linked worktree\n",
    );

    // 1) 主仓库的快照对关联工作区一无所知
    let snapshot = fixture.snapshot(SnapshotKind::Manual);
    let diff = fixture
        .manager
        .diff(fixture.repo_id, snapshot)
        .expect("读差异失败");
    let touches_worktree = |paths: &[String]| {
        paths
            .iter()
            .any(|path| path.starts_with("committed.txt") || path.starts_with("untracked.txt"))
    };
    assert!(
        !touches_worktree(&diff.untracked_restorable) && !touches_worktree(&diff.untracked_extra),
        "主仓库快照不得把关联工作区的文件当成自己的：{diff:?}"
    );

    // 2) 删脏的 worktree 必须显式 --force
    let refused = git(
        dir,
        &["worktree", "remove", linked_dir.to_string_lossy().as_ref()],
    );
    assert!(
        !refused.success(),
        "git 必须拒绝删除有未提交改动的关联工作区（这是不可恢复边界的第一道闸）"
    );

    // 3) --force 删掉之后：已提交内容可从分支引用找回，未提交内容没有退路
    git_ok(
        dir,
        &[
            "worktree",
            "remove",
            "--force",
            linked_dir.to_string_lossy().as_ref(),
        ],
    );
    assert!(!linked_dir.exists(), "--force 之后目录应当已经消失");

    git_ok(
        dir,
        &[
            "worktree",
            "add",
            "-q",
            linked_dir.to_string_lossy().as_ref(),
            "feature-wt",
        ],
    );
    assert_eq!(
        std::fs::read(linked_dir.join("committed.txt")).expect("读回建的 worktree"),
        b"committed in the worktree\n".to_vec(),
        "已提交的内容必须能从分支引用原样找回（目录可恢复的那一半）"
    );
    assert!(
        !linked_dir.join("untracked.txt").exists(),
        "未提交内容没有任何退路——这正是必须向用户明示的不可恢复边界"
    );

    // 主仓库本身不受影响，快照照常可回滚
    fixture.restore_verified(snapshot);
}

// ================================================================ 场景 12

/// # 12 回滚过程中崩溃 → 重启后仍可回滚（幂等）
///
/// 用**子进程**模拟：子进程注入"进入 `Untracked` 阶段前 `abort()`"，由父进程用
/// `cargo test` 的二进制重新拉起。进程当场消失（不跑 Drop、不跑回退），
/// 与用户 `kill -9` 的效果一致——"失败会清掉进度标记，崩溃不会"正是这两条
/// 路径的全部区别。重启（父进程用新连接 + 新管理器）后：
/// `pending_restore` 必须报告"上次没走完"，继续回滚必须成功且逐字节一致。
///
/// 场景选"回滚要写回被删掉的未跟踪文件"：HEAD 与索引阶段对这份破坏是 no-op
/// （破坏只删了未跟踪文件），所以"文件还没回来"就是回滚**确实停在了半路**的
/// 可观察证据。
#[test]
fn a_restore_killed_mid_flight_is_still_rollbackable_after_a_restart() {
    let fixture = Fixture::new("crash-restore");
    let dir = fixture.workdir();

    write(dir, "a.txt", b"base\n");
    support::commit_all(dir, "base");
    let content = b"untracked file that the destructive op deletes\n".to_vec();
    write(dir, "generated.log", &content);
    let snapshot = fixture.snapshot(SnapshotKind::PreWorktreeChange);
    let before = fixture.bytes();

    // 破坏：放弃（删除）未跟踪文件——HEAD 与索引不动，只有内容备份救得回它
    fixture
        .workspace()
        .discard(
            fixture.repo_id,
            DiscardSpec {
                tracked: Vec::new(),
                untracked: vec![RepoPath::from("generated.log")],
            },
        )
        .expect("放弃失败");
    assert!(!dir.join("generated.log").exists(), "破坏必须真的发生");

    // 子进程在 Untracked 阶段前自杀（HEAD / 索引阶段已完成）
    let status = std::process::Command::new(std::env::current_exe().expect("定位测试二进制失败"))
        .args([
            "--exact",
            "crash_child_aborts_before_the_untracked_stage",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("FORGEDESK_MATRIX_ROLE", "crash-child")
        .env("FORGEDESK_MATRIX_DB", &fixture.db_path)
        .env("FORGEDESK_MATRIX_REPO", dir.to_string_lossy().as_ref())
        .env("FORGEDESK_MATRIX_REPO_ID", fixture.repo_id.to_string())
        .env("FORGEDESK_MATRIX_SNAPSHOT", snapshot.to_string())
        .env("FORGEDESK_MATRIX_STAGE", "untracked")
        .status()
        .expect("启动子进程失败");
    assert!(!status.success(), "子进程应当以异常结束（注入的崩溃）");

    // 崩溃留下了现场：文件还没写回来（回滚确实停在了半路）
    assert!(
        !dir.join("generated.log").exists(),
        "未跟踪文件必须还没恢复——否则这条用例没有模拟到'中途'的崩溃"
    );

    // "重启"：新数据库连接 + 新管理器（不是复用父进程里的那份状态）
    let restarted = RefSnapshotManager::new(
        Arc::new(fresh_engines()),
        Arc::new(open_database(&fixture.db_path)),
    )
    .with_backup_root(fixture.home.path().join("snapshots"));

    let pending = restarted
        .pending_restore(fixture.repo_id)
        .expect("读进度标记失败");
    assert!(
        pending.is_some(),
        "重启后必须报告'上次回滚没走完'（进度标记留在库里）"
    );
    assert_eq!(
        pending.and_then(|pending| pending.stage),
        Some(RestoreStage::Untracked),
        "标记里应当写着停在了哪一步"
    );

    // 继续回滚：完整成功，文件逐字节回来
    let report = restarted
        .restore(fixture.repo_id, snapshot)
        .expect("继续回滚失败");
    assert_eq!(report.outcome, RestoreOutcomeKind::Completed);
    assert!(report.verified);
    assert_eq!(report.untracked_restored, 1, "文件必须在这一步写回来");
    fixture.assert_repo_equals(&before);
    assert_eq!(
        std::fs::read(dir.join("generated.log")).expect("读恢复后的文件"),
        content,
        "内容必须逐字节一致"
    );

    // 幂等：同一份快照再回滚一次，结果一致（栈与分支不许翻倍）
    let again = restarted
        .restore(fixture.repo_id, snapshot)
        .expect("第二次回滚失败");
    assert_eq!(again.outcome, RestoreOutcomeKind::Completed);
    assert_eq!(again.stash_restored, 0, "第二次回滚不该再放一遍 stash");
    assert_eq!(again.branches_restored, 0, "第二次回滚不该再建一遍分支");
    fixture.assert_repo_equals(&before);
    assert!(
        restarted
            .pending_restore(fixture.repo_id)
            .expect("读进度标记失败")
            .is_none(),
        "回滚完成后进度标记必须清掉"
    );
}

/// 第 12 条的**子进程**：由
/// `a_restore_killed_mid_flight_is_still_rollbackable_after_a_restart` 用测试
/// 二进制重新拉起，在指定阶段前 `abort()`。环境变量不在时立刻返回——
/// 所以它在父进程的正常执行里是一个空测试（0 断言）。
#[test]
fn crash_child_aborts_before_the_untracked_stage() {
    let Ok(role) = std::env::var("FORGEDESK_MATRIX_ROLE") else {
        return;
    };
    if role != "crash-child" {
        return;
    }
    let db_path = std::env::var("FORGEDESK_MATRIX_DB").expect("缺 DB 路径");
    let repo_id: i64 = std::env::var("FORGEDESK_MATRIX_REPO_ID")
        .expect("缺 repo_id")
        .parse()
        .expect("repo_id 不是数字");
    let snapshot_id: i64 = std::env::var("FORGEDESK_MATRIX_SNAPSHOT")
        .expect("缺快照 id")
        .parse()
        .expect("快照 id 不是数字");
    let stage = RestoreStage::from_key(&std::env::var("FORGEDESK_MATRIX_STAGE").expect("缺阶段"))
        .expect("阶段短名认不出");

    let database = open_database(Path::new(&db_path));
    let manager = RefSnapshotManager::new(Arc::new(fresh_engines()), Arc::new(database))
        .with_backup_root(
            Path::new(&db_path)
                .parent()
                .expect("数据库必有父目录")
                .join("snapshots"),
        );
    manager.inject_restore_crash(stage, 1);

    // abort() 不会返回：进程在这里消失，进度标记留在库里
    let _ = manager.restore(repo_id, snapshot_id);
    panic!("注入的崩溃没有发生（restore 正常返回了）");
}

// ================================================================ 场景 13

/// # 13 磁盘空间不足时创建快照 → 提前拒绝并提示，不产生半成品
///
/// 磁盘真满难以在测试里可靠模拟，用**配额**模拟同一条路径：单份快照的内容
/// 备份上限（默认 200 MiB）就是"这地方放不下"的判定点。要求：
/// 动手前 `estimate` 能预告（危险操作对话框的数据源）；动手后内容**整体**
/// 不备份 + 明确告警，磁盘上没有 `.tmp-*` 半成品，快照与回滚照常可用。
#[test]
fn an_oversized_snapshot_leaves_no_half_products_behind() {
    // 备份根的配额压到 64 字节：一个 100 字节的未跟踪文件就足以触发
    let fixture = Fixture::new("quota");
    let dir = fixture.workdir();

    write(dir, "a.txt", b"base\n");
    support::commit_all(dir, "base");
    let big = vec![b'x'; 100];
    write(dir, "big.bin", &big);

    let limited: &'static RefSnapshotManager = Box::leak(Box::new(
        RefSnapshotManager::new(
            Arc::new(fresh_engines()),
            Arc::new(open_database(&fixture.db_path)),
        )
        .with_backup_root(fixture.home.path().join("snapshots"))
        .with_limits(SnapshotLimits {
            max_snapshot_bytes: 64,
            ..SnapshotLimits::default()
        }),
    ));

    // 动手之前：estimate 必须能预告（"提前拒绝并提示"的数据源）
    let estimate = limited.estimate(fixture.repo_id).expect("预估失败");
    assert!(!estimate.within_limit, "100 字节超出 64 字节上限，必须预告");
    assert_eq!(estimate.would_skip, 1, "超限的文件数必须如实报告");
    assert_eq!(estimate.untracked_bytes, 100);

    let outcome = limited
        .create(&SnapshotRequest {
            repo_id: fixture.repo_id,
            workdir: dir,
            label: SnapshotKind::Manual.key(),
            kind: SnapshotKind::Manual,
        })
        .expect("超限时快照本身仍要创建（不阻断危险操作）");

    assert!(
        outcome.warnings.iter().any(|warning| matches!(
            warning,
            SnapshotWarning::UntrackedBackupSkipped {
                count: 1,
                bytes: 100,
                ..
            }
        )),
        "必须有一条'整体未备份'的告警：{:?}",
        outcome.warnings
    );
    assert_eq!(outcome.backed_up, 0, "超限的内容一个字节都不该备份");

    // 不产生半成品：备份根下没有 `.tmp-*`，也没有这个快照的目录
    let backup_root = fixture.home.path().join("snapshots");
    let leftovers = collect_dirs(&backup_root);
    assert!(
        leftovers.iter().all(|path| !path
            .to_string_lossy()
            .contains(&format!(".tmp-{}", std::process::id()))),
        "不允许留下复制到一半的临时目录：{leftovers:?}"
    );
    let expected_dir = backup_root
        .join(fixture.repo_id.to_string())
        .join(outcome.id.to_string());
    assert!(
        !expected_dir.exists(),
        "没有内容备份的快照不该有备份目录：{:?}",
        collect_dirs(&backup_root)
    );
    let usage = limited.usage(fixture.repo_id).expect("读占用失败");
    assert_eq!(usage.backup_bytes, 0, "没有内容备份就没有占用");

    // 快照本身照常可用：HEAD / 索引回得去，只是那个文件找不回来（如实报告）
    let before = fixture.bytes();
    write(dir, "a.txt", b"changed\n");
    // 刻意只 add a.txt：`git add --all` 会把 big.bin 一起提交，
    // 那样"超限未备份"的边界就被夹具自己抹掉了
    git_ok(dir, &["add", "a.txt"]);
    git_ok(dir, &["commit", "-q", "-m", "destroy"]);
    let report = limited
        .restore(fixture.repo_id, outcome.id)
        .expect("回滚失败");
    assert_eq!(report.outcome, RestoreOutcomeKind::Completed);
    fixture.assert_repo_equals(&before);
    assert_eq!(
        std::fs::read(dir.join("big.bin")).expect("读 big.bin"),
        big,
        "未跟踪文件从未被备份也从未被删，内容必须原样"
    );
}

/// 列出目录下的**子目录**（不存在时为空表）。
fn collect_dirs(root: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return dirs;
    };
    for entry in entries.flatten() {
        if entry.path().is_dir() {
            dirs.push(entry.path());
        }
    }
    dirs
}

// ================================================================ 场景 14

/// # 14 仓库只读（权限）→ 明确错误且不损坏
///
/// 跨平台地真正拒绝写入：Unix 用权限位（目录 0o555），Windows 用 `icacls`
/// 拒绝当前用户的写权限（只读属性对 git 无效——它会先清掉属性再写）。
/// 选"需要新建文件"的操作（切到一个带新文件的提交）：Unix 上目录不可写
/// 挡住创建，Windows 上 icacls 挡住创建，同一断言在两端都有意义。
///
/// `Drop` 时务必恢复权限，否则临时目录清不掉（夹具清理会静默失败）。
#[test]
fn a_read_only_worktree_fails_loudly_and_damages_nothing() {
    let fixture = Fixture::new("read-only");
    let dir = fixture.workdir();

    write(dir, "a.txt", b"base\n");
    support::commit_all(dir, "base");
    git_ok(dir, &["checkout", "-q", "-b", "feature"]);
    write(dir, "new.txt", b"only on the branch\n");
    support::commit_all(dir, "adds a file");
    git_ok(dir, &["checkout", "-q", "main"]);

    let before = fixture.bytes();
    let snapshot = fixture.snapshot(SnapshotKind::PreHeadMove);
    let guard = ReadOnlyGuard::apply(dir);

    // 动手：git 无法在工作区新建文件 → 明确的错误（不是 panic，也不是静默成功）
    let attempt =
        fixture
            .branch()
            .switch_by_id(fixture.repo_id, "feature", SwitchStrategy::Force, true);
    assert!(attempt.is_err(), "只读工作区必须让操作失败：{attempt:?}");
    assert!(!dir.join("new.txt").exists(), "失败的操作不许留下目标文件");

    // 关键断言：仓库没有被损坏（半个切换都不允许）。
    // 指纹要等权限恢复之后再读：只读目录连"读状态"都会被拒（libgit2 打不开目录），
    // 这本身就是"仓库不可用时一切操作都要失败"的一部分。
    drop(guard);
    let after = fixture.bytes();
    assert_eq!(
        after.head, before.head,
        "HEAD 不许动：写工作区失败时切换必须整体中止"
    );
    assert_eq!(
        after.files, before.files,
        "工作区必须原样：失败的操作不许留下半套文件"
    );
    assert_eq!(after.index, before.index, "索引必须原样");

    // 快照照常可用（安全网在失败时也有意义）
    fixture.restore_verified(snapshot);
    fixture.assert_repo_equals(&before);

    // 恢复可写之后，同一个操作就能成功（证明刚才只是权限问题，不是仓库坏了）
    fixture
        .branch()
        .switch_by_id(fixture.repo_id, "feature", SwitchStrategy::Force, true)
        .expect("恢复可写后应当成功");
}

/// 只读目录的 RAII 守卫：构造时拒绝写入，`Drop` 时恢复。
///
/// 两个平台的"拒绝"都必须是**整树**的，否则场景 14 的前提不成立：
/// Windows 的 `icacls /deny` 天然继承到子目录；Unix 的权限位**不递归**——
/// 只挡顶层会让 `.git/`（子目录，权限不变）照常可写，git 就能先移动 HEAD
/// 再在工作区文件上失败，"整体中止"的前提（什么都别动）就不成立了
/// （2026-10-07 CI 首次在 Linux 上跑出这个真实差异）。
struct ReadOnlyGuard {
    #[allow(dead_code)] // Unix 用权限位，不需要记主账号
    principal: Option<String>,
    /// 被"只读化"的目录清单（含根）；`Drop` 时逐个恢复。
    readonly_dirs: Vec<PathBuf>,
}

#[cfg(unix)]
fn collect_directories(root: &Path, out: &mut Vec<PathBuf>) {
    out.push(root.to_path_buf());
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_type().is_ok_and(|file_type| file_type.is_dir()) {
            collect_directories(&entry.path(), out);
        }
    }
}

impl ReadOnlyGuard {
    fn apply(path: &Path) -> Self {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut readonly_dirs = Vec::new();
            collect_directories(path, &mut readonly_dirs);
            for dir in &readonly_dirs {
                std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o555))
                    .expect("设置只读权限失败");
            }
            Self {
                principal: None,
                readonly_dirs,
            }
        }
        #[cfg(not(unix))]
        {
            let domain = std::env::var("USERDOMAIN").unwrap_or_default();
            let user = std::env::var("USERNAME").unwrap_or_default();
            let principal = format!("{domain}\\{user}");
            let output = std::process::Command::new("icacls")
                .arg(path)
                .args(["/deny", &format!("{principal}:(W)")])
                .output()
                .expect("运行 icacls 失败");
            assert!(
                output.status.success(),
                "icacls 拒绝写失败：{}",
                String::from_utf8_lossy(&output.stderr)
            );
            Self {
                principal: Some(principal),
                readonly_dirs: vec![path.to_path_buf()],
            }
        }
    }
}

impl Drop for ReadOnlyGuard {
    fn drop(&mut self) {
        // 恢复失败只能尽力而为：夹具的 TempDir 清理本来就容忍失败
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for dir in &self.readonly_dirs {
                let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755));
            }
        }
        #[cfg(not(unix))]
        {
            if let Some(principal) = &self.principal {
                // 根目录就是 readonly_dirs[0]（apply 时放入）
                let _ = std::process::Command::new("icacls")
                    .arg(&self.readonly_dirs[0])
                    .args(["/remove:d", principal])
                    .output();
            }
        }
    }
}

// ================================================================ 场景 15

/// # 15 外部进程并发写仓库 → 指纹校验发现并提示刷新
///
/// 用户绕过界面直接在终端操作是常态，不是异常。断言三件事：
/// 1. 指纹**能发现**外部改动（HEAD 与未跟踪集合两项都要能报）；
/// 2. 发现之后产品有明确的出口——旧计划被拒绝（`PlanStale`，提示重新预览），
///    而不是拿着过期的计划去动仓库；
/// 3. 外部改动不妨碍安全网：快照照常可回滚。
#[test]
fn an_external_write_is_detected_and_the_stale_plan_is_refused() {
    let fixture = Fixture::new("external-write");
    let dir = fixture.workdir();

    write(dir, "a.txt", b"base\n");
    support::commit_all(dir, "base");
    let snapshot = fixture.snapshot(SnapshotKind::PreHeadMove);
    let fingerprint_before = fixture.fingerprint();

    // "外部进程"：直接调 git（不经产品）
    write(dir, "external.txt", b"typed in a terminal\n");
    support::commit_all(dir, "external commit");

    // 1) 指纹发现
    let fingerprint_after = fixture.fingerprint();
    let diff = compare_fingerprints(&fingerprint_before, &fingerprint_after);
    assert!(!diff.is_identical(), "外部改动必须能被指纹发现");
    assert!(
        diff.head_changed,
        "HEAD 变了必须报出来：{:?}",
        diff.changed_fields()
    );

    // 2) 旧计划被拒（提示刷新）：计划生成之后 HEAD 又被外部进程动了一次
    let history = fixture.history_ops();
    let plan = history
        .reset_prepare(fixture.repo_id, &ResetSpec::to("HEAD", ResetMode::Mixed))
        .expect("计划失败");
    write(dir, "another-external.txt", b"again\n");
    support::commit_all(dir, "another external commit");
    let refused = history.reset_execute(fixture.repo_id, &plan.plan_id, None);
    let error = refused.expect_err("计划过期的执行必须被拒绝");
    assert_eq!(
        error.code,
        ErrorCode::PlanStale,
        "HEAD 变了之后旧计划必须以 PlanStale 拒绝：{error:?}"
    );

    // 2b) 不动 HEAD 的并发写同样要能报出来：外部进程只写了一个未提交文件，
    // 指纹在**未跟踪集合**上把它报出来（"提示刷新"的另一种形态）
    write(dir, "stray.txt", b"uncommitted external file\n");
    let fingerprint_final = fixture.fingerprint();
    let untracked_diff = compare_fingerprints(&fingerprint_after, &fingerprint_final);
    assert!(
        untracked_diff.untracked_changed,
        "未跟踪集合的变化必须能被指纹发现：{:?}",
        untracked_diff.changed_fields()
    );

    // 3) 安全网不失效：外部改动之后回滚照样完整
    let report = fixture.restore_verified(snapshot);
    assert_eq!(report.outcome, RestoreOutcomeKind::Completed);
    let after = fixture.bytes();
    assert_ne!(after.fingerprint, fingerprint_final, "回滚必须真的改变仓库");
    assert!(
        !dir.join("external.txt").exists(),
        "外部提交里的文件随回滚消失（它在快照时刻还不存在）"
    );
}

// ================================================================ 补充：清理策略

/// 保留策略与回滚的交互（PLAN §10.7"清理策略"）：被清理的快照不再可回滚，
/// 操作历史如果不知道这一点就会给出必然失败的按钮——`restorable` 是那道闸。
#[test]
fn pruned_snapshots_are_no_longer_reported_as_restorable() {
    let fixture = Fixture::new("prune");
    let dir = fixture.workdir();

    write(dir, "a.txt", b"base\n");
    support::commit_all(dir, "base");
    let snapshot = fixture.snapshot(SnapshotKind::Manual);
    assert_eq!(
        fixture
            .manager
            .restorable(fixture.repo_id, &[snapshot])
            .expect("查可回滚失败"),
        vec![snapshot]
    );

    // 把保留上限压到 0 条：prune 必须清掉它
    let policy = RetentionPolicy {
        max_count: 0,
        max_age_days: 30,
    };
    let keeper = RefSnapshotManager::new(
        Arc::new(fresh_engines()),
        Arc::new(open_database(&fixture.db_path)),
    )
    .with_backup_root(fixture.home.path().join("snapshots"));
    let pruned = keeper.prune(fixture.repo_id, &policy).expect("清理失败");
    assert_eq!(pruned, vec![snapshot], "超过保留上限的快照必须被清理");

    // 清理之后：记录与锚点一起没了——"还能不能回滚"必须如实回答"不能"，
    // 差异摘要则直接报"快照不存在"（UI 里它也不再出现，没有悬挂引用）
    let restorable = fixture
        .manager
        .restorable(fixture.repo_id, &[snapshot])
        .expect("查可回滚失败");
    assert!(
        restorable.is_empty(),
        "被清理的快照不得再报告为可回滚：{restorable:?}"
    );
    assert!(
        fixture.manager.diff(fixture.repo_id, snapshot).is_err(),
        "记录已删除：差异摘要必须报'快照不存在'"
    );
    assert!(
        fixture.manager.restore(fixture.repo_id, snapshot).is_err(),
        "记录没了的回滚必须明确失败，而不是假装成功"
    );
}

/// 清单（manifest）里引用类事实的**持久化**：第 7、9、12 条都依赖
/// "数据库里的清单 JSON 能在另一个进程里原样读回来"。
#[test]
fn the_manifest_round_trips_stash_and_branch_references() {
    let manifest = BackupManifest {
        entries: Vec::new(),
        bytes: 0,
        stash: vec![StashedRef {
            oid: "a".repeat(40),
            message: "WIP on main: 1a2b3c4 base".to_owned(),
        }],
        branches: vec![BranchRef {
            name: "feature".to_owned(),
            oid: "b".repeat(40),
        }],
    };
    assert_eq!(BackupManifest::from_json(&manifest.to_json()), manifest);
    // 旧版本（没有这两个字段）的清单必须还能读：空表而不是解析失败
    let legacy = BackupManifest::from_json("{\"entries\":[],\"bytes\":0}");
    assert!(legacy.stash.is_empty() && legacy.branches.is_empty());
}
