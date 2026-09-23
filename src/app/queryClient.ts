import { QueryClient } from '@tanstack/react-query';

/**
 * 全局 QueryClient。
 *
 * 状态归属约定（AGENTS.md §6「单一真相源」）：
 *   - **服务端状态**（Git 状态、提交历史、GitHub API 数据…）一律走 TanStack Query。
 *   - **客户端 UI 状态**（面板开合、选中项、编辑器标签…）走 Zustand。
 *   禁止把 Git 状态放进 Zustand，否则会出现双份真相与不一致。
 *
 * 默认值说明：
 *   - `staleTime` 30s：Git 操作后的新鲜度由后端事件（`repo:changed`）驱动失效，
 *     因此不需要极短的 staleTime 去轮询。
 *   - `refetchOnWindowFocus: false`：桌面应用切窗口频繁，重新拉取只会造成无谓 IO。
 *   - `retry: 1`：IPC 失败通常是确定性的（参数/状态问题），重试意义有限。
 */
export const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 30_000,
      gcTime: 5 * 60_000,
      retry: 1,
      refetchOnWindowFocus: false,
    },
    mutations: {
      retry: 0,
    },
  },
});
