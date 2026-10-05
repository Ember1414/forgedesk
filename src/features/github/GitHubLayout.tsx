import { useTranslation } from 'react-i18next';
import { NavLink, Outlet } from 'react-router-dom';

import { RateLimitBanner } from '@/features/github/RateLimitBanner';
import { cn } from '@/lib/utils';

/**
 * 代码托管（GitHub）区域的公共外壳。
 *
 * 命名说明：界面文案统一用「代码托管」这一中性与原创的表述，
 * 避免把第三方品牌名做进产品导航（AGENTS.md 红线 R4：产品名与界面不背负他人品牌）。
 * 路由段落仍保留 `github`，因为它描述的是**接入的服务**，而非产品自身名称。
 */
const GITHUB_TABS = [
  { segment: 'dashboard', labelKey: 'pages.githubDashboard.title' },
  { segment: 'repos', labelKey: 'pages.githubRepos.title' },
  { segment: 'pull-requests', labelKey: 'pages.githubPullRequests.title' },
  { segment: 'issues', labelKey: 'pages.githubIssues.title' },
  { segment: 'actions', labelKey: 'pages.githubActions.title' },
] as const;

export function GitHubLayout() {
  const { t } = useTranslation('shell');

  return (
    <section className="flex h-full flex-col gap-3">
      <nav aria-label={t('tabs.github')} className="flex flex-wrap gap-1 border-b border-line pb-2">
        {GITHUB_TABS.map((tab) => (
          <NavLink
            key={tab.segment}
            to={tab.segment}
            className={({ isActive }) =>
              cn(
                'fd-transition rounded-md px-2.5 py-1 text-13',
                isActive
                  ? 'bg-brand-subtle font-medium text-brand'
                  : 'text-fg-muted hover:bg-surface-sunken hover:text-fg',
              )
            }
          >
            {t(tab.labelKey)}
          </NavLink>
        ))}
      </nav>

      {/* 限流横幅（T4.10）：额度耗尽时告知"正在展示缓存数据 + 重置时间"，
          覆盖 GitHub 区域的所有页面；额度充足时零占位 */}
      <RateLimitBanner />

      <div className="min-h-0 flex-1 overflow-auto">
        <Outlet />
      </div>
    </section>
  );
}
