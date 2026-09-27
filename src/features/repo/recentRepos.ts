import { useQuery } from '@tanstack/react-query';

import { repoRecentList } from '@/lib/ipc';
import type { RecentRepository } from '@/lib/ipc';
import { RECENT_REPOS_QUERY_KEY } from '@/lib/queryKeys';
import { useUiStore } from '@/stores/uiStore';

/**
 * 最近打开的仓库（真实数据源）。
 *
 * M0 时这里是一份**写死的占位列表**，因为当时还没有本地存储。T1.3 之后有了
 * `repo_recent_list`（本地记录，含打开时间、当前分支、是否已打开），因此
 * 外壳的三个使用者——顶栏的仓库切换器、底部状态栏、仓库页的标题——都读这一处。
 *
 * 三条约定：
 *
 * 1. **只有一个缓存条目**（键里不带 limit）：切换器要 20 条、仪表盘只显示 12 条，
 *    但它们读的是同一份数据。带 limit 的键会分裂成两个缓存，失效时得记得失效两处；
 * 2. **失败不抛给界面**：拿不到列表时当作空列表（切换器显示"还没有打开过仓库"）。
 *    一个读不到最近记录的仓库切换器不该把整条外壳拖成错误页；
 * 3. **id 是字符串**：路由段与 store 里都是字符串，而存储层的记录 id 是数字，
 *    转换只在这一个模块里做（`String(repo.id)`），别处不要再各自转换。
 */
export const RECENT_REPOS_LIMIT = 20;

export interface RecentReposState {
  readonly repos: readonly RecentRepository[];
  /** 首次加载中（骨架屏用）。 */
  readonly isPending: boolean;
  /** 读取失败（界面用空态 + 说明，不弹错误）。 */
  readonly isError: boolean;
}

/** 最近打开的仓库；失败时返回空列表。 */
export function useRecentRepos(): RecentReposState {
  const query = useQuery({
    queryKey: [RECENT_REPOS_QUERY_KEY],
    queryFn: () => repoRecentList(RECENT_REPOS_LIMIT),
  });

  return {
    repos: query.data ?? [],
    isPending: query.isPending,
    isError: query.isError,
  };
}

/**
 * 按 id 查一个仓库（id 是路由段/ store 里的字符串形式）。
 *
 * 找不到时返回 `undefined`：可能是深链指向一个已被移出列表的仓库，
 * 也可能列表还在加载。界面在两种情况下都只少一个名字，不该因此报错。
 */
export function useRepoById(repoId: string | undefined): RecentRepository | undefined {
  const { repos } = useRecentRepos();

  if (repoId === undefined) {
    return undefined;
  }
  return repos.find((repo) => String(repo.id) === repoId);
}

/** 当前选中的仓库（store 里的 id）。 */
export function useCurrentRepo(): RecentRepository | undefined {
  const currentRepoId = useUiStore((state) => state.currentRepoId);
  return useRepoById(currentRepoId ?? undefined);
}

/** 仓库记录 id → 路由段/存储里用的字符串 id。 */
export function repoIdOf(repo: RecentRepository): string {
  return String(repo.id);
}
