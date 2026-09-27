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
/** 提交历史（M2 起有真实查询；现在先占位，避免 M2 再加一处需要同步的清单）。 */
export const LOG_QUERY_KEY = 'log';
/** 分支列表（同上）。 */
export const BRANCHES_QUERY_KEY = 'branches';
/** 快照列表（T1.9）。 */
export const SNAPSHOTS_QUERY_KEY = 'snapshots';

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
