/**
 * 路由表（单一真相源）。
 *
 * 路由库选择：**react-router 7**（已在 T0.1 作为依赖引入）。
 *   为什么不用文件路由（TanStack Router）：桌面应用的页面数量少且稳定（当前 17 条），
 *   文件路由带来的代码生成/插件配置与"路由树也是真相源"的双重来源，
 *   在 M0 阶段的收益低于复杂度成本。后续若要引入类型安全路由，本文件是唯一改动点。
 *
 * 为什么用 HashRouter 而不是 BrowserRouter：
 *   生产环境下前端资源由 Tauri 的自定义协议提供，深链（形如 `/repo/x/status`）
 *   在页面刷新时能否回落到 index.html 取决于宿主实现；而桌面应用**没有**SEO 与
 *   分享链接需求，hash 路由把这个不确定性直接消除（刷新永远回到 index.html）。
 *
 * 工作区等常用入口同步装配；终端、编辑器、托管平台和插件在访问时加载，
 * 避免尚未打开的功能及其依赖进入首屏。路由 lazy 同时覆盖直接进入与站内导航。
 */
import { useTranslation } from 'react-i18next';
import { Navigate, createHashRouter } from 'react-router-dom';
import type { RouteObject } from 'react-router-dom';

import { AppShell } from '@/app/shell/AppShell';
import { NotFoundPage } from '@/app/shell/NotFoundPage';
import { CommitPage } from '@/features/commit/CommitPage';
import { DashboardPage } from '@/features/dashboard/DashboardPage';
import { OperationsPage } from '@/features/operations/OperationsPage';
import { SnapshotsPage } from '@/features/snapshots/SnapshotsPage';
import { RepoSettingsPage } from '@/features/repo/RepoSettingsPage';
import { GitHubLayout } from '@/features/github/GitHubLayout';
import { RepoBranchesPage } from '@/features/repo/RepoBranchesPage';
import { RepoConflictPage } from '@/features/repo/RepoConflictPage';
import { RepoHistoryPage } from '@/features/repo/RepoHistoryPage';
import { RepoLayout } from '@/features/repo/RepoLayout';
import { RepoStatusPage } from '@/features/repo/RepoStatusPage';
import { AdvancedSettingsPage } from '@/features/settings/AdvancedSettingsPage';
import { AppearanceSettingsPage } from '@/features/settings/AppearanceSettingsPage';
import { GeneralSettingsPage } from '@/features/settings/GeneralSettingsPage';
import { GitHubSettingsPage } from '@/features/settings/GitHubSettingsPage';
import { NetworkSettingsPage } from '@/features/settings/NetworkSettingsPage';
import { PluginSettingsPage } from '@/features/settings/PluginSettingsPage';
import { PrivacySettingsPage } from '@/features/settings/PrivacySettingsPage';
import { SettingsLayout } from '@/features/settings/SettingsLayout';
import { LayoutSettingsPage } from '@/features/settings/LayoutSettingsPage';
import { ShortcutsSettingsPage } from '@/features/settings/ShortcutsSettingsPage';
import { TerminalSettingsPage } from '@/features/settings/TerminalSettingsPage';

/** 直接进入延迟路由时仍提供明确的加载反馈（不能留下空白窗口）。 */
function RouteLoading() {
  const { t } = useTranslation('common');
  return (
    <p role="status" className="p-4 text-13 text-fg-muted">
      {t('state.loading')}
    </p>
  );
}

/** 开发专用路由（生产构建不包含这几个预览页）。 */
const devRoutes: RouteObject[] = import.meta.env.DEV
  ? [
      {
        path: '__dev__/design',
        lazy: async () => ({
          Component: (await import('@/ui/__dev__/DesignSystemPage')).DesignSystemPage,
        }),
      },
      {
        path: '__dev__/components',
        lazy: async () => ({
          Component: (await import('@/ui/__dev__/ComponentsPage')).ComponentsPage,
        }),
      },
      {
        path: '__dev__/graph-perf',
        lazy: async () => ({
          Component: (await import('@/ui/__dev__/GraphPerfPanel')).GraphPerfRoute,
        }),
      },
      {
        path: '__dev__/pty-spike',
        lazy: async () => ({
          Component: (await import('@/ui/__dev__/PtySpikePanel')).PtySpikePanel,
        }),
      },
    ]
  : [];

export const appRoutes: RouteObject[] = [
  {
    path: '/',
    element: <AppShell />,
    hydrateFallbackElement: <RouteLoading />,
    children: [
      { index: true, element: <DashboardPage /> },

      {
        path: 'repo/:repoId',
        element: <RepoLayout />,
        children: [
          { index: true, element: <Navigate to="status" replace /> },
          { path: 'status', element: <RepoStatusPage /> },
          { path: 'commit', element: <CommitPage /> },
          { path: 'snapshots', element: <SnapshotsPage /> },
          { path: 'operations', element: <OperationsPage /> },
          { path: 'history', element: <RepoHistoryPage /> },
          { path: 'branches', element: <RepoBranchesPage /> },
          { path: 'conflict', element: <RepoConflictPage /> },
          {
            path: 'terminal',
            lazy: async () => ({
              Component: (await import('@/features/repo/RepoTerminalPage')).RepoTerminalPage,
            }),
          },
          {
            path: 'editor',
            lazy: async () => ({
              Component: (await import('@/features/editor/RepoEditorPage')).RepoEditorPage,
            }),
          },
          {
            path: 'plugin-panels',
            lazy: async () => ({
              Component: (await import('@/features/plugins/PluginPanelsPage')).PluginPanelsPage,
            }),
          },
          { path: 'settings', element: <RepoSettingsPage /> },
        ],
      },

      {
        path: 'github',
        element: <GitHubLayout />,
        children: [
          { index: true, element: <Navigate to="repos" replace /> },
          {
            path: 'repos',
            lazy: async () => ({
              Component: (await import('@/features/github/GitHubReposPage')).GitHubReposPage,
            }),
          },
          {
            path: 'pull-requests',
            lazy: async () => ({
              Component: (await import('@/features/github/GitHubPullRequestsPage'))
                .GitHubPullRequestsPage,
            }),
          },
          {
            path: 'issues',
            lazy: async () => ({
              Component: (await import('@/features/github/GitHubIssuesPage')).GitHubIssuesPage,
            }),
          },
          {
            path: 'actions',
            lazy: async () => ({
              Component: (await import('@/features/github/GitHubActionsPage')).GitHubActionsPage,
            }),
          },
          {
            path: 'dashboard',
            lazy: async () => ({
              Component: (await import('@/features/github/GitHubDashboardPage'))
                .GitHubDashboardPage,
            }),
          },
        ],
      },

      {
        path: 'settings',
        element: <SettingsLayout />,
        children: [
          { index: true, element: <Navigate to="general" replace /> },
          { path: 'general', element: <GeneralSettingsPage /> },
          { path: 'appearance', element: <AppearanceSettingsPage /> },
          { path: 'plugins', element: <PluginSettingsPage /> },
          { path: 'network', element: <NetworkSettingsPage /> },
          { path: 'github', element: <GitHubSettingsPage /> },
          { path: 'terminal', element: <TerminalSettingsPage /> },
          { path: 'layout', element: <LayoutSettingsPage /> },
          { path: 'shortcuts', element: <ShortcutsSettingsPage /> },
          { path: 'advanced', element: <AdvancedSettingsPage /> },
          { path: 'privacy', element: <PrivacySettingsPage /> },
        ],
      },

      {
        path: 'plugins',
        lazy: async () => ({
          Component: (await import('@/features/plugins/PluginsPage')).PluginsPage,
        }),
      },

      // T5.4 命令字典（全局，不依赖仓库）
      {
        path: 'commands',
        lazy: async () => ({
          Component: (await import('@/features/terminal/CommandDictionaryPage'))
            .CommandDictionaryPage,
        }),
      },

      ...devRoutes,

      { path: '*', element: <NotFoundPage /> },
    ],
  },
];

/** 应用实际使用的路由实例（hash 路由，见文件头说明）。 */
export const router = createHashRouter(appRoutes);
