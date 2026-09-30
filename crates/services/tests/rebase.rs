//! rebase 执行引擎（T3.7）的集成测试：三种结局 + edit 恢复，全部在真实
//! 仓库上跑真实 `git rebase -i`（todo 经 GIT_SEQUENCE_EDITOR 注入）。
//!
//! # 本文件钉住的行为（任务书验收项）
//!
//! 1. **完成**：squash 三个提交（pick+pick+... 的重排由 preview 断言数量），
//!    HEAD 前进、无暂停标记。
//! 2. **冲突暂停**：rebase 停在冲突 → `PausedConflict` 带清单 →
//!    解决后走 T3.1 的 `git rebase --continue`（应用不做额外动作）。
//! 3. **edit 暂停**：`PausedEdit` 带 REBASE_HEAD → 用户改内容 →
//!    `continue_after_edit`（amend 接住 + continue）→ 完成。
//! 4. **幂等**：edit 恢复后重复调用 `continue_after_edit` 必须拒绝。
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use forgedesk_domain::git::{RebaseOutcome, RebasePlan, ReorderAction, ReorderStep};
use forgedesk_git_engine::engines::GitEngines;
use forgedesk_services::RebaseService;
use forgedesk_snapshot::SnapshotManager;
use support::{commit_all, git, git_ok, init_repo, write, TempDir};

// ---------------------------------------------------------------- 夹具

/// RebaseService 只需要引擎（快照由 RecordingSnapshots 占位——与 merge.rs
/// 同一写法；本文件不关心快照内容，只关心不缺位）。
/// 全测试共享的内存库（Fixture::new 与 rebase_service 必须同一实例）。
static DATABASE: std::sync::OnceLock<forgedesk_storage::Database> = std::sync::OnceLock::new();

struct Fixture {
    dir: TempDir,
    repo_id: i64,
    engines: &'static GitEngines,
}

fn rebase_service(fixture: &Fixture) -> RebaseService<'_> {
    // DATABASE 是模块级 static（与 Fixture::new 共享同一实例）
    // 快照管理器在引擎测试里没有 Database 可供真实实现——用一个 no-op
    //（快照的"有没有打"由 services/tests/merge.rs 的 RecordingSnapshots 钉）
    #[derive(Debug, Default)]
    struct NoopSnapshots;
    impl SnapshotManager for NoopSnapshots {
        fn create(
            &self,
            _request: &forgedesk_snapshot::SnapshotRequest<'_>,
        ) -> Result<forgedesk_snapshot::SnapshotId, forgedesk_snapshot::SnapshotError> {
            Ok(1)
        }
        fn list(
            &self,
            _: i64,
            _: i64,
        ) -> Result<Vec<forgedesk_snapshot::SnapshotMeta>, forgedesk_snapshot::SnapshotError>
        {
            Ok(Vec::new())
        }
        fn restore(
            &self,
            _: i64,
            _: forgedesk_snapshot::SnapshotId,
        ) -> Result<forgedesk_snapshot::RestoreReport, forgedesk_snapshot::SnapshotError> {
            Err(forgedesk_snapshot::SnapshotError::NotFound(0))
        }
        fn diff(
            &self,
            _: i64,
            _: forgedesk_snapshot::SnapshotId,
        ) -> Result<forgedesk_snapshot::SnapshotDiff, forgedesk_snapshot::SnapshotError> {
            Err(forgedesk_snapshot::SnapshotError::NotFound(0))
        }
        fn prune(
            &self,
            _: i64,
            _: &forgedesk_snapshot::RetentionPolicy,
        ) -> Result<Vec<forgedesk_snapshot::SnapshotId>, forgedesk_snapshot::SnapshotError>
        {
            Ok(Vec::new())
        }
    }
    static SNAPSHOTS: std::sync::OnceLock<NoopSnapshots> = std::sync::OnceLock::new();
    let database = DATABASE.get_or_init(|| {
        let database = forgedesk_storage::Database::open_in_memory().expect("打开内存库失败");
        forgedesk_storage::migrate(&database).expect("迁移失败");
        database
    });
    RebaseService::new(
        fixture.engines,
        forgedesk_storage::RepositoryStore::new(database),
        SNAPSHOTS.get_or_init(NoopSnapshots::default),
    )
}

impl Fixture {
    fn new(prefix: &str) -> Self {
        let dir = TempDir::new(prefix);
        init_repo(dir.path());
        let engines: &'static GitEngines =
            Box::leak(Box::new(GitEngines::new().expect("创建引擎失败")));
        // RebaseService 的 resolve_workdir 需要仓库记录：登记进内存库
        let database = DATABASE.get_or_init(|| {
            let database = forgedesk_storage::Database::open_in_memory().expect("打开内存库失败");
            forgedesk_storage::migrate(&database).expect("迁移失败");
            database
        });
        let repo_id = forgedesk_storage::RepositoryStore::new(database)
            .upsert(
                &forgedesk_storage::RepositoryUpsert {
                    name: "fixture".to_owned(),
                    path: dir.path().to_string_lossy().to_string(),
                    default_branch: Some("main".to_owned()),
                    provider_id: None,
                    size_class: None,
                },
                0,
            )
            .expect("登记仓库失败");
        Self {
            dir,
            repo_id,
            engines,
        }
    }

    fn path(&self) -> &std::path::Path {
        self.dir.path()
    }

    fn head(&self) -> String {
        git(self.path(), &["rev-parse", "HEAD"])
            .stdout_lossy()
            .trim()
            .to_owned()
    }

    /// 区间的左端：linear() 的第一个提交（base 本身，不在重排区间内）。
    fn base(&self) -> String {
        git(self.path(), &["rev-parse", "HEAD~3"])
            .stdout_lossy()
            .trim()
            .to_owned()
    }

    /// 三个提交：one / two / three（HEAD=three，base=one 的父）。
    fn linear(&mut self) {
        write(self.path(), "f0.txt", b"0\n");
        commit_all(self.path(), "base");
        write(self.path(), "f1.txt", b"1\n");
        commit_all(self.path(), "one");
        write(self.path(), "f2.txt", b"2\n");
        commit_all(self.path(), "two");
        write(self.path(), "f3.txt", b"3\n");
        commit_all(self.path(), "three");
    }

    /// 生成"倒置前两个 + 保留第三个"的重排计划（验证重排真的发生）。
    fn plan(&self) -> RebasePlan {
        let base = self.base();
        let chain: Vec<String> = ["HEAD~2", "HEAD~1", "HEAD"]
            .iter()
            .map(|rev| {
                git(self.path(), &["rev-parse", rev])
                    .stdout_lossy()
                    .trim()
                    .to_owned()
            })
            .collect();
        let head = chain[2].clone();
        // pick two（先落）再 pick one：真重排
        let steps = vec![
            ReorderStep {
                oid: chain[1].clone(),
                action: ReorderAction::Pick,
                new_message: None,
            },
            ReorderStep {
                oid: chain[0].clone(),
                action: ReorderAction::Pick,
                new_message: None,
            },
            ReorderStep {
                oid: head.clone(),
                action: ReorderAction::Pick,
                new_message: None,
            },
        ];
        RebasePlan {
            base,
            head,
            steps,
            allow_flatten_merges: false,
            autosquash: false,
        }
    }
}

// ---------------------------------------------------------------- 三种结局

#[test]
fn a_simple_reorder_completes_and_moves_the_head() {
    let mut fixture = Fixture::new("rebase-complete");
    fixture.linear();
    let head_before = fixture.head();
    let plan = fixture.plan();

    let (outcome, snapshot_id) = rebase_service(&fixture)
        .execute(fixture.repo_id, &plan)
        .expect("执行失败");
    assert!(matches!(outcome, RebaseOutcome::Completed { .. }));
    assert_eq!(
        snapshot_id,
        Some(1),
        "执行前必须打 pre-head-move 快照并把 id 回传给界面（回滚入口依赖它）"
    );
    assert_ne!(fixture.head(), head_before, "重写后 HEAD 必须移动");
    // 重排真的发生：todo 里 two 先于 one（重放后 log 新→旧 = three,one,two）
    let subjects = git(
        fixture.path(),
        &["log", "--format=%s", &format!("{}..HEAD", plan.base)],
    )
    .stdout_lossy()
    .lines()
    .map(|l| l.to_owned())
    .collect::<Vec<_>>();
    assert_eq!(subjects, vec!["three", "one", "two"]);
}

#[test]
fn a_conflict_pauses_and_the_conflict_state_machine_takes_over() {
    // 把 feature 的两个提交重放到 main（shared=OTHER）上：
    // pick c1 的改动（shared: base→ONE）应用到 OTHER 上 → 第一行三方全不同 → 冲突。
    // 这是"rebase 到另一个分支的内容上"的标准冲突形态。
    let fixture = Fixture::new("rebase-conflict");
    write(
        fixture.path(),
        "shared.txt",
        b"base
",
    );
    commit_all(fixture.path(), "base");
    git_ok(fixture.path(), &["checkout", "-b", "feature"]);
    write(
        fixture.path(),
        "shared.txt",
        b"ONE
",
    );
    commit_all(fixture.path(), "one");
    write(
        fixture.path(),
        "shared.txt",
        b"TWO
two
",
    );
    commit_all(fixture.path(), "two");
    // main 上一个独立提交（shared 改成别的）
    git_ok(fixture.path(), &["checkout", "main"]);
    write(
        fixture.path(),
        "shared.txt",
        b"OTHER
",
    );
    commit_all(fixture.path(), "main change");
    let onto = git(fixture.path(), &["rev-parse", "main"])
        .stdout_lossy()
        .trim()
        .to_owned();
    let one = git(fixture.path(), &["rev-parse", "feature~1"])
        .stdout_lossy()
        .trim()
        .to_owned();
    let head = git(fixture.path(), &["rev-parse", "feature"])
        .stdout_lossy()
        .trim()
        .to_owned();
    // 当前分支停在 main；rebase 必须在 feature 上执行
    git_ok(fixture.path(), &["checkout", "feature"]);

    let steps = vec![
        ReorderStep {
            oid: one.clone(),
            action: ReorderAction::Pick,
            new_message: None,
        },
        ReorderStep {
            oid: head.clone(),
            action: ReorderAction::Pick,
            new_message: None,
        },
    ];
    let plan = RebasePlan {
        base: onto.clone(),
        head: head.clone(),
        steps,
        allow_flatten_merges: false,
        autosquash: false,
    };

    let (outcome, _snapshot) = rebase_service(&fixture)
        .execute(fixture.repo_id, &plan)
        .expect("执行失败");
    match &outcome {
        RebaseOutcome::PausedConflict { conflicts } => {
            assert_eq!(
                conflicts.iter().map(|p| p.to_string()).collect::<Vec<_>>(),
                vec!["shared.txt"]
            );
        }
        other => panic!("期望冲突暂停，得到 {other:?}"),
    }

    // 解决冲突（采用 c1 的树内容：c2 的改动随后作为独立提交干净重放）
    // 后走 T3.1 的 rebase --continue
    write(
        fixture.path(),
        "shared.txt",
        b"ONE
",
    );
    git_ok(fixture.path(), &["add", "shared.txt"]);
    git_ok(
        fixture.path(),
        &["-c", "core.editor=true", "rebase", "--continue"],
    );
    assert_ne!(fixture.head(), onto);
    // 重放完成：两个提交都在，且 three（main 的独立提交）不在重放历史里
    let subjects = git(
        fixture.path(),
        &["log", "--format=%s", &format!("{onto}..HEAD")],
    )
    .stdout_lossy()
    .lines()
    .map(|l| l.to_owned())
    .collect::<Vec<_>>();
    assert_eq!(subjects, vec!["two", "one"]);
}

#[test]
fn an_edit_step_pauses_and_continue_after_edit_amends_and_finishes() {
    let fixture = Fixture::new("rebase-edit");
    // base / c1（要 edit 的提交）/ c2（其后照常重放）
    write(
        fixture.path(),
        "a.txt",
        b"base
",
    );
    commit_all(fixture.path(), "base");
    write(
        fixture.path(),
        "f1.txt",
        b"1
",
    );
    commit_all(fixture.path(), "one");
    // 附带 body：钉住 edit 恢复对完整信息（subject + 空行 + body）的保真
    //（曾出现过 `\n\n` 被写成真实换行 + 行首冒号的事故）
    git_ok(
        fixture.path(),
        &["commit", "--amend", "-q", "-m", "one", "-m", "one body"],
    );
    write(
        fixture.path(),
        "f2.txt",
        b"2
",
    );
    commit_all(fixture.path(), "two");
    let base = git(fixture.path(), &["rev-parse", "HEAD~2"])
        .stdout_lossy()
        .trim()
        .to_owned();
    let one = git(fixture.path(), &["rev-parse", "HEAD~1"])
        .stdout_lossy()
        .trim()
        .to_owned();
    let two = git(fixture.path(), &["rev-parse", "HEAD"])
        .stdout_lossy()
        .trim()
        .to_owned();

    let plan = RebasePlan {
        base: base.clone(),
        head: two.clone(),
        steps: vec![
            ReorderStep {
                oid: one.clone(),
                action: ReorderAction::Edit,
                new_message: None,
            },
            ReorderStep {
                oid: two.clone(),
                action: ReorderAction::Pick,
                new_message: None,
            },
        ],
        allow_flatten_merges: false,
        autosquash: false,
    };

    let service = rebase_service(&fixture);
    let (outcome, _snapshot) = service.execute(fixture.repo_id, &plan).expect("执行失败");
    let edit_oid = match &outcome {
        RebaseOutcome::PausedEdit { oid } => oid.clone(),
        other => panic!("期望 edit 暂停，得到 {other:?}"),
    };

    // edit 停点：HEAD 就是被编辑的提交（one 的重放）
    let head_at_pause = fixture.head();
    assert_eq!(head_at_pause, edit_oid, "edit 停点时 HEAD 就是被编辑的提交");

    // 用户改工作区内容并暂存（amend 的 IncludeStaged 收暂存区）
    write(
        fixture.path(),
        "f1.txt",
        b"1 edited
",
    );
    git_ok(fixture.path(), &["add", "f1.txt"]);
    service
        .continue_after_edit(fixture.repo_id)
        .expect("恢复失败");

    // edit 的提交被改写（内容变了、oid 变了），two 重放其上，完成
    let content = std::fs::read_to_string(fixture.path().join("f1.txt")).unwrap();
    assert_eq!(
        content,
        "1 edited
"
    );
    assert_ne!(fixture.head(), head_at_pause);
    // edit 恢复必须逐字保留带 body 的提交信息（subject + 空行 + body）
    let edit_log = git(fixture.path(), &["log", "-1", "--format=%B", "HEAD~1"]);
    assert_eq!(
        edit_log.stdout_lossy().replace("\r\n", "\n").trim_end(),
        "one\n\none body",
        "edit 恢复后带 body 的提交信息必须逐字不变（曾出现缺失空行的事故）"
    );
    let subjects = git(
        fixture.path(),
        &["log", "--format=%s", &format!("{base}..HEAD")],
    )
    .stdout_lossy()
    .lines()
    .map(|l| l.to_owned())
    .collect::<Vec<_>>();
    assert_eq!(
        subjects,
        vec!["two", "one"],
        "edit 保留原信息（--amend 不带 -m），two 照常重放"
    );

    // 幂等：完成后再调必须拒绝
    let error = service
        .continue_after_edit(fixture.repo_id)
        .expect_err("不在 edit 停点必须拒绝");
    assert_eq!(
        error.code,
        forgedesk_domain::ErrorCode::Validation,
        "{error:?}"
    );
}

// ---------------------------------------------------------------- T3.6 面板地基

#[test]
fn rebase_range_lists_the_whole_range_oldest_first_with_parents() {
    let mut fixture = Fixture::new("rebase-range");
    fixture.linear();
    let base = fixture.base();
    let head = fixture.head();

    let commits = rebase_service(&fixture)
        .range(fixture.repo_id, &base, &head)
        .expect("range 查询失败");

    let subjects: Vec<&str> = commits.iter().map(|c| c.subject.as_str()).collect();
    assert_eq!(subjects, vec!["one", "two", "three"], "从旧到新");
    assert!(!commits.iter().any(|c| c.oid == base), "区间不含 base 本身");
    assert_eq!(
        commits[2].parents,
        vec![commits[1].oid.clone()],
        "父链信息必须随清单返回（面板据此识别 merge 与拓扑）"
    );
}

#[test]
fn preview_warns_only_for_commits_reachable_from_a_remote_tracking_ref() {
    let mut fixture = Fixture::new("rebase-pushed");
    fixture.linear();
    let base = fixture.base();
    let head = fixture.head();
    let oid_at = |rev: &str| {
        git(fixture.path(), &["rev-parse", rev])
            .stdout_lossy()
            .trim()
            .to_owned()
    };
    let one = oid_at("HEAD~2");
    let two = oid_at("HEAD~1");
    let three = oid_at("HEAD");
    // 模拟一次 fetch：远端跟踪 ref 指向区间内的第一个提交（只有它被推送过）
    git_ok(
        fixture.path(),
        &["update-ref", "refs/remotes/origin/main", &one],
    );

    let plan_with = |reworded: &str| RebasePlan {
        base: base.clone(),
        head: head.clone(),
        steps: vec![one.clone(), two.clone(), three.clone()]
            .into_iter()
            .map(|oid| ReorderStep {
                action: if oid == reworded {
                    ReorderAction::Reword
                } else {
                    ReorderAction::Pick
                },
                new_message: (oid == reworded).then(|| "rewritten".to_owned()),
                oid,
            })
            .collect(),
        allow_flatten_merges: false,
        autosquash: false,
    };

    let service = rebase_service(&fixture);
    let preview = service
        .preview_only(fixture.repo_id, &plan_with(&one))
        .expect("预览失败");
    assert!(
        preview.touches_pushed,
        "重写已推送提交必须触发 force-with-lease 警告"
    );
    assert!(
        preview.todo_text.contains("reword") && preview.todo_text.contains("pick"),
        "preview 必须携带等价 todo：{}",
        preview.todo_text
    );

    let preview = service
        .preview_only(fixture.repo_id, &plan_with(&three))
        .expect("预览失败");
    assert!(
        !preview.touches_pushed,
        "重写未推送的提交不应触发警告（判定必须逐个提交，而不是有远端就整体警告）"
    );
}
