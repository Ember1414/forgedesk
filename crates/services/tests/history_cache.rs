//! 日志分页缓存（T2.9）的集成测试。
//!
//! # 被钉住的行为
//!
//! 缓存路径与直连路径是**同一契约的两种实现**——差分断言（同参数、结果全等）
//! 是这里的主菜：页内容、`next_cursor`、以及"越过头/越过上限"的边界都必须
//! 一致。此外还有两条只有缓存才有的语义：
//!
//! 1. **tips 变化必须失效**：缓存之后新提交到达，下一页不能再按旧前缀切
//!    （否则新提交会凭空消失，或旧行被重复服务）；
//! 2. **越过单条目上限必须绕过**：绕过路径走引擎直连，结果是旧行为——
//!    慢但正确，绝不能因为"缓存满了"就给出空页或错页。
//!
//! # 为什么夹具的提交时间要固定
//!
//! 服务层按 git 的提交时间排序取页，"第 N 页从哪条提交开始"只有在时间戳
//! 严格递增时可断言（与 `tests/history.rs` 的 `commit_at` 同一约定）。
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

use forgedesk_services::{
    GitEngines, HistoryQuery, HistoryService, LogPageCache, RepositoryService,
};
use forgedesk_storage::{Database, RepositoryStore};

mod support;

use support::{init_repo, memory_database, write, TempDir};

/// 用固定且互不相同的时间戳创建第 `index` 号提交。
fn commit_at(dir: &Path, index: usize) {
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

/// 建一个 N 次提交的线性仓库。
fn linear_repo(dir: &Path, commits: usize) {
    for index in 0..commits {
        commit_at(dir, index);
    }
}

struct Fixture {
    _dir: TempDir,
    repo_id: i64,
    /// 接了缓存的服务（被测路径）。
    cached: HistoryService<'static>,
    /// 直连引擎的服务（参照路径）。
    direct: HistoryService<'static>,
    /// 缓存本体（个别用例要看条目数）。
    cache: &'static LogPageCache,
    dir: std::path::PathBuf,
}

fn build_fixture(entry_cap: Option<usize>) -> Fixture {
    let dir = TempDir::new("history-cache");
    let dir_path = dir.path().to_path_buf();
    init_repo(&dir_path);
    let engines: &'static GitEngines =
        Box::leak(Box::new(GitEngines::new().expect("创建引擎失败")));
    let database: &'static Database = Box::leak(Box::new(memory_database()));

    use forgedesk_services::repository::OpenRepoRegistry;
    let open = OpenRepoRegistry::new();
    let repository = RepositoryService::new(engines, RepositoryStore::new(database), &open);
    let opened = repository.open(&dir_path).expect("打开仓库失败");

    let cache: &'static LogPageCache = Box::leak(Box::new(match entry_cap {
        Some(cap) => LogPageCache::new().with_entry_cap(cap),
        None => LogPageCache::new(),
    }));
    let cached = HistoryService::new(engines, RepositoryStore::new(database)).with_log_cache(cache);
    let direct = HistoryService::new(engines, RepositoryStore::new(database));

    Fixture {
        _dir: dir,
        repo_id: opened.record_id,
        cached,
        direct,
        cache,
        dir: dir_path,
    }
}

fn page_query(page_size: usize, cursor: Option<u32>) -> HistoryQuery {
    HistoryQuery {
        page_size,
        cursor,
        ..Default::default()
    }
}

/// 断言两个服务在同一查询下结果全等（oid 序列 + 游标 + 行号）。
fn assert_pages_equal(
    cached: &forgedesk_services::HistoryPage,
    direct: &forgedesk_services::HistoryPage,
) {
    let cached_oids: Vec<&str> = cached
        .commits
        .iter()
        .map(|commit| commit.oid.as_str())
        .collect();
    let direct_oids: Vec<&str> = direct
        .commits
        .iter()
        .map(|commit| commit.oid.as_str())
        .collect();
    assert_eq!(cached_oids, direct_oids, "页内容与直连不一致");
    assert_eq!(cached.next_cursor, direct.next_cursor, "游标与直连不一致");
    let cached_rows: Vec<u32> = cached.layout.rows.iter().map(|row| row.row).collect();
    let direct_rows: Vec<u32> = direct.layout.rows.iter().map(|row| row.row).collect();
    assert_eq!(cached_rows, direct_rows, "布局行号与直连不一致");
}

#[test]
fn paged_results_from_the_cache_are_identical_to_the_direct_walk() {
    let fixture = build_fixture(None);
    linear_repo(&fixture.dir, 35);

    for cursor in [None, Some(0), Some(10), Some(20), Some(30)] {
        let cached = fixture
            .cached
            .page(fixture.repo_id, &page_query(10, cursor))
            .expect("缓存路径取页失败");
        let direct = fixture
            .direct
            .page(fixture.repo_id, &page_query(10, cursor))
            .expect("直连路径取页失败");
        assert_pages_equal(&cached, &direct);
    }
    // 每页都装满了 10 条，直到末页
    let last = fixture
        .cached
        .page(fixture.repo_id, &page_query(10, Some(30)))
        .expect("末页取页失败");
    assert_eq!(last.commits.len(), 5, "末页应只有剩余的 5 条");
    assert_eq!(last.next_cursor, None, "末页不应再有游标");
}

#[test]
fn a_tip_change_invalidates_the_cached_prefix() {
    let fixture = build_fixture(None);
    linear_repo(&fixture.dir, 20);

    let first = fixture
        .cached
        .page(fixture.repo_id, &page_query(10, None))
        .expect("首页取页失败");
    assert_eq!(first.commits.len(), 10);

    // 新提交到达：tips 指纹变化，同一查询必须落到新前缀上
    commit_at(&fixture.dir, 100);
    let after = fixture
        .cached
        .page(fixture.repo_id, &page_query(10, None))
        .expect("失效后取页失败");
    assert_ne!(
        after.commits[0].oid, first.commits[0].oid,
        "新提交到达后首页第一条必须变化"
    );
    // 与直连路径仍然全等（差分是失效正确性的最终判据）
    let direct = fixture
        .direct
        .page(fixture.repo_id, &page_query(10, None))
        .expect("直连取页失败");
    assert_pages_equal(&after, &direct);

    // 深页也要一致：旧前缀若被错误复用，第二页会重复出现旧行
    let cached_second = fixture
        .cached
        .page(fixture.repo_id, &page_query(10, Some(10)))
        .expect("缓存路径第二页失败");
    let direct_second = fixture
        .direct
        .page(fixture.repo_id, &page_query(10, Some(10)))
        .expect("直连第二页失败");
    assert_pages_equal(&cached_second, &direct_second);
}

#[test]
fn a_deep_jump_fills_the_gap_and_revisiting_earlier_pages_stays_consistent() {
    let fixture = build_fixture(None);
    linear_repo(&fixture.dir, 35);

    // 首页 → 直接跳到第三页（跳过第二页）：缓存必须把 10..30 的空档填上
    let first = fixture
        .cached
        .page(fixture.repo_id, &page_query(10, None))
        .expect("首页取页失败");
    assert_eq!(first.next_cursor, Some(10));
    let third = fixture
        .cached
        .page(fixture.repo_id, &page_query(10, Some(20)))
        .expect("跳页取页失败");
    let direct_third = fixture
        .direct
        .page(fixture.repo_id, &page_query(10, Some(20)))
        .expect("直连跳页失败");
    assert_pages_equal(&third, &direct_third);

    // 回头看第二页：必须与直连一致（前缀已在缓存里，这页是纯切片）
    let second = fixture
        .cached
        .page(fixture.repo_id, &page_query(10, Some(10)))
        .expect("回看第二页失败");
    let direct_second = fixture
        .direct
        .page(fixture.repo_id, &page_query(10, Some(10)))
        .expect("直连第二页失败");
    assert_pages_equal(&second, &direct_second);
}

#[test]
fn requests_beyond_the_entry_cap_bypass_and_stay_correct() {
    // 单条目上限 5、页大小 2：第 3 页起 need_end 越过上限，
    // 缓存只能服务到前缀末尾，其余必须直连且结果不差分毫
    let fixture = build_fixture(Some(5));
    linear_repo(&fixture.dir, 12);

    let direct_page = |cursor: Option<u32>| {
        fixture
            .direct
            .page(fixture.repo_id, &page_query(2, cursor))
            .expect("直连取页失败")
    };

    // 第一、二页正常进缓存
    let first = fixture
        .cached
        .page(fixture.repo_id, &page_query(2, None))
        .expect("首页取页失败");
    assert_pages_equal(&first, &direct_page(Some(0)));
    let second = fixture
        .cached
        .page(fixture.repo_id, &page_query(2, Some(2)))
        .expect("第二页取页失败");
    assert_pages_equal(&second, &direct_page(Some(2)));

    // 第三页 need_end = 6 > 5：缓存填到 5 为止，服务 [4, 5) —— 只有 1 条，
    // has_more = true 且 next_cursor = 5（与直连不同：直连给满 2 条。
    // 这是缓存上限的真实语义——页边界由服务方保证单调推进，页大小只是期望值）
    let third = fixture
        .cached
        .page(fixture.repo_id, &page_query(2, Some(4)))
        .expect("第三页取页失败");
    assert_eq!(
        third.commits.len(),
        1,
        "越过上限前只能服务前缀内的最后 1 条"
    );
    assert_eq!(third.next_cursor, Some(5), "游标必须推进到前缀末尾");

    // 从游标 5 起：越过上限，走绕过路径，与直连全等
    for cursor in [Some(5), Some(8), Some(10), Some(12), Some(40)] {
        let bypassed = fixture
            .cached
            .page(fixture.repo_id, &page_query(2, cursor))
            .expect("绕过路径取页失败");
        let direct = direct_page(cursor);
        assert_pages_equal(&bypassed, &direct);
    }
    // 游标越过历史末端：两路径都给空页、无游标
    let tail = fixture
        .cached
        .page(fixture.repo_id, &page_query(2, Some(40)))
        .expect("越界取页失败");
    assert!(tail.commits.is_empty(), "越界页应为空");
    assert_eq!(tail.next_cursor, None, "越界页不应有游标");
}

#[test]
fn the_cache_is_reused_for_repeated_first_pages() {
    let fixture = build_fixture(None);
    linear_repo(&fixture.dir, 15);

    let first = fixture
        .cached
        .page(fixture.repo_id, &page_query(10, None))
        .expect("首次首页失败");
    let again = fixture
        .cached
        .page(fixture.repo_id, &page_query(10, None))
        .expect("重复首页失败");
    assert_pages_equal(&first, &again);
    // 同键同形状应只有一条缓存条目（重复请求复用它，而不是各自新建）
    assert_eq!(fixture.cache.len(), 1, "重复请求应命中同一条目");
}
