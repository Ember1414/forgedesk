//! 历史查询（T2.1）：分页 + 布局编排。
//!
//! 为什么查询与布局在同一个调用里：界面上"加载下一页"需要的是
//!(提交、图位置、边)三件套，分开返回只会让前端在两次 IPC 之间拿到的
//! 数据不一致。布局本身是纯函数（`domain::history::layout`），这里的职责
//! 只有两个：把界面的查询意图翻译成引擎参数，以及**校验游标**。

use std::path::PathBuf;

use forgedesk_domain::git::{Commit, LogQuery, Page, RepoId, RepoPath};
use forgedesk_domain::history::{GraphLayout, LayoutMode, LayoutOptions};
use forgedesk_domain::{AppError, AppResult, ErrorCode};
use forgedesk_git_engine::engine::GitEngine;

use crate::engines::GitEngines;
use forgedesk_storage::RepositoryStore;

/// 每页大小的上限（PLAN §5.5 的 IPC 上限约束）。
pub const MAX_PAGE_SIZE: usize = 500;

/// 缺省查询：第一页、每页 100 条。
impl Default for HistoryQuery {
    fn default() -> Self {
        Self {
            revision: None,
            all_branches: false,
            paths: Vec::new(),
            author: None,
            since: None,
            until: None,
            message_contains: None,
            first_parent_only: false,
            follow_renames: false,
            collapse_merged_branches: false,
            page_size: 100,
            cursor: None,
        }
    }
}

/// 历史查询参数。
///
/// `cursor` 是**下一页第一行的序号**（0 起始；首页传 `None`）。
///
/// # 为什么用序号而不是 oid
///
/// 任务书要求"基于提交序号而非 skip/limit 以避免全量扫描"。序号游标的真正优势
/// 在于**增量刷新**：新提交到达时，受影响的只是序号发生变化的区间，布局器可以
/// 局部重排（T2.1 第 5 条/T2.9 的缓存），而不是从第一页开始重算。
///
/// 代价要如实说明：**深分页的代价是 O(已加载行数)**（`git log --skip=N` 与
/// libgit2 的 walk 都一样），这是 git 本身的行为，任何客户端都绕不开；
/// "完全不重扫"要靠按 `(repo_id, tips, mode)` 缓存布局（T2.9）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryQuery {
    /// 起始引用（分支名、tag、oid）；`None` = HEAD。
    pub revision: Option<String>,
    /// 包含所有引用（`--all`）。
    pub all_branches: bool,
    /// 路径过滤（文件历史）。
    pub paths: Vec<RepoPath>,
    /// 作者过滤（姓名或邮箱子串，忽略大小写）。
    pub author: Option<String>,
    /// 时间下界（Unix 秒，含）。
    pub since: Option<i64>,
    /// 时间上界（Unix 秒，含）。
    pub until: Option<i64>,
    /// 提交信息包含的子串（字面、区分大小写）。
    pub message_contains: Option<String>,
    /// 只看 first-parent 链。
    pub first_parent_only: bool,
    /// 跟随重命名（文件历史；恰好一条路径时有效）。
    pub follow_renames: bool,
    /// 折叠已合并分支（T2.1 第 4 条；判定与标记形状见
    /// `forgedesk_domain::history::LayoutOptions`）。
    ///
    /// 本开关是"尽力而为"：折叠只在**完整窗口**上提供，窗口不完整
    /// （后面还有页）时静默回退为不折叠——见 [`HistoryService::page`]。
    pub collapse_merged_branches: bool,
    /// 每页条数（1..=500；缺省 100）。
    pub page_size: usize,
    /// 游标（下一页第一行的序号）。
    pub cursor: Option<u32>,
}

/// 一页历史：提交 + 这一段的图布局 + 下一页游标。
#[derive(Debug, Clone)]
pub struct HistoryPage {
    /// 本页提交（新 → 旧）。
    pub commits: Vec<Commit>,
    /// 本页的泳道布局（与 `commits` 一一对应）。
    pub layout: GraphLayout,
    /// 下一页游标；`None` 表示没有更多了。
    pub next_cursor: Option<u32>,
}

/// 历史查询用例服务。
pub struct HistoryService<'a> {
    engines: &'a GitEngines,
    store: RepositoryStore<'a>,
}

impl<'a> HistoryService<'a> {
    /// 绑定引擎与仓库记录存储（与 `WorkspaceService` 同一批基础设施）。
    pub fn new(engines: &'a GitEngines, store: RepositoryStore<'a>) -> Self {
        Self { engines, store }
    }

    /// 解析记录 id 为工作区路径；记录不存在时返回 `NOT_FOUND`。
    ///
    /// 与 `WorkspaceService::resolve_workdir` 同一个实现（那边的注释解释了
    /// 为什么用 `head().is_err()` 一类判据）——这里不重复，只复用形状。
    fn resolve_workdir(&self, repo_id: i64) -> AppResult<PathBuf> {
        let record = self.store.find_by_id(repo_id)?.ok_or(
            AppError::new(ErrorCode::NotFound, "the repository record does not exist")
                .with_detail(format!("repo_id: {repo_id}")),
        )?;
        Ok(PathBuf::from(record.path))
    }

    /// 取一页历史并计算布局。
    ///
    /// `first_parent_only` 同时作用于**查询与布局**：查询过滤掉支线提交、
    /// 布局只画第一父的边——两者必须一致，否则"只看主线"会画出断头路。
    pub fn page(&self, repo_id: i64, query: &HistoryQuery) -> AppResult<HistoryPage> {
        let page_size = query.page_size.clamp(1, MAX_PAGE_SIZE);
        let cursor = query.cursor.unwrap_or(0);

        if query.follow_renames && query.paths.len() != 1 {
            return Err(AppError::new(
                ErrorCode::Validation,
                "follow_renames requires exactly one path",
            ));
        }

        let workdir = self.resolve_workdir(repo_id)?;
        let repo = RepoId::new(workdir);

        let log_query = LogQuery {
            revision: query.revision.clone(),
            limit: page_size,
            // 游标即下一页第一行的序号：本页 layout 的 row 0 对应全历史的第 cursor 行
            skip: usize::try_from(cursor).unwrap_or(usize::MAX),
            all_branches: query.all_branches,
            paths: query.paths.clone(),
            author: query.author.clone(),
            since: query.since,
            until: query.until,
            message_contains: query.message_contains.clone(),
            first_parent_only: query.first_parent_only,
            follow_renames: query.follow_renames,
        };

        let page: Page<Commit> = self.engines.read().log(&repo, log_query)?;
        let mode = if query.first_parent_only {
            LayoutMode::FirstParentOnly
        } else {
            LayoutMode::AllBranches
        };
        // 折叠只在**完整窗口**上提供：has_more == true 时祖先可能落在窗外，
        // "第二父祖先 ⊆ 第一父祖先集"会误判（窗外祖先看不见），因此静默回退。
        // has_more 在取页后就已知，而 layout 需要页内容——先取页、再判定、后布局。
        let collapse = query.collapse_merged_branches && !page.has_more;
        // 布局器的 row 是"输入数组下标"（0 起始）；服务层把它平移成**全局行号**：
        // 渲染层的滚动、迷你地图与跨页选中都按全局行号工作，第二页的第一行
        // 必须是全历史的第 cursor 行，而不是"本页的第 0 行"。
        let mut layout = forgedesk_domain::history::layout(
            &page.items,
            LayoutOptions {
                mode,
                collapse_merged_branches: collapse,
            },
        );
        for row in &mut layout.rows {
            row.row = row.row.saturating_add(cursor);
        }

        let next_cursor = if page.has_more {
            Some(cursor + u32::try_from(page.items.len()).unwrap_or(u32::MAX))
        } else {
            None
        };

        Ok(HistoryPage {
            commits: page.items,
            layout,
            next_cursor,
        })
    }
}
