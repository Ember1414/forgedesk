/**
 * 左侧主导航的数据源（单一真相源）。
 *
 * 为什么把导航做成数据而不是 JSX：导航项同时被三处使用——
 * 侧栏渲染、路由断言（测试逐条验证可达）、以及命令面板（M5 起）。
 * 只有一份清单才能保证"侧栏能看到的路由"与"实际存在的路由"不会漂移。
 *
 * 关于图标（AGENTS.md 红线 R2 / R3）：
 *   全部使用**通用几何/物件**图标（面板、时钟、分叉路径、警示三角…），
 *   刻意避开 lucide 中与 Git / GitHub 官方标识同形的品牌图标（如 Github、GitBranch），
 *   避免与第三方标识产生视觉联想。
 */
import {
  BookOpen,
  Cloud,
  History,
  LayoutDashboard,
  PanelsTopLeft,
  Puzzle,
  Settings,
  ScrollText,
  SquareTerminal,
  TriangleAlert,
  Waypoints,
} from 'lucide-react';
import type { LucideIcon } from 'lucide-react';

export interface NavItem {
  /** 稳定标识，用于测试定位与埋点，不参与 i18n。 */
  readonly id: string;
  /** i18n key（shell 命名空间下的 items.*）。 */
  readonly labelKey: string;
  readonly icon: LucideIcon;
  /**
   * 路径片段（相对根）。
   * 仓库级条目的最终路径是 `/repo/:repoId/<segment>`，其余是 `/<segment>`。
   */
  readonly segment: string;
  /** 是否依赖"当前已打开仓库"；为 true 且未打开仓库时导航项处于禁用态。 */
  readonly repoScoped: boolean;
}

export interface NavSection {
  readonly id: string;
  /** i18n key（shell 命名空间下的 nav.sections.*）；分组标题在折叠态下不显示。 */
  readonly titleKey: string;
  readonly items: readonly NavItem[];
}

export const NAV_SECTIONS: readonly NavSection[] = [
  {
    id: 'repository',
    titleKey: 'nav.sections.repository',
    items: [
      {
        id: 'dashboard',
        labelKey: 'items.dashboard',
        icon: LayoutDashboard,
        segment: '',
        repoScoped: false,
      },
      {
        id: 'status',
        labelKey: 'items.status',
        icon: PanelsTopLeft,
        segment: 'status',
        repoScoped: true,
      },
      {
        id: 'history',
        labelKey: 'items.history',
        icon: History,
        segment: 'history',
        repoScoped: true,
      },
      {
        id: 'branches',
        labelKey: 'items.branches',
        icon: Waypoints,
        segment: 'branches',
        repoScoped: true,
      },
      {
        id: 'operations',
        labelKey: 'items.operations',
        icon: ScrollText,
        segment: 'operations',
        repoScoped: true,
      },
      {
        id: 'conflict',
        labelKey: 'items.conflict',
        icon: TriangleAlert,
        segment: 'conflict',
        repoScoped: true,
      },
      {
        id: 'terminal',
        labelKey: 'items.terminal',
        icon: SquareTerminal,
        segment: 'terminal',
        repoScoped: true,
      },
      {
        id: 'pluginPanels',
        labelKey: 'items.pluginPanels',
        icon: Puzzle,
        segment: 'plugin-panels',
        repoScoped: true,
      },
    ],
  },
  {
    id: 'integrations',
    titleKey: 'nav.sections.integrations',
    items: [
      {
        id: 'github',
        labelKey: 'items.github',
        icon: Cloud,
        segment: 'github',
        repoScoped: false,
      },
      {
        id: 'plugins',
        labelKey: 'items.plugins',
        icon: Puzzle,
        segment: 'plugins',
        repoScoped: false,
      },
    ],
  },
  {
    id: 'application',
    titleKey: 'nav.sections.application',
    items: [
      {
        id: 'commands',
        labelKey: 'items.commands',
        icon: BookOpen,
        segment: 'commands',
        repoScoped: false,
      },
      {
        id: 'settings',
        labelKey: 'items.settings',
        icon: Settings,
        segment: 'settings',
        repoScoped: false,
      },
    ],
  },
];

/** 计算导航项的目标路径。未打开仓库时仓库级条目返回 null（调用方应渲染为禁用态）。 */
export function navItemPath(item: NavItem, repoId: string | null): string | null {
  if (item.repoScoped) {
    return repoId === null ? null : `/repo/${repoId}/${item.segment}`;
  }
  return item.segment === '' ? '/' : `/${item.segment}`;
}
