/**
 * 限流状态（T4.10）：命令 DTO 与具名封装。
 *
 * # 镜像与来源
 *
 * `RateLimitSnapshot` 镜像 `crates/provider/src/rate_limit.rs` 的
 * `RateLimitState`。快照由 HTTP 底座逐响应捕获（本地读取，零网络成本）；
 * `repoRateLimitRefresh` 走 `GET /rate_limit` 主动刷新（不耗配额）。
 *
 * # 降级是后端的事，横幅只管显示
 *
 * 触发限流时后端已自动回退 ETag 缓存（`etag_cache` 模块）；前端按
 * `remaining === 0` 显示"额度耗尽、正在展示缓存数据"的横幅与重置时间。
 */
import { invokeCommand } from './client';

/** 限流快照（HTTP 底座最近一次捕获的 `x-ratelimit-*` 头）。 */
export interface RateLimitSnapshot {
  /** 配额桶名（`core` / `graphql` / `search`…）。 */
  readonly resource?: string | null;
  /** 配额上限（如 5000）。 */
  readonly limit: number;
  /** 剩余额度。 */
  readonly remaining: number;
  /** 当前窗口已用额度。 */
  readonly used: number;
  /** 配额恢复时间（UNIX 秒）。 */
  readonly resetUnixSecs: number;
}

/** 最近一次限流快照（会话还没发过请求时为 null）。 */
export function repoRateLimitState(): Promise<RateLimitSnapshot | null> {
  return invokeCommand<RateLimitSnapshot | null>('repo_rate_limit_state', {});
}

/** 主动刷新限流额度（GET /rate_limit，不耗配额）。 */
export function repoRateLimitRefresh(host: string, repoId?: number): Promise<RateLimitSnapshot> {
  return invokeCommand<RateLimitSnapshot>('repo_rate_limit_refresh', {
    host,
    ...(repoId === undefined ? {} : { repoId }),
  });
}
