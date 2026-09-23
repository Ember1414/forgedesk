import { PanelLeftClose, PanelLeftOpen } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { NavLink } from 'react-router-dom';

import { NAV_SECTIONS, navItemPath } from '@/app/shell/navItems';
import type { NavItem } from '@/app/shell/navItems';
import { cn } from '@/lib/utils';
import { useUiStore } from '@/stores/uiStore';

/**
 * 左侧主导航。
 *
 * 设计决策（原创）：
 *   导航按「仓库 / 集成 / 应用」三段分组，而不是一条平铺的列表——
 *   前两组是"围绕某个仓库干活"，最后一组是"设置这台机器"，混在一起会让
 *   用户在主任务与配置之间反复迷路。分组标题在折叠态下隐藏。
 *
 * 折叠态（图标模式）下用 `sr-only` 隐藏文字而不是直接移除：
 *   这样无障碍名称与测试查询在两种形态下保持一致，避免"折叠后找不到元素"的脆弱测试。
 *
 * 仓库级条目在未打开仓库时渲染为**原生 disabled 按钮**：
 *   它如实表达"现在不能点"，不会被 Tab 聚焦后却什么都不发生（那是最容易误导键盘用户的形态）。
 */
export interface SideNavProps {
  /** 最终是否折叠（用户选择 + 窗口过窄，由 AppShell 计算后传入）。 */
  readonly collapsed: boolean;
  /** 折叠是否由窗口过窄自动触发（用于给出解释性提示）。 */
  readonly autoCollapsed: boolean;
}

interface NavEntryProps {
  readonly item: NavItem;
  readonly collapsed: boolean;
  readonly repoId: string | null;
  readonly disabledHint: string;
}

function NavEntry({ item, collapsed, repoId, disabledHint }: NavEntryProps) {
  const { t } = useTranslation('shell');
  const path = navItemPath(item, repoId);
  const label = t(item.labelKey);
  const Icon = item.icon;

  const baseClassName = cn(
    'fd-transition flex h-8 w-full items-center gap-2 rounded-md px-2 text-13',
    collapsed && 'justify-center px-0',
  );
  const labelClassName = cn('truncate', collapsed && 'sr-only');

  if (path === null) {
    return (
      <li>
        <button
          type="button"
          disabled
          title={disabledHint}
          className={cn(baseClassName, 'cursor-not-allowed text-fg-subtle opacity-60')}
        >
          <Icon aria-hidden="true" className="size-4 shrink-0" />
          <span className={labelClassName}>{label}</span>
        </button>
      </li>
    );
  }

  return (
    <li>
      <NavLink
        to={path}
        end={item.segment === ''}
        title={collapsed ? label : undefined}
        className={({ isActive }) =>
          cn(
            baseClassName,
            isActive
              ? 'bg-brand-subtle font-medium text-brand'
              : 'text-fg-muted hover:bg-surface-sunken hover:text-fg',
          )
        }
      >
        <Icon aria-hidden="true" className="size-4 shrink-0" />
        <span className={labelClassName}>{label}</span>
      </NavLink>
    </li>
  );
}

export function SideNav({ collapsed, autoCollapsed }: SideNavProps) {
  const { t } = useTranslation('shell');
  const currentRepoId = useUiStore((state) => state.currentRepoId);
  const collapsedByUser = useUiStore((state) => state.sidebarCollapsed);
  const toggleSidebar = useUiStore((state) => state.toggleSidebar);

  const toggleTitle = collapsed
    ? autoCollapsed
      ? t('nav.narrowHint')
      : t('nav.expand')
    : t('nav.collapse');

  return (
    <nav
      aria-label={t('nav.ariaLabel')}
      className={cn(
        'flex shrink-0 flex-col border-r border-line bg-surface',
        collapsed ? 'w-12' : 'w-52',
      )}
    >
      <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto p-1.5">
        {NAV_SECTIONS.map((section) => (
          <div key={section.id} className="flex flex-col gap-0.5">
            <p
              className={cn(
                'px-2 pb-0.5 pt-1 text-12 font-medium uppercase tracking-wide text-fg-subtle',
                collapsed && 'sr-only',
              )}
            >
              {t(section.titleKey)}
            </p>
            <ul className="flex flex-col gap-0.5">
              {section.items.map((item) => (
                <NavEntry
                  key={item.id}
                  item={item}
                  collapsed={collapsed}
                  repoId={currentRepoId}
                  disabledHint={t('nav.requiresRepo')}
                />
              ))}
            </ul>
          </div>
        ))}
      </div>

      <div className="border-t border-line p-1.5">
        <button
          type="button"
          onClick={() => {
            toggleSidebar();
          }}
          // 自动折叠时仍允许展开：否则窄窗口用户永久失去完整导航
          title={toggleTitle}
          aria-label={toggleTitle}
          aria-pressed={collapsedByUser}
          className={cn(
            'fd-transition flex h-8 w-full items-center gap-2 rounded-md px-2 text-12 text-fg-subtle',
            'hover:bg-surface-sunken hover:text-fg',
            collapsed && 'justify-center px-0',
          )}
        >
          {collapsed ? (
            <PanelLeftOpen aria-hidden="true" className="size-4 shrink-0" />
          ) : (
            <PanelLeftClose aria-hidden="true" className="size-4 shrink-0" />
          )}
          <span className={cn('truncate', collapsed && 'sr-only')}>{toggleTitle}</span>
        </button>
      </div>
    </nav>
  );
}
