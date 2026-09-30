//! 合并用例（T3.4）的集成测试：prepare/execute 全部在真实仓库上跑真实 git。
//!
//! # 本文件钉住的行为（任务书验收项）
//!
//! 1. **ff 与非 ff 判定**：线性历史的合并判为可快进；分叉历史判为真合并。
//! 2. **squash 不产生合并提交**：HEAD 不动、变更进索引、outcome 为 `Squash`。
//! 3. **预检与真实执行一致**：同一场景先 `prepare` 看预检的冲突清单，再
//!    `execute` 让冲突真实发生，两份清单必须相同。
//! 4. **PLAN_STALE**：prepare 之后 HEAD 被外部改动 → execute 拒绝。
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use forgedesk_domain::git::{FfVerdict, MergeKind, MergeSpec, MergeStrategy};
use forgedesk_domain::ErrorCode;
use forgedesk_git_engine::engines::GitEngines;
use forgedesk_services::{MergePlanRegistry, MergeService};
use forgedesk_snapshot::{
    RestoreReport, RetentionPolicy, SnapshotDiff, SnapshotError, SnapshotId, SnapshotKind,
    SnapshotManager, SnapshotMeta, SnapshotRequest,
};
use support::{commit_all, git, git_ok, init_repo, write, TempDir};

// ---------------------------------------------------------------- 夹具

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
    fn create(&self, request: &SnapshotRequest<'_>) -> Result<SnapshotId, SnapshotError> {
        self.kinds.lock().unwrap().push(request.kind);
        Ok(1)
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
}

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

struct Fixture {
    dir: TempDir,
    repo_id: i64,
    engines: &'static GitEngines,
    database: &'static forge_setup::LeakDatabase,
    snapshots: &'static RecordingSnapshots,
    plans: &'static MergePlanRegistry,
}

impl Fixture {
    fn new(prefix: &str) -> Self {
        let dir = TempDir::new(prefix);
        init_repo(dir.path());
        let engines: &'static GitEngines =
            Box::leak(Box::new(GitEngines::new().expect("创建引擎失败")));
        let database: &'static forge_setup::LeakDatabase =
            Box::leak(Box::new(forge_setup::leak_database()));
        let snapshots: &'static RecordingSnapshots =
            Box::leak(Box::new(RecordingSnapshots::default()));
        let plans: &'static MergePlanRegistry = Box::leak(Box::new(MergePlanRegistry::new()));
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

    fn merge(&self) -> MergeService<'_> {
        MergeService::new(
            self.engines,
            forge_setup::store(self.database),
            self.snapshots,
            self.plans,
        )
    }

    fn head(&self) -> String {
        git(self.path(), &["rev-parse", "HEAD"])
            .stdout_lossy()
            .trim()
            .to_owned()
    }

    /// 当前分支上的提交主题（HEAD 的第一父历史）。
    fn subjects(&self) -> Vec<String> {
        let output = git(self.path(), &["log", "--format=%s"]);
        output
            .stdout_lossy()
            .lines()
            .map(|l| l.to_owned())
            .collect()
    }
}

/// 基提交 + feature 分支上的一个提交（main 停在 base）。
fn branched_fixture(prefix: &str) -> Fixture {
    let fixture = Fixture::new(prefix);
    write(fixture.path(), "a.txt", b"base\n");
    commit_all(fixture.path(), "base");
    git_ok(fixture.path(), &["checkout", "-b", "feature"]);
    write(fixture.path(), "feature.txt", b"from feature\n");
    commit_all(fixture.path(), "feature commit");
    git_ok(fixture.path(), &["checkout", "main"]);
    fixture
}

/// 分叉后两边都改 a.txt 的同一行（必然冲突）。
fn conflicting_fixture(prefix: &str) -> Fixture {
    let fixture = Fixture::new(prefix);
    write(fixture.path(), "a.txt", b"base\n");
    commit_all(fixture.path(), "base");
    git_ok(fixture.path(), &["checkout", "-b", "feature"]);
    write(fixture.path(), "a.txt", b"feature\n");
    commit_all(fixture.path(), "feature change");
    git_ok(fixture.path(), &["checkout", "main"]);
    write(fixture.path(), "a.txt", b"main\n");
    commit_all(fixture.path(), "main change");
    fixture
}

// ---------------------------------------------------------------- prepare

#[test]
fn prepare_reports_fast_forward_for_a_linear_history() {
    let fixture = branched_fixture("merge-prepare-ff");

    let plan = fixture
        .merge()
        .prepare(
            fixture.repo_id,
            &MergeSpec::new("feature").with_strategy(MergeStrategy::Merge),
        )
        .expect("prepare 失败");

    assert_eq!(plan.verdict, FfVerdict::FastForward);
    assert_eq!(plan.source, "feature");
    assert_eq!(plan.source_commit_count, 1);
    assert_eq!(plan.source_only_commits.len(), 1);
    assert_eq!(plan.source_only_commits[0].subject, "feature commit");
    assert!(plan.preview.available);
    assert!(plan.preview.conflicted.is_empty());
    assert_eq!(plan.default_message, "Merge branch 'feature' into main");
    assert_eq!(plan.equivalent_command, "git merge feature");
    assert_eq!(plan.head_before, fixture.head());
    // 计划进注册表
    assert_eq!(fixture.plans.len(), 1);
}

#[test]
fn prepare_reports_true_merge_and_preview_conflicts_for_a_diverged_history() {
    let fixture = conflicting_fixture("merge-prepare-conflict");

    let plan = fixture
        .merge()
        .prepare(
            fixture.repo_id,
            &MergeSpec::new("feature").with_strategy(MergeStrategy::Merge),
        )
        .expect("prepare 失败");

    assert_eq!(plan.verdict, FfVerdict::TrueMerge);
    assert!(plan.preview.available, "本机 git 2.54 支持 merge-tree");
    assert_eq!(
        plan.preview
            .conflicted
            .iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>(),
        vec!["a.txt".to_owned()],
        "预检必须发现 a.txt 冲突"
    );
}

#[test]
fn prepare_reports_up_to_date_when_source_is_already_merged() {
    let fixture = branched_fixture("merge-prepare-uptodate");
    // 把 feature 合进 main（快进），再准备一次合并同一分支
    git_ok(fixture.path(), &["merge", "feature"]);

    let plan = fixture
        .merge()
        .prepare(fixture.repo_id, &MergeSpec::new("feature"))
        .expect("prepare 失败");

    assert_eq!(plan.verdict, FfVerdict::UpToDate);
    assert_eq!(plan.source_commit_count, 0);
}

#[test]
fn prepare_rejects_a_missing_source() {
    let fixture = Fixture::new("merge-prepare-missing");
    write(fixture.path(), "a.txt", b"base\n");
    commit_all(fixture.path(), "base");

    let error = fixture
        .merge()
        .prepare(fixture.repo_id, &MergeSpec::new("no-such-branch"))
        .expect_err("不存在的 source 必须失败");
    assert_eq!(error.code, ErrorCode::Validation, "{error:?}");
}

// ---------------------------------------------------------------- execute

#[test]
fn execute_fast_forwards_without_creating_a_merge_commit() {
    let fixture = branched_fixture("merge-execute-ff");
    let head_before = fixture.head();

    let plan = fixture
        .merge()
        .prepare(fixture.repo_id, &MergeSpec::new("feature"))
        .expect("prepare 失败");
    let outcome = fixture
        .merge()
        .execute(fixture.repo_id, &plan.plan_id, None)
        .expect("execute 失败");

    assert_eq!(outcome.kind, MergeKind::FastForward);
    assert_eq!(
        outcome.oid.as_deref(),
        Some(plan.source_only_commits[0].oid.as_str())
    );
    assert_eq!(fixture.head(), plan.source_only_commits[0].oid);
    assert_ne!(fixture.head(), head_before);
    // 快照（PreSync）与审计关联
    assert_eq!(outcome.snapshot_id, Some(1));
    assert_eq!(fixture.snapshots.keys(), vec!["pre-sync"]);
}

#[test]
fn execute_with_no_ff_creates_a_merge_commit() {
    let fixture = branched_fixture("merge-execute-noff");

    let plan = fixture
        .merge()
        .prepare(
            fixture.repo_id,
            &MergeSpec::new("feature").with_strategy(MergeStrategy::NoFf),
        )
        .expect("prepare 失败");
    let outcome = fixture
        .merge()
        .execute(
            fixture.repo_id,
            &plan.plan_id,
            Some("merge feature by hand".to_owned()),
        )
        .expect("execute 失败");

    assert_eq!(outcome.kind, MergeKind::MergeCommit);
    assert_eq!(outcome.oid.as_deref(), Some(fixture.head().as_str()));
    // 合并提交在 HEAD 上，信息是用户编辑过的
    let subjects = fixture.subjects();
    assert_eq!(subjects[0], "merge feature by hand");
    // 第二父是 feature 的提交（真合并的标志）
    let parents = git(fixture.path(), &["rev-parse", "HEAD^2"])
        .stdout_lossy()
        .trim()
        .to_owned();
    assert_eq!(parents, plan.source_only_commits[0].oid);
}

#[test]
fn execute_squash_stages_changes_without_any_commit() {
    let fixture = branched_fixture("merge-execute-squash");
    let head_before = fixture.head();

    let plan = fixture
        .merge()
        .prepare(
            fixture.repo_id,
            &MergeSpec::new("feature").with_strategy(MergeStrategy::Squash),
        )
        .expect("prepare 失败");
    let outcome = fixture
        .merge()
        .execute(
            fixture.repo_id,
            &plan.plan_id,
            Some("ignored for squash".to_owned()),
        )
        .expect("execute 失败");

    // squash：HEAD 不动、没有合并提交、变更在索引里
    assert_eq!(outcome.kind, MergeKind::Squash);
    assert_eq!(outcome.oid, None);
    assert_eq!(fixture.head(), head_before, "squash 不移动 HEAD");
    let subjects = fixture.subjects();
    assert_eq!(subjects[0], "base", "不能产生合并提交");
    // 变更进索引：feature.txt 出现在已暂存清单
    let staged = git(fixture.path(), &["diff", "--name-only", "--cached"]);
    assert!(
        staged.stdout_lossy().contains("feature.txt"),
        "{}",
        staged.stdout_lossy()
    );
}

#[test]
fn execute_stops_on_the_same_conflicts_the_preview_reported() {
    // 任务书验收：预检与真实执行的结果一致
    let fixture = conflicting_fixture("merge-preview-consistency");

    let plan = fixture
        .merge()
        .prepare(fixture.repo_id, &MergeSpec::new("feature"))
        .expect("prepare 失败");
    assert_eq!(
        plan.preview
            .conflicted
            .iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>(),
        vec!["a.txt".to_owned()]
    );

    let outcome = fixture
        .merge()
        .execute(fixture.repo_id, &plan.plan_id, None)
        .expect("冲突是结果不是错误");

    assert_eq!(outcome.kind, MergeKind::Conflicted);
    assert_eq!(
        outcome
            .conflicts
            .iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>(),
        plan.preview
            .conflicted
            .iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>(),
        "执行后的冲突清单必须与预检一致"
    );
    // 冲突后计划已被取走；解决冲突（工作区写入并 add）后用默认信息完成合并
    write(fixture.path(), "a.txt", b"resolved\n");
    git_ok(fixture.path(), &["add", "a.txt"]);
    fixture
        .merge()
        .continue_merge(fixture.repo_id, None)
        .expect("continue 失败");
    let subjects = fixture.subjects();
    // 默认信息沿用 prepare 写进 MERGE_MSG 的完整惯例（含 into main）
    assert_eq!(subjects[0], "Merge branch 'feature' into main");
}

#[test]
fn execute_rejects_a_stale_plan() {
    let fixture = branched_fixture("merge-plan-stale");

    let plan = fixture
        .merge()
        .prepare(fixture.repo_id, &MergeSpec::new("feature"))
        .expect("prepare 失败");
    // prepare 之后 HEAD 被外部改动
    write(fixture.path(), "other.txt", b"external\n");
    commit_all(fixture.path(), "external commit");

    let error = fixture
        .merge()
        .execute(fixture.repo_id, &plan.plan_id, None)
        .expect_err("HEAD 变了必须拒绝");
    assert_eq!(error.code, ErrorCode::PlanStale, "{error:?}");
}

#[test]
fn execute_rejects_an_unknown_or_spent_plan_id() {
    let fixture = branched_fixture("merge-plan-unknown");
    let error = fixture
        .merge()
        .execute(fixture.repo_id, "no-such-plan", None)
        .expect_err("未知 plan 必须失败");
    assert_eq!(error.code, ErrorCode::NotFound, "{error:?}");

    // 取走即失效：同一个 plan 不能执行两次
    let plan = fixture
        .merge()
        .prepare(fixture.repo_id, &MergeSpec::new("feature"))
        .expect("prepare 失败");
    fixture
        .merge()
        .execute(fixture.repo_id, &plan.plan_id, None)
        .expect("第一次执行应成功");
    let error = fixture
        .merge()
        .execute(fixture.repo_id, &plan.plan_id, None)
        .expect_err("第二次执行必须失败");
    assert_eq!(error.code, ErrorCode::NotFound, "{error:?}");
}

// ---------------------------------------------------------------- continue

#[test]
fn continue_merge_uses_the_edited_message() {
    let fixture = conflicting_fixture("merge-continue-message");

    let plan = fixture
        .merge()
        .prepare(fixture.repo_id, &MergeSpec::new("feature"))
        .expect("prepare 失败");
    let outcome = fixture
        .merge()
        .execute(fixture.repo_id, &plan.plan_id, None)
        .expect("execute 失败");
    assert_eq!(outcome.kind, MergeKind::Conflicted);

    // 解决冲突（采用本方内容）后用编辑过的信息继续
    write(fixture.path(), "a.txt", b"resolved\n");
    git_ok(fixture.path(), &["add", "a.txt"]);
    fixture
        .merge()
        .continue_merge(fixture.repo_id, Some("custom merge message".to_owned()))
        .expect("continue 失败");

    let subjects = fixture.subjects();
    assert_eq!(subjects[0], "custom merge message");
}

#[test]
fn continue_merge_rejects_unresolved_conflicts() {
    let fixture = conflicting_fixture("merge-continue-unresolved");

    let plan = fixture
        .merge()
        .prepare(fixture.repo_id, &MergeSpec::new("feature"))
        .expect("prepare 失败");
    let outcome = fixture
        .merge()
        .execute(fixture.repo_id, &plan.plan_id, None)
        .expect("execute 失败");
    assert_eq!(outcome.kind, MergeKind::Conflicted);

    let error = fixture
        .merge()
        .continue_merge(fixture.repo_id, None)
        .expect_err("有未解决冲突时 continue 必须失败");
    assert_eq!(error.code, ErrorCode::ConflictUnresolved, "{error:?}");
}
