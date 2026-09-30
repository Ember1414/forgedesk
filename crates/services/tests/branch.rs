//! 分支与标签管理（T2.5）的集成测试。
//!
//! # 本文件钉住的行为（任务书验收项）
//!
//! 1. **切换三策略**：Clean 拒绝不干净工作区；Stash 自动储藏并在切换后恢复；
//!    Force 无显式确认时拒绝、确认时先打快照再丢弃。
//! 2. **未合并删除确认**：`-d` 被拒（git）；force 无 `confirm_unmerged` 被拒
//!    （服务层）；确认后可删且先行快照；当前分支不可删。
//! 3. **分支名校验**：服务层拒绝非法名（带人话原因）。
//! 4. 标签 CRUD（轻量/附注）与比较（ahead/behind/独有清单）。
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::path::Path;

use forgedesk_domain::git::{
    validate_ref_name, BranchCreateSpec, BranchDeleteSpec, BranchRenameSpec, SwitchStrategy,
    TagCreateSpec, TagDeleteSpec,
};
use forgedesk_domain::ErrorCode;
use forgedesk_git_engine::engines::GitEngines;
use forgedesk_services::BranchService;
use forgedesk_snapshot::{SnapshotError, SnapshotId, SnapshotManager, SnapshotRequest};
use support::{git, git_ok, init_repo, write, TempDir};

/// 测试用快照管理器：记录 create 调用（断言"危险操作先打快照"）。
#[derive(Default)]
struct RecordingSnapshots {
    calls: std::sync::Mutex<Vec<&'static str>>,
}

impl std::fmt::Debug for RecordingSnapshots {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecordingSnapshots").finish()
    }
}

impl SnapshotManager for RecordingSnapshots {
    fn create(
        &self,
        request: &SnapshotRequest<'_>,
    ) -> Result<forgedesk_snapshot::SnapshotOutcome, SnapshotError> {
        self.calls.lock().unwrap().push(match request.kind {
            forgedesk_snapshot::SnapshotKind::PreHeadMove => "pre_head_move",
            _ => "other",
        });
        Ok(forgedesk_snapshot::SnapshotOutcome::bare(77))
    }

    fn list(
        &self,
        _repo_id: i64,
        _limit: i64,
    ) -> Result<Vec<forgedesk_snapshot::SnapshotMeta>, SnapshotError> {
        Ok(Vec::new())
    }

    fn restore(
        &self,
        _repo_id: i64,
        _snapshot_id: SnapshotId,
    ) -> Result<forgedesk_snapshot::RestoreReport, SnapshotError> {
        Err(SnapshotError::NotFound(0))
    }

    fn diff(
        &self,
        _repo_id: i64,
        _snapshot_id: SnapshotId,
    ) -> Result<forgedesk_snapshot::SnapshotDiff, SnapshotError> {
        Err(SnapshotError::NotFound(0))
    }

    fn prune(
        &self,
        _repo_id: i64,
        _policy: &forgedesk_snapshot::RetentionPolicy,
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

fn service(dir: &TempDir) -> (BranchService<'static>, &'static RecordingSnapshots) {
    let engines: &'static GitEngines =
        Box::leak(Box::new(GitEngines::new().expect("创建引擎失败")));
    let database: &'static forge_setup::LeakDatabase =
        Box::leak(Box::new(forge_setup::leak_database()));
    let snapshots: &'static RecordingSnapshots = Box::leak(Box::new(RecordingSnapshots::default()));
    let store = forge_setup::store(database);
    forge_setup::register(&store, dir.path());
    let service = BranchService::new(engines, store, snapshots);
    (service, snapshots)
}

/// 借用生命周期的小包装（与 history.rs 的 Box::leak 同一技巧，集中在这里避免重复）。
mod forge_setup {
    use forgedesk_storage::{Database, RepositoryStore, RepositoryUpsert};
    use std::path::Path;

    pub struct LeakDatabase {
        inner: Database,
    }

    pub fn leak_database() -> LeakDatabase {
        let database = Database::open_in_memory().expect("打开内存库失败");
        forgedesk_storage::migrate(&database).expect("迁移失败");
        LeakDatabase { inner: database }
    }

    pub fn store(database: &'static LeakDatabase) -> RepositoryStore<'static> {
        // LeakDatabase 永不 drop（Box::leak），所以借用 'static 安全
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

/// base → main 提交 + 未合并的 feature 分支（独有一条提交）。
fn fixture_with_unmerged_branch(label: &str) -> TempDir {
    let dir = TempDir::new(label);
    init_repo(dir.path());
    write(dir.path(), "base.txt", b"base\n");
    commit_at(dir.path(), "base", 1_700_000_000);

    write(dir.path(), "main.txt", b"main\n");
    commit_at(dir.path(), "main work", 1_700_000_060);

    git_ok(dir.path(), &["checkout", "-q", "-b", "feature"]);
    write(dir.path(), "feature.txt", b"feature\n");
    commit_at(dir.path(), "feature work", 1_700_000_120);
    git_ok(dir.path(), &["checkout", "-q", "main"]);
    dir
}

// ------------------------------------------------------------ 切换三策略

#[test]
fn clean_strategy_refuses_a_dirty_worktree() {
    let dir = fixture_with_unmerged_branch("switch-clean");
    write(dir.path(), "main.txt", b"dirty\n");
    let (service, _) = service(&dir);

    let error = service
        .switch_by_id(1, "feature", SwitchStrategy::Clean, false)
        .expect_err("不干净工作区应拒绝");
    assert_eq!(error.code, ErrorCode::Validation);
}

#[test]
fn stash_strategy_stashes_and_restores_around_the_switch() {
    let dir = fixture_with_unmerged_branch("switch-stash");
    write(dir.path(), "main.txt", b"dirty but precious\n");
    let (service, _) = service(&dir);

    service
        .switch_by_id(1, "feature", SwitchStrategy::Stash, false)
        .expect("stash 策略应成功");

    // 已在 feature 上
    let output = git(dir.path(), &["rev-parse", "--abbrev-ref", "HEAD"]);
    assert_eq!(output.stdout_lossy().trim(), "feature");
    // 修改被恢复回来了（stash 自动 pop）
    let content = std::fs::read_to_string(dir.path().join("main.txt")).unwrap();
    assert!(content.contains("dirty but precious"), "修改应随切换恢复");
    // stash 栈空了
    let output = git(dir.path(), &["stash", "list"]);
    assert!(
        output.stdout_lossy().trim().is_empty(),
        "stash 应已恢复并清空"
    );
}

#[test]
fn stash_strategy_keeps_the_stash_when_the_switch_fails() {
    let dir = fixture_with_unmerged_branch("switch-stash-fail");
    // 制造"必然失败的切换"：目标分支不存在（但名称合法）
    write(dir.path(), "main.txt", b"dirty\n");
    let (service, _) = service(&dir);

    let error = service
        .switch_by_id(1, "nonexistent-branch", SwitchStrategy::Stash, false)
        .expect_err("切换到不存在的分支应失败");
    assert_eq!(error.code, ErrorCode::Internal);

    // stash 留在栈里：用户的工作没有丢
    let output = git(dir.path(), &["stash", "list"]);
    assert!(
        !output.stdout_lossy().trim().is_empty(),
        "失败的切换不得吞掉 stash"
    );
}

#[test]
fn force_strategy_requires_confirmation_then_snapshots_and_discards() {
    let dir = fixture_with_unmerged_branch("switch-force");
    write(dir.path(), "main.txt", b"doomed changes\n");
    let (service, snapshots) = service(&dir);

    // 无确认：拒绝
    let error = service
        .switch_by_id(1, "feature", SwitchStrategy::Force, false)
        .expect_err("force 无确认应拒绝");
    assert_eq!(error.code, ErrorCode::Validation);
    assert!(
        snapshots.calls.lock().unwrap().is_empty(),
        "拒绝路径不得先打快照"
    );

    // 确认：先快照，再丢弃
    service
        .switch_by_id(1, "feature", SwitchStrategy::Force, true)
        .expect("确认后应成功");
    assert_eq!(
        snapshots.calls.lock().unwrap().as_slice(),
        &["pre_head_move"]
    );
    let output = git(dir.path(), &["rev-parse", "--abbrev-ref", "HEAD"]);
    assert_eq!(output.stdout_lossy().trim(), "feature");
    let content = std::fs::read_to_string(dir.path().join("main.txt")).unwrap();
    assert!(!content.contains("doomed"), "force 应丢弃未提交修改");
}

// ------------------------------------------------------------ 删除与确认

#[test]
fn deleting_an_unmerged_branch_requires_confirmation_and_snapshots() {
    let dir = fixture_with_unmerged_branch("delete-unmerged");
    let (service, snapshots) = service(&dir);

    // 普通 -d：git 拒绝未合并分支
    let plain = BranchDeleteSpec {
        names: vec!["feature".to_owned()],
        force: false,
        also_delete_remote: false,
    };
    let error = service
        .branch_delete(1, &plain, false)
        .expect_err("未合并分支 -d 应失败");
    assert_eq!(error.code, ErrorCode::Internal, "git 的拒绝以内部错误传播");

    // force 但没确认：服务层拒绝
    let forced = BranchDeleteSpec {
        names: vec!["feature".to_owned()],
        force: true,
        also_delete_remote: false,
    };
    let error = service
        .branch_delete(1, &forced, false)
        .expect_err("force 未确认应拒绝");
    assert_eq!(error.code, ErrorCode::Validation);

    // 确认后：先快照再删
    service
        .branch_delete(1, &forced, true)
        .expect("确认后应删除成功");
    assert_eq!(
        snapshots.calls.lock().unwrap().as_slice(),
        &["pre_head_move"]
    );
    let output = git(dir.path(), &["branch", "--list", "feature"]);
    assert!(output.stdout_lossy().trim().is_empty(), "分支应已删除");
}

#[test]
fn deleting_the_current_branch_is_rejected_up_front() {
    let dir = fixture_with_unmerged_branch("delete-current");
    let (service, _) = service(&dir);
    let spec = BranchDeleteSpec {
        names: vec!["main".to_owned()],
        force: false,
        also_delete_remote: false,
    };
    let error = service
        .branch_delete(1, &spec, false)
        .expect_err("当前分支不可删");
    assert_eq!(error.code, ErrorCode::Validation);
}

#[test]
fn a_merged_branch_deletes_without_confirmation() {
    let dir = fixture_with_unmerged_branch("delete-merged");
    git_ok(
        dir.path(),
        &["merge", "-q", "--no-ff", "-m", "merge feature", "feature"],
    );
    let (service, _) = service(&dir);
    let spec = BranchDeleteSpec {
        names: vec!["feature".to_owned()],
        force: false,
        also_delete_remote: false,
    };
    let outcome = service
        .branch_delete(1, &spec, false)
        .expect("已合并分支直接删");
    assert_eq!(outcome.deleted, vec!["feature".to_owned()]);
}

// ------------------------------------------------------------ 校验与列表

#[test]
fn invalid_names_are_rejected_with_reasons() {
    let dir = fixture_with_unmerged_branch("create-invalid");
    let (service, _) = service(&dir);

    for name in ["feat branch", "..dots", "ends-with-.lock"] {
        let spec = BranchCreateSpec {
            name: name.to_owned(),
            start_point: None,
            checkout: false,
            track_upstream: None,
        };
        let error = service.branch_create(1, &spec).expect_err(name);
        assert_eq!(error.code, ErrorCode::Validation, "{name}");
    }
}

#[test]
fn branch_create_checkout_and_rename_round_trip() {
    let dir = fixture_with_unmerged_branch("create-round-trip");
    let (service, _) = service(&dir);

    // 创建（不切换）
    let spec = BranchCreateSpec {
        name: "topic".to_owned(),
        start_point: Some("main".to_owned()),
        checkout: false,
        track_upstream: None,
    };
    service.branch_create(1, &spec).expect("创建失败");
    let branches = service.branch_list(1, false).expect("列表失败");
    assert!(branches.iter().any(|b| b.name == "topic"));
    assert!(branches.iter().find(|b| b.name == "main").unwrap().is_head);

    // 创建并切换
    let spec = BranchCreateSpec {
        name: "topic2".to_owned(),
        start_point: None,
        checkout: true,
        track_upstream: None,
    };
    service.branch_create(1, &spec).expect("创建+切换失败");
    let output = git(dir.path(), &["rev-parse", "--abbrev-ref", "HEAD"]);
    assert_eq!(output.stdout_lossy().trim(), "topic2");

    // 重命名
    let rename = BranchRenameSpec {
        old: "topic2".to_owned(),
        new: "topic-renamed".to_owned(),
        rename_remote: false,
    };
    service.branch_rename(1, &rename).expect("重命名失败");
    let branches = service.branch_list(1, false).expect("列表失败");
    assert!(branches.iter().any(|b| b.name == "topic-renamed"));
    assert!(!branches.iter().any(|b| b.name == "topic2"));

    // 列表排序：HEAD 置顶
    assert!(branches.first().unwrap().is_head);
}

// ------------------------------------------------------------ 比较

#[test]
fn branch_compare_reports_ahead_and_only_commits() {
    let dir = fixture_with_unmerged_branch("compare");
    let (service, _) = service(&dir);

    let comparison = service
        .branch_compare(1, "feature", "main")
        .expect("比较失败");
    // 夹具：feature 从 main 的 tip 分出后多一条 feature work → ahead=1、behind=0
    assert_eq!(
        (comparison.ahead, comparison.behind),
        (1, 0),
        "feature 独有 feature work；main 无独有提交"
    );
    assert_eq!(comparison.only_in_a.len(), 1);
    assert_eq!(comparison.only_in_a[0].1, "feature work");
}

// ------------------------------------------------------------ 标签

#[test]
fn tag_create_list_delete_round_trip() {
    let dir = fixture_with_unmerged_branch("tags");
    let (service, _) = service(&dir);

    // 轻量
    let light = TagCreateSpec {
        name: "v1.0.0".to_owned(),
        target: Some("main".to_owned()),
        message: None,
        sign: false,
        force: false,
    };
    service.tag_create(1, &light).expect("轻量标签失败");

    // 附注
    let annotated = TagCreateSpec {
        name: "v1.1.0".to_owned(),
        target: None,
        message: Some("second release".to_owned()),
        sign: false,
        force: false,
    };
    service.tag_create(1, &annotated).expect("附注标签失败");

    let tags = service.tag_list(1).expect("标签列表失败");
    assert_eq!(tags.len(), 2);
    let annotated_tag = tags.iter().find(|t| t.name == "v1.1.0").unwrap();
    assert!(annotated_tag.annotated);
    assert_eq!(annotated_tag.message.as_deref(), Some("second release"));
    let light_tag = tags.iter().find(|t| t.name == "v1.0.0").unwrap();
    assert!(!light_tag.annotated);

    // 重名：不带 force 被拒
    let error = service.tag_create(1, &light).expect_err("重名标签应失败");
    assert_eq!(error.code, ErrorCode::Internal);

    // 删除
    let spec = TagDeleteSpec {
        names: vec!["v1.0.0".to_owned(), "v1.1.0".to_owned()],
        also_delete_remote: false,
    };
    service.tag_delete(1, &spec).expect("删除标签失败");
    assert!(service.tag_list(1).expect("列表失败").is_empty());
}

#[test]
fn invalid_tag_names_are_rejected() {
    let dir = fixture_with_unmerged_branch("tags-invalid");
    let (service, _) = service(&dir);
    let spec = TagCreateSpec {
        name: "bad tag".to_owned(),
        target: None,
        message: None,
        sign: false,
        force: false,
    };
    let error = service
        .tag_create(1, &spec)
        .expect_err("含空格的标签应拒绝");
    assert_eq!(error.code, ErrorCode::Validation);
}

#[test]
fn ref_validation_covers_the_documented_rules() {
    assert!(validate_ref_name("ok/name.42").is_ok());
    assert!(validate_ref_name("a..b").is_err());
}
