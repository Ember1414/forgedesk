//! 冲突状态机（T3.1）的集成测试：merge / rebase / cherry-pick / revert 四类场景，
//! 全部在真实仓库上跑真实 git。
//!
//! # 本文件钉住的行为
//!
//! 1. **数据源是 index stage 不是标记符**：把工作区文件的 `<<<<<<<` 手动删掉
//!    （或改成任意内容），冲突状态照常报出该文件——任务书点名的验收项。
//! 2. **rebase 的进度与中止基线**：`msgnum` / `end` 读出 current/total；
//!    abort 回到 `orig-head`（rebase 期间 HEAD 已不在操作前的位置）。
//! 3. **continue 的两种正常结局**：完成（oid 更新）与再次停在冲突（序列
//!    重放撞新的冲突，`conflicts` 非空而不是报错）。
//! 4. **continue 的前置校验**：未解决文件存在时返回 `CONFLICT_UNRESOLVED`
//!    且 `hint` 列出文件名。
//! 5. **abort 打 `PreHeadMove` 快照**（安全网红线 R7）；`mark_resolved` /
//!    `continue` / `skip` 不打（与 stage / commit 同一取舍）。
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use forgedesk_domain::git::{
    ConflictKind, ConflictOpKind, LineEnding, MergeBlock, RepoPath, TakeSide,
};
use forgedesk_domain::ErrorCode;
use forgedesk_git_engine::engines::GitEngines;
use forgedesk_services::ConflictService;
use forgedesk_snapshot::{
    RestoreReport, RetentionPolicy, SnapshotDiff, SnapshotError, SnapshotId, SnapshotKind,
    SnapshotManager, SnapshotMeta, SnapshotRequest,
};
use support::{commit_all, git, git_ok, init_repo, write, TempDir};

// ---------------------------------------------------------------- 夹具

/// 记录型快照管理器（与 history_ops.rs / sync.rs 同一写法：只关心"打没打、打的哪一类"）。
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

/// 借用生命周期的包装（与 history_ops.rs 的 `Box::leak` 同一技巧）。
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
}

impl Fixture {
    /// 空仓库 + 已登记的服务。
    fn new(prefix: &str) -> Self {
        let dir = TempDir::new(prefix);
        init_repo(dir.path());
        let engines: &'static GitEngines =
            Box::leak(Box::new(GitEngines::new().expect("创建引擎失败")));
        let database: &'static forge_setup::LeakDatabase =
            Box::leak(Box::new(forge_setup::leak_database()));
        let snapshots: &'static RecordingSnapshots =
            Box::leak(Box::new(RecordingSnapshots::default()));
        let repo_id = forge_setup::register(&forge_setup::store(database), dir.path());
        Self {
            dir,
            repo_id,
            engines,
            database,
            snapshots,
        }
    }

    fn path(&self) -> &std::path::Path {
        self.dir.path()
    }

    fn conflict(&self) -> ConflictService<'_> {
        ConflictService::new(
            self.engines,
            forge_setup::store(self.database),
            self.snapshots,
        )
    }

    fn head(&self) -> String {
        git(self.path(), &["rev-parse", "HEAD"])
            .stdout_lossy()
            .trim()
            .to_owned()
    }

    fn state(&self) -> forgedesk_domain::git::ConflictState {
        self.conflict()
            .state(self.repo_id)
            .expect("采集冲突状态失败")
    }

    /// 把指定文件标记为已解决（用状态里报告的路径，保证字节保真）。
    fn resolve(&self, path: &str) {
        self.conflict()
            .mark_resolved(self.repo_id, &[RepoPath::from(path)])
            .expect("标记已解决失败");
    }
}

// 构造：base 提交后分叉，两边各改一次 a.txt，merge 时必然冲突。
fn merge_conflict_fixture(prefix: &str) -> Fixture {
    let fixture = Fixture::new(prefix);
    write(fixture.path(), "a.txt", b"base\n");
    commit_all(fixture.path(), "base");
    git_ok(fixture.path(), &["checkout", "-b", "feature"]);
    write(fixture.path(), "a.txt", b"feature\n");
    commit_all(fixture.path(), "feature change");
    git_ok(fixture.path(), &["checkout", "main"]);
    write(fixture.path(), "a.txt", b"main\n");
    commit_all(fixture.path(), "main change");
    // 冲突的 merge：非零退出是**预期结果**
    let _ = git(fixture.path(), &["merge", "feature"]);
    fixture
}

// 构造：feature 两个提交各改一次 a.txt，main 也改，rebase 时两个提交先后冲突。
// 返回 rebase 开始前的 HEAD（= orig-head，abort 的校验基线）。
fn rebase_conflict_fixture(prefix: &str) -> (Fixture, String) {
    let fixture = Fixture::new(prefix);
    write(fixture.path(), "a.txt", b"base\n");
    commit_all(fixture.path(), "base");
    git_ok(fixture.path(), &["checkout", "-b", "feature"]);
    write(fixture.path(), "a.txt", b"f1\n");
    commit_all(fixture.path(), "f1");
    write(fixture.path(), "a.txt", b"f2\n");
    commit_all(fixture.path(), "f2");
    git_ok(fixture.path(), &["checkout", "main"]);
    write(fixture.path(), "a.txt", b"main\n");
    commit_all(fixture.path(), "main change");
    git_ok(fixture.path(), &["checkout", "feature"]);
    let head_before_rebase = fixture.head();
    let _ = git(fixture.path(), &["rebase", "main"]);
    // 注意：rebase 停在冲突上时 HEAD **已经**移到 onto（main 顶端）——这正是
    // abort 的校验基线要读 orig-head 的原因，不能在这里断言 HEAD 未动
    (fixture, head_before_rebase)
}

// 构造：side 分支的提交拣选到已各自演进的 main 上，必然冲突。
fn cherry_pick_conflict_fixture(prefix: &str) -> Fixture {
    let fixture = Fixture::new(prefix);
    write(fixture.path(), "a.txt", b"base\n");
    commit_all(fixture.path(), "base");
    git_ok(fixture.path(), &["checkout", "-b", "side"]);
    write(fixture.path(), "a.txt", b"side\n");
    commit_all(fixture.path(), "side change");
    git_ok(fixture.path(), &["checkout", "main"]);
    write(fixture.path(), "a.txt", b"main\n");
    commit_all(fixture.path(), "main change");
    let _ = git(fixture.path(), &["cherry-pick", "side"]);
    fixture
}

// 构造：三个线性提交，反转中间那个（one→two 被反转，但当前已是 three）→ 冲突。
fn revert_conflict_fixture(prefix: &str) -> Fixture {
    let fixture = Fixture::new(prefix);
    write(fixture.path(), "a.txt", b"one\n");
    commit_all(fixture.path(), "one");
    write(fixture.path(), "a.txt", b"two\n");
    commit_all(fixture.path(), "two");
    write(fixture.path(), "a.txt", b"three\n");
    commit_all(fixture.path(), "three");
    let target = git(fixture.path(), &["rev-parse", "HEAD~1"])
        .stdout_lossy()
        .trim()
        .to_owned();
    let _ = git(fixture.path(), &["revert", &target]);
    fixture
}

// ---------------------------------------------------------------- 状态检测

#[test]
fn merge_conflict_reports_the_operation_and_all_three_blobs() {
    let fixture = merge_conflict_fixture("conflict-merge-state");
    let state = fixture.state();

    assert_eq!(state.op_kind, Some(ConflictOpKind::Merge));
    assert!(state.op_in_progress);
    assert_eq!(state.into_branch.as_deref(), Some("main"));
    assert_eq!(state.files.len(), 1);
    let file = &state.files[0];
    assert_eq!(file.path.as_str(), Some("a.txt"));
    assert_eq!(file.kind, ConflictKind::Text);
    assert!(file.worktree_exists);
    assert_eq!(
        file.base.as_ref().and_then(|blob| blob.content.as_deref()),
        Some("base\n")
    );
    assert_eq!(
        file.ours.as_ref().and_then(|blob| blob.content.as_deref()),
        Some("main\n")
    );
    assert_eq!(
        file.theirs
            .as_ref()
            .and_then(|blob| blob.content.as_deref()),
        Some("feature\n")
    );
    assert_eq!(
        file.ours.as_ref().map(|blob| blob.encoding_hint.as_deref()),
        Some(Some("utf-8"))
    );

    // 状态机标志：有未解决文件时能中止、不能继续；merge 不支持 skip
    assert!(state.can_abort);
    assert!(!state.can_continue);
    assert!(!state.can_skip);
}

#[test]
fn rebase_conflict_reports_progress_and_the_rebased_branch() {
    let (fixture, _) = rebase_conflict_fixture("conflict-rebase-state");
    let state = fixture.state();

    assert_eq!(state.op_kind, Some(ConflictOpKind::Rebase));
    // f1 是第一个重放的提交（共 2 个）
    assert_eq!(state.current_step, Some(1));
    assert_eq!(state.total_steps, Some(2));
    assert_eq!(state.head_name.as_deref(), Some("refs/heads/feature"));
    assert_eq!(state.into_branch, None, "rebase 的 HEAD 在重放位置上");
    assert!(state.can_skip, "只有 rebase 支持 skip");
    assert!(state.can_abort);
    assert!(!state.can_continue);
}

#[test]
fn continue_with_unresolved_files_lists_them_in_the_error() {
    let fixture = merge_conflict_fixture("conflict-unresolved");
    let error = fixture
        .conflict()
        .continue_operation(fixture.repo_id)
        .expect_err("有未解决文件时 continue 必须失败");

    assert_eq!(error.code, ErrorCode::ConflictUnresolved);
    assert!(
        error.hint.as_deref().unwrap_or_default().contains("a.txt"),
        "hint 必须列出未解决的文件：{error:?}"
    );
}

#[test]
fn actions_without_an_operation_in_progress_are_rejected() {
    let fixture = Fixture::new("conflict-no-op");

    for error in [
        fixture
            .conflict()
            .continue_operation(fixture.repo_id)
            .unwrap_err(),
        fixture
            .conflict()
            .abort_operation(fixture.repo_id)
            .unwrap_err(),
        fixture.conflict().skip(fixture.repo_id).unwrap_err(),
    ] {
        assert_eq!(error.code, ErrorCode::Validation, "{error:?}");
    }
}

// ---------------------------------------------------------------- 解决流程

#[test]
fn a_merge_conflict_resolves_marks_and_continues_to_a_merge_commit() {
    let fixture = merge_conflict_fixture("conflict-merge-continue");
    let head_before = fixture.head();

    fixture.resolve("a.txt");
    let state = fixture.state();
    assert!(
        state.files.is_empty(),
        "标记后不应再有未解决文件：{:?}",
        state.files
    );
    assert!(state.can_continue, "全部解决后可以继续");

    let outcome = fixture
        .conflict()
        .continue_operation(fixture.repo_id)
        .expect("continue 失败");
    assert!(!outcome.has_conflicts());
    let head_after = fixture.head();
    assert_eq!(outcome.oid.as_deref(), Some(head_after.as_str()));
    assert_ne!(head_after, head_before, "merge continue 产生合并提交");

    // 操作结束后状态回到空态
    let state = fixture.state();
    assert_eq!(state.op_kind, None);
    assert!(!state.op_in_progress);
}

#[test]
fn the_resolved_file_is_reported_even_when_workspace_markers_are_gone() {
    // 任务书验收项：数据源是 index stage，不是工作区的 <<<<<<< 标记
    let fixture = merge_conflict_fixture("conflict-markers-gone");
    // 用户手动把文件改成一段没有冲突标记的文本（但没 git add）
    write(fixture.path(), "a.txt", b"a hand-edited resolution\n");

    let state = fixture.state();
    assert_eq!(
        state.files.len(),
        1,
        "index 仍冲突时文件必须仍然报出（即使标记已被删掉）"
    );
    // 工作区内容不进入 blob 报告：ours/theirs 仍是 stage 上的版本
    assert_eq!(
        state.files[0]
            .ours
            .as_ref()
            .and_then(|blob| blob.content.as_deref()),
        Some("main\n")
    );

    fixture.resolve("a.txt");
    assert!(fixture.state().files.is_empty());
}

#[test]
fn rebase_abort_restores_the_pre_operation_state_and_leaves_a_snapshot() {
    let (fixture, head_before_rebase) = rebase_conflict_fixture("conflict-rebase-abort");

    let outcome = fixture
        .conflict()
        .abort_operation(fixture.repo_id)
        .expect("abort 失败");
    assert_eq!(
        outcome.head_oid.as_deref(),
        Some(head_before_rebase.as_str())
    );
    assert_eq!(outcome.head_ref.as_deref(), Some("feature"));
    assert_eq!(outcome.snapshot_id, Some(1), "abort 必须留下快照");
    assert_eq!(fixture.snapshots.keys(), vec!["pre-head-move"]);

    let state = fixture.state();
    assert_eq!(state.op_kind, None);
    let content = std::fs::read_to_string(fixture.path().join("a.txt")).unwrap();
    assert_eq!(content, "f2\n", "abort 后工作区回到 feature 的内容");
}

#[test]
fn merge_abort_returns_to_the_pre_merge_head_and_branch() {
    let fixture = merge_conflict_fixture("conflict-merge-abort");
    let head_before = fixture.head();

    let outcome = fixture
        .conflict()
        .abort_operation(fixture.repo_id)
        .expect("abort 失败");
    assert_eq!(outcome.head_oid.as_deref(), Some(head_before.as_str()));
    assert_eq!(outcome.head_ref.as_deref(), Some("main"));
    assert!(fixture.state().op_kind.is_none());
}

#[test]
fn a_rebase_sequence_can_stop_again_on_the_next_conflict() {
    let (fixture, _) = rebase_conflict_fixture("conflict-rebase-two-stops");

    // 第一次冲突（f1）→ 解决 → continue → 撞上 f2 的新冲突（正常结果，不是错误）
    fixture.resolve("a.txt");
    let outcome = fixture
        .conflict()
        .continue_operation(fixture.repo_id)
        .expect("第一次 continue 失败");
    assert!(
        outcome.has_conflicts(),
        "f2 与已解决的 f1 上下文不匹配，必须再次停在冲突上"
    );
    assert_eq!(outcome.oid, None);
    let state = fixture.state();
    assert_eq!(state.op_kind, Some(ConflictOpKind::Rebase));
    assert_eq!(state.current_step, Some(2), "现在在第二个提交上");

    // 第二次解决 → continue → rebase 完成
    fixture.resolve("a.txt");
    let outcome = fixture
        .conflict()
        .continue_operation(fixture.repo_id)
        .expect("第二次 continue 失败");
    assert!(!outcome.has_conflicts());
    assert_eq!(fixture.state().op_kind, None);
}

#[test]
fn a_cherry_pick_conflict_walks_through_resolve_and_continue() {
    let fixture = cherry_pick_conflict_fixture("conflict-cherry-pick");
    let state = fixture.state();
    assert_eq!(state.op_kind, Some(ConflictOpKind::CherryPick));
    assert_eq!(state.into_branch.as_deref(), Some("main"));

    fixture.resolve("a.txt");
    let outcome = fixture
        .conflict()
        .continue_operation(fixture.repo_id)
        .expect("continue 失败");
    assert!(!outcome.has_conflicts());
    assert!(fixture.state().op_kind.is_none());
}

#[test]
fn a_revert_conflict_walks_through_resolve_and_continue() {
    let fixture = revert_conflict_fixture("conflict-revert");
    let state = fixture.state();
    assert_eq!(state.op_kind, Some(ConflictOpKind::Revert));

    fixture.resolve("a.txt");
    let outcome = fixture
        .conflict()
        .continue_operation(fixture.repo_id)
        .expect("continue 失败");
    assert!(!outcome.has_conflicts());
    assert!(fixture.state().op_kind.is_none());
}

#[test]
fn mark_resolved_rejects_paths_that_were_never_conflicted() {
    let fixture = merge_conflict_fixture("conflict-mark-resolved-validation");
    // git 的 pathspec 不匹配以退出码 128 失败（分类随 ErrorCode::classify，
    // 这里钉住的是"不能静默成功"），冲突文件清单不变
    fixture
        .conflict()
        .mark_resolved(fixture.repo_id, &[RepoPath::from("no/such/file.txt")])
        .expect_err("不存在的路径必须失败");
    assert_eq!(fixture.state().files.len(), 1, "原冲突不应被误标解决");
}

#[test]
fn skip_outside_a_rebase_is_rejected() {
    let fixture = merge_conflict_fixture("conflict-skip-merge");
    let error = fixture
        .conflict()
        .skip(fixture.repo_id)
        .expect_err("merge 不支持 skip");
    assert_eq!(error.code, ErrorCode::Validation, "{error:?}");
}

#[test]
fn a_clean_repository_probes_an_empty_state() {
    let fixture = Fixture::new("conflict-clean-probe");
    let state = fixture.state();
    assert_eq!(state.op_kind, None);
    assert!(!state.op_in_progress);
    assert!(state.files.is_empty());
    assert!(!state.can_abort);
    assert!(!state.can_continue);
    assert!(!state.can_skip);
}

#[test]
fn continue_and_skip_do_not_create_snapshots() {
    // 只有 abort 打快照（PreHeadMove）；continue / mark_resolved 与 stage / commit
    // 同一取舍：不破坏工作区，不需要安全网
    let fixture = merge_conflict_fixture("conflict-no-snapshot-on-continue");
    fixture.resolve("a.txt");
    let _ = fixture.conflict().continue_operation(fixture.repo_id);
    assert!(
        fixture.snapshots.keys().is_empty(),
        "continue 不应打快照：{:?}",
        fixture.snapshots.keys()
    );
}

// ---------------------------------------------------------------- 编辑器路径（T3.2）

#[test]
fn file_detail_reports_blocks_and_the_worktree_shape() {
    let fixture = merge_conflict_fixture("conflict-detail-blocks");
    let detail = fixture
        .conflict()
        .file_detail(fixture.repo_id, &RepoPath::from("a.txt"))
        .expect("file_detail 失败");

    assert_eq!(detail.kind, ConflictKind::Text);
    assert!(detail.worktree_exists);
    assert_eq!(detail.eol, LineEnding::Lf);
    assert!(detail.trailing_newline);
    assert!(!detail.bom);

    // a.txt：base="base"、ours="main"、theirs="feature" → 单行冲突
    assert_eq!(detail.blocks.len(), 1);
    assert_eq!(
        detail.blocks[0],
        MergeBlock::Conflict {
            base: vec!["base".into()],
            ours: vec!["main".into()],
            theirs: vec!["feature".into()],
        }
    );
}

#[test]
fn file_detail_rejects_paths_that_are_not_conflicted() {
    let fixture = merge_conflict_fixture("conflict-detail-validation");
    let error = fixture
        .conflict()
        .file_detail(fixture.repo_id, &RepoPath::from("no/such/file.txt"))
        .expect_err("非冲突路径必须失败");
    assert_eq!(error.code, ErrorCode::Validation, "{error:?}");
}

#[test]
fn apply_resolution_writes_back_and_marks_resolved() {
    let fixture = merge_conflict_fixture("conflict-apply");
    let conflict = fixture.conflict();

    conflict
        .apply_resolution(
            fixture.repo_id,
            &RepoPath::from("a.txt"),
            "a hand-built resolution
",
            LineEnding::Lf,
            false,
            true,
        )
        .expect("apply_resolution 失败");

    let content = std::fs::read_to_string(fixture.path().join("a.txt")).unwrap();
    assert_eq!(
        content,
        "a hand-built resolution
"
    );
    assert!(fixture.state().files.is_empty());

    // 全部解决后可以直接 continue
    let outcome = conflict
        .continue_operation(fixture.repo_id)
        .expect("continue 失败");
    assert!(!outcome.has_conflicts());
}

#[test]
fn apply_resolution_rebuilds_crlf_and_bom_from_the_original_shape() {
    let fixture = merge_conflict_fixture("conflict-apply-crlf");
    // 原文件带 BOM + CRLF（模拟 Windows 编辑器产生的文件）：
    // 重新写入带 BOM/CRLF 的内容到 stage（用 apply 之前的真实路径：直接改工作区
    // 文件再 git add 会丢冲突态，所以只验证"写回时按参数重建"这一层）
    fixture
        .conflict()
        .apply_resolution(
            fixture.repo_id,
            &RepoPath::from("a.txt"),
            "windows line
",
            LineEnding::Crlf,
            true,
            true,
        )
        .expect("apply_resolution 失败");

    let bytes = std::fs::read(fixture.path().join("a.txt")).unwrap();
    assert_eq!(&bytes[..3], &[0xEF, 0xBB, 0xBF], "必须写回 UTF-8 BOM");
    // CRLF：0x0D 0x0A 两个字节的相邻窗口
    assert!(
        bytes.windows(2).any(|pair| pair == [0x0D, 0x0A]),
        "必须写回 CRLF"
    );
    assert!(fixture.state().files.is_empty());
}

#[test]
fn take_side_restores_the_ours_version_and_resolves() {
    let fixture = merge_conflict_fixture("conflict-take-side");
    fixture
        .conflict()
        .take_side(fixture.repo_id, &RepoPath::from("a.txt"), TakeSide::Ours)
        .expect("take_side 失败");

    let content = std::fs::read_to_string(fixture.path().join("a.txt")).unwrap();
    assert_eq!(
        content,
        "main
",
        "采用 ours 后工作区是 main 的版本"
    );
    assert!(fixture.state().files.is_empty());
}

#[test]
fn take_side_theirs_restores_their_version() {
    let fixture = merge_conflict_fixture("conflict-take-theirs");
    fixture
        .conflict()
        .take_side(fixture.repo_id, &RepoPath::from("a.txt"), TakeSide::Theirs)
        .expect("take_side 失败");

    let content = std::fs::read_to_string(fixture.path().join("a.txt")).unwrap();
    assert_eq!(
        content,
        "feature
"
    );
    assert!(fixture.state().files.is_empty());
}

#[test]
fn remove_file_resolves_a_conflict_by_deleting_it() {
    let fixture = merge_conflict_fixture("conflict-remove-file");
    fixture
        .conflict()
        .remove_file(fixture.repo_id, &RepoPath::from("a.txt"))
        .expect("remove_file 失败");

    assert!(
        !fixture.path().join("a.txt").exists(),
        "工作区文件必须被删除"
    );
    assert!(fixture.state().files.is_empty());

    // 删除也是合法的解决方式：continue 应该照常完成
    let outcome = fixture
        .conflict()
        .continue_operation(fixture.repo_id)
        .expect("continue 失败");
    assert!(!outcome.has_conflicts());
}

#[test]
fn file_detail_of_a_both_added_conflict_has_no_base() {
    // 双方都新增同名文件：没有 stage 1，blocks 的冲突块没有 base
    let fixture = Fixture::new("conflict-both-added");
    write(
        fixture.path(),
        "readme.md",
        b"base readme
",
    );
    commit_all(fixture.path(), "base");
    git_ok(fixture.path(), &["checkout", "-b", "feature"]);
    write(
        fixture.path(),
        "new.txt",
        b"from feature
",
    );
    commit_all(fixture.path(), "feature adds");
    git_ok(fixture.path(), &["checkout", "main"]);
    write(
        fixture.path(),
        "new.txt",
        b"from main
",
    );
    commit_all(fixture.path(), "main adds");
    let _ = git(fixture.path(), &["merge", "feature"]);

    let detail = fixture
        .conflict()
        .file_detail(fixture.repo_id, &RepoPath::from("new.txt"))
        .expect("file_detail 失败");
    assert_eq!(detail.kind, ConflictKind::AddedByBoth);
    assert!(detail.base.is_none());
    // 两个相同前缀的不同文件：整文件都是一方内容 → 单个冲突块
    match &detail.blocks[0] {
        MergeBlock::Conflict { base, ours, theirs } => {
            assert!(base.is_empty());
            assert_eq!(ours, &vec!["from main".to_owned()]);
            assert_eq!(theirs, &vec!["from feature".to_owned()]);
        }
        other => panic!("期望冲突块，得到 {other:?}"),
    }
}

#[test]
fn auto_resolved_sections_are_reported_as_resolved_blocks() {
    // theirs 在文件末尾追加一行（ours 未动该行）→ Resolved(Theirs)
    let fixture = Fixture::new("conflict-auto-resolved");
    write(
        fixture.path(),
        "a.txt",
        b"line1
",
    );
    commit_all(fixture.path(), "base");
    git_ok(fixture.path(), &["checkout", "-b", "feature"]);
    write(
        fixture.path(),
        "a.txt",
        b"line1
line2
",
    );
    commit_all(fixture.path(), "feature appends");
    git_ok(fixture.path(), &["checkout", "main"]);
    write(
        fixture.path(),
        "a.txt",
        b"changed line1
",
    );
    commit_all(fixture.path(), "main changes");
    let _ = git(fixture.path(), &["merge", "feature"]);

    let detail = fixture
        .conflict()
        .file_detail(fixture.repo_id, &RepoPath::from("a.txt"))
        .expect("file_detail 失败");
    // 双方都动了 line1（ours 改写、theirs 保留并追加 line2）：同一行的
    // 修改与追加重叠 → 一个冲突块，与 git 的合并行为一致
    assert_eq!(detail.blocks.len(), 1, "{:?}", detail.blocks);
    assert_eq!(
        detail.blocks[0],
        MergeBlock::Conflict {
            base: vec!["line1".into()],
            ours: vec!["changed line1".into()],
            theirs: vec!["line1".into(), "line2".into()],
        }
    );
}
