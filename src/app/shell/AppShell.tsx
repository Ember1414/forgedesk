import { useTranslation } from 'react-i18next';
import { Outlet } from 'react-router-dom';

import { SideNav } from '@/app/shell/SideNav';
import { StatusBar } from '@/app/shell/StatusBar';
import { TitleBar } from '@/app/shell/TitleBar';
import { useIsNarrowViewport } from '@/app/shell/useIsNarrowViewport';
import { useUiStore } from '@/stores/uiStore';

/**
 * 应用外壳：顶栏 / 左侧导航 / 主内容区 / 状态栏。
 *
 * 布局决策（原创，未参考任何竞品）：
 *   「固定顶栏 + 固定侧栏 + 唯一滚动区」的经典三明治结构，
 *   但滚动只发生在主内容区（`min-h-0 + overflow-auto`），
 *   这样顶栏、侧栏、状态栏永远可见——桌面工具里它们承载的是持续性的上下文，
 *   随内容滚走会让用户反复"回到顶部"确认自己在哪个仓库。
 *
 * 折叠逻辑：最终折叠 = 用户手工折叠 ‖ 窗口过窄。
 *   两者分离存储（autoCollapsed 只由尺寸推导），因此窗口拉宽后能恢复用户的选择，
 *   而不是把"尺寸导致的折叠"误存成用户的偏好。
 *
 * 键盘可达性：
 *   - 页面首个可聚焦元素是"跳到主内容"链接（屏幕阅读器/键盘用户可跳过整条导航）；
 *   - 主内容区带 tabIndex={-1}，跳转后焦点能落到这里。
 */
export function AppShell() {
  const { t } = useTranslation('shell');
  const collapsedByUser = useUiStore((state) => state.sidebarCollapsed);
  const autoCollapsed = useIsNarrowViewport();
  const collapsed = collapsedByUser || autoCollapsed;

  return (
    <div className="flex h-full flex-col bg-canvas text-fg">
      <a
        href="#main-content"
        className="sr-only focus:not-sr-only focus:absolute focus:left-3 focus:top-3 focus:z-50 focus:rounded-sm focus:bg-surface-raised focus:px-3 focus:py-2 focus:text-13 focus:shadow-lg"
      >
        {t('nav.skipToContent')}
      </a>

      <TitleBar />

      <div className="flex min-h-0 flex-1">
        <SideNav collapsed={collapsed} autoCollapsed={autoCollapsed} />
        <main id="main-content" tabIndex={-1} className="min-w-0 flex-1 overflow-auto p-4">
          <Outlet />
        </main>
      </div>

      <StatusBar />
    </div>
  );
}
