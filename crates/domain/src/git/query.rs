//! 查询条件与分页结果。

use super::path::RepoPath;

/// 提交日志查询条件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogQuery {
    /// 起始引用（分支名、tag、oid）。`None` 表示 `HEAD`。
    pub revision: Option<String>,
    /// 最多返回多少条（分页大小）。
    pub limit: usize,
    /// 跳过多少条（分页游标）。
    pub skip: usize,
    /// 是否包含所有引用（`--all`）而不只是当前分支。
    pub all_branches: bool,
    /// 路径过滤（只返回改动过这些路径的提交）。
    pub paths: Vec<RepoPath>,
    /// 作者过滤（匹配姓名或邮箱的子串）。
    pub author: Option<String>,
}

/// 默认分页大小。
///
/// 100 是"一屏装得下、又不用频繁翻页"的经验值；大仓库上的首屏渲染
/// （PLAN M2 要求 ≤3s）靠的是虚拟化与增量加载，不是一次多取。
pub const DEFAULT_PAGE_SIZE: usize = 100;

impl Default for LogQuery {
    fn default() -> Self {
        Self {
            revision: None,
            limit: DEFAULT_PAGE_SIZE,
            skip: 0,
            all_branches: false,
            paths: Vec::new(),
            author: None,
        }
    }
}

impl LogQuery {
    /// 创建默认查询。
    pub fn new() -> Self {
        Self::default()
    }

    /// 指定起始引用。
    #[must_use]
    pub fn with_revision(mut self, revision: impl Into<String>) -> Self {
        self.revision = Some(revision.into());
        self
    }

    /// 指定分页大小。
    #[must_use]
    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    /// 指定分页偏移。
    #[must_use]
    pub fn with_skip(mut self, skip: usize) -> Self {
        self.skip = skip;
        self
    }

    /// 包含所有引用。
    #[must_use]
    pub fn with_all_branches(mut self, all_branches: bool) -> Self {
        self.all_branches = all_branches;
        self
    }

    /// 附加路径过滤。
    #[must_use]
    pub fn with_paths(mut self, paths: Vec<RepoPath>) -> Self {
        self.paths = paths;
        self
    }

    /// 附加作者过滤。
    #[must_use]
    pub fn with_author(mut self, author: impl Into<String>) -> Self {
        self.author = Some(author.into());
        self
    }
}

/// 一页结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page<T> {
    /// 本页条目。
    pub items: Vec<T>,
    /// 是否还有下一页。
    ///
    /// 由"多取一条"判断，而不是靠 `items.len() == limit`：
    /// 恰好取满时后者会误报"还有下一页"，界面于是显示一个永远点不出东西的按钮。
    pub has_more: bool,
    /// 总条数。`None` 表示未知——大仓库上精确计数要遍历全部提交，
    /// 为了一个数字把首屏拖慢到秒级不划算。
    pub total: Option<usize>,
}

impl<T> Page<T> {
    /// 用"多取一条"的原始结果构造一页。
    ///
    /// `raw` 是实际请求 `limit + 1` 条得到的结果；这里负责判断并裁掉多余的那条。
    /// 把这段逻辑放在领域层（而不是各引擎里各写一遍）是为了保证两个实现的
    /// 分页语义完全一致。
    pub fn from_over_fetch(mut raw: Vec<T>, limit: usize) -> Self {
        let has_more = raw.len() > limit;
        raw.truncate(limit);
        Self {
            items: raw,
            has_more,
            total: None,
        }
    }

    /// 空页。
    pub fn empty() -> Self {
        Self {
            items: Vec::new(),
            has_more: false,
            total: Some(0),
        }
    }

    /// 映射条目类型，保留分页信息。
    pub fn map<U>(self, mut f: impl FnMut(T) -> U) -> Page<U> {
        Page {
            items: self.items.into_iter().map(&mut f).collect(),
            has_more: self.has_more,
            total: self.total,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{LogQuery, Page, DEFAULT_PAGE_SIZE};

    #[test]
    fn default_query_pages_from_head_without_filters() {
        let query = LogQuery::default();

        assert_eq!(query.revision, None);
        assert_eq!(query.limit, DEFAULT_PAGE_SIZE);
        assert_eq!(query.skip, 0);
        assert!(!query.all_branches);
        assert!(query.paths.is_empty());
    }

    #[test]
    fn over_fetch_of_exactly_the_limit_does_not_claim_another_page() {
        // 取满 limit 但没多出来 → 没有下一页
        let page = Page::from_over_fetch(vec![1, 2, 3], 3);

        assert_eq!(page.items, vec![1, 2, 3]);
        assert!(!page.has_more);
    }

    #[test]
    fn over_fetch_of_limit_plus_one_truncates_and_reports_more() {
        let page = Page::from_over_fetch(vec![1, 2, 3, 4], 3);

        assert_eq!(page.items, vec![1, 2, 3], "多取的那条不能泄漏到界面上");
        assert!(page.has_more);
    }

    #[test]
    fn mapping_preserves_pagination_information() {
        let page = Page::from_over_fetch(vec![1, 2, 3, 4], 3).map(|n| n * 10);

        assert_eq!(page.items, vec![10, 20, 30]);
        assert!(page.has_more);
    }

    #[test]
    fn empty_page_reports_a_known_total_of_zero() {
        let page: Page<u8> = Page::empty();

        assert!(page.items.is_empty());
        assert!(!page.has_more);
        assert_eq!(page.total, Some(0));
    }
}
