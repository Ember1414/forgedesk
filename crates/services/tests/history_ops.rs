//! 历史操作（T2.8）的集成测试：储藏 / 拣选 / 反转 / 重置 / reflog，全部在真实仓库上跑。
//!
//! # 本文件钉住的行为
//!
//! 1. **stash 的三父结构**：`-u` 创建的 stash，未跟踪文件在第三个父提交里，
//!    "相对 base 的 diff"**不包含**它们。这是任务书点名的易错点。
//! 2. **冲突是结果不是错误**：应用储藏 / 拣选冲突时，仓库进入冲突状态并给出
//!    冲突文件清单（而不是一句"命令失败"）。
//! 3. **重置计划**：将被丢弃的提交数、已暂存 / 工作区改动、**远端是否已有这些提交**、
//!    是否需要输入确认词。
//! 4. **重置执行的护栏**：`--hard` 必须输入确认词；HEAD 变过（或计划被用过一次）
//!    即 `PLAN_STALE`；执行前留下 `PreHeadMove` 快照。
//! 5. **reflog 恢复**：把丢掉的提交恢复成**新分支**（不动任何现有引用）。
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use forgedesk_domain::git::{
    CherryPickSpec, MergeKind, ResetMode, ResetSpec, RevertSpec, StashSpec,
};
use forgedesk_domain::ErrorCode;
use forgedesk_git_engine::engines::GitEngines;
use forgedesk_services::{HistoryOpsService, ResetPlanRegistry, StashService};
use forgedesk_snapshot::{
    RestoreReport, RetentionPolicy, SnapshotDiff, SnapshotError, SnapshotId, SnapshotKind,
    SnapshotManager, SnapshotMeta, SnapshotRequest,
};
use support::{commit_all, git, git_ok, init_repo, write, TempDir};

// ---------------------------------------------------------------- 夹具

/// 记录型快照管理器（与 sync.rs 同一写法：只关心"打没打、打的哪一类"）。
#[derive(Default)]
struct RecordingSnapshots {
    kinds: std::sync::Mutex<Vec<SnapshotKind>>,
}

impl std::fmt::Debug for RecordingSnapshots {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecordingSnapshots").finish()
    }
}

impl RecordingSnapshots {
    /// 已记录的快照类别短名，按发生顺序。
    fn keys(&self) -> Vec<&'static str> {
        self.kinds
            .lock()
            .unwrap()
            .iter()
            .map(|kind| kind.key())
            .collect()
    }
}

impl SnapshotManager for RecordingSnapshots {
    fn create(
        &self,
        request: &SnapshotRequest<'_>,
    ) -> Result<forgedesk_snapshot::SnapshotOutcome, SnapshotError> {
        self.kinds.lock().unwrap().push(request.kind);
        Ok(forgedesk_snapshot::SnapshotOutcome::bare(1))
    }

    fn list(&self, _repo_id: i64, _limit: i64) -> Result<Vec<SnapshotMeta>, SnapshotError> {
        Ok(Vec::new())
    }

    fn restore(
        &self,
        _repo_id: i64,
        _snapshot_id: SnapshotId,
    ) -> Result<RestoreReport, SnapshotError> {
        Err(SnapshotError::NotFound(0))
    }

    fn diff(&self, _repo_id: i64, _snapshot_id: SnapshotId) -> Result<SnapshotDiff, SnapshotError> {
        Err(SnapshotError::NotFound(0))
    }

    fn prune(
        &self,
        _repo_id: i64,
        _policy: &RetentionPolicy,
    ) -> Result<Vec<SnapshotId>, SnapshotError> {
        Ok(Vec::new())
    }

    fn estimate(
        &self,
        _repo_id: i64,
    ) -> Result<forgedesk_snapshot::SnapshotEstimate, SnapshotError> {
        Ok(forgedesk_snapshot::SnapshotEstimate::default())
    }

    fn usage(&self, _repo_id: i64) -> Result<forgedesk_snapshot::SnapshotUsage, SnapshotError> {
        Ok(forgedesk_snapshot::SnapshotUsage::default())
    }

    fn cleanup(&self, _repo_id: i64) -> Result<forgedesk_snapshot::CleanupOutcome, SnapshotError> {
        Ok(forgedesk_snapshot::CleanupOutcome::default())
    }
}

/// 借用生命周期的包装（与 branch.rs / sync.rs 的 `Box::leak` 同一技巧）。
mod forge_setup {
    use std::path::Path;

    use forgedesk_storage::{Database, RepositoryStore, RepositoryUpsert};

    pub struct LeakDatabase {
        pub inner: Database,
    }

    pub fn leak_database() -> LeakDatabase {
        let database = Database::open_in_memory().expect("打开内存库失败");
        forgedesk_storage::migrate(&database).expect("迁移失败");
        LeakDatabase { inner: database }
    }

    pub fn store(database: &'static LeakDatabase) -> RepositoryStore<'static> {
        RepositoryStore::new(&database.inner)
    }

    pub fn register(store: &RepositoryStore<'_>, path: &Path) -> i64 {
        store
            .upsert(
                &RepositoryUpsert {
                    name: "fixture".to_owned(),
                    path: path.to_string_lossy().to_string(),
                    default_branch: Some("main".to_owned()),
                    provider_id: None,
                    size_class: None,
                },
                0,
            )
            .expect("登记仓库失败")
    }
}

/// 一个已有一个提交的临时仓库 + 两个服务。
struct Fixture {
    dir: TempDir,
    repo_id: i64,
    engines: &'static GitEngines,
    database: &'static forge_setup::LeakDatabase,
    snapshots: &'static RecordingSnapshots,
    plans: &'static ResetPlanRegistry,
}

impl Fixture {
    /// 建仓库并提交一个 `a.txt`。
    fn new(prefix: &str) -> Self {
        let dir = TempDir::new(prefix);
        init_repo(dir.path());
        write(dir.path(), "a.txt", b"one\n");
        commit_all(dir.path(), "first");

        let engines: &'static GitEngines =
            Box::leak(Box::new(GitEngines::new().expect("创建引擎失败")));
        let database: &'static forge_setup::LeakDatabase =
            Box::leak(Box::new(forge_setup::leak_database()));
        let snapshots: &'static RecordingSnapshots =
            Box::leak(Box::new(RecordingSnapshots::default()));
        let plans: &'static ResetPlanRegistry = Box::leak(Box::new(ResetPlanRegistry::new()));
        let repo_id = forge_setup::register(&forge_setup::store(database), dir.path());

        Self {
            dir,
            repo_id,
            engines,
            database,
            snapshots,
            plans,
        }
    }

    fn path(&self) -> &std::path::Path {
        self.dir.path()
    }

    fn stash(&self) -> StashService<'_> {
        StashService::new(
            self.engines,
            forge_setup::store(self.database),
            self.snapshots,
        )
    }

    fn history(&self) -> HistoryOpsService<'_> {
        HistoryOpsService::new(
            self.engines,
            forge_setup::store(self.database),
            self.snapshots,
            self.plans,
        )
    }

    /// 当前 HEAD 的 oid。
    fn head(&self) -> String {
        git(self.path(), &["rev-parse", "HEAD"])
            .stdout_lossy()
            .trim()
            .to_owned()
    }

    /// 某个文件当前的内容。
    fn read(&self, relative: &str) -> String {
        std::fs::read_to_string(self.path().join(relative)).expect("读文件失败")
    }
}

// ---------------------------------------------------------------- stash

#[test]
fn a_stash_of_untracked_files_keeps_them_out_of_the_base_diff() {
    // 任务书点名的易错点：stash 是多父提交，相对 base 的 diff **不含**未跟踪文件
    let fixture = Fixture::new("stash-three-parents");
    write(fixture.path(), "a.txt", b"one changed\n");
    write(fixture.path(), "fresh.txt", b"brand new\n");

    let saved = fixture
        .stash()
        .save(
            fixture.repo_id,
            &StashSpec {
                include_untracked: true,
                message: Some("wip".to_owned()),
                ..StashSpec::push(None)
            },
        )
        .expect("储藏失败");

    assert!(saved.stashed);
    let entry = saved.entry.expect("应当有新的 stash");
    assert!(entry.includes_untracked);
    assert!(
        entry.untracked_oid.is_some(),
        "未跟踪文件的父提交 oid 必须被解析出来：{entry:?}"
    );

    let shown = fixture.stash().show(fixture.repo_id, 0).expect("show 失败");

    // 相对 base：只有已跟踪文件 a.txt 的改动
    let tracked: Vec<String> = shown
        .diff
        .files
        .iter()
        .map(|file| file.path.to_string())
        .collect();
    assert_eq!(tracked, vec!["a.txt".to_owned()], "{shown:?}");

    // 未跟踪文件在**另一份** diff 里（与空树比较 → 全是新增）
    let untracked = shown.untracked.expect("应当有未跟踪文件的 diff");
    let paths: Vec<String> = untracked
        .files
        .iter()
        .map(|file| file.path.to_string())
        .collect();
    assert_eq!(paths, vec!["fresh.txt".to_owned()], "{untracked:?}");

    // 储藏之后工作区是干净的（未跟踪文件也被带走了）
    assert!(!fixture.path().join("fresh.txt").exists());
    assert_eq!(fixture.read("a.txt"), "one\n");
    assert_eq!(
        fixture.snapshots.keys(),
        vec!["pre-worktree-change"],
        "储藏前必须打工作区快照（红线 R7）"
    );
}

#[test]
fn stashing_a_clean_worktree_reports_that_nothing_was_stashed() {
    let fixture = Fixture::new("stash-clean");

    let saved = fixture
        .stash()
        .save(fixture.repo_id, &StashSpec::push(None))
        .expect("干净工作区的储藏不应失败");

    assert!(!saved.stashed, "没有内容可储藏时不该假装成功：{saved:?}");
    assert!(saved.entry.is_none());
    assert!(fixture
        .stash()
        .list(fixture.repo_id)
        .expect("列表")
        .is_empty());
}

#[test]
fn applying_a_stash_over_a_newer_commit_reports_the_conflict_and_keeps_the_entry() {
    let fixture = Fixture::new("stash-conflict");

    // 在第一次提交的基础上改 a.txt 并储藏
    write(fixture.path(), "a.txt", b"stashed line\n");
    fixture
        .stash()
        .save(fixture.repo_id, &StashSpec::push(None))
        .expect("储藏失败");
    let stashed_oid = fixture.stash().list(fixture.repo_id).expect("列表")[0]
        .oid
        .clone();

    // 再让 HEAD 往前走，并且改同一行 → 应用必然冲突
    write(fixture.path(), "a.txt", b"committed line\n");
    commit_all(fixture.path(), "second");

    let outcome = fixture
        .stash()
        .apply(fixture.repo_id, &StashSpec::apply(0))
        .expect("冲突不该被压成错误");

    assert!(outcome.has_conflicts(), "{outcome:?}");
    assert_eq!(outcome.conflicts[0].to_string(), "a.txt");

    // `apply` 保留那条 stash；`pop` 冲突时 git 同样会保留，我们如实呈现
    let entries = fixture.stash().list(fixture.repo_id).expect("列表");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].oid, stashed_oid);

    // 仓库确实停在冲突状态（这是 M3 冲突向导的输入）
    let status = git(fixture.path(), &["status", "--porcelain=v2"]);
    assert!(
        status
            .stdout_lossy()
            .lines()
            .any(|line| line.starts_with("u ")),
        "索引里应当留下未合并条目：{}",
        status.stdout_lossy()
    );
}

#[test]
fn dropping_and_clearing_report_the_entries_that_disappeared() {
    let fixture = Fixture::new("stash-drop");

    write(fixture.path(), "a.txt", b"first stash\n");
    fixture
        .stash()
        .save(fixture.repo_id, &StashSpec::push(Some("first".to_owned())))
        .expect("储藏 1");
    write(fixture.path(), "a.txt", b"second stash\n");
    fixture
        .stash()
        .save(fixture.repo_id, &StashSpec::push(Some("second".to_owned())))
        .expect("储藏 2");

    let before = fixture.stash().list(fixture.repo_id).expect("列表");
    assert_eq!(before.len(), 2);

    // 丢最新那条（index 0）
    let dropped = fixture.stash().drop_one(fixture.repo_id, 0).expect("drop");
    assert_eq!(dropped.dropped.len(), 1);
    assert_eq!(dropped.dropped[0].oid, before[0].oid);
    assert_eq!(
        fixture.stash().list(fixture.repo_id).expect("列表").len(),
        1
    );

    // 一次清空，并把被清掉的条目交出来（审计与自救都靠这些 oid）
    let cleared = fixture.stash().clear(fixture.repo_id).expect("clear");
    assert_eq!(cleared.dropped.len(), 1);
    assert!(fixture
        .stash()
        .list(fixture.repo_id)
        .expect("列表")
        .is_empty());
}

#[test]
fn a_branch_can_be_created_from_a_stash_and_recovers_it_cleanly() {
    let fixture = Fixture::new("stash-branch");

    // 先让 base 变掉（应用会冲突），再从 stash 建分支 → 一定干净
    write(fixture.path(), "a.txt", b"stashed line\n");
    fixture
        .stash()
        .save(fixture.repo_id, &StashSpec::push(None))
        .expect("储藏");
    write(fixture.path(), "a.txt", b"committed line\n");
    commit_all(fixture.path(), "second");

    let base_before = git(fixture.path(), &["rev-parse", "HEAD~1"])
        .stdout_lossy()
        .trim()
        .to_owned();

    fixture
        .stash()
        .branch(fixture.repo_id, 0, "recover-wip")
        .expect("从 stash 建分支");

    // 新分支应当从 stash 的 base（第一次提交）开始，并已切过去
    let branch = git(fixture.path(), &["rev-parse", "--abbrev-ref", "HEAD"])
        .stdout_lossy()
        .trim()
        .to_owned();
    assert_eq!(branch, "recover-wip");
    assert_eq!(fixture.head(), base_before);
    // 储藏的内容已经应用到这里（不再留在工作区之外）
    assert_eq!(fixture.read("a.txt"), "stashed line\n");
    assert!(fixture
        .stash()
        .list(fixture.repo_id)
        .expect("列表")
        .is_empty());
    assert_eq!(
        fixture.snapshots.keys(),
        vec!["pre-worktree-change", "pre-head-move"],
        "建分支会移动 HEAD → 必须打 PreHeadMove 快照"
    );
}

#[test]
fn a_branch_name_is_validated_before_touching_the_repository() {
    let fixture = Fixture::new("stash-branch-invalid");

    let error = fixture
        .stash()
        .branch(fixture.repo_id, 0, "bad..name")
        .expect_err("非法分支名必须被拒");

    assert_eq!(error.code, ErrorCode::Validation);
    assert!(
        fixture.snapshots.keys().is_empty(),
        "校验失败时不该已经打了快照"
    );
}

// ---------------------------------------------------------------- 重置计划

#[test]
fn the_reset_plan_counts_what_will_be_discarded_and_requires_confirmation_for_hard() {
    let fixture = Fixture::new("reset-plan");
    write(fixture.path(), "b.txt", b"b\n");
    commit_all(fixture.path(), "second");
    write(fixture.path(), "c.txt", b"c\n");
    commit_all(fixture.path(), "third");

    // 再制造一份已暂存改动与一份工作区改动
    write(fixture.path(), "a.txt", b"staged change\n");
    git_ok(fixture.path(), &["add", "a.txt"]);
    write(fixture.path(), "b.txt", b"worktree change\n");

    let target_subject = "first";
    let plan = fixture
        .history()
        .reset_prepare(fixture.repo_id, &ResetSpec::to("HEAD~2", ResetMode::Hard))
        .expect("计划失败");

    assert_eq!(plan.mode, ResetMode::Hard);
    assert_eq!(plan.discarded_count, 2);
    assert_eq!(plan.discarded.len(), 2);
    assert!(!plan.discarded_truncated);
    assert_eq!(plan.discarded[0].subject, "third");
    assert_eq!(plan.target_subject, target_subject);
    assert!(plan.requires_confirmation, "--hard 必须要求输入确认词");
    assert!(plan.snapshot_required);

    let staged: Vec<String> = plan
        .lost_staged
        .iter()
        .map(|entry| entry.path.to_string())
        .collect();
    assert_eq!(staged, vec!["a.txt".to_owned()], "{plan:?}");
    let worktree: Vec<String> = plan
        .lost_worktree
        .iter()
        .map(|entry| entry.path.to_string())
        .collect();
    assert_eq!(worktree, vec!["b.txt".to_owned()], "{plan:?}");

    // 没有上游：这些提交只存在于本地
    assert!(plan.remote.upstream.is_none());
    assert_eq!(plan.remote.not_on_remote, 2);
}

#[test]
fn a_soft_reset_plan_claims_nothing_about_the_worktree() {
    let fixture = Fixture::new("reset-plan-soft");
    write(fixture.path(), "b.txt", b"b\n");
    commit_all(fixture.path(), "second");
    write(fixture.path(), "a.txt", b"staged change\n");
    git_ok(fixture.path(), &["add", "a.txt"]);
    write(fixture.path(), "b.txt", b"worktree change\n");

    let plan = fixture
        .history()
        .reset_prepare(fixture.repo_id, &ResetSpec::to("HEAD~1", ResetMode::Soft))
        .expect("计划失败");

    assert!(!plan.requires_confirmation, "--soft 不该要求输入确认词");
    // --soft 只移动 HEAD：索引与工作区原样保留
    assert!(plan.lost_staged.is_empty(), "{plan:?}");
    assert!(plan.lost_worktree.is_empty(), "{plan:?}");
    assert!(plan.untracked_to_remove.is_empty());
    assert_eq!(plan.discarded_count, 1);
}

#[test]
fn a_hard_reset_plan_lists_the_untracked_files_it_would_overwrite() {
    // `git reset --hard` 会删掉"挡在路上的"未跟踪文件：只有目标树里也有该路径时才会
    let fixture = Fixture::new("reset-plan-untracked");
    write(fixture.path(), "doomed.txt", b"tracked later\n");
    commit_all(fixture.path(), "second");
    // 回到第一次提交：doomed.txt 在目标树里不存在 → 不会被删
    let plan = fixture
        .history()
        .reset_prepare(fixture.repo_id, &ResetSpec::to("HEAD~1", ResetMode::Hard))
        .expect("计划失败");
    assert!(plan.untracked_to_remove.is_empty(), "{plan:?}");

    // 现在把 doomed.txt 变成未跟踪文件，而目标提交里**有**它 → 会被覆盖
    git_ok(fixture.path(), &["rm", "--cached", "-q", "doomed.txt"]);
    write(fixture.path(), "doomed.txt", b"untracked now\n");
    let plan = fixture
        .history()
        .reset_prepare(fixture.repo_id, &ResetSpec::to("HEAD", ResetMode::Hard))
        .expect("计划失败");
    let paths: Vec<String> = plan
        .untracked_to_remove
        .iter()
        .map(|path| path.to_string())
        .collect();
    assert_eq!(paths, vec!["doomed.txt".to_owned()], "{plan:?}");
}

#[test]
fn the_plan_reports_which_commits_the_remote_already_has() {
    let fixture = Fixture::new("reset-plan-remote");
    let remote = TempDir::new("reset-plan-remote-bare");
    git_ok(remote.path(), &["init", "--bare", "-q", "-b", "main", "."]);
    let url = support::file_url(remote.path());
    git_ok(fixture.path(), &["remote", "add", "origin", &url]);
    git_ok(fixture.path(), &["push", "-q", "-u", "origin", "main"]);

    // 推一个、再本地加一个：只有后者是"远端没有"的
    write(fixture.path(), "b.txt", b"pushed\n");
    commit_all(fixture.path(), "pushed");
    git_ok(fixture.path(), &["push", "-q"]);
    write(fixture.path(), "c.txt", b"local only\n");
    commit_all(fixture.path(), "local only");

    let plan = fixture
        .history()
        .reset_prepare(fixture.repo_id, &ResetSpec::to("HEAD~1", ResetMode::Hard))
        .expect("计划失败");

    assert_eq!(plan.remote.upstream.as_deref(), Some("origin/main"));
    assert_eq!(
        plan.remote.not_on_remote, 1,
        "只有一个提交是远端没有的：{plan:?}"
    );
}

// ---------------------------------------------------------------- 重置执行

#[test]
fn executing_a_hard_reset_requires_the_confirmation_word_and_snapshots_first() {
    let fixture = Fixture::new("reset-execute");
    write(fixture.path(), "b.txt", b"b\n");
    commit_all(fixture.path(), "second");
    let target = git(fixture.path(), &["rev-parse", "HEAD~1"])
        .stdout_lossy()
        .trim()
        .to_owned();

    let plan = fixture
        .history()
        .reset_prepare(fixture.repo_id, &ResetSpec::to("HEAD~1", ResetMode::Hard))
        .expect("计划失败");

    // 1) 没输入确认词 → 拒绝，且仓库纹丝不动
    let error = fixture
        .history()
        .reset_execute(fixture.repo_id, &plan.plan_id, Some("yes"))
        .expect_err("确认词不对必须被拒");
    assert_eq!(error.code, ErrorCode::Validation);
    assert_ne!(fixture.head(), target, "被拒的执行不能改动仓库");
    assert!(fixture.snapshots.keys().is_empty());

    // 计划已被取走：第二次必须重新预览（避免"同一个计划点两次"）
    let error = fixture
        .history()
        .reset_execute(fixture.repo_id, &plan.plan_id, Some("reset"))
        .expect_err("计划只能执行一次");
    assert_eq!(error.code, ErrorCode::PlanStale);

    // 2) 重新预览 + 正确确认词 → 执行，并且先打了 PreHeadMove 快照
    let plan = fixture
        .history()
        .reset_prepare(fixture.repo_id, &ResetSpec::to("HEAD~1", ResetMode::Hard))
        .expect("计划失败");
    let outcome = fixture
        .history()
        .reset_execute(fixture.repo_id, &plan.plan_id, Some("RESET"))
        .expect("执行失败");

    assert_eq!(outcome.head_after, target);
    assert_eq!(outcome.discarded_count, 1);
    assert!(outcome.snapshot_id.is_some(), "{outcome:?}");
    assert_eq!(fixture.snapshots.keys(), vec!["pre-head-move"]);
    // --hard 丢掉了 b.txt
    assert!(!fixture.path().join("b.txt").exists());
}

#[test]
fn a_plan_prepared_before_new_commits_is_rejected_as_stale() {
    let fixture = Fixture::new("reset-stale");
    write(fixture.path(), "b.txt", b"b\n");
    commit_all(fixture.path(), "second");
    let plan = fixture
        .history()
        .reset_prepare(fixture.repo_id, &ResetSpec::to("HEAD~1", ResetMode::Hard))
        .expect("计划失败");

    // 计划之后又提交了一次：将被丢弃的东西已经不同
    write(fixture.path(), "c.txt", b"c\n");
    commit_all(fixture.path(), "third");

    let error = fixture
        .history()
        .reset_execute(fixture.repo_id, &plan.plan_id, Some("reset"))
        .expect_err("HEAD 变过必须拒绝");
    assert_eq!(error.code, ErrorCode::PlanStale);
    assert!(fixture.snapshots.keys().is_empty(), "被拒的执行不该打快照");
}

// ---------------------------------------------------------------- 拣选 / 反转

#[test]
fn cherry_picking_a_conflicting_commit_reports_the_files_and_snapshots_first() {
    let fixture = Fixture::new("cherry-conflict");

    // feature 分支上改 a.txt
    git_ok(fixture.path(), &["checkout", "-q", "-b", "feature"]);
    write(fixture.path(), "a.txt", b"feature line\n");
    commit_all(fixture.path(), "feature change");
    let feature_oid = fixture.head();

    // main 上改同一行为别的内容 → 拣选必然冲突
    git_ok(fixture.path(), &["checkout", "-q", "main"]);
    write(fixture.path(), "a.txt", b"main line\n");
    commit_all(fixture.path(), "main change");

    let outcome = fixture
        .history()
        .cherry_pick(fixture.repo_id, &CherryPickSpec::new(&feature_oid))
        .expect("冲突不该被压成错误");

    assert_eq!(outcome.kind, MergeKind::Conflicted, "{outcome:?}");
    assert_eq!(outcome.conflicts[0].to_string(), "a.txt");
    assert!(outcome.has_conflicts());
    assert_eq!(
        fixture.snapshots.keys(),
        vec!["pre-head-move"],
        "会移动 HEAD 的操作必须先打快照"
    );
}

#[test]
fn cherry_picking_with_record_source_notes_where_the_change_came_from() {
    let fixture = Fixture::new("cherry-x");
    git_ok(fixture.path(), &["checkout", "-q", "-b", "feature"]);
    write(fixture.path(), "b.txt", b"b\n");
    commit_all(fixture.path(), "feature change");
    let feature_oid = fixture.head();

    git_ok(fixture.path(), &["checkout", "-q", "main"]);
    fixture
        .history()
        .cherry_pick(
            fixture.repo_id,
            &CherryPickSpec {
                record_source: true,
                ..CherryPickSpec::new(&feature_oid)
            },
        )
        .expect("拣选失败");

    let output = git(fixture.path(), &["log", "-1", "--format=%B"]);
    let message = output.stdout_lossy();
    assert!(
        message.contains("cherry picked from commit"),
        "-x 必须把来源写进提交信息：{message}"
    );
    assert!(message.contains(&feature_oid[..7]), "{message}");
}

#[test]
fn reverting_a_merge_commit_uses_the_selected_mainline() {
    let fixture = Fixture::new("revert-merge");

    git_ok(fixture.path(), &["checkout", "-q", "-b", "feature"]);
    write(fixture.path(), "b.txt", b"from feature\n");
    commit_all(fixture.path(), "feature change");
    git_ok(fixture.path(), &["checkout", "-q", "main"]);
    write(fixture.path(), "main.txt", b"main side\n");
    commit_all(fixture.path(), "main change");
    git_ok(
        fixture.path(),
        &["merge", "--no-ff", "-q", "-m", "merge feature", "feature"],
    );
    let merge_oid = fixture.head();

    // 没有 -m 时 git 会拒绝（合并提交有两个父）；这里显式给出主父 1
    let outcome = fixture
        .history()
        .revert(
            fixture.repo_id,
            &RevertSpec {
                mainline: Some(1),
                ..RevertSpec::new(&merge_oid)
            },
        )
        .expect("反转合并提交失败");

    assert!(!outcome.has_conflicts(), "{outcome:?}");
    // 反转主父 1 的合并 = 撤掉 feature 带进来的改动
    assert!(!fixture.path().join("b.txt").exists(), "b.txt 应当被反转掉");
}

// ---------------------------------------------------------------- reflog

#[test]
fn a_lost_commit_can_be_recovered_as_a_new_branch_from_the_reflog() {
    let fixture = Fixture::new("reflog-recover");
    write(fixture.path(), "b.txt", b"b\n");
    commit_all(fixture.path(), "doomed");
    let doomed = fixture.head();

    // 用重置把那次提交丢掉（这是 reflog 恢复的典型场景）
    git_ok(fixture.path(), &["reset", "--hard", "-q", "HEAD~1"]);
    assert_ne!(fixture.head(), doomed);

    // 找到指向被丢掉提交的那条 reflog
    let index = fixture
        .history()
        .reflog(fixture.repo_id, 50)
        .expect("reflog")
        .into_iter()
        .find(|entry| entry.oid == doomed)
        .map(|entry| entry.index)
        .expect("reflog 里应当还有那条提交");

    fixture
        .history()
        .create_branch_from_reflog(fixture.repo_id, index, "recovered")
        .expect("恢复成新分支失败");

    // 恢复**不切换**分支，也不动当前分支
    let branch = git(fixture.path(), &["rev-parse", "--abbrev-ref", "HEAD"])
        .stdout_lossy()
        .trim()
        .to_owned();
    assert_eq!(branch, "main");
    let recovered = git(fixture.path(), &["rev-parse", "recovered"])
        .stdout_lossy()
        .trim()
        .to_owned();
    assert_eq!(recovered, doomed);
    // 建分支**不碰工作区**：重置 --hard 删掉的 b.txt 不会因为"恢复成一个分支"而回来
    assert!(!fixture.path().join("b.txt").exists());
    assert_eq!(fixture.read("a.txt"), "one\n");
}

#[test]
fn creating_a_branch_from_the_reflog_into_a_taken_name_is_refused() {
    let fixture = Fixture::new("reflog-name-taken");
    let head_before = fixture.head();

    fixture
        .history()
        .create_branch_from_reflog(fixture.repo_id, 0, "main")
        .expect_err("已存在的分支名必须被拒");

    // 失败必须是"什么都没发生"：当前分支与 HEAD 都还在原处
    assert_eq!(fixture.head(), head_before);
    let branch = git(fixture.path(), &["rev-parse", "--abbrev-ref", "HEAD"])
        .stdout_lossy()
        .trim()
        .to_owned();
    assert_eq!(branch, "main");
}
