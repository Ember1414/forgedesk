import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { render } from '@testing-library/react';
import { RouterProvider, createMemoryRouter } from 'react-router-dom';

import { appRoutes } from '@/app/routes';

/**
 * 测试用渲染函数：用真实路由表 + 内存路由渲染整个应用。
 *
 * 为什么不用 HashRouter：jsdom 里的 hash 操作要触发 history 变化比较绕，
 * 而 `createMemoryRouter` 接受初始路径、且用的是**同一份** appRoutes，
 * 因此测试验证的仍然是真实的路由结构（不会出现"测试通过但应用打不开"）。
 *
 * 为什么在这里提供 QueryClientProvider：真实入口（src/app/App.tsx）就是这样的层级，
 * 少一层会让用到查询的页面直接抛错——那种失败是"测试环境不同"而不是产品缺陷。
 * 每次渲染新建 QueryClient，避免用例之间共享缓存（否则前一个用例的数据会污染后一个）。
 */
export function renderApp(initialPath = '/') {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  const router = createMemoryRouter(appRoutes, { initialEntries: [initialPath] });

  return {
    router,
    queryClient,
    ...render(
      <QueryClientProvider client={queryClient}>
        <RouterProvider router={router} />
      </QueryClientProvider>,
    ),
  };
}
