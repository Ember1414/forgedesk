/**
 * 测试用的 TanStack Query 客户端。
 *
 * 与生产配置的两点区别（都是为了测试可控）：
 *   - `retry: false`：失败态立刻可见，不必等重试拖慢用例；
 *   - `gcTime: Infinity`：组件卸载后缓存不立即回收，避免"卸载后清理"引发的
 *     时序不确定（表现为随机的 act 警告）。
 */
import { QueryClient } from '@tanstack/react-query';

export function createTestQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: {
        retry: false,
        gcTime: Number.POSITIVE_INFINITY,
        staleTime: 0,
      },
      mutations: {
        retry: false,
      },
    },
  });
}
