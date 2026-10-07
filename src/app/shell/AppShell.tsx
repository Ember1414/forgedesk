import { useEffect } from 'react';
import { Outlet, useLocation } from 'react-router-dom';
import { useTranslation } from 'react-i18next';

import { setWindowTitle } from '@/lib/ipc';
import { NAV_SECTIONS } from '@/app/shell/navItems';
import { SideNav } from '@/app/shell/SideNav';
import { StatusBar } from '@/app/shell/StatusBar';
import { TitleBar } from '@/app/shell/TitleBar';
import { useIsNarrowViewport } from '@/app/shell/useIsNarrowViewport';
import { ShortcutManagerWithPalette } from '@/features/commands/ShortcutManager';
import { StartupRecoveryNotice } from '@/features/system/StartupRecoveryNotice';
import { UpdateBanner } from '@/features/system/UpdateBanner';
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
  const location = useLocation();
  const { t, i18n } = useTranslation('shell');
  const collapsedByUser = useUiStore((state) => state.sidebarCollapsed);
  const autoCollapsed = useIsNarrowViewport();
  const collapsed = collapsedByUser || autoCollapsed;

  // 窗口标题跟随语言与页面（T6.7）。结构 = "ForgeDesk — 页面名"；
  // 应用名不翻译（R4：产品名固定）。失败静默（标题不更新不影响使用）。
  const pageLabel = (() => {
    for (const section of NAV_SECTIONS) {
      for (const item of section.items) {
        const path = item.repoScoped ? `/repo/:repoId/${item.segment}` : `/${item.segment}`;
        // 列表项与当前路径的前缀匹配（仓库级路由的真实 id 会替换 :repoId）
        const pattern = new RegExp(`^${path.replace(':repoId', '[0-9]+')}(/|$)`);
        if (item.segment.length > 0 && pattern.test(location.pathname)) {
          return t(item.labelKey);
        }
        if (item.segment.length === 0 && location.pathname === '/') {
          return t(item.labelKey);
        }
      }
    }
    return null;
  })();
  useEffect(() => {
    const title = pageLabel === null ? 'ForgeDesk' : `ForgeDesk — ${pageLabel}`;
    setWindowTitle(title).catch(() => undefined);
  }, [pageLabel, i18n.resolvedLanguage]);

  return (
    <div className="flex h-full flex-col bg-canvas text-fg">
      <a
        href="#main-content"
        className="sr-only focus:not-sr-only focus:absolute focus:left-3 focus:top-3 focus:z-50 focus:rounded-sm focus:bg-surface-raised focus:px-3 focus:py-2 focus:text-13 focus:shadow-lg"
      >
        {t('nav.skipToContent')}
      </a>

      <TitleBar />

      {/* T7.5：崩溃恢复提示（模态，仅异常退出时）与安全模式常驻横幅 */}
      <StartupRecoveryNotice />

      {/* T7.1：有新版本时的提示横幅（未配置更新源或无更新时不渲染） */}
      <UpdateBanner />

      <div className="flex min-h-0 flex-1">
        <SideNav collapsed={collapsed} autoCollapsed={autoCollapsed} />
        {/* 内边距用密度变量（见 src/styles/index.css）：设置页里切换"界面密度"会立刻生效 */}
        <main
          id="main-content"
          tabIndex={-1}
          className="min-w-0 flex-1 overflow-auto p-[var(--fd-content-pad)]"
        >
          <Outlet />
        </main>
      </div>

      {/* T5.9：命令面板 + 快捷键分发（渲染 null；编辑器页时 editorActive 生效） */}
      <ShortcutManagerWithPalette editorActive={location.pathname.includes('/editor')} />
      <StatusBar />
    </div>
  );
}
