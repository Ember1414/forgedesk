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
    /// 只返回该时间（Unix 秒）**之后**的提交（`--since`）。
    pub since: Option<i64>,
    /// 只返回该时间（Unix 秒）**之前**的提交（`--until`）。
    pub until: Option<i64>,
    /// 提交信息（**含正文**）包含的子串：字面匹配、区分大小写（`--grep --fixed-strings`）。
    ///
    /// 刻意不支持正则：界面上"搜提交"几乎总是找一句话，正则的转义负担
    /// 会变成"搜不出来但不知道为什么"。
    pub message_contains: Option<String>,
    /// 只沿 first-parent 链走（`--first-parent`）。
    pub first_parent_only: bool,
    /// 跟随重命名（`--follow`）。仅在 `paths` 恰好是一条时有意义；
    /// libgit2 不支持该选项，置位时会返回 `UnsupportedByEngine`。
    pub follow_renames: bool,
    /// 分支多选（T2.3）：非空时遍历这些 tip 的**并集**，此时 `revision` 与
    /// `all_branches` 被忽略（界面把"全部分支"与"多选分支"建模成互斥选项）。
    pub revisions: Vec<String>,
    /// 关键词匹配忽略大小写（配合 `message_contains`；CLI 侧即 `--grep -i`）。
    pub case_insensitive: bool,
    /// 只显示合并提交（CLI `--merges`；libgit2 按 parent_count 过滤）。
    pub merges_only: bool,
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
            since: None,
            until: None,
            message_contains: None,
            first_parent_only: false,
            follow_renames: false,
            revisions: Vec::new(),
            case_insensitive: false,
            merges_only: false,
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

/// 一个提交作者（按邮箱去重后的身份；T2.3 的作者筛选列表）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorSummary {
    /// 姓名：同一邮箱出现多个名字时取提交数最多者（并列取字典序）。
    pub name: String,
    /// 邮箱（去重键：git 身份里稳定的是邮箱，名字随时会改）。
    pub email: String,
    /// 提交数（`--all` 范围内）。
    pub commit_count: u64,
}

/// 把 (姓名, 邮箱) 记录流汇总成去重的作者列表（纯函数，导出供单测）。
///
/// 规则：按邮箱分组（git 身份里稳定的是邮箱，名字随时会改）；组内姓名取
/// 出现最多者（并列取字典序，保证同一份输入永远得到同一个结果）；输出按
/// 提交数降序、数量并列时按邮箱升序。
pub fn summarize_authors(
    records: impl IntoIterator<Item = (String, String)>,
) -> Vec<AuthorSummary> {
    use std::collections::HashMap;

    // email -> (name -> 次数, 总提交数)
    let mut groups: HashMap<String, (HashMap<String, u64>, u64)> = HashMap::new();
    for (name, email) in records {
        let entry = groups.entry(email).or_default();
        *entry.0.entry(name).or_insert(0) += 1;
        entry.1 += 1;
    }

    let mut out: Vec<AuthorSummary> = groups
        .into_iter()
        .map(|(email, (names, total))| {
            let (name, _) = names
                .into_iter()
                .max_by(|left, right| left.1.cmp(&right.1).then_with(|| right.0.cmp(&left.0)))
                .unwrap_or_default();
            AuthorSummary {
                name,
                email,
                commit_count: total,
            }
        })
        .collect();
    out.sort_by(|left, right| {
        right
            .commit_count
            .cmp(&left.commit_count)
            .then_with(|| left.email.cmp(&right.email))
    });
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod author_tests {
    use super::summarize_authors;

    #[test]
    fn authors_are_grouped_by_email_and_sorted_by_count() {
        let out = summarize_authors([
            ("Alice".to_owned(), "a@x.dev".to_owned()),
            ("Bob".to_owned(), "b@x.dev".to_owned()),
            ("Alice".to_owned(), "a@x.dev".to_owned()),
            ("Carol".to_owned(), "c@x.dev".to_owned()),
        ]);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].email, "a@x.dev");
        assert_eq!(out[0].name, "Alice");
        assert_eq!(out[0].commit_count, 2);
    }

    #[test]
    fn one_email_with_many_names_takes_the_most_frequent_then_lexicographic() {
        let out = summarize_authors([
            ("Zed".to_owned(), "x@x.dev".to_owned()),
            ("Amy".to_owned(), "x@x.dev".to_owned()),
            ("Amy".to_owned(), "x@x.dev".to_owned()),
        ]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "Amy", "并列时取字典序：Amy < Zed");
    }

    #[test]
    fn empty_input_yields_an_empty_list() {
        assert!(summarize_authors([]).is_empty());
    }
}
