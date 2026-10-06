import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { QueryClientProvider } from '@tanstack/react-query';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { PluginSettingsPage } from '@/features/settings/PluginSettingsPage';
import {
  pluginGrant,
  pluginInstallFromDir,
  pluginList,
  pluginRevoke,
  pluginSetEnabled,
  pluginUninstall,
} from '@/lib/ipc';
import type { PluginSummary } from '@/lib/ipc';
import { createTestQueryClient } from '@/test/queryClient';

/**
 * 插件管理页（T6.4）。
 *
 * 钉住授权流程的状态机（T6.4 验收链路的 UI 面）：
 *   缺授权的启用 → 授权对话框 → 危险权限必须显式确认 → 授权并启用；
 *   已全授权的启用 → 不弹对话框直接启用；
 *   撤销 → plugin_revoke；卸载 → plugin_uninstall；
 *   空态 → 引导文案；无权限插件 → 不渲染权限区。
 * 后端状态机（管理器/引擎/撤权即时性）由 Rust 侧测试钉住。
 */
vi.mock('@/lib/ipc', () => ({
  pluginList: vi.fn(),
  pluginSetEnabled: vi.fn(),
  pluginGrant: vi.fn(),
  pluginRevoke: vi.fn(),
  pluginUninstall: vi.fn(),
  pluginReload: vi.fn(),
  pluginLogs: vi.fn(async () => []),
  pluginInstallFromDir: vi.fn(),
}));

const listMock = vi.mocked(pluginList);
const setEnabledMock = vi.mocked(pluginSetEnabled);
const grantMock = vi.mocked(pluginGrant);
const revokeMock = vi.mocked(pluginRevoke);
const uninstallMock = vi.mocked(pluginUninstall);
const installMock = vi.mocked(pluginInstallFromDir);

function plugin(overrides: Partial<PluginSummary> = {}): PluginSummary {
  return {
    id: 'com.example.stats',
    name: 'Repo Stats',
    version: '1.0.0',
    author: 'someone',
    license: 'MIT',
    description: 'A stats panel.',
    state: 'disabled',
    declaredPermissions: ['git:read', 'ui:panel'],
    grantedPermissions: [],
    permissionUsage: [],
    ...overrides,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  setEnabledMock.mockResolvedValue(undefined);
  grantMock.mockResolvedValue(undefined);
  revokeMock.mockResolvedValue(undefined);
  uninstallMock.mockResolvedValue(true);
  installMock.mockResolvedValue({
    id: 'com.example.stats',
    name: 'Repo Stats',
    version: '1.0.0',
    sha256: 'a'.repeat(64),
    insideRoot: true,
    declaredPermissions: ['git:read'],
  });
});

afterEach(() => {
  // 对话框由组件状态驱动，显式清理保证用例间不串 DOM
  cleanup();
});

function renderPage(): void {
  const client = createTestQueryClient();
  render(
    <QueryClientProvider client={client}>
      <PluginSettingsPage />
    </QueryClientProvider>,
  );
}

describe('插件管理页', () => {
  it('空列表渲染引导文案', async () => {
    listMock.mockResolvedValue([]);

    renderPage();

    expect(await screen.findByText('还没有安装任何插件')).toBeInTheDocument();
  });

  it('列表卡片展示名称、状态徽章与权限行', async () => {
    listMock.mockResolvedValue([plugin()]);

    renderPage();

    expect(await screen.findByTestId('plugin-card-com.example.stats')).toBeInTheDocument();
    expect(screen.getByText('已禁用')).toBeInTheDocument();
    expect(screen.getByText('git:read')).toBeInTheDocument();
    expect(screen.getByText('读取仓库状态与历史。')).toBeInTheDocument();
  });

  it('缺授权的启用先弹授权对话框，确认后先授权再启用', async () => {
    listMock.mockResolvedValue([plugin()]);

    renderPage();
    fireEvent.click(await screen.findByRole('button', { name: '启用' }));

    // 对话框出现，缺的权限默认勾选
    const dialog = await screen.findByRole('alertdialog');
    expect(within(dialog).getByRole('checkbox', { name: 'git:read' })).toBeChecked();
    const confirm = within(dialog).getByRole('button', { name: '授权并启用' });
    expect(confirm).toBeEnabled();

    fireEvent.click(confirm);

    await waitFor(() => {
      expect(grantMock).toHaveBeenCalledWith('com.example.stats', ['git:read', 'ui:panel']);
      expect(setEnabledMock).toHaveBeenCalledWith('com.example.stats', true);
    });
  });

  it('勾选危险权限后，未显式确认前不能提交', async () => {
    listMock.mockResolvedValue([
      plugin({
        declaredPermissions: ['fs:write', 'ui:toast'],
      }),
    ]);

    renderPage();
    fireEvent.click(await screen.findByRole('button', { name: '启用' }));

    const dialog = await screen.findByRole('alertdialog');
    const confirm = within(dialog).getByRole('button', { name: '授权并启用' });
    // fs:write 默认勾选（危险）→ 必须先勾独立的理解确认
    expect(within(dialog).getByRole('checkbox', { name: 'fs:write' })).toBeChecked();
    const understand = within(dialog).getByLabelText('我了解上述危险权限的影响');
    expect(understand).not.toBeChecked();

    fireEvent.click(confirm);
    expect(grantMock).not.toHaveBeenCalled();

    fireEvent.click(within(dialog).getByLabelText('我了解上述危险权限的影响'));
    expect(confirm).toBeEnabled();
    fireEvent.click(confirm);

    await waitFor(() => {
      expect(grantMock).toHaveBeenCalled();
    });
  });

  it('取消授权对话框不触发任何调用', async () => {
    listMock.mockResolvedValue([plugin()]);

    renderPage();
    fireEvent.click(await screen.findByRole('button', { name: '启用' }));

    const dialog = await screen.findByRole('alertdialog');
    fireEvent.click(within(dialog).getByRole('button', { name: '取消' }));

    expect(grantMock).not.toHaveBeenCalled();
    expect(setEnabledMock).not.toHaveBeenCalled();
  });

  it('已全授权的启用不弹对话框，直接启用', async () => {
    listMock.mockResolvedValue([
      plugin({ grantedPermissions: ['git:read', 'ui:panel'], state: 'disabled' }),
    ]);

    renderPage();
    fireEvent.click(await screen.findByRole('button', { name: '启用' }));

    await waitFor(() => {
      expect(setEnabledMock).toHaveBeenCalledWith('com.example.stats', true);
    });
    expect(grantMock).not.toHaveBeenCalled();
    expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument();
  });

  it('撤销权限调用 plugin_revoke 并展示用量', async () => {
    listMock.mockResolvedValue([
      plugin({
        state: 'enabled',
        grantedPermissions: ['git:read', 'ui:panel'],
        permissionUsage: [['git:read', 7]],
      }),
    ]);

    renderPage();
    // 运行中的已授权限行有撤销按钮
    const revokeButtons = await screen.findAllByRole('button', { name: '撤销' });
    expect(revokeButtons).toHaveLength(2);
    expect(screen.getByText(/7 次调用/)).toBeInTheDocument();

    const firstRevoke = revokeButtons.at(0);
    expect(firstRevoke).toBeDefined();
    fireEvent.click(firstRevoke!);
    await waitFor(() => {
      expect(revokeMock).toHaveBeenCalled();
    });
  });

  it('卸载经确认对话框调用 plugin_uninstall', async () => {
    listMock.mockResolvedValue([plugin()]);

    renderPage();
    fireEvent.click(await screen.findByRole('button', { name: '卸载' }));

    const dialog = await screen.findByRole('alertdialog');
    fireEvent.click(within(dialog).getByRole('button', { name: '卸载' }));

    await waitFor(() => {
      expect(uninstallMock).toHaveBeenCalledWith('com.example.stats');
    });
  });

  it('无权限插件不渲染权限区，但保留启用入口', async () => {
    listMock.mockResolvedValue([plugin({ declaredPermissions: [] })]);

    renderPage();
    expect(await screen.findByText('此插件未申请任何权限。')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '启用' })).toBeInTheDocument();
  });
});
