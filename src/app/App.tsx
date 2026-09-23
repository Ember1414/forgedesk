import { useEffect } from 'react';

import { QueryClientProvider } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { RouterProvider } from 'react-router-dom';

import { router } from '@/app/routes';
import { queryClient } from '@/app/queryClient';
import { applyThemeMode, watchSystemTheme } from '@/app/theme';
import { Toaster } from '@/ui/components/toast';
import { TooltipProvider } from '@/ui/components/tooltip';
import { useSettingsStore } from '@/stores/settingsStore';
import { useUiStore } from '@/stores/uiStore';

/**
 * 应用根组件。
 *
 * 层级（自上而下，每一层的职责不同）：
 *   QueryClientProvider —— 服务端状态缓存，必须在路由之外，否则切页会重建缓存；
 *   TooltipProvider     —— Radix 用它统一管理提示的延迟与全局行为；
 *   SystemThemeSync     —— 跟随系统外观时的运行时同步；
 *   RouterProvider      —— 页面；
 *   Toaster             —— 全局提示出口，放最后以便浮在最上层。
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
  const { t } = useTranslation('common');

  useEffect(() => {
    // 启动时拉一次设置：界面密度这类"用起来就该已经生效"的项必须在首屏就应用，
    // 而不是等用户进设置页才生效。失败会被 store 记录，由设置页负责展示。
    void useSettingsStore.getState().load();
  }, []);

  return (
    <QueryClientProvider client={queryClient}>
      <TooltipProvider delayDuration={300}>
        <SystemThemeSync />
        <RouterProvider router={router} />
        <Toaster closeLabel={t('actions.dismiss')} />
      </TooltipProvider>
    </QueryClientProvider>
  );
}
