/**
 * 同步状态（T2.6）：当前分支、上游名、ahead/behind。
 *
 * # 为什么是一个"合成查询"而不是直接读分支列表
 *
 * `git_branch_list` 的 `ahead` / `behind` 只有能廉价算出来时才非空（读引擎走
 * libgit2，不为了列表去跑 `rev-list`）。因此这里的分工是：
 *
 *   1. 分支列表告诉我们**当前分支**与它的**上游名**；
 *   2. 缺失的 ahead/behind 用 `git_branch_compare` 补齐（一次 `rev-list --count`）。
 *
 * 两者合成同一个查询，是为了让同步条**不会**先画出一个 0/0 再跳成 2/1
 * ——那个中间态会被用户理解成"刚刚同步过了"。
 */
import { useQuery } from '@tanstack/react-query';

import { gitBranchCompare, gitBranchList } from '@/lib/ipc';
import type { Branch } from '@/lib/ipc';
import { syncStatusKey } from '@/lib/queryKeys';

/** 同步条要展示的那几项。 */
export interface SyncStatus {
  /** 当前分支名；还没有提交（未出生的分支）时为 null。 */
  readonly branch: string | null;
  /** 上游短名（如 `origin/main`）；未配置或已消失时为 null。 */
  readonly upstream: string | null;
  /** 上游所属的远端名（`origin/main` → `origin`）。 */
  readonly remote: string | null;
  readonly ahead: number;
  readonly behind: number;
}

/**
 * 从上游短名里取远端名。
 *
 * 只切**第一个**斜杠：分支名本身可以带斜杠（`origin/feature/xyz` → `origin`）。
 */
export function remoteOf(upstream: string | null): string | null {
  if (upstream === null) {
    return null;
  }
  const separator = upstream.indexOf('/');
  return separator > 0 ? upstream.slice(0, separator) : upstream;
}

/** 没有上游（或没有当前分支）时的状态。 */
export function detachedStatus(branch: string | null): SyncStatus {
  return { branch, upstream: null, remote: null, ahead: 0, behind: 0 };
}

/** 读取同步状态（分支列表 + 必要的比较，见文件头）。 */
export async function loadSyncStatus(repoId: number): Promise<SyncStatus> {
  const branches: readonly Branch[] = await gitBranchList(repoId);
  const head = branches.find((branch) => branch.isHead);
  if (head === undefined) {
    return detachedStatus(null);
  }
  // `upstreamGone` 与"没有上游"必须区分：前者要提示"上游没了"，而它同样不能
  // 拿去算 ahead/behind（远端分支已经不存在，比较只会失败）。
  if (head.upstream === null || head.upstreamGone) {
    return detachedStatus(head.name);
  }

  const remote = remoteOf(head.upstream);
  if (head.ahead !== null && head.behind !== null) {
    return {
      branch: head.name,
      upstream: head.upstream,
      remote,
      ahead: head.ahead,
      behind: head.behind,
    };
  }

  const comparison = await gitBranchCompare(repoId, head.name, head.upstream);
  return {
    branch: head.name,
    upstream: head.upstream,
    remote,
    ahead: comparison.ahead,
    behind: comparison.behind,
  };
}

/** 订阅某个仓库的同步状态（`repo:changed` 会按 refs 失效它）。 */
export function useSyncStatus(repoId: number) {
  return useQuery({
    queryKey: syncStatusKey(repoId),
    queryFn: () => loadSyncStatus(repoId),
    enabled: Number.isFinite(repoId),
    staleTime: 5_000,
  });
}
