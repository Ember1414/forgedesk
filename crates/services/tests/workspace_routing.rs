//! 状态引擎分流（方案 B）的集成测试。
//!
//! # 被钉住的行为
//!
//! [`WorkspaceService::status`] 的读路径按**索引条目数**分流：低于阈值走
//! libgit2（无进程开销），达到阈值走 CLI（大仓库上快 10 倍）。两条路径的
//! 结果必须**完全一致**——这正是分流的正确性契约：切换引擎对界面只能是
//! 快慢差异，不能是字段或内容差异。
//!
//! # 为什么用真实大索引夹具
//!
//! 阈值判定发生在服务层内部（先探测再选引擎），mock 引擎只会测到替身。
//! 夹具 = 1 个基线提交 + 阈值以上的文件被 `git add` 进索引（含修改未提交），
//! 让 `index_entry_count` 真实越过 [`STATUS_CLI_ENTRY_THRESHOLD`]。
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use forgedesk_domain::git::StatusQuery;
use forgedesk_git_engine::engine::GitEngine;
use forgedesk_services::workspace::{should_route_status_to_cli, STATUS_CLI_ENTRY_THRESHOLD};
use forgedesk_services::{GitEngines, WorkspaceService};
use forgedesk_storage::{Database, RepositoryStore};

mod support;

use support::{init_repo, TempDir};

struct Fixture {
    dir: TempDir,
    repo_id: i64,
    service: WorkspaceService<'static>,
    engines: &'static GitEngines,
}

fn build_service(label: &str) -> Fixture {
    let dir = TempDir::new(label);
    init_repo(dir.path());
    let engines: &'static GitEngines =
        Box::leak(Box::new(GitEngines::new().expect("创建引擎失败")));
    let database: &'static Database = Box::leak(Box::new({
        let database = Database::open_in_memory().expect("打开内存库失败");
        forgedesk_storage::migrate(&database).expect("迁移失败");
        database
    }));

    use forgedesk_services::repository::OpenRepoRegistry;
    let opened = {
        let open = OpenRepoRegistry::new();
        let repository = forgedesk_services::RepositoryService::new(
            engines,
            RepositoryStore::new(database),
            &open,
        );
        repository.open(dir.path()).expect("打开仓库失败")
    };

    // WorkspaceService 持 &'static 引用（engines/database 都是 Box::leak 出来的），
    // 注册表也得活到 'static：同样泄漏。服务内部目前未消费它（M2 预留），
    // 但生命周期形状必须成立。
    let open: &'static OpenRepoRegistry = Box::leak(Box::new(OpenRepoRegistry::new()));
    let service = WorkspaceService::new(engines, RepositoryStore::new(database), open);
    Fixture {
        dir,
        repo_id: opened.record_id,
        service,
        engines,
    }
}

/// 写 N 个文件并 `git add` 全部（不改 HEAD：索引条目数 = N，与基线提交无关）。
fn stage_n_files(dir: &std::path::Path, count: usize) {
    for index in 0..count {
        let path = dir.join(format!("routing/file-{index:05}.txt"));
        std::fs::create_dir_all(path.parent().expect("父目录")).expect("建目录失败");
        std::fs::write(path, format!("{index}\n")).expect("写文件失败");
    }
    let output = std::process::Command::new("git")
        .current_dir(dir)
        .args(["add", "-A"])
        .output()
        .expect("git 运行失败");
    assert!(output.status.success(), "git add 失败");
}

#[test]
fn a_large_index_is_routed_to_the_cli_and_stays_consistent_with_it() {
    let fixture = build_service("status-routing");
    let repo = forgedesk_domain::git::RepoId::new(fixture.dir.path().to_path_buf());

    // 索引条目数越过阈值：决策函数必须给出"走 CLI"
    stage_n_files(
        fixture.dir.path(),
        STATUS_CLI_ENTRY_THRESHOLD as usize + 100,
    );
    let count = fixture
        .engines
        .read()
        .index_entry_count(&repo)
        .expect("数索引失败");
    assert!(
        should_route_status_to_cli(count),
        "条目数 {count} 应达到阈值"
    );

    // 服务路径（内部已按条目数切到 CLI）与 CLI 直连全等
    let via_service = fixture
        .service
        .status(fixture.repo_id, false)
        .expect("服务 status 失败");
    let via_cli = fixture
        .engines
        .write()
        .status(&repo, &StatusQuery::default())
        .expect("CLI 直连失败");
    assert_eq!(via_service.entries.len(), via_cli.entries.len());
    for (service_entry, cli_entry) in via_service.entries.iter().zip(&via_cli.entries) {
        assert_eq!(service_entry.path, cli_entry.path);
        assert_eq!(service_entry.kind, cli_entry.kind);
        assert_eq!(service_entry.size_bytes, cli_entry.size_bytes);
        assert_eq!(service_entry.index_status, cli_entry.index_status);
        assert_eq!(service_entry.worktree_status, cli_entry.worktree_status);
    }
}

#[test]
fn a_small_index_stays_on_the_read_engine() {
    let fixture = build_service("status-routing-small");
    let repo = forgedesk_domain::git::RepoId::new(fixture.dir.path().to_path_buf());

    stage_n_files(fixture.dir.path(), 5);
    let count = fixture
        .engines
        .read()
        .index_entry_count(&repo)
        .expect("数索引失败");
    assert!(
        !should_route_status_to_cli(count),
        "条目数 {count} 不应达到阈值"
    );

    // 小仓库照常工作（走 libgit2）：结果与 CLI 直连同样一致——分流的存在
    // 不改变"任何一条路径都能替代另一条"的既有契约
    let via_service = fixture
        .service
        .status(fixture.repo_id, false)
        .expect("服务 status 失败");
    let via_cli = fixture
        .engines
        .write()
        .status(&repo, &StatusQuery::default())
        .expect("CLI 直连失败");
    assert_eq!(via_service.entries.len(), via_cli.entries.len());
}
