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
 * 页面全部为 M0 骨架，除「外壳导航」与「外观设置」外不实现功能。
 */
import { Navigate, createHashRouter } from 'react-router-dom';
import type { RouteObject } from 'react-router-dom';

import { AppShell } from '@/app/shell/AppShell';
import { NotFoundPage } from '@/app/shell/NotFoundPage';
import { CommitPage } from '@/features/commit/CommitPage';
import { DashboardPage } from '@/features/dashboard/DashboardPage';
import { SnapshotsPage } from '@/features/snapshots/SnapshotsPage';
import { GitHubActionsPage } from '@/features/github/GitHubActionsPage';
import { GitHubIssuesPage } from '@/features/github/GitHubIssuesPage';
import { GitHubLayout } from '@/features/github/GitHubLayout';
import { GitHubPullRequestsPage } from '@/features/github/GitHubPullRequestsPage';
import { GitHubReposPage } from '@/features/github/GitHubReposPage';
import { PluginsPage } from '@/features/plugins/PluginsPage';
import { RepoBranchesPage } from '@/features/repo/RepoBranchesPage';
import { RepoConflictPage } from '@/features/repo/RepoConflictPage';
import { RepoHistoryPage } from '@/features/repo/RepoHistoryPage';
import { RepoLayout } from '@/features/repo/RepoLayout';
import { RepoStatusPage } from '@/features/repo/RepoStatusPage';
import { RepoTerminalPage } from '@/features/repo/RepoTerminalPage';
import { AdvancedSettingsPage } from '@/features/settings/AdvancedSettingsPage';
import { AppearanceSettingsPage } from '@/features/settings/AppearanceSettingsPage';
import { GeneralSettingsPage } from '@/features/settings/GeneralSettingsPage';
import { GitSettingsPage } from '@/features/settings/GitSettingsPage';
import { GitHubSettingsPage } from '@/features/settings/GitHubSettingsPage';
import { SettingsLayout } from '@/features/settings/SettingsLayout';
import { ComponentsPage } from '@/ui/__dev__/ComponentsPage';
import { DesignSystemPage } from '@/ui/__dev__/DesignSystemPage';

/** 开发专用路由（生产构建不包含这两个预览页）。 */
const devRoutes: RouteObject[] = import.meta.env.DEV
  ? [
      { path: '__dev__/design', element: <DesignSystemPage /> },
      { path: '__dev__/components', element: <ComponentsPage /> },
    ]
  : [];

export const appRoutes: RouteObject[] = [
  {
    path: '/',
    element: <AppShell />,
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
          { path: 'history', element: <RepoHistoryPage /> },
          { path: 'branches', element: <RepoBranchesPage /> },
          { path: 'conflict', element: <RepoConflictPage /> },
          { path: 'terminal', element: <RepoTerminalPage /> },
        ],
      },

      {
        path: 'github',
        element: <GitHubLayout />,
        children: [
          { index: true, element: <Navigate to="repos" replace /> },
          { path: 'repos', element: <GitHubReposPage /> },
          { path: 'pull-requests', element: <GitHubPullRequestsPage /> },
          { path: 'issues', element: <GitHubIssuesPage /> },
          { path: 'actions', element: <GitHubActionsPage /> },
        ],
      },

      {
        path: 'settings',
        element: <SettingsLayout />,
        children: [
          { index: true, element: <Navigate to="general" replace /> },
          { path: 'general', element: <GeneralSettingsPage /> },
          { path: 'appearance', element: <AppearanceSettingsPage /> },
          { path: 'git', element: <GitSettingsPage /> },
          { path: 'github', element: <GitHubSettingsPage /> },
          { path: 'advanced', element: <AdvancedSettingsPage /> },
        ],
      },

      { path: 'plugins', element: <PluginsPage /> },

      ...devRoutes,

      { path: '*', element: <NotFoundPage /> },
    ],
  },
];

/** 应用实际使用的路由实例（hash 路由，见文件头说明）。 */
export const router = createHashRouter(appRoutes);
