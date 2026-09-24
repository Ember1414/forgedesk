//! 提交两段式（T1.7）的集成测试。
//!
//! # 为什么这些用例必须跑在真实仓库上
//!
//! `prepare` 的产物（文件清单、索引指纹、钩子列表、等价命令）全部来自真实的
//! `git status` / `git write-tree` / 钩子目录探测；而 `execute` 的成败取决于
//! git 自己怎么看那次提交（钩子、空索引、amend）。用替身模拟这些，测的就只是
//! "我们的替身有没有按我们想的那样回答"。
//!
//! # 覆盖的三类失败分支（任务验收要求）
//!
//! 计划过期（`PLAN_STALE`）、没有可提交的内容（`EMPTY_COMMIT`）、钩子拒绝
//! （`HOOK_REJECTED`）。三者都断言了**仓库没有被改动**——失败分支最容易写错的地方
//! 不是错误码，而是"报错的同时已经写了一半"。
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use forgedesk_domain::git::{AmendMode, RepoId, SignMode, Signature};
use forgedesk_domain::ErrorCode;
use forgedesk_git_engine::engine::GitEngine;
use forgedesk_services::{
    CommitPlanRegistry, CommitService, GitEngines, MillisClock, OpenRepoRegistry, PrepareRequest,
    RepositoryService,
};
use forgedesk_snapshot::{
    NoopSnapshotManager, RestoreReport, RetentionPolicy, SnapshotDiff, SnapshotError,
    SnapshotManager, SnapshotMeta, SnapshotRequest,
};
use forgedesk_storage::{Database, OperationStore, RepositoryStore};

mod support;

use support::{commit_all, git, git_ok, init_repo, memory_database, write, TempDir};

const FILE: &str = "base.txt";

/// 一个待提交的仓库夹具。
struct Fixture {
    engines: GitEngines,
    database: Database,
    snapshots: NoopSnapshotManager,
    plans: CommitPlanRegistry,
    clock: Arc<AtomicI64>,
    dir: TempDir,
    repo_id: i64,
}

impl Fixture {
    /// 建仓库 + 一个初始提交。
    fn new(label: &str) -> Self {
        let dir = TempDir::new(label);
        init_repo(dir.path());
        write(dir.path(), FILE, b"base\n");
        commit_all(dir.path(), "base");

        let engines = GitEngines::new().expect("创建引擎失败");
        let database = memory_database();
        // 注册表只在"打开仓库"这一步需要：提交本身不要求仓库已打开
        let open = OpenRepoRegistry::default();
        let repository = RepositoryService::new(&engines, RepositoryStore::new(&database), &open);
        let opened = repository.open(dir.path()).expect("打开仓库失败");

        Self {
            engines,
            database,
            snapshots: NoopSnapshotManager,
            plans: CommitPlanRegistry::new(),
            clock: Arc::new(AtomicI64::new(1_700_000_000_000)),
            dir,
            repo_id: opened.record_id,
        }
    }

    fn service(&self) -> CommitService<'_> {
        CommitService::new(
            &self.engines,
            RepositoryStore::new(&self.database),
            OperationStore::new(&self.database),
            &self.snapshots,
            &self.plans,
        )
        .with_clock(clock_of(&self.clock))
    }

    fn repo(&self) -> RepoId {
        RepoId::new(self.dir.path().to_path_buf())
    }

    /// 推进时钟（模拟"用户开着预览对话框去喝了杯咖啡"）。
    fn advance(&self, millis: i64) {
        self.clock.fetch_add(millis, Ordering::SeqCst);
    }

    /// 当前提交数（读走 libgit2，与产品同一路径）。
    fn commit_count(&self) -> usize {
        self.engines
            .read()
            .log(&self.repo(), forgedesk_domain::git::LogQuery::new())
            .expect("读取提交历史失败")
            .items
            .len()
    }

    fn head_subject(&self) -> String {
        self.engines
            .read()
            .log(
                &self.repo(),
                forgedesk_domain::git::LogQuery::new().with_limit(1),
            )
            .expect("读取提交历史失败")
            .items
            .first()
            .map(|commit| commit.subject.clone())
            .unwrap_or_default()
    }

    /// 改工作区并暂存（准备一次可提交的改动）。
    fn stage_change(&self, content: &[u8]) {
        write(self.dir.path(), FILE, content);
        git_ok(self.dir.path(), &["add", "--", FILE]);
    }

    /// HEAD 的 oid。
    fn head_oid(&self) -> String {
        stdout(self.dir.path(), &["rev-parse", "HEAD"])
    }

    /// HEAD 的树 oid —— "提交内容"的规范摘要。
    fn head_tree(&self) -> String {
        stdout(self.dir.path(), &["rev-parse", "HEAD^{tree}"])
    }

    /// 索引里相对 HEAD 的变更文件。
    ///
    /// 刻意用 git 命令而不是产品代码读：断言"产品说它做了什么"与
    /// "git 说发生了什么"是两件事，测试要的是后者。
    fn staged_files(&self) -> Vec<String> {
        stdout(self.dir.path(), &["diff", "--cached", "--name-only"])
            .lines()
            .map(str::to_owned)
            .filter(|line| !line.is_empty())
            .collect()
    }

    fn operations(&self) -> Vec<forgedesk_storage::OperationRecord> {
        OperationStore::new(&self.database)
            .recent(self.repo_id, 10)
            .expect("读取操作记录失败")
    }
}

/// 把共享计数器包成可注入的时间源（测试要能推进时间来看 TTL）。
fn clock_of(value: &Arc<AtomicI64>) -> MillisClock {
    let value = Arc::clone(value);
    Arc::new(move || value.load(Ordering::SeqCst))
}

/// 一个最简的提交请求。
fn request(subject: &str) -> PrepareRequest {
    PrepareRequest {
        subject: subject.to_owned(),
        description: None,
        amend: false,
        amend_mode: AmendMode::default(),
        sign_off: false,
        no_verify: false,
        sign: SignMode::Auto,
        author: None,
    }
}

// ---------------------------------------------------------------- 正常路径

#[test]
fn a_prepared_plan_describes_exactly_what_will_be_committed() {
    let fixture = Fixture::new("plan");
    fixture.stage_change(b"changed\n");

    let plan = fixture
        .service()
        .prepare(fixture.repo_id, &request("feat: change the base file"))
        .expect("准备计划失败");

    assert_eq!(plan.files.len(), 1);
    assert_eq!(plan.files[0].path.to_string(), FILE);
    assert_eq!(
        plan.files[0].index_status,
        forgedesk_domain::git::ChangeKind::Modified,
        "预览要按状态分组，因此计划里必须带着它"
    );
    assert_eq!(plan.message, "feat: change the base file\n");
    assert_eq!(plan.review.subject, "feat: change the base file");
    assert!(plan.hooks.is_empty(), "这个仓库没有钩子");
    assert_eq!(plan.head_oid.as_deref().map(str::len), Some(40));
    assert_eq!(plan.index_fingerprint.len(), 40, "指纹是树 oid");
    assert!(
        plan.equivalent_command
            .starts_with("git commit -m \"feat: change the base file\""),
        "等价命令要能直接粘贴：{}",
        plan.equivalent_command
    );
}

#[test]
fn executing_a_plan_commits_the_index_and_leaves_an_audit_record() {
    let fixture = Fixture::new("execute");
    fixture.stage_change(b"changed\n");

    let service = fixture.service();
    let plan = service
        .prepare(fixture.repo_id, &request("feat: commit it"))
        .expect("准备计划失败");
    let outcome = service.execute(&plan.plan_id).expect("执行计划失败");

    assert_eq!(outcome.oid.len(), 40);
    assert_eq!(outcome.subject, "feat: commit it");
    assert_eq!(outcome.snapshot_id, None, "M3 之前没有快照");
    assert_eq!(fixture.commit_count(), 2);
    assert_eq!(fixture.head_subject(), "feat: commit it");

    let records = fixture.operations();
    assert_eq!(records.len(), 1, "每次执行都要留一条审计");
    assert_eq!(records[0].op_type, "commit");
    assert_eq!(records[0].exit_code, Some(0));
    assert!(records[0].ended_at_ms.is_some(), "收尾时间必须写进去");
    assert!(
        !records[0].reversible,
        "没有快照就必须如实记为不可回滚，不能假装有"
    );
    assert!(
        records[0]
            .args_json
            .as_deref()
            .is_some_and(|args| args.contains("feat: commit it")),
        "参数摘要要能指出这是哪次提交"
    );
}

#[test]
fn a_plan_can_only_be_executed_once() {
    let fixture = Fixture::new("once");
    fixture.stage_change(b"changed\n");

    let service = fixture.service();
    let plan = service
        .prepare(fixture.repo_id, &request("feat: once"))
        .expect("准备计划失败");
    service.execute(&plan.plan_id).expect("第一次执行应当成功");

    let error = service
        .execute(&plan.plan_id)
        .expect_err("同一个计划不能执行两次");
    assert_eq!(error.code, ErrorCode::PlanStale);
    assert_eq!(fixture.commit_count(), 2, "第二次没有产生提交");
}

// ---------------------------------------------------------------- 失败分支

#[test]
fn an_empty_index_is_reported_as_an_empty_commit() {
    let fixture = Fixture::new("empty-index");

    let error = fixture
        .service()
        .prepare(fixture.repo_id, &request("feat: nothing staged"))
        .expect_err("没有暂存内容时不能准备计划");

    assert_eq!(error.code, ErrorCode::EmptyCommit);
    assert_eq!(fixture.operations().len(), 0, "被拦下的操作不进审计");
}

#[test]
fn an_index_identical_to_head_is_also_an_empty_commit() {
    let fixture = Fixture::new("empty-diff");
    // 暂存后又把内容改回原样：索引与 HEAD 相同，git 也会说 nothing to commit
    fixture.stage_change(b"base\n");

    let error = fixture
        .service()
        .prepare(fixture.repo_id, &request("feat: no actual change"))
        .expect_err("索引与 HEAD 相同时不能准备计划");

    assert_eq!(error.code, ErrorCode::EmptyCommit);
}

#[test]
fn a_plan_whose_index_changed_is_rejected_before_anything_is_written() {
    let fixture = Fixture::new("stale");
    fixture.stage_change(b"changed\n");

    let service = fixture.service();
    let plan = service
        .prepare(fixture.repo_id, &request("feat: stale"))
        .expect("准备计划失败");

    // 用户在另一个终端里又 add 了一个文件（这是完全正常的用法）
    write(fixture.dir.path(), "extra.txt", b"extra\n");
    git_ok(fixture.dir.path(), &["add", "--", "extra.txt"]);

    let error = service
        .execute(&plan.plan_id)
        .expect_err("索引变了就必须拒绝执行");

    assert_eq!(error.code, ErrorCode::PlanStale);
    assert_eq!(
        fixture.commit_count(),
        1,
        "被拒绝的计划不能产生提交（仓库里只有初始那一个提交）"
    );
    assert_eq!(
        fixture.operations().len(),
        0,
        "指纹校验发生在审计之前：被拦下的执行没有写任何仓库状态"
    );
}

#[test]
fn an_expired_plan_is_rejected() {
    let fixture = Fixture::new("expired");
    fixture.stage_change(b"changed\n");

    let service = fixture.service();
    let plan = service
        .prepare(fixture.repo_id, &request("feat: expired"))
        .expect("准备计划失败");

    fixture.advance(forgedesk_domain::git::COMMIT_PLAN_TTL_MS + 1);

    let error = service
        .execute(&plan.plan_id)
        .expect_err("过期计划必须被拒绝");
    assert_eq!(error.code, ErrorCode::PlanStale);
    assert_eq!(fixture.commit_count(), 1);
}

#[test]
fn a_broken_message_is_rejected_before_any_git_command_runs() {
    let fixture = Fixture::new("blank-message");
    fixture.stage_change(b"changed\n");

    let error = fixture
        .service()
        .prepare(fixture.repo_id, &request("   \n"))
        .expect_err("空白提交信息必须被拒绝");

    assert_eq!(error.code, ErrorCode::Validation);
    assert_eq!(fixture.operations().len(), 0);
}

#[test]
fn a_conflicted_repository_cannot_prepare_a_commit() {
    let fixture = Fixture::new("conflict");
    let dir = fixture.dir.path().to_path_buf();

    // 造一个真实的合并冲突
    git_ok(&dir, &["checkout", "-q", "-b", "other"]);
    write(&dir, FILE, b"other\n");
    commit_all(&dir, "other change");
    git_ok(&dir, &["checkout", "-q", "main"]);
    write(&dir, FILE, b"main\n");
    commit_all(&dir, "main change");
    let merge = git(&dir, &["merge", "other"]);
    assert!(
        !merge.success(),
        "这个夹具必须真的产生冲突，否则用例没有意义"
    );

    let error = fixture
        .service()
        .prepare(fixture.repo_id, &request("feat: while conflicted"))
        .expect_err("冲突状态下不能准备提交");

    assert_eq!(error.code, ErrorCode::GitConflict);
}

// ---------------------------------------------------------------- 钩子

#[test]
fn a_hook_rejection_is_reported_with_its_raw_output_and_can_be_retried_without_hooks() {
    let fixture = Fixture::new("hook");
    fixture.stage_change(b"changed\n");
    install_pre_commit(&fixture.dir);
    git_ok(fixture.dir.path(), &["add", "--all"]);

    let service = fixture.service();
    let plan = service
        .prepare(fixture.repo_id, &request("feat: hooked"))
        .expect("准备计划失败");
    assert_eq!(plan.hooks, vec!["pre-commit".to_owned()]);

    let error = service
        .execute(&plan.plan_id)
        .expect_err("钩子拒绝后必须报错");
    assert_eq!(error.code, ErrorCode::HookRejected);
    assert!(
        error
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("nope")),
        "原始输出必须保留给用户看：{:?}",
        error.detail
    );
    assert_eq!(error.hint.as_deref(), Some("pre-commit"), "hint 只放数据");
    assert_eq!(fixture.commit_count(), 1, "被钩子拒绝的提交不该落地");

    let records = fixture.operations();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].exit_code, Some(1));

    // "禁用钩子重试"的路径：显式 no_verify 才跳过钩子
    let retry = service
        .prepare(
            fixture.repo_id,
            &PrepareRequest {
                no_verify: true,
                ..request("feat: hooked")
            },
        )
        .expect("准备计划失败");
    service
        .execute(&retry.plan_id)
        .expect("跳过钩子的提交应当成功");
    assert_eq!(fixture.commit_count(), 2);
}

#[test]
fn the_author_override_reaches_the_commit() {
    let fixture = Fixture::new("author");
    fixture.stage_change(b"changed\n");

    let service = fixture.service();
    let plan = service
        .prepare(
            fixture.repo_id,
            &PrepareRequest {
                author: Some(Signature::new("Ada Lovelace", "ada@example.com")),
                ..request("feat: authored")
            },
        )
        .expect("准备计划失败");
    service.execute(&plan.plan_id).expect("执行失败");

    let head = fixture
        .engines
        .read()
        .log(
            &fixture.repo(),
            forgedesk_domain::git::LogQuery::new().with_limit(1),
        )
        .expect("读取提交失败")
        .items
        .remove(0);
    assert_eq!(head.author.name, "Ada Lovelace");
    assert!(plan.equivalent_command.contains("Ada Lovelace"));
}

#[test]
fn a_signing_off_commit_carries_the_trailer() {
    let fixture = Fixture::new("signoff");
    fixture.stage_change(b"changed\n");

    let service = fixture.service();
    let plan = service
        .prepare(
            fixture.repo_id,
            &PrepareRequest {
                sign_off: true,
                ..request("feat: dco")
            },
        )
        .expect("准备计划失败");
    service.execute(&plan.plan_id).expect("执行失败");

    // `--signoff` 把 Signed-off-by 写进提交信息（正文里）
    let output = git(fixture.dir.path(), &["log", "-1", "--format=%B"]);
    assert!(
        output.stdout_lossy().contains("Signed-off-by:"),
        "提交信息里应当有签名行：{}",
        output.stdout_lossy()
    );
}

#[test]
fn amend_changes_only_the_message_when_nothing_new_is_staged() {
    let fixture = Fixture::new("amend");
    let service = fixture.service();

    let plan = service
        .prepare(
            fixture.repo_id,
            &PrepareRequest {
                amend: true,
                ..request("base (fixed)")
            },
        )
        .expect("准备 amend 计划失败");
    service.execute(&plan.plan_id).expect("amend 失败");

    assert_eq!(fixture.commit_count(), 1, "amend 不增加提交数");
    assert_eq!(fixture.head_subject(), "base (fixed)");
    assert!(
        plan.equivalent_command.contains("--amend"),
        "等价命令要带上 --amend：{}",
        plan.equivalent_command
    );
}

// ---------------------------------------------------------------- amend（T1.8）

/// 执行一条 git 命令并取回 trim 后的 stdout（断言成功）。
fn stdout(dir: &Path, command: &[&str]) -> String {
    String::from_utf8_lossy(&git(dir, command).stdout)
        .trim()
        .to_owned()
}

/// 建一个裸远端并把 HEAD 推上去（造出"已推送"的场景）。
///
/// 返回的 `TempDir` 必须活到测试结束：它 `Drop` 时会删掉远端目录。
fn push_to_bare_remote(fixture: &Fixture) -> TempDir {
    let remote = TempDir::new("bare-origin");
    git_ok(remote.path(), &["init", "--bare", "-q", "-b", "main"]);
    let url = remote.path().to_string_lossy().into_owned();
    git_ok(fixture.dir.path(), &["remote", "add", "origin", &url]);
    git_ok(
        fixture.dir.path(),
        &["push", "-q", "-u", "origin", "HEAD:main"],
    );
    remote
}

#[test]
fn amending_with_message_only_keeps_the_staged_content_out_of_the_commit() {
    let fixture = Fixture::new("amend-message-only");
    fixture.stage_change(b"changed\n");
    let head_before = fixture.head_oid();
    let tree_before = fixture.head_tree();

    let spec = PrepareRequest {
        amend: true,
        amend_mode: AmendMode::MessageOnly,
        ..request("fix: typo in the message")
    };
    let plan = fixture
        .service()
        .prepare(fixture.repo_id, &spec)
        .expect("准备计划失败");
    assert_eq!(
        plan.files.len(),
        1,
        "预览仍要如实列出索引里的内容（只是这次不会提交它）"
    );

    let outcome = fixture.service().execute(&plan.plan_id).expect("执行失败");

    assert_eq!(fixture.commit_count(), 1, "amend 不增加提交数");
    assert_ne!(outcome.oid, head_before, "提交被替换，oid 必须变化");
    assert_eq!(
        fixture.head_tree(),
        tree_before,
        "只改信息时提交内容必须一模一样"
    );
    assert_eq!(fixture.head_subject(), "fix: typo in the message");
    assert_eq!(
        fixture.staged_files(),
        vec![FILE.to_owned()],
        "暂存的改动必须原样留在索引里，不能被顺手提交掉"
    );
}

#[test]
fn amending_with_the_staged_content_folds_it_into_the_last_commit() {
    let fixture = Fixture::new("amend-include-staged");
    fixture.stage_change(b"changed\n");
    let tree_before = fixture.head_tree();
    let head_before = fixture.head_oid();

    let spec = PrepareRequest {
        amend: true,
        amend_mode: AmendMode::IncludeStaged,
        ..request("feat: more of the same")
    };
    let plan = fixture
        .service()
        .prepare(fixture.repo_id, &spec)
        .expect("准备计划失败");
    let outcome = fixture.service().execute(&plan.plan_id).expect("执行失败");

    assert_eq!(fixture.commit_count(), 1, "amend 不增加提交数");
    assert_ne!(outcome.oid, head_before);
    assert_ne!(
        fixture.head_tree(),
        tree_before,
        "这条路径的提交内容必须变（改动被并进去了）"
    );
    assert!(
        fixture.staged_files().is_empty(),
        "索引内容进了提交，索引随之变干净"
    );
}

#[test]
fn amend_context_reports_the_last_message_and_whether_it_reached_a_remote() {
    let fixture = Fixture::new("amend-context");

    // 还没有远端：不能提示"可能已推送"（假警报会让用户对提示失去信任）
    let before = fixture
        .service()
        .amend_context(fixture.repo_id)
        .expect("读取 amend 语境失败");
    assert_eq!(before.subject.as_deref(), Some("base"));
    assert_eq!(
        before.head_oid.as_deref(),
        Some(fixture.head_oid().as_str())
    );
    assert!(!before.pushed, "没推过就不该说推过");

    let _remote = push_to_bare_remote(&fixture);

    let after = fixture
        .service()
        .amend_context(fixture.repo_id)
        .expect("读取 amend 语境失败");
    assert!(after.pushed, "推过之后必须提示改写历史的后果");
    assert_eq!(after.pushed_refs, vec!["origin/main".to_owned()]);
}

#[test]
fn hooks_are_listed_from_the_effective_hooks_directory() {
    let fixture = Fixture::new("hooks-list");
    write(
        fixture.dir.path(),
        ".git/hooks/pre-commit",
        b"#!/bin/sh\nexit 0\n",
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = fixture.dir.path().join(".git/hooks/pre-commit");
        let mut permissions = std::fs::metadata(&path).expect("读权限失败").permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).expect("设置权限失败");
    }

    let hooks = fixture
        .service()
        .hooks(fixture.repo_id)
        .expect("读取钩子失败");

    let pre_commit = hooks
        .iter()
        .find(|hook| hook.name == "pre-commit")
        .expect("pre-commit 必须在清单里");
    assert!(pre_commit.commit_hook, "它属于提交时会执行的那三类");
}

// ---------------------------------------------------------------- 提示与快照

#[test]
fn message_hints_come_from_recent_subjects_and_the_branch_prefix() {
    let fixture = Fixture::new("hint");
    fixture.stage_change(b"second\n");
    commit_all(fixture.dir.path(), "feat: second");
    git_ok(
        fixture.dir.path(),
        &["checkout", "-q", "-b", "feat/login-page"],
    );

    let hint = fixture
        .service()
        .message_hint(fixture.repo_id)
        .expect("读取提示失败");

    assert_eq!(
        hint.recent_messages,
        vec!["feat: second".to_owned(), "base".to_owned()],
        "新 → 旧"
    );
    assert_eq!(hint.branch_style.as_deref(), Some("feat"));
    assert_eq!(hint.template.as_deref(), Some("feat: "));
}

#[test]
fn a_snapshot_manager_that_fails_does_not_block_the_commit() {
    // M3 之前没有快照；M3 之后这里会变成"用户可见的降级提示"，但**不能**让提交失败：
    // 用户要提交的意图与"本地能不能打点"是两件事
    #[derive(Debug)]
    struct FailingSnapshots;

    impl SnapshotManager for FailingSnapshots {
        fn create(&self, _request: &SnapshotRequest<'_>) -> Result<i64, SnapshotError> {
            Err(SnapshotError::Failed("disk full".to_owned()))
        }

        fn list(&self, _repo_id: i64, _limit: i64) -> Result<Vec<SnapshotMeta>, SnapshotError> {
            Ok(Vec::new())
        }

        fn restore(&self, _repo_id: i64, snapshot_id: i64) -> Result<RestoreReport, SnapshotError> {
            Err(SnapshotError::NotFound(snapshot_id))
        }

        fn diff(&self, _repo_id: i64, snapshot_id: i64) -> Result<SnapshotDiff, SnapshotError> {
            Err(SnapshotError::NotFound(snapshot_id))
        }

        fn prune(
            &self,
            _repo_id: i64,
            _policy: &RetentionPolicy,
        ) -> Result<Vec<i64>, SnapshotError> {
            Ok(Vec::new())
        }
    }

    let fixture = Fixture::new("snapshot-failure");
    fixture.stage_change(b"changed\n");
    let snapshots = FailingSnapshots;

    let service = CommitService::new(
        &fixture.engines,
        RepositoryStore::new(&fixture.database),
        OperationStore::new(&fixture.database),
        &snapshots,
        &fixture.plans,
    )
    .with_clock(clock_of(&fixture.clock));

    let plan = service
        .prepare(fixture.repo_id, &request("feat: despite snapshot failure"))
        .expect("准备计划失败");
    let outcome = service
        .execute(&plan.plan_id)
        .expect("提交不该被快照失败拦住");

    assert_eq!(outcome.snapshot_id, None);
    assert_eq!(fixture.commit_count(), 2);
    assert!(
        !fixture.operations()[0].reversible,
        "没有快照就必须记成不可回滚"
    );
}

/// 装一个必定失败的 `pre-commit` 钩子。
fn install_pre_commit(dir: &TempDir) {
    let hooks = dir.path().join(".git/hooks");
    std::fs::create_dir_all(&hooks).expect("创建 hooks 目录失败");
    let hook = hooks.join("pre-commit");
    std::fs::write(&hook, "#!/bin/sh\necho 'pre-commit: nope' >&2\nexit 1\n").expect("写钩子失败");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&hook)
            .expect("读取权限失败")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&hook, permissions).expect("设置权限失败");
    }
}
