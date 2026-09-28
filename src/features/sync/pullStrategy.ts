/**
 * 拉取策略的持久化（T2.6）。
 *
 * # 为什么存在**仓库级设置**里
 *
 * 策略是"这个仓库的习惯"：团队要求变基的仓库和只允许快进的仓库不该互相影响。
 * 机制与历史筛选（`history.filters`）完全一致——`settings_get` / `settings_set`
 * （scope=`repo`），值是一个 JSON 字符串，后端不理解它的含义。
 *
 * # 为什么缺省是 `fastForwardOnly`
 *
 * 三条策略里只有它**不会**在用户不知情时产生新提交或改写历史：
 * 快进不了就报错，用户再决定是合并还是变基。默认值应该是"最不可能让人后悔"的那个。
 */
import type { PullStrategy } from '@/lib/ipc/sync';

/** 仓库级设置里存拉取策略的键。 */
export const PULL_STRATEGY_SETTING_KEY = 'sync.pullStrategy';

/** 可选策略（顺序即下拉里的展示顺序：由安全到激进）。 */
export const PULL_STRATEGIES = ['fastForwardOnly', 'merge', 'rebase'] as const;

/** 缺省策略。 */
export const DEFAULT_PULL_STRATEGY: PullStrategy = 'fastForwardOnly';

/** 是否为合法的策略取值。 */
export function isPullStrategy(value: unknown): value is PullStrategy {
  return typeof value === 'string' && (PULL_STRATEGIES as readonly string[]).includes(value);
}

/**
 * 设置里存的值 → 策略。
 *
 * 三种坏值都必须回退而不是抛错：`null`（还没存过）、空串、以及"存了个别的
 * 版本写进去的值"。一个被改坏的设置不该让同步条渲染不出来。
 */
export function parsePullStrategy(raw: string | null | undefined): PullStrategy {
  if (raw === null || raw === undefined || raw === '') {
    return DEFAULT_PULL_STRATEGY;
  }
  try {
    const parsed: unknown = JSON.parse(raw);
    return isPullStrategy(parsed) ? parsed : DEFAULT_PULL_STRATEGY;
  } catch {
    return DEFAULT_PULL_STRATEGY;
  }
}

/** 策略 → 设置里存的值（JSON 串，与后端只校验形状的约定一致）。 */
export function serializePullStrategy(strategy: PullStrategy): string {
  return JSON.stringify(strategy);
}
