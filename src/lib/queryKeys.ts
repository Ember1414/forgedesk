/**
 * TanStack Query 的键（唯一来源）。
 *
 * # 为什么集中定义
 *
 * `repo:changed` 的处理必须按**同一批键**去失效。键散在各个页面里时，
 * 任何一次改名都会让失效静默地不再命中——查询永远陈旧，却没有任何报错，
 * 而这类 bug 只会在"界面看起来没刷新"的模糊描述里出现。
 *
 * 这里只放"键的形状"，不放查询函数与组件（那些属于各自的页面）。
 */

/** 工作区状态（`workspace_status`）。 */
export const STATUS_QUERY_KEY = 'status';
/** 单文件 diff（`workspace_diff`）。 */
export const DIFF_QUERY_KEY = 'diff';
/** 提交历史（`git_log_page`，T2.2 起有真实查询）。 */
export const LOG_QUERY_KEY = 'log';
/** 分支列表（同上）。 */
export const BRANCHES_QUERY_KEY = 'branches';
/** 快照列表（T1.9）。 */
export const SNAPSHOTS_QUERY_KEY = 'snapshots';
/**
 * 最近打开的仓库（T1.3 的本地记录）。
 *
 * 放在这里而不是某个页面里：顶栏切换器、底部状态栏、仪表盘、仓库页标题都读它，
 * 而"关闭/移出仓库之后列表必须跟着变"这件事只能靠键的形状统一来保证。
 */
export const RECENT_REPOS_QUERY_KEY = 'recent-repositories';

/** 某个仓库的状态查询键。 */
export function statusKey(repoId: number): readonly [string, number] {
  return [STATUS_QUERY_KEY, repoId];
}

/** 某个仓库的 diff 查询键前缀（完整键还带 target / path / 参数）。 */
export function diffKeyPrefix(repoId: number): readonly [string, number] {
  return [DIFF_QUERY_KEY, repoId];
}

/** diff 查询键里路径所在的下标（0=类别，1=repoId，2=target，3=path…）。 */
export const DIFF_KEY_PATH_INDEX = 3;

/**
 * 某一页提交历史的查询键。
 *
 * # 为什么把 cursor 与筛选条件都写进键
 *
 * 历史是**分页追加**而不是"整表替换"：滚动到底部加载下一页时，上一页的数据
 * 必须还在缓存里（否则界面会闪一下"空列表"）。把 cursor 编进键，每一页就是
 * 一条独立的缓存记录，TanStack Query 天然帮我们保存与失效。
 *
 * `filters` 是一个**稳定的签名字符串**（由 `@/features/history/useGraphQuery`
 * 的 `filtersSignature` 生成，字段排序后序列化）。为什么不是对象：
 * Query 的键按**结构化相等**比较，对象里字段顺序不同就会被当成两个键，
 * 结果是同一份数据被请求两次、失效时又只能命中其中一份。
 *
 * # 为什么前两位必须是 LOG_QUERY_KEY + repoId
 *
 * `repo:changed` 按 `[LOG_QUERY_KEY, repoId]` 前缀失效**所有页**
 * （见 `src/lib/repoChanged.ts`）。改这个前缀会让失效静默地只命中第一页。
 */
export function logKey(
  repoId: number,
  cursor: number,
  filters: string,
): readonly [string, number, number, string] {
  return [LOG_QUERY_KEY, repoId, cursor, filters];
}

/** 某个仓库全部历史页的键前缀（整表失效时用；与 `repoChanged.ts` 的口径一致）。 */
export function logKeyPrefix(repoId: number): readonly [string, number] {
  return [LOG_QUERY_KEY, repoId];
}

/** 仓库作者列表（`git_log_authors`，T2.3 的作者筛选下拉）。 */
export const AUTHORS_QUERY_KEY = 'authors';

/** 某仓库作者列表的查询键。 */
export function authorsKey(repoId: number): readonly [string, number] {
  return [AUTHORS_QUERY_KEY, repoId];
}

/** 单次提交详情（`git_commit_detail`，T2.4）。 */
export const COMMIT_DETAIL_QUERY_KEY = 'commitDetail';

/**
 * 某个提交详情的查询键。
 *
 * `oid` 与 `parentIndex` 都进键：合并提交的"相对第一父 / 相对第二父"是**同一个
 * 提交的两份视图**，各自占一条缓存让切换瞬时完成（配合 `placeholderData`
 * 消掉切换时的闪空）。失效按 `[COMMIT_DETAIL_QUERY_KEY, repoId]` 前缀
 * （`repoChanged.ts`）——`isHead` / `isPushed` / refs 都会随引用移动而变化。
 */
export function commitDetailKey(
  repoId: number,
  oid: string,
  parentIndex: number,
): readonly [string, number, string, number] {
  return [COMMIT_DETAIL_QUERY_KEY, repoId, oid, parentIndex];
}

/** 某个仓库全部提交详情的键前缀（`refs` / `large` 失效用）。 */
export function commitDetailKeyPrefix(repoId: number): readonly [string, number] {
  return [COMMIT_DETAIL_QUERY_KEY, repoId];
}
