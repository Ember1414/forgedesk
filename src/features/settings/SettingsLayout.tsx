import { useTranslation } from 'react-i18next';
import { NavLink, Outlet } from 'react-router-dom';

import { cn } from '@/lib/utils';

/**
 * 设置区域的公共外壳：左侧分类 + 右侧内容。
 *
 * 为什么设置用「二级侧栏」而不是顶栏标签：设置项会随里程碑持续增长
 * （Git、账号、插件、诊断…），纵向列表比横向标签更耐扩展，也不会挤到标题栏。
 */
const SETTINGS_SECTIONS = [
  { segment: 'general', labelKey: 'pages.settingsGeneral.title' },
  { segment: 'appearance', labelKey: 'pages.settingsAppearance.title' },
  { segment: 'git', labelKey: 'pages.settingsGit.title' },
  { segment: 'github', labelKey: 'pages.settingsGithub.title' },
  { segment: 'terminal', labelKey: 'pages.settingsTerminal.title' },
  { segment: 'shortcuts', labelKey: 'pages.settingsShortcuts.title' },
  { segment: 'layout', labelKey: 'pages.settingsLayout.title' },
  { segment: 'plugins', labelKey: 'pages.settingsPlugins.title' },
  { segment: 'advanced', labelKey: 'pages.settingsAdvanced.title' },
] as const;

export function SettingsLayout() {
  const { t } = useTranslation('shell');

  return (
    <div className="flex h-full gap-4">
      <nav aria-label={t('tabs.settings')} className="w-40 shrink-0">
        <ul className="flex flex-col gap-0.5">
          {SETTINGS_SECTIONS.map((section) => (
            <li key={section.segment}>
              <NavLink
                to={section.segment}
                className={({ isActive }) =>
                  cn(
                    'fd-transition block rounded-md px-2.5 py-1.5 text-13',
                    isActive
                      ? 'bg-brand-subtle font-medium text-brand'
                      : 'text-fg-muted hover:bg-surface-sunken hover:text-fg',
                  )
                }
              >
                {t(section.labelKey)}
              </NavLink>
            </li>
          ))}
        </ul>
      </nav>

      <div className="min-h-0 min-w-0 flex-1 overflow-auto">
        <Outlet />
      </div>
    </div>
  );
}
