import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { QueryClientProvider } from '@tanstack/react-query';
import { MemoryRouter } from 'react-router-dom';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { PluginPanelsPage } from '@/features/plugins/PluginPanelsPage';
import { pluginInvokeCommand, pluginList, pluginRegistrations, pluginRenderPanel } from '@/lib/ipc';
import type { PluginRegistration, PluginSummary } from '@/lib/ipc';
import { createTestQueryClient } from '@/test/queryClient';

/**
 * 插件面板页（T6.3/T6.5 挂载点）：
 *   - 已注册面板 → 每 panel 一个 Tab，内容走 render DSL；
 *   - 没有面板 → 引导空态；
 *   - **声明了面板但未启用 → 列出插件与启用入口**（2026-10-08：仅看运行态注册表时，
 *     "没有面板"与"声明了面板但没启用"都表现为空页面，用户以为插件功能根本不存在）；
 *   - 面板按钮 → plugin_invoke_command 链路；
 *   - DSL 坏数据 → PanelRenderer 兜底卡片（此处验证页面不崩）。
 */
vi.mock('@/lib/ipc', () => ({
  pluginRegistrations: vi.fn(),
  pluginRenderPanel: vi.fn(),
  pluginInvokeCommand: vi.fn(),
  pluginList: vi.fn(),
}));

const registrationsMock = vi.mocked(pluginRegistrations);
const renderMock = vi.mocked(pluginRenderPanel);
const invokeMock = vi.mocked(pluginInvokeCommand);
const listMock = vi.mocked(pluginList);

function pluginSummary(overrides: Partial<PluginSummary> = {}): PluginSummary {
  return {
    id: 'com.example.repo-stats',
    name: 'Repo Stats',
    version: '0.1.0',
    author: 'ForgeDesk contributors',
    license: 'Apache-2.0',
    description: '示例插件',
    state: 'disabled',
    declaredPermissions: ['git:read', 'ui:panel'],
    grantedPermissions: [],
    permissionUsage: [],
    declaredPanels: [{ id: 'stats', title: '仓库统计', location: 'sidebar' }],
    ...overrides,
  };
}

function registration(overrides: Partial<PluginRegistration> = {}): PluginRegistration {
  return {
    pluginId: 'com.example.stats',
    kind: 'panel',
    id: 'com.example.stats.stats',
    title: '仓库统计',
    location: 'sidebar',
    ...overrides,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  renderMock.mockResolvedValue(
    JSON.stringify([
      { type: 'heading', text: '统计' },
      { type: 'button', command: 'com.example.stats.refresh', label: '刷新' },
    ]),
  );
  invokeMock.mockResolvedValue('{}');
  // 默认：没有已安装插件（各用例按需覆盖）
  listMock.mockResolvedValue([]);
});

afterEach(() => {
  vi.clearAllMocks();
});

function renderPage(): void {
  const client = createTestQueryClient();
  render(
    <QueryClientProvider client={client}>
      <MemoryRouter>
        <PluginPanelsPage />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

describe('插件面板页', () => {
  it('没有面板注册时渲染引导空态', async () => {
    registrationsMock.mockResolvedValue([]);

    renderPage();

    expect(await screen.findByText('还没有已启用插件提供面板。')).toBeInTheDocument();
  });

  it('声明了面板但未启用的插件会被列出来，并给出启用入口', async () => {
    registrationsMock.mockResolvedValue([]);
    listMock.mockResolvedValue([pluginSummary()]);

    renderPage();

    expect(await screen.findByText('已安装但未启用的面板')).toBeInTheDocument();
    expect(screen.getByText('Repo Stats')).toBeInTheDocument();
    // 面板标题要显示出来（让用户知道启用后能得到什么）
    expect(screen.getByText('仓库统计')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '去启用' })).toBeInTheDocument();
    // 此时不应再显示"还没有已启用插件提供面板"以外的误导性内容
    expect(screen.queryByText('已启用')).not.toBeInTheDocument();
  });

  it('已启用的插件不再出现在"未启用"列表里', async () => {
    registrationsMock.mockResolvedValue([]);
    listMock.mockResolvedValue([pluginSummary({ state: 'enabled' })]);

    renderPage();

    expect(await screen.findByText('还没有已启用插件提供面板。')).toBeInTheDocument();
    expect(screen.queryByText('已安装但未启用的面板')).not.toBeInTheDocument();
  });

  it('命令类注册不出现在面板 Tabs 里', async () => {
    registrationsMock.mockResolvedValue([
      registration(),
      {
        pluginId: 'com.example.stats',
        kind: 'command',
        id: 'com.example.stats.refresh',
        title: '刷新',
      },
    ]);

    renderPage();

    expect(await screen.findByRole('tab', { name: /仓库统计/ })).toBeInTheDocument();
    expect(screen.queryByRole('tab', { name: '刷新' })).not.toBeInTheDocument();
  });

  it('面板内容渲染 DSL 且按钮走命令链路', async () => {
    registrationsMock.mockResolvedValue([registration()]);

    renderPage();

    expect(await screen.findByRole('heading', { name: '统计' })).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '刷新' }));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith(
        'com.example.stats',
        'com.example.stats.refresh',
        '{}',
      );
    });
  });

  it('渲染接口失败时展示错误与重试入口（页面不崩）', async () => {
    registrationsMock.mockResolvedValue([registration()]);
    renderMock.mockRejectedValue(new Error('boom'));

    renderPage();

    expect(await screen.findByText('插件面板暂时无法显示')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '重试' })).toBeInTheDocument();
  });

  it('DSL 坏数据由兜底卡片接住', async () => {
    registrationsMock.mockResolvedValue([registration()]);
    renderMock.mockResolvedValue('[{"type": "iframe"}]');

    renderPage();

    expect(await screen.findByRole('alert')).toBeInTheDocument();
  });
});
