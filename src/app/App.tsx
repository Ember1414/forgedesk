import { useEffect } from 'react';

import { QueryClientProvider } from '@tanstack/react-query';
import { RouterProvider } from 'react-router-dom';

import { router } from '@/app/routes';
import { queryClient } from '@/app/queryClient';
import { applyThemeMode, watchSystemTheme } from '@/app/theme';
import { useUiStore } from '@/stores/uiStore';

/**
 * 应用根组件。
 *
 * 结构：QueryClientProvider（服务端状态）→ SystemThemeSync（跟随系统外观）→ RouterProvider。
 * 注意 QueryClientProvider 必须在路由之外：所有页面都会用到查询，
 * 挂在路由内部会导致切换页面时缓存被重建。
 */
function SystemThemeSync() {
  const themeMode = useUiStore((state) => state.themeMode);

  useEffect(() => {
    if (themeMode !== 'system') {
      return undefined;
    }
    // 只在"跟随系统"时订阅：用户显式选定 light/dark 后系统变化不应该覆盖用户选择
    return watchSystemTheme(() => {
      applyThemeMode('system');
    });
  }, [themeMode]);

  return null;
}

export function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <SystemThemeSync />
      <RouterProvider router={router} />
    </QueryClientProvider>
  );
}
