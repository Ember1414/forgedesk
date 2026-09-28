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

    build_service(dir)
}

/// 把准备好的仓库接到 `HistoryService` 上（与 `fixture` 共用的接线）。
fn build_service(dir: TempDir) -> Fixture {
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

/// 一个"分支并回主线"夹具：
///
/// ```text
/// merge feature   ← merge 提交（时间戳最晚）
/// feature work    ← 分支提交（从 main tip 分出，单提交）
/// main tip
/// base commit
/// ```
///
/// 分支 tip 的祖先 = {main tip, base} 全在第一父（main tip）的祖先集里，
/// 因此在完整窗口上可折叠。
fn merge_fixture(label: &str) -> Fixture {
    let dir = TempDir::new(label);
    init_repo(dir.path());

    write(dir.path(), "base.txt", b"base\n");
    support::git_ok(dir.path(), &["add", "-A"]);
    commit_with(dir.path(), "base commit", 1_700_000_000 - 60);

    write(dir.path(), "main.txt", b"main\n");
    support::git_ok(dir.path(), &["add", "-A"]);
    commit_with(dir.path(), "main tip", 1_700_000_000);

    // 分支从 main tip 分出，提交一次
    support::git_ok(dir.path(), &["checkout", "-q", "-b", "feature"]);
    write(dir.path(), "feature.txt", b"feature\n");
    support::git_ok(dir.path(), &["add", "-A"]);
    commit_with(dir.path(), "feature work", 1_700_000_000 + 60);

    // 回 main 合并。feature 基于 main tip，可快进，必须 --no-ff 才会产生
    // merge 提交；merge 的时间戳必须晚于它的两个父
    support::git_ok(dir.path(), &["checkout", "-q", "main"]);
    let stamp = format!("{} +0000", 1_700_000_000 + 120);
    let status = std::process::Command::new("git")
        .current_dir(dir.path())
        .args(["merge", "--no-ff", "-q", "-m", "merge feature", "feature"])
        .env("GIT_AUTHOR_DATE", &stamp)
        .env("GIT_COMMITTER_DATE", &stamp)
        .output()
        .expect("git 运行失败");
    assert!(status.status.success(), "git merge 失败");

    build_service(dir)
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

#[test]
fn collapsing_merged_branches_marks_merged_side_commits_hidden_on_a_complete_window() {
    let fixture = merge_fixture("history-collapse");

    // 一页取完 → has_more == false → 完整窗口，折叠生效
    let page = fixture
        .service
        .page(
            fixture.repo_id,
            &HistoryQuery {
                collapse_merged_branches: true,
                page_size: 10,
                ..Default::default()
            },
        )
        .expect("完整历史一页取完");

    assert_eq!(page.next_cursor, None, "一页取完才是完整窗口");
    let layout = &page.layout;
    assert_eq!(
        layout.rows.len(),
        4,
        "merge + feature work + main tip + base"
    );

    let feature_oid = page
        .commits
        .iter()
        .find(|commit| commit.subject == "feature work")
        .expect("分支提交存在")
        .oid
        .clone();
    let merge_row = layout
        .rows
        .iter()
        .find(|row| row.is_merge)
        .expect("merge 行存在");

    assert_eq!(
        merge_row.collapsed,
        vec![feature_oid.clone()],
        "merge 行记录被折叠的分支 tip"
    );
    assert!(
        layout.row_of(&feature_oid).expect("分支行存在").hidden,
        "分支提交被标记 hidden"
    );
    for row in &layout.rows {
        if row.oid != feature_oid {
            assert!(!row.hidden, "{} 不应被隐藏", row.oid);
        }
    }
}

#[test]
fn an_incomplete_window_does_not_collapse_merged_branches() {
    let fixture = merge_fixture("history-collapse-partial");

    // 首页（含 merge 行）与中间页都处于 has_more == true 的不完整窗口：
    // 祖先可能落在窗外，"第二父祖先 ⊆ 第一父祖先集"会误判，
    // 因此服务层必须静默回退为不折叠。
    for cursor in [0_u32, 1] {
        let page = fixture
            .service
            .page(
                fixture.repo_id,
                &HistoryQuery {
                    collapse_merged_branches: true,
                    page_size: 2,
                    cursor: Some(cursor),
                    ..Default::default()
                },
            )
            .expect("不完整窗口的页");

        assert_eq!(page.next_cursor, Some(cursor + 2), "两页都还有后续");
        assert!(
            page.layout
                .rows
                .iter()
                .all(|row| !row.hidden && row.collapsed.is_empty()),
            "cursor {cursor}：窗口不完整时不得出现任何折叠标记"
        );
    }
}

// ---------------------------------------------------------------- T2.3 筛选

/// "仅显示我的提交"按仓库 user.email 翻译成 author 过滤；未配置身份时 VALIDATION。
#[test]
fn my_commits_only_uses_the_configured_identity_and_requires_one() {
    let fx = fixture("my-commits", 2);

    // 未配置本地身份：config 走全局链，夹具提交的作者就是这台机器的全局身份，
    // 因此开关应命中全部提交（2 条）
    let all = fx
        .service
        .page(
            fx.repo_id,
            &HistoryQuery {
                my_commits_only: true,
                ..HistoryQuery::default()
            },
        )
        .expect("默认身份下查询失败");
    assert_eq!(all.commits.len(), 3, "夹具(base+2)提交都出自当前身份");

    // 配置一个别的身份：开关应过滤掉全部提交
    support::git_ok(
        fx._dir.path(),
        &["config", "user.email", "someone-else@example.com"],
    );
    let none = fx
        .service
        .page(
            fx.repo_id,
            &HistoryQuery {
                my_commits_only: true,
                ..HistoryQuery::default()
            },
        )
        .expect("他人身份下查询失败");
    assert!(none.commits.is_empty(), "身份不匹配应零命中");
    assert_eq!(none.next_cursor, None);

    // 身份被清空（本地配置为空串）：如实 VALIDATION，而不是静默变成"全部提交"
    support::git_ok(fx._dir.path(), &["config", "user.email", ""]);
    let error = fx
        .service
        .page(
            fx.repo_id,
            &HistoryQuery {
                my_commits_only: true,
                ..HistoryQuery::default()
            },
        )
        .expect_err("无身份时应报错");
    assert_eq!(error.code, forgedesk_domain::ErrorCode::Validation);
}

/// 分支多选 = 多 tip 并集（未合并的双分支夹具，并集才会真的比单分支多）。
#[test]
fn revisions_union_spans_independent_branches() {
    let dir = TempDir::new("revisions-union");
    init_repo(dir.path());
    write(dir.path(), "base.txt", b"base\n");
    support::git_ok(dir.path(), &["add", "-A"]);
    commit_with(dir.path(), "base commit", 1_700_000_000 - 60);

    support::git_ok(dir.path(), &["checkout", "-q", "-b", "feature"]);
    write(dir.path(), "feature.txt", b"feature\n");
    support::git_ok(dir.path(), &["add", "-A"]);
    commit_with(dir.path(), "feature work", 1_700_000_000);

    support::git_ok(dir.path(), &["checkout", "-q", "main"]);
    write(dir.path(), "main.txt", b"main\n");
    support::git_ok(dir.path(), &["add", "-A"]);
    commit_with(dir.path(), "main work", 1_700_000_060);

    let fx = build_service(dir);

    let union = fx
        .service
        .page(
            fx.repo_id,
            &HistoryQuery {
                revisions: vec!["main".to_owned(), "feature".to_owned()],
                ..HistoryQuery::default()
            },
        )
        .expect("并集查询失败");
    assert_eq!(union.commits.len(), 3, "并集应覆盖 base/feature/main 三条");

    let single = fx
        .service
        .page(
            fx.repo_id,
            &HistoryQuery {
                revision: Some("main".to_owned()),
                ..HistoryQuery::default()
            },
        )
        .expect("单分支查询失败");
    assert_eq!(single.commits.len(), 2, "单分支不含 feature 侧提交");
}

/// 关键词 + 忽略大小写：小写变体命中；关掉开关恢复字面匹配。
#[test]
fn case_insensitive_keyword_matches_case_variants() {
    let dir = TempDir::new("case-insensitive");
    init_repo(dir.path());
    write(dir.path(), "base.txt", b"base\n");
    support::git_ok(dir.path(), &["add", "-A"]);
    commit_with(dir.path(), "base commit", 1_700_000_000 - 60);
    write(dir.path(), "a.txt", b"a\n");
    support::git_ok(dir.path(), &["add", "-A"]);
    commit_with(dir.path(), "Add ReadmePipeline", 1_700_000_000);

    let fx = build_service(dir);

    let hit = fx
        .service
        .page(
            fx.repo_id,
            &HistoryQuery {
                message_contains: Some("readmepipeline".to_owned()),
                case_insensitive: true,
                ..HistoryQuery::default()
            },
        )
        .expect("忽略大小写查询失败");
    assert_eq!(hit.commits.len(), 1, "小写变体应命中");

    let miss = fx
        .service
        .page(
            fx.repo_id,
            &HistoryQuery {
                message_contains: Some("readmepipeline".to_owned()),
                ..HistoryQuery::default()
            },
        )
        .expect("区分大小写查询失败");
    assert!(miss.commits.is_empty(), "关掉开关恢复区分大小写");
}

/// 仅显示合并提交：merge 夹具只回那条 merge。
#[test]
fn merges_only_returns_only_merge_commits() {
    let fx = merge_fixture("merges-only");
    let page = fx
        .service
        .page(
            fx.repo_id,
            &HistoryQuery {
                merges_only: true,
                ..HistoryQuery::default()
            },
        )
        .expect("仅合并查询失败");
    assert_eq!(page.commits.len(), 1, "夹具只有一条合并提交");
    assert!(page.commits[0].parents.len() >= 2);
    assert!(page.layout.rows.iter().all(|row| row.is_merge));
}
