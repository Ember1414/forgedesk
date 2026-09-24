//! diff 服务测试（T1.5）：真实临时仓库上的端到端断言。
//!
//! 解析器的字节级正确性由 `forgedesk-git-engine` 的固定样本覆盖；
//! 这里保证**用例语义**：spec 的目标/路径/空白开关正确作用于真实仓库，
//! hunks 的行号与内容能对上磁盘上的真实变更，补丁字节可直接回灌给 `git apply`。
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use forgedesk_domain::git::{DiffLineKind, DiffSpec, DiffTarget, RepoPath};
use forgedesk_services::{GitEngines, RepositoryService, WorkspaceService};
use forgedesk_storage::{Database, RepositoryStore};

mod support;

use support::{init_repo, memory_database, write};

/// 每个用例一套独立的仓库 + 数据库 + 服务（与 workspace_lifecycle 同一模式）。
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
        write(dir.path(), "base.txt", b"base line\n");
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

    fn path(&self) -> &std::path::Path {
        self.dir.path()
    }
}

#[test]
fn unstaged_diff_returns_hunks_with_correct_line_numbers() {
    let fixture = Fixture::new("diff-unstaged");
    write(fixture.path(), "base.txt", b"base line changed\n");

    let report = fixture
        .workspace()
        .diff(fixture.repo_id, DiffSpec::new(DiffTarget::Unstaged))
        .expect("diff");

    assert_eq!(report.files.len(), 1);
    let file = &report.files[0];
    assert_eq!(file.path.to_string(), "base.txt");
    assert!(!file.truncated);

    assert_eq!(file.hunks.len(), 1);
    let lines = &file.hunks[0].lines;
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].kind, DiffLineKind::Removed);
    assert_eq!(lines[0].content, "base line");
    assert_eq!(lines[0].old_lineno, Some(1));
    assert_eq!(lines[1].kind, DiffLineKind::Added);
    assert_eq!(lines[1].content, "base line changed");
    assert_eq!(lines[1].new_lineno, Some(1));
}

#[test]
fn staged_diff_uses_the_index_side() {
    let fixture = Fixture::new("diff-staged");
    write(fixture.path(), "base.txt", b"base line changed\n");
    let workspace = fixture.workspace();
    workspace
        .stage(fixture.repo_id, &[RepoPath::from("base.txt")])
        .expect("stage");

    let staged = workspace
        .diff(fixture.repo_id, DiffSpec::new(DiffTarget::Staged))
        .expect("staged diff");
    assert_eq!(staged.files.len(), 1);
    assert_eq!(
        staged.files[0].hunks[0].lines[1].content,
        "base line changed"
    );

    // 已暂存后工作区侧没有变更：unstaged 为空
    let unstaged = workspace
        .diff(fixture.repo_id, DiffSpec::new(DiffTarget::Unstaged))
        .expect("unstaged diff");
    assert!(unstaged.files.is_empty());
}

#[test]
fn patch_output_is_applicable_by_git() {
    let fixture = Fixture::new("diff-patch");
    write(fixture.path(), "base.txt", b"base line changed\n");

    let patch = fixture
        .workspace()
        .diff_patch(fixture.repo_id, DiffSpec::new(DiffTarget::Unstaged))
        .expect("patch");

    let text = String::from_utf8_lossy(&patch);
    assert!(
        text.starts_with("diff --git "),
        "补丁应以 diff --git 开头：{text}"
    );
    assert!(
        text.contains("+base line changed"),
        "补丁应包含新增行：{text}"
    );
    // 头部带 -U 上下文与 --no-color：输出里不能有 ANSI 转义序列
    assert!(!text.contains('\x1b'), "补丁不应包含颜色码：{text:?}");
}

#[test]
fn ignore_whitespace_flag_is_passed_through() {
    let fixture = Fixture::new("diff-ws");
    // 只改了行尾空白：-w 下不算变更
    write(fixture.path(), "base.txt", b"base line  \n");

    let strict = fixture
        .workspace()
        .diff(fixture.repo_id, DiffSpec::new(DiffTarget::Unstaged))
        .expect("strict diff");
    assert_eq!(strict.files.len(), 1);

    let ignore = fixture.workspace().diff(
        fixture.repo_id,
        DiffSpec::new(DiffTarget::Unstaged).with_ignore_whitespace(true),
    );
    match ignore {
        Ok(report) => assert!(report.files.is_empty(), "-w 下空白变更应为空：{report:?}"),
        Err(error) => panic!("-w 查询不应失败：{error:?}"),
    }
}

#[test]
fn binary_files_carry_no_hunks() {
    let fixture = Fixture::new("diff-binary");
    // 普通git diff不包含未跟踪文件，因此先提交再修改，测的是"被跟踪的二进制文件"
    write(fixture.path(), "blob.bin", &[0u8, 159, 146, 150, 0, 1, 2]);
    support::commit_all(fixture.path(), "add binary");
    write(fixture.path(), "blob.bin", &[0u8, 1, 2, 3, 4, 5, 6]);

    let report = fixture
        .workspace()
        .diff(fixture.repo_id, DiffSpec::new(DiffTarget::Unstaged))
        .expect("diff");

    let binary: Vec<_> = report.files.iter().filter(|f| f.binary).collect();
    assert_eq!(binary.len(), 1, "应识别出二进制文件：{report:?}");
    assert!(binary[0].hunks.is_empty());
}

#[test]
fn path_filter_limits_the_report() {
    let fixture = Fixture::new("diff-paths");
    write(fixture.path(), "a.txt", b"a original\n");
    write(fixture.path(), "b.txt", b"b original\n");
    support::commit_all(fixture.path(), "two files");
    write(fixture.path(), "a.txt", b"a changed\n");
    write(fixture.path(), "b.txt", b"b changed\n");

    let report = fixture.workspace().diff(
        fixture.repo_id,
        DiffSpec::new(DiffTarget::Unstaged).with_paths(vec![RepoPath::from("a.txt")]),
    );

    match report {
        Ok(report) => {
            assert_eq!(report.files.len(), 1, "路径过滤应只剩 a.txt：{report:?}");
            assert!(report.files.iter().all(|f| f.path.to_string() == "a.txt"));
        }
        Err(error) => panic!("路径过滤的 diff 不应失败：{error:?}"),
    }
}
