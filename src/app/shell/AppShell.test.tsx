import { cleanup, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { initialUiState, useUiStore } from '@/stores/uiStore';
import { renderApp } from '@/test/renderApp';

/**
 * 应用外壳的交互级测试（T0.4 验收项）。
 *
 * 这里刻意测"交互"而不是"渲染"：外壳的价值在于
 *   ① 键盘能到达所有导航；② 折叠/展开状态正确；③ 菜单能被 Esc 关闭且焦点不丢。
 * 只断言"元素存在"会漏掉这些真正的回归点。
 */
beforeEach(() => {
  window.localStorage.clear();
  document.documentElement.removeAttribute('data-theme');
  useUiStore.setState(initialUiState);
});

afterEach(() => {
  // 顺序很重要：先卸载组件树，再复位 store。
  // 若在挂载状态下 setState，React 会在 act 作用域之外触发这些订阅者的重渲染
  // （表现为一串 "not wrapped in act" 警告），噪音会掩盖真正的失败。
  cleanup();
  useUiStore.setState(initialUiState);
});

/**
 * 打开仓库切换菜单。
 *
 * 必须显式派发 pointerdown：Radix 的菜单在**鼠标按下**时打开
 * （这样"按住拖动"不会误开菜单），只派发 click 在 jsdom 里不会触发。
 */
function openRepoSwitcher(): HTMLElement {
  const trigger = screen.getByRole('button', { name: /当前仓库/ });
  fireEvent.pointerDown(trigger, { pointerId: 1, pointerType: 'mouse', button: 0 });
  fireEvent.click(trigger);
  return trigger;
}

describe('外壳结构', () => {
  it('渲染主导航、主内容区与状态栏', () => {
    renderApp('/');

    expect(screen.getByRole('navigation', { name: '主导航' })).toBeInTheDocument();
    expect(screen.getByRole('main')).toBeInTheDocument();
    expect(screen.getByRole('contentinfo', { name: '状态栏' })).toBeInTheDocument();
  });

  it('状态栏显示未打开仓库与空闲状态', () => {
    renderApp('/');
    const statusBar = screen.getByRole('contentinfo', { name: '状态栏' });

    expect(within(statusBar).getByText('未打开仓库')).toBeInTheDocument();
    expect(within(statusBar).getByText('空闲')).toBeInTheDocument();
    expect(within(statusBar).getByText('后台任务')).toBeInTheDocument();
  });

  it('提供"跳到主内容"的跳转链接（键盘用户可跳过导航）', () => {
    renderApp('/');
    expect(screen.getByRole('link', { name: '跳到主内容' })).toHaveAttribute(
      'href',
      '#main-content',
    );
  });
});

describe('导航可用性', () => {
  it('未打开仓库时，仓库级条目禁用并给出原因', () => {
    renderApp('/');

    const statusEntry = screen.getByRole('button', { name: '工作区' });
    expect(statusEntry).toBeDisabled();
    expect(statusEntry).toHaveAttribute('title', '请先选择一个仓库');
  });

  it('非仓库级条目始终可用', () => {
    renderApp('/');
    expect(screen.getByRole('link', { name: '仪表盘' })).toBeInTheDocument();
    expect(screen.getByRole('link', { name: '设置' })).toBeInTheDocument();
  });

  it('选择仓库后仓库级条目变为可用链接，并跳转到工作区', async () => {
    renderApp('/');

    openRepoSwitcher();
    const menu = screen.getByRole('menu');
    fireEvent.click(within(menu).getByRole('menuitem', { name: /forgedesk/ }));

    expect(useUiStore.getState().currentRepoId).toBe('example-forgedesk');

    // 侧栏里的条目已从「禁用按钮」变成「可用链接」（用导航容器限定，避免与仓库内标签页重名）
    const sidebar = screen.getByRole('navigation', { name: '主导航' });
    expect(within(sidebar).getByRole('link', { name: '工作区' })).toBeInTheDocument();

    // 跳转是异步的（React Router 的导航走 transition），用 findBy 等待并让 act 收尾
    expect(await screen.findByRole('heading', { name: '工作区' })).toBeInTheDocument();
  });
});

describe('仓库切换器', () => {
  it('展开与收起', () => {
    renderApp('/');
    const trigger = screen.getByRole('button', { name: /当前仓库/ });
    expect(trigger).toHaveAttribute('aria-expanded', 'false');

    openRepoSwitcher();
    expect(trigger).toHaveAttribute('aria-expanded', 'true');
    expect(screen.getByRole('menu')).toBeInTheDocument();
  });

  it('Esc 关闭菜单并把焦点还给触发按钮', async () => {
    renderApp('/');
    const trigger = openRepoSwitcher();
    const menu = screen.getByRole('menu');

    // Esc 由菜单内部处理（焦点在菜单里），焦点归还由 Radix 负责
    fireEvent.keyDown(menu, { key: 'Escape' });

    expect(screen.queryByRole('menu')).not.toBeInTheDocument();
    // 焦点归还发生在关闭动画之后（异步），必须等待而不是立即断言
    await waitFor(() => {
      expect(trigger).toHaveFocus();
    });
  });
});

describe('全局搜索', () => {
  it('Ctrl+K 聚焦搜索框', () => {
    renderApp('/');
    const searchBox = screen.getByRole('searchbox', { name: '全局搜索' });

    fireEvent.keyDown(window, { key: 'k', ctrlKey: true });

    expect(searchBox).toHaveFocus();
  });

  it('Esc 清空输入并移出焦点', () => {
    renderApp('/');
    const searchBox = screen.getByRole('searchbox', { name: '全局搜索' });

    fireEvent.change(searchBox, { target: { value: 'rebase' } });
    expect(searchBox).toHaveValue('rebase');

    fireEvent.keyDown(searchBox, { key: 'Escape' });
    expect(searchBox).toHaveValue('');
    expect(searchBox).not.toHaveFocus();
  });
});

describe('侧栏折叠', () => {
  it('折叠按钮切换状态并同步 aria-pressed', async () => {
    renderApp('/');
    const collapseButton = screen.getByRole('button', { name: '折叠侧栏' });
    expect(collapseButton).toHaveAttribute('aria-pressed', 'false');

    fireEvent.click(collapseButton);
    expect(useUiStore.getState().sidebarCollapsed).toBe(true);

    const expandButton = await screen.findByRole('button', { name: '展开侧栏' });
    expect(expandButton).toHaveAttribute('aria-pressed', 'true');
  });

  it('折叠后导航仍可被无障碍名称定位（图标模式用 sr-only 保留文字）', async () => {
    renderApp('/');
    fireEvent.click(screen.getByRole('button', { name: '折叠侧栏' }));

    const dashboard = await screen.findByRole('link', { name: '仪表盘' });
    expect(dashboard).toHaveAttribute('title', '仪表盘');
  });
});

describe('详情面板位置', () => {
  it('可切换到右侧 / 底部 / 隐藏', async () => {
    renderApp('/repo/example-forgedesk/status');

    const panel = () => screen.queryByRole('complementary', { name: '详情' });
    expect(panel()).toBeInTheDocument();

    // ToggleGroup 的单选项是 radio 语义（radigroup + radio），不是 button
    fireEvent.click(screen.getByRole('radio', { name: '底部' }));
    expect(useUiStore.getState().detailPanel).toBe('bottom');
    expect(panel()).toBeInTheDocument();

    fireEvent.click(screen.getByRole('radio', { name: '隐藏' }));
    expect(useUiStore.getState().detailPanel).toBe('hidden');
    expect(panel()).not.toBeInTheDocument();
  });
});
