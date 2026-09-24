//! 工作区服务测试（T1.4）：状态分组、暂存往返、放弃的安全语义。
//!
//! 这些是"应用第一次真正操作用户仓库"的测试：
//! 引擎层的解析测试保证字节级正确，这里的测试保证**用例语义**正确
//! （分组、计数、暂存往返、放弃后磁盘的真实状态）。

use std::path::Path;

use forgedesk_domain::git::{DiscardSpec, EntryKind, RepoPath};
use forgedesk_services::{GitEngines, RepositoryService, WorkspaceService};
use forgedesk_storage::{Database, RepositoryStore};

mod support;

use support::{init_repo, memory_database, write};

/// 每个用例一套独立的仓库 + 数据库 + 服务。
struct Fixture {
    engines: GitEngines,
    database: Database,
    open: forgedesk_services::OpenRepoRegistry,
    dir: support::TempDir,
    repo_id: i64,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let dir = support::TempDir::new(label);
        init_repo(dir.path());
        write(dir.path(), "base.txt", b"base\n");
        support::commit_all(dir.path(), "base");

        let engines = GitEngines::new().expect("engines");
        let database = memory_database();
        let open = forgedesk_services::OpenRepoRegistry::default();
        let repository = RepositoryService::new(&engines, RepositoryStore::new(&database), &open);
        let opened = repository.open(dir.path()).expect("open");

        Self {
            engines,
            database,
            open,
            dir,
            repo_id: opened.record_id,
        }
    }

    fn workspace(&self) -> WorkspaceService<'_> {
        WorkspaceService::new(
            &self.engines,
            RepositoryStore::new(&self.database),
            &self.open,
        )
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }
}

#[test]
fn status_reports_groups_and_respects_include_ignored() {
    let fixture = Fixture::new("ws-status");
    write(fixture.path(), "tracked.txt", b"changed\n");
    write(fixture.path(), "new.txt", b"new\n");
    write(fixture.path(), ".gitignore", b"secret*\n");
    write(fixture.path(), "secret.key", b"ignored\n");

    let workspace = fixture.workspace();

    // 默认：不统计也不返回被忽略文件
    let default_status = workspace.status(fixture.repo_id, false).expect("status");
    assert_eq!(default_status.ignored_count, None);
    assert!(default_status
        .entries
        .iter()
        .all(|entry| entry.kind != EntryKind::Ignored));

    // 显式请求：被忽略文件返回并计数
    let with_ignored = workspace.status(fixture.repo_id, true).expect("status");
    assert_eq!(with_ignored.ignored_count, Some(1));
    let ignored_paths: Vec<Vec<u8>> = with_ignored
        .entries
        .iter()
        .filter(|entry| entry.kind == EntryKind::Ignored)
        .map(|entry| entry.path.as_bytes().to_vec())
        .collect();
    assert!(ignored_paths.contains(&b"secret.key".to_vec()));

    // 富化字段：未跟踪的工作区文件有大小、非二进制
    let untracked = with_ignored
        .entries
        .iter()
        .find(|entry| entry.path.as_bytes() == b"new.txt")
        .expect("untracked entry");
    assert_eq!(untracked.size_bytes, Some(4));
    assert!(!untracked.is_binary);
}

#[test]
fn unknown_repo_id_is_not_found() {
    let fixture = Fixture::new("ws-missing");

    let error = fixture
        .workspace()
        .status(999_999, false)
        .expect_err("unknown id must fail");
    assert_eq!(error.code.as_str(), "NOT_FOUND");
}

#[test]
fn stage_and_unstage_round_trip_the_index() {
    let fixture = Fixture::new("ws-stage");
    write(fixture.path(), "new.txt", b"new\n");

    let workspace = fixture.workspace();
    let paths = [RepoPath::from("new.txt")];

    workspace.stage(fixture.repo_id, &paths).expect("stage");
    let staged = workspace.status(fixture.repo_id, false).expect("status");
    let entry = staged
        .entries
        .iter()
        .find(|entry| entry.path.as_bytes() == b"new.txt")
        .expect("entry");
    assert_eq!(entry.index_status.as_char(), 'A');

    workspace.unstage(fixture.repo_id, &paths).expect("unstage");
    let unstaged = workspace.status(fixture.repo_id, false).expect("status");
    let entry = unstaged
        .entries
        .iter()
        .find(|entry| entry.path.as_bytes() == b"new.txt")
        .expect("entry");
    assert_eq!(entry.index_status.as_char(), '.');
}

#[test]
fn discard_restores_tracked_and_deletes_untracked() {
    let fixture = Fixture::new("ws-discard");
    // 先把 tracked.txt 提交进 HEAD，再制造它的未暂存修改
    write(fixture.path(), "tracked.txt", b"base\n");
    support::commit_all(fixture.path(), "add tracked.txt");
    // 已跟踪的修改 + 嵌套目录里的未跟踪文件
    write(fixture.path(), "tracked.txt", b"changed\n");
    let nested = fixture.path().join("nested").join("deep");
    std::fs::create_dir_all(&nested).expect("mkdir");
    std::fs::write(nested.join("untracked.txt"), b"gone\n").expect("write");

    let workspace = fixture.workspace();
    workspace
        .discard(
            fixture.repo_id,
            DiscardSpec {
                tracked: vec![RepoPath::from("tracked.txt")],
                untracked: vec![RepoPath::from("nested/deep/untracked.txt")],
            },
        )
        .expect("discard");

    // 已跟踪：内容回到 HEAD 版本
    assert_eq!(
        std::fs::read(fixture.path().join("tracked.txt")).expect("read"),
        b"base\n"
    );
    // 未跟踪：文件与变空的父目录一起消失
    assert!(!fixture.path().join("nested/deep/untracked.txt").exists());
    assert!(!fixture.path().join("nested").exists());
}

#[test]
fn discard_rejects_empty_requests() {
    let fixture = Fixture::new("ws-discard-empty");

    let error = fixture
        .workspace()
        .discard(
            fixture.repo_id,
            DiscardSpec {
                tracked: vec![],
                untracked: vec![],
            },
        )
        .expect_err("empty discard must fail");
    assert_eq!(error.code.as_str(), "VALIDATION");
}
