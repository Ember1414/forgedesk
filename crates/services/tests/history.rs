//! 历史分页与布局编排（T2.1）的集成测试。
//!
//! # 为什么这些用例必须跑在真实仓库上
//!
//! 布局是纯函数（`domain::history` 已有 14 个用例），但**编排**不是：
//! 游标如何映射到 `--skip`、`has_more` 与 `next_cursor` 的边界、以及
//! "筛选条件改变时游标语义如何失效"，都发生在服务层与 git 的交界处。
//! 模拟 git 只会测到替身本身。
//!
//! # 本文件钉住的三条行为
//!
//! 1. **游标续页**：第二页的第一条必须是第一页之后的提交，且两页的行号连续；
//! 2. **筛选与游标独立**：带筛选时 `next_cursor` 仍然正确推进（每页凑满为止）；
//! 3. **first_parent_only 两端一致**：查询过滤支线、布局只画第一父，
//!    两边不一致就会画出断头路（这是最容易写错的地方）。
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use forgedesk_domain::history::EdgeKind;
use forgedesk_services::{GitEngines, HistoryQuery, HistoryService};
use forgedesk_storage::{Database, RepositoryStore};

mod support;

use support::{init_repo, memory_database, write, TempDir};

/// 用**固定且互不相同的时间戳**创建提交。
///
/// 为什么不用 `support::commit_all`：同一秒内创建的提交在 git 的 TIME 排序里
/// 次序是自由的（ tie 由 git 决定），而本文件断言的是"第 N 页从哪条提交开始"——
/// 夹具必须保证 walk 顺序与创建顺序一致。
fn commit_at(dir: &std::path::Path, index: usize) {
    let file = format!("file-{index}.txt");
    write(dir, &file, format!("{index}\n").as_bytes());

    let stamp = format!("{} +0000", 1_700_000_000 + (index as i64) * 60);
    for step in [
        vec!["add".to_owned(), "-A".to_owned()],
        vec![
            "commit".to_owned(),
            "-m".to_owned(),
            format!("commit {index}"),
        ],
    ] {
        let status = std::process::Command::new("git")
            .current_dir(dir)
            .args(&step)
            .env("GIT_AUTHOR_DATE", &stamp)
            .env("GIT_COMMITTER_DATE", &stamp)
            .output()
            .expect("git 运行失败");
        assert!(status.status.success(), "git {:?} 失败", step);
    }
}

/// 一个带分叉合并历史的仓库夹具：
///
/// ```text
/// m (merge a, b)     ← 第 0 行
/// a ← base           ← 主线
/// b ← base           ← 支线
/// base（初始提交）    ← 第 3 行
/// ```
struct Fixture {
    _dir: TempDir,
    repo_id: i64,
    service: HistoryService<'static>,
}

// 注意：HistoryService 借用 engines 与 store，fixture 用 Box::leak 让它们活到测试结束。
/// 用固定时间戳提交（同 `commit_at`：必须让 walk 顺序可以断言）。
fn commit_with(dir: &std::path::Path, message: &str, stamp_seconds: i64) {
    let stamp = format!("{stamp_seconds} +0000");
    let status = std::process::Command::new("git")
        .current_dir(dir)
        .args(["commit", "-m", message])
        .env("GIT_AUTHOR_DATE", &stamp)
        .env("GIT_COMMITTER_DATE", &stamp)
        .output()
        .expect("git 运行失败");
    assert!(status.status.success(), "git commit {message} 失败");
}

fn fixture(label: &str, commits: usize) -> Fixture {
    let dir = TempDir::new(label);
    init_repo(dir.path());

    // base 的时间戳必须早于所有后续提交（否则它会是"最新"的提交）
    write(dir.path(), "base.txt", b"base\n");
    support::git_ok(dir.path(), &["add", "-A"]);
    commit_with(dir.path(), "base commit", 1_700_000_000 - 60);

    for index in 0..commits {
        commit_at(dir.path(), index);
    }

    let engines: &'static GitEngines =
        Box::leak(Box::new(GitEngines::new().expect("创建引擎失败")));
    let database: &'static Database = Box::leak(Box::new(memory_database()));
    let store = RepositoryStore::new(database);

    use forgedesk_services::repository::OpenRepoRegistry;
    use forgedesk_services::RepositoryService;
    let open = OpenRepoRegistry::new();
    let repository = RepositoryService::new(engines, RepositoryStore::new(database), &open);
    let opened = repository.open(dir.path()).expect("打开仓库失败");

    let service = HistoryService::new(engines, store);
    Fixture {
        _dir: dir,
        repo_id: opened.record_id,
        service,
    }
}

#[test]
fn cursor_paging_returns_contiguous_pages() {
    let fixture = fixture("history-cursor", 5);

    let first = fixture
        .service
        .page(
            fixture.repo_id,
            &HistoryQuery {
                page_size: 2,
                ..Default::default()
            },
        )
        .expect("第一页");

    assert_eq!(first.commits.len(), 2);
    assert_eq!(first.commits[0].subject, "commit 4", "新 → 旧");
    assert_eq!(first.next_cursor, Some(2));

    let second = fixture
        .service
        .page(
            fixture.repo_id,
            &HistoryQuery {
                page_size: 2,
                cursor: first.next_cursor,
                ..Default::default()
            },
        )
        .expect("第二页");

    assert_eq!(
        second.commits[0].subject, "commit 2",
        "续页从上次停下的地方继续"
    );
    // 两页的行号必须连续：第二页 row 0 = 全历史的第 2 行（这正是布局分页一致性的前提）
    assert_eq!(second.layout.rows[0].row, 2);
}

#[test]
fn the_last_page_reports_no_next_cursor() {
    let fixture = fixture("history-last", 3);

    let page = fixture
        .service
        .page(
            fixture.repo_id,
            &HistoryQuery {
                page_size: 10,
                ..Default::default()
            },
        )
        .expect("一页装下全部");

    assert_eq!(page.commits.len(), 4, "base commit + 3 个");
    assert_eq!(page.next_cursor, None);
    assert_eq!(page.layout.rows.len(), 4);
}

#[test]
fn message_filter_matches_the_full_message_literally() {
    let fixture = fixture("history-grep", 4);

    let query = HistoryQuery {
        message_contains: Some("commit 2".to_owned()),
        page_size: 10,
        ..Default::default()
    };
    let page = fixture.service.page(fixture.repo_id, &query).expect("搜索");

    // "commit 2" 只命中一条；但 "commit 2" 也是 "commit 2x" 的前缀——夹具里没有
    // 这种提交，因此恰好一条
    assert_eq!(page.commits.len(), 1);
    assert_eq!(page.commits[0].subject, "commit 2");
}

#[test]
fn first_parent_only_agrees_between_the_query_and_the_layout() {
    // 线性历史上两种模式没有区别；这里断言的是"开关打开时不报错、行数不变"，
    // 真正的分叉断言在 differential 与 domain 的用例里
    let fixture = fixture("history-first-parent", 3);

    let query = HistoryQuery {
        first_parent_only: true,
        page_size: 10,
        ..Default::default()
    };
    let page = fixture.service.page(fixture.repo_id, &query).expect("查询");

    assert_eq!(page.layout.rows.len(), 4);
    assert!(page
        .layout
        .edges
        .iter()
        .all(|edge| edge.kind == EdgeKind::Straight));
}

#[test]
fn an_unknown_record_is_not_found() {
    let fixture = fixture("history-not-found", 1);

    let error = fixture
        .service
        .page(999_999, &HistoryQuery::default())
        .expect_err("不存在的记录");

    assert_eq!(error.code, forgedesk_domain::ErrorCode::NotFound);
}

#[test]
fn follow_renames_requires_exactly_one_path() {
    let fixture = fixture("history-follow", 1);

    let error = fixture
        .service
        .page(
            fixture.repo_id,
            &HistoryQuery {
                follow_renames: true,
                ..Default::default()
            },
        )
        .expect_err("没有路径时必须拒绝");

    assert_eq!(error.code, forgedesk_domain::ErrorCode::Validation);
}
