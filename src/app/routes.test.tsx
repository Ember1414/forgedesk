import { cleanup, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { NAV_SECTIONS, navItemPath } from '@/app/shell/navItems';
import i18n from '@/lib/i18n';
import { initialUiState, useUiStore } from '@/stores/uiStore';
import { renderApp } from '@/test/renderApp';

/**
 * 路由可达性测试。
 *
 * 为什么用"表驱动"逐条断言：T0.4 的验收标准是"所有路由可跳转"，
 * 而路由表出错（漏注册、拼错 segment）在白屏之前都不会有任何征兆。
 * 把每条路由写进表里，新增页面时只需补一行，漏注册会立刻失败。
 *
 * 注意：断言页面的**标题**而不是"渲染没报错"——后者在 React 里几乎恒真。
 */
const ROUTE_CASES: readonly (readonly [path: string, heading: string])[] = [
  ['/', '仪表盘'],
  // index 重定向：/repo/:id 应落到工作区
  ['/repo/example-forgedesk', '工作区'],
  ['/repo/example-forgedesk/status', '工作区'],
  ['/repo/example-forgedesk/history', '历史'],
  ['/repo/example-forgedesk/branches', '分支'],
  ['/repo/example-forgedesk/conflict', '冲突'],
  ['/repo/example-forgedesk/terminal', '终端'],
  ['/github', '远程仓库'],
  ['/github/repos', '远程仓库'],
  ['/github/pull-requests', '拉取请求'],
  ['/github/issues', '议题'],
  ['/github/actions', '流水线'],
  ['/settings', '通用'],
  ['/settings/general', '通用'],
  ['/settings/appearance', '外观'],
  ['/settings/github', '代码托管账号'],
  ['/settings/advanced', '高级'],
  ['/settings/privacy', '隐私'],
  ['/plugins', '插件'],
  ['/commands', '命令字典'],
  // 未知路径必须给出明确出口，而不是白屏
  ['/no-such-page', '页面不存在'],
];

beforeEach(() => {
  window.localStorage.clear();
  useUiStore.setState(initialUiState);
});

afterEach(() => {
  // 先卸载再复位 store，避免在挂载状态下 setState 触发 act 之外的更新（见 AppShell.test.tsx）
  cleanup();
  useUiStore.setState(initialUiState);
});

describe('路由表', () => {
  it.each([...ROUTE_CASES])('%s 渲染「%s」页面', (path, heading) => {
    renderApp(path);
    expect(screen.getByRole('heading', { name: heading })).toBeInTheDocument();
  });

  it('开发专用路由可访问设计系统预览页（生产构建不注册）', async () => {
    renderApp('/__dev__/design');
    expect(await screen.findByRole('heading', { name: 'ForgeDesk 设计系统' })).toBeInTheDocument();
  });
});

describe('导航与路由的一致性', () => {
  it('每个导航项都能解析出路径，且该路径下侧栏对应条目被标记为当前页', () => {
    const repoId = 'example-forgedesk';
    const items = NAV_SECTIONS.flatMap((section) => section.items);

    for (const item of items) {
      const path = navItemPath(item, repoId);
      expect(path).not.toBeNull();

      // 仓库级条目只有在"已打开仓库"时才渲染为链接，因此先按该状态准备 store
      useUiStore.setState({ ...initialUiState, currentRepoId: repoId });

      // 逐条渲染：确认侧栏上的每一项都真的能打开（而不是只改了高亮）。
      // 用 aria-current="page" 作为判据——它由路由的真实匹配结果产生，
      // 因此能同时发现"路由漏注册"和"路径算错"两类问题。
      const { unmount } = renderApp(path ?? '/');
      const sidebar = screen.getByRole('navigation', { name: '主导航' });
      const label = i18n.t(item.labelKey, { ns: 'shell' });

      expect(
        within(sidebar).getByRole('link', { name: label, current: 'page' }),
      ).toBeInTheDocument();
      unmount();
    }

    expect(items.length).toBeGreaterThan(0);
  });

  it('未打开仓库时，仓库级导航项不产生路径', () => {
    const repoScoped = NAV_SECTIONS.flatMap((section) => section.items).filter(
      (item) => item.repoScoped,
    );
    expect(repoScoped.length).toBeGreaterThan(0);
    for (const item of repoScoped) {
      expect(navItemPath(item, null)).toBeNull();
    }
  });
});
