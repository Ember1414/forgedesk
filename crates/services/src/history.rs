//! 历史查询（T2.1）：分页 + 布局编排。
//!
//! 为什么查询与布局在同一个调用里：界面上"加载下一页"需要的是
//!(提交、图位置、边)三件套，分开返回只会让前端在两次 IPC 之间拿到的
//! 数据不一致。布局本身是纯函数（`domain::history::layout`），这里的职责
//! 只有两个：把界面的查询意图翻译成引擎参数，以及**校验游标**。

use std::path::PathBuf;
use std::sync::MutexGuard;

use forgedesk_domain::git::{AuthorSummary, Branch, Commit, LogQuery, Page, RepoId, RepoPath};
use forgedesk_domain::history::{GraphLayout, LayoutMode, LayoutOptions};
use forgedesk_domain::{AppError, AppResult, ErrorCode};
use forgedesk_git_engine::engine::GitEngine;
use serde::{Deserialize, Deserializer, Serialize};

use crate::engines::GitEngines;
use crate::history_cache::{
    lock_ignoring_poison, CachedWalk, LogPageCache, LogQueryKey, TipsFingerprint,
};
use forgedesk_storage::RepositoryStore;

/// 每页大小的上限（PLAN §5.5 的 IPC 上限约束）。
pub const MAX_PAGE_SIZE: usize = 500;

/// 缺省每页大小（与 `Default` impl 一致）。
fn default_page_size() -> usize {
    100
}

/// 反序列化 `paths`：JSON 形状是 `string[]`，转为 `Vec<RepoPath>`。
///
/// 与 `crates/commands/src/workspace.rs` 中 `DiffRequest.paths` 的先例一致：
/// 前端传 `string[]`，后端用 `RepoPath::from(String)` 转换。
fn deserialize_paths<'de, D>(deserializer: D) -> Result<Vec<RepoPath>, D::Error>
where
    D: Deserializer<'de>,
{
    let strings: Vec<String> = Vec::deserialize(deserializer)?;
    Ok(strings.into_iter().map(RepoPath::from).collect())
}

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
            revisions: Vec::new(),
            case_insensitive: false,
            merges_only: false,
            my_commits_only: false,
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
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryQuery {
    /// 起始引用（分支名、tag、oid）；`None` = HEAD。
    #[serde(default)]
    pub revision: Option<String>,
    /// 包含所有引用（`--all`）。
    #[serde(default)]
    pub all_branches: bool,
    /// 路径过滤（文件历史）。
    #[serde(default, deserialize_with = "deserialize_paths")]
    pub paths: Vec<RepoPath>,
    /// 作者过滤（姓名或邮箱子串，忽略大小写）。
    #[serde(default)]
    pub author: Option<String>,
    /// 时间下界（Unix 秒，含）。
    #[serde(default)]
    pub since: Option<i64>,
    /// 时间上界（Unix 秒，含）。
    #[serde(default)]
    pub until: Option<i64>,
    /// 提交信息包含的子串（字面、区分大小写）。
    #[serde(default)]
    pub message_contains: Option<String>,
    /// 只看 first-parent 链。
    #[serde(default)]
    pub first_parent_only: bool,
    /// 跟随重命名（文件历史；恰好一条路径时有效）。
    #[serde(default)]
    pub follow_renames: bool,
    /// 折叠已合并分支（T2.1 第 4 条；判定与标记形状见
    /// `forgedesk_domain::history::LayoutOptions`）。
    ///
    /// 本开关是"尽力而为"：折叠只在**完整窗口**上提供，窗口不完整
    /// （后面还有页）时静默回退为不折叠——见 [`HistoryService::page`]。
    #[serde(default)]
    pub collapse_merged_branches: bool,
    /// 分支多选（T2.3）：遍历这些 tip 的并集；非空时 `revision` 与
    /// `all_branches` 被忽略（前端把三者建模成互斥选项）。
    #[serde(default)]
    pub revisions: Vec<String>,
    /// 关键词匹配忽略大小写（配合 `messageContains`；缺省 false = 与旧契约一致）。
    #[serde(default)]
    pub case_insensitive: bool,
    /// 只显示合并提交（`--merges` 口径）。
    #[serde(default)]
    pub merges_only: bool,
    /// 只显示"我的提交"：服务层把当前身份（仓库 `user.email`，跟随
    /// local→global 解析链）翻译成 author 过滤。未配置身份时返回
    /// `VALIDATION`——静默当成"全部提交"会让用户以为开关坏了。
    #[serde(default)]
    pub my_commits_only: bool,
    /// 每页条数（1..=500；缺省 100）。
    #[serde(default = "default_page_size")]
    pub page_size: usize,
    /// 游标（下一页第一行的序号）。
    #[serde(default)]
    pub cursor: Option<u32>,
}

/// 一页历史：提交 + 这一段的图布局 + 下一页游标。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
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
    /// 日志分页缓存（T2.9）；`None` = 不缓存（默认，测试与探针的直连口径）。
    log_cache: Option<&'a LogPageCache>,
}

impl<'a> HistoryService<'a> {
    /// 绑定引擎与仓库记录存储（与 `WorkspaceService` 同一批基础设施）。
    pub fn new(engines: &'a GitEngines, store: RepositoryStore<'a>) -> Self {
        Self {
            engines,
            store,
            log_cache: None,
        }
    }

    /// 接上日志分页缓存（T2.9）。
    ///
    /// 与 `with_credential_gate` 同一先例：缓存是**可选能力**，命令层接线时
    /// 打开，测试与探针缺省直连引擎——差分测试因此能同时钉住两条路径。
    pub fn with_log_cache(mut self, cache: &'a LogPageCache) -> Self {
        self.log_cache = Some(cache);
        self
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

        // "仅显示我的提交"在服务层翻译成 author 过滤：身份的真相源是仓库的
        // user.email（跟随 local→global 解析链）。与显式 author 同设时本开关
        // 优先——两个作者过滤器在 git 里是 OR 关系（交集做不到），叠加会
        // 产生"既不是我、又是那个人"的意外并集。
        let mut author = query.author.clone();
        if query.my_commits_only {
            let email = self.engines.write().config_value(&repo, "user.email")?;
            let Some(email) = email else {
                return Err(AppError::new(
                    ErrorCode::Validation,
                    "my_commits_only requires user.email to be configured",
                ));
            };
            author = Some(email);
        }

        let log_query = LogQuery {
            revision: query.revision.clone(),
            limit: page_size,
            // 游标即下一页第一行的序号：本页 layout 的 row 0 对应全历史的第 cursor 行
            skip: usize::try_from(cursor).unwrap_or(usize::MAX),
            all_branches: query.all_branches,
            paths: query.paths.clone(),
            author,
            since: query.since,
            until: query.until,
            message_contains: query.message_contains.clone(),
            first_parent_only: query.first_parent_only,
            follow_renames: query.follow_renames,
            revisions: query.revisions.clone(),
            case_insensitive: query.case_insensitive,
            merges_only: query.merges_only,
        };

        // 有缓存时走"累积前缀"路径（T2.9）：重复的首页与相邻深页不再从 tip 重扫。
        // 两条路径的结果必须完全一致——services/tests/history_cache.rs 的差分用例钉住。
        let (items, has_more) = match self.log_cache {
            None => {
                let page: Page<Commit> = self.engines.read().log(&repo, log_query)?;
                (page.items, page.has_more)
            }
            Some(cache) => {
                self.page_via_cache(cache, repo_id, &repo, &log_query, cursor, page_size)?
            }
        };

        let mode = if query.first_parent_only {
            LayoutMode::FirstParentOnly
        } else {
            LayoutMode::AllBranches
        };
        // 折叠只在**完整窗口**上提供：has_more == true 时祖先可能落在窗外，
        // "第二父祖先 ⊆ 第一父祖先集"会误判（窗外祖先看不见），因此静默回退。
        // has_more 在取页后就已知，而 layout 需要页内容——先取页、再判定、后布局。
        let collapse = query.collapse_merged_branches && !has_more;
        // 布局器的 row 是"输入数组下标"（0 起始）；服务层把它平移成**全局行号**：
        // 渲染层的滚动、迷你地图与跨页选中都按全局行号工作，第二页的第一行
        // 必须是全历史的第 cursor 行，而不是"本页的第 0 行"。
        let mut layout = forgedesk_domain::history::layout(
            &items,
            LayoutOptions {
                mode,
                collapse_merged_branches: collapse,
            },
        );
        for row in &mut layout.rows {
            row.row = row.row.saturating_add(cursor);
        }

        let next_cursor = if has_more {
            Some(cursor + u32::try_from(items.len()).unwrap_or(u32::MAX))
        } else {
            None
        };

        Ok(HistoryPage {
            commits: items,
            layout,
            next_cursor,
        })
    }

    /// 经缓存取一页：查指纹 → 取/建前缀条目 → 需要时向引擎续填 → 切片返回。
    ///
    /// 返回 (本页提交, 是否还有更多)。与直连路径的契约一致：
    /// `items` 是全历史的第 `cursor..cursor+items.len()` 行。
    fn page_via_cache(
        &self,
        cache: &LogPageCache,
        repo_id: i64,
        repo: &RepoId,
        log_query: &LogQuery,
        cursor: u32,
        page_size: usize,
    ) -> AppResult<(Vec<Commit>, bool)> {
        let entry_cap = cache.entry_cap();

        // 游标在前缀能力之外（大仓库深翻）：直连引擎，**连条目都不建**——
        // 否则每次深翻都会白填一段永远服务不到的前缀。这是深翻的旧行为：
        // 慢但正确，且不受前缀上限约束。
        if cursor as usize >= entry_cap {
            let page = self.engines.read().log(repo, log_query.clone())?;
            return Ok((page.items, page.has_more));
        }

        // 指纹先于条目：外部 git 进程改过 refs 时，这次请求会落到**新键**上，
        // 旧前缀留在缓存里等 LRU 驱逐（下次同一形状还能用回来）。
        let tips = self.tips_fingerprint(repo)?;
        let query_key = LogQueryKey {
            revision: log_query.revision.clone(),
            all_branches: log_query.all_branches,
            paths: log_query.paths.clone(),
            author: log_query.author.clone(),
            since: log_query.since,
            until: log_query.until,
            message_contains: log_query.message_contains.clone(),
            first_parent_only: log_query.first_parent_only,
            follow_renames: log_query.follow_renames,
            revisions: log_query.revisions.clone(),
            case_insensitive: log_query.case_insensitive,
            merges_only: log_query.merges_only,
        };
        let entry = cache.entry(repo_id, query_key, tips);
        let mut walk: MutexGuard<'_, CachedWalk> = lock_ignoring_poison(&entry);
        let need_end = cursor as usize + page_size;

        while !walk.exhausted && walk.commits.len() < need_end.min(entry_cap) {
            let skip = walk.commits.len();
            let limit = need_end.min(entry_cap) - skip;
            let more = self.engines.read().log(
                repo,
                LogQuery {
                    skip,
                    limit,
                    ..log_query.clone()
                },
            )?;
            let reached_end = !more.has_more;
            let added = more.items.len();
            walk.commits.extend(more.items);
            if reached_end || added == 0 {
                // added == 0 且引擎声称还有更多：防病态引擎死循环的护栏，
                // 正常实现不会走到（has_more 由"取满 limit"推导）。
                walk.exhausted = true;
            }
            // 表锁与条目锁的顺序固定为"先条目后表"，与 `entry()` 不嵌套，无死锁环
            cache.record_added(added);
            if walk.commits.len() >= entry_cap {
                break;
            }
        }

        let start = (cursor as usize).min(walk.commits.len());
        let end = need_end.min(walk.commits.len());
        let items = walk.commits[start..end].to_vec();
        let has_more = if walk.exhausted {
            end < walk.commits.len()
        } else {
            // 前缀还没探到头：即便本页正好切到前缀末尾，后面也可能（且多半）还有
            true
        };
        Ok((items, has_more))
    }

    /// 计算 tips 指纹：分支/标签/HEAD 的名字与 oid。
    ///
    /// 引用数量与历史规模无关（10 万提交的仓库通常也只有几百个引用），
    /// 每次请求枚举一遍的成本是毫秒级——换来的是不依赖事件通知的失效语义。
    fn tips_fingerprint(&self, repo: &RepoId) -> AppResult<TipsFingerprint> {
        let engine = self.engines.read();
        let mut refs: Vec<(String, String)> = engine
            .branch_list(repo)?
            .into_iter()
            .map(|branch| (branch.name, branch.target))
            .collect();
        refs.extend(engine.tag_list(repo)?.into_iter().map(|tag| {
            // 附注标签指向 tag 对象；布局与日志关心的是它解引用后的提交
            let oid = tag.commit.unwrap_or(tag.target);
            (tag.name, oid)
        }));
        refs.sort();
        let head = engine.head_oid(repo)?;
        Ok(TipsFingerprint { refs, head })
    }

    /// 列出分支（T2.3 的分支多选下拉；`include_remote` 控制是否带远端跟踪分支）。
    ///
    /// 读取走 libgit2（分支列表是纯引用读取）。**T2.5 的分支管理会把这条
    /// 能力搬进专门的分支服务**——现在挂在历史服务上是因为筛选栏是它唯一
    /// 的消费者，先满足"单一消费者、最小接线"。
    pub fn branches(&self, repo_id: i64, include_remote: bool) -> AppResult<Vec<Branch>> {
        let workdir = self.resolve_workdir(repo_id)?;
        let mut branches = self.engines.read().branch_list(&RepoId::new(workdir))?;
        if !include_remote {
            // trait 的 branch_list 不带开关（分支列表是整体读取的）；远端过滤
            // 在这里做——is_remote 是引擎给出的结构化标记，不是名字前缀猜测
            branches.retain(|branch| !branch.is_remote);
        }
        Ok(branches)
    }

    /// 列出仓库作者（T2.3 的作者筛选下拉）。范围与 `--all` 一致。
    ///
    /// 只走 CLI 引擎（libgit2 侧未实现该读取，与 `remote_refs_containing`
    /// 同一先例）；作者列表是低频低量数据（去重后通常 < 100 行）。
    pub fn authors(&self, repo_id: i64) -> AppResult<Vec<AuthorSummary>> {
        let workdir = self.resolve_workdir(repo_id)?;
        self.engines.write().authors(&RepoId::new(workdir))
    }
}
