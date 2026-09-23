import { QueryClientProvider } from '@tanstack/react-query';

import { queryClient } from '@/app/queryClient';
import { DesignSystemPage } from '@/ui/__dev__/DesignSystemPage';

/**
 * 应用根组件（M0 阶段临时实现）。
 *
 * T0.3 阶段渲染设计系统预览页，用于可视化验收 token 体系与 IPC 通路。
 * T0.4 会替换为完整外壳（顶部栏 / 侧栏导航 / 主内容区 / 状态栏）+ 路由，
 * 届时设计系统页会迁移到仅开发环境可用的路由（如 `/__dev__/design`）。
 *
 * 注意：`QueryClientProvider` 从 M0 起就挂在根部，后续所有页面都依赖它。
 */
export function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <DesignSystemPage />
    </QueryClientProvider>
  );
}
