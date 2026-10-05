/**
 * Dashboard 聚合（T4.11）：命令 DTO 与具名封装。
 *
 * # 镜像与来源
 *
 * 镜像 `crates/services/src/host_repos.rs` 的 `RepoDashboard` 三件套
 * （docs/API.md「Dashboard 聚合」节）。单仓库失败降级：`pulls`/`runs`
 * 为 `null` 并在 `errors` 里带说明——UI 对 null 渲染"获取失败"而不是
 * 整页报错。
 */
import { invokeCommand } from './client';

/** 单仓库的 open PR 摘要。 */
export interface PullsDigest {
  /** open PR 数（首页 100 条内的计数）。 */
  readonly openTotal: number;
  /** 超过 100 条被截断。 */
  readonly openTruncated: boolean;
  /** 被请求审查且对象含当前账号的 open PR 数。 */
  readonly awaitingReview: number;
}

/** 单仓库最近一次 workflow run。 */
export interface RunDigest {
  readonly name: string;
  /** `queued` / `in_progress` / `completed`。 */
  readonly status: string;
  readonly conclusion: string | null;
}

/** 单仓库的聚合摘要（部分失败降级）。 */
export interface RepoDashboard {
  readonly owner: string;
  readonly repo: string;
  readonly pulls: PullsDigest | null;
  readonly runs: RunDigest | null;
  /** 降级说明（开发者读，英文）。 */
  readonly errors: readonly string[];
}

/** 聚合目标上限（后端同款校验，前端先截断）。 */
export const MAX_DASHBOARD_TARGETS = 10;

/** 多仓库聚合视图。 */
export function repoDashboard(request: {
  readonly host: string;
  readonly targets: readonly { owner: string; repo: string }[];
  readonly repoId?: number;
}): Promise<{ readonly repos: readonly RepoDashboard[] }> {
  return invokeCommand<{ readonly repos: readonly RepoDashboard[] }>('repo_dashboard', request);
}
