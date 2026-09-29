/**
 * 大仓库性能模式（T2.9）。
 *
 * # 它解决什么
 *
 * 万级变更文件的仓库里，字符级 diff 高亮、大上下文行数、大页历史都会把
 * 渲染与 IPC 拖进秒级。性能模式在"仓库确实大"时自动收紧这些开关：
 *
 *   - diff 视图强制关闭字符级高亮、上下文行数压到 1；
 *   - 历史图每页行数减半（200 → 100），首屏更快、滚动更跟手。
 *
 * # 三档设置
 *
 * `auto`（默认）按"状态条目数 ≥ 阈值"判定；`on` 强制开（低端机/网络盘）；
 * `off` 强制关（用户接受等待换细节）。判定只依赖前端已有的状态查询数据，
 * 不加 IPC——状态数据什么时候到，性能模式就什么时候生效。
 */
import { useMemo, useSyncExternalStore } from 'react';

import { useQueryClient } from '@tanstack/react-query';

import { statusKey } from '@/lib/queryKeys';
import type { WorkspaceStatus } from '@/lib/ipc/workspace';

import { useSettingsStore } from '@/stores/settingsStore';

/** 性能模式的设置键（SQLite 设置表；前端自管，后端不理解语义）。 */
export const PERFORMANCE_MODE_KEY = 'history.performanceMode';

/** 性能模式的三档取值。 */
export const PERFORMANCE_MODES = ['auto', 'on', 'off'] as const;
export type PerformanceMode = (typeof PERFORMANCE_MODES)[number];

/** 缺省档位。 */
export const DEFAULT_PERFORMANCE_MODE: PerformanceMode = 'auto';

/**
 * 触发 `auto` 档的状态条目数阈值。
 *
 * 实测参照（docs/PERF-BASELINE.md §3）：1 万条已跟踪修改的状态计算是秒级，
 * 2 千条时各路径仍在百毫秒内——阈值取在"体验开始可感知变慢"的位置。
 */
export const PERFORMANCE_MODE_ENTRY_THRESHOLD = 2_000;

/** 性能模式下 diff 的上下文行数（普通模式缺省 3）。 */
export const PERFORMANCE_CONTEXT_LINES = 1;

/** 性能模式下历史图的每页行数（普通模式 200）。 */
export const PERFORMANCE_PAGE_SIZE = 100;

/** 从存储值解析档位；坏值回落 `auto`。 */
export function parsePerformanceMode(value: string | null | undefined): PerformanceMode {
  if (value === null || value === undefined) {
    return DEFAULT_PERFORMANCE_MODE;
  }
  try {
    const parsed: unknown = JSON.parse(value);
    return typeof parsed === 'string' && (PERFORMANCE_MODES as readonly string[]).includes(parsed)
      ? (parsed as PerformanceMode)
      : DEFAULT_PERFORMANCE_MODE;
  } catch {
    return DEFAULT_PERFORMANCE_MODE;
  }
}

/** 状态条目数：判定用的口径 = 暂存 + 未暂存 + 未跟踪 + 冲突。 */
export function statusEntryCount(status: WorkspaceStatus | undefined | null): number {
  if (!status) {
    return 0;
  }
  return (
    status.staged.length +
    status.unstaged.length +
    status.untracked.length +
    status.conflicted.length
  );
}

/** 纯判定函数：给定档位与条目数，性能模式是否生效。 */
export function performanceModeEnabled(mode: PerformanceMode, entryCount: number): boolean {
  switch (mode) {
    case 'on':
      return true;
    case 'off':
      return false;
    case 'auto':
      return entryCount >= PERFORMANCE_MODE_ENTRY_THRESHOLD;
  }
}

/**
 * 观察性能模式是否生效。
 *
 * 档位来自设置 store（响应式）；条目数来自**查询缓存里已有的**状态数据——
 * 状态查询的真相源在状态页，这里只是搭车观察（`useSyncExternalStore` 订阅
 * 查询缓存：不发起请求，只随缓存更新重算快照），多发一次请求就违背了
 * "性能"模式的初衷。缓存里还没有状态数据时条目数为 0，auto 档不生效。
 */
export function usePerformanceMode(repoId: number): boolean {
  const mode = useSettingsStore((state) =>
    parsePerformanceMode(state.values[PERFORMANCE_MODE_KEY]),
  );
  const queryClient = useQueryClient();

  const subscribe = useMemo(
    () => (onChange: () => void) => queryClient.getQueryCache().subscribe(onChange),
    [queryClient],
  );
  const getSnapshot = useMemo(
    () => (): number => {
      if (!Number.isFinite(repoId)) {
        return 0;
      }
      const status = queryClient.getQueryData<WorkspaceStatus>(statusKey(repoId));
      return statusEntryCount(status);
    },
    [queryClient, repoId],
  );
  const entryCount = useSyncExternalStore(subscribe, getSnapshot, () => 0);

  return performanceModeEnabled(mode, entryCount);
}
