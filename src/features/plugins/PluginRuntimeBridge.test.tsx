/** 真实命令注册表 + 插件 IPC 边界：覆盖入口、结果、权限变更后的移除与通知。 */
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { QueryClientProvider } from '@tanstack/react-query';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { PluginRuntimeBridge } from '@/features/plugins/PluginRuntimeBridge';
import { CommandPalette } from '@/features/commands/CommandPalette';
import {
  initialCommandRegistryState,
  useCommandRegistry,
} from '@/features/commands/commandRegistry';
import { pluginInvokeCommand, pluginRegistrations } from '@/lib/ipc';
import { listenEvent } from '@/lib/ipc/client';
import { useToastStore } from '@/stores/toastStore';
import { createTestQueryClient } from '@/test/queryClient';

vi.mock('@/lib/ipc', () => ({ pluginRegistrations: vi.fn(), pluginInvokeCommand: vi.fn() }));
vi.mock('@/lib/ipc/client', () => ({ isTauriRuntime: () => true, listenEvent: vi.fn() }));
const registration = {
  pluginId: 'com.example.commit-template',
  kind: 'command' as const,
  id: 'com.example.commit-template.fill-feat',
  title: '提交信息模板：feat',
};
const callbacks = new Map<string, (payload: unknown) => void>();
const unlisten = vi.fn();

beforeEach(() => {
  vi.clearAllMocks();
  callbacks.clear();
  useCommandRegistry.setState(initialCommandRegistryState);
  useToastStore.setState({ toasts: [] });
  vi.mocked(pluginRegistrations).mockResolvedValue([registration]);
  vi.mocked(pluginInvokeCommand).mockResolvedValue(JSON.stringify({ message: 'feat(core): test' }));
  vi.mocked(listenEvent).mockImplementation(async (event, handler) => {
    callbacks.set(event, handler as (payload: unknown) => void);
    return unlisten;
  });
});
afterEach(() => {
  cleanup();
  useCommandRegistry.setState(initialCommandRegistryState);
});

describe('插件命令与通知接线', () => {
  it('清单命令进入命令面板，执行后保留可复制的完整结果', async () => {
    render(
      <QueryClientProvider client={createTestQueryClient()}>
        <PluginRuntimeBridge />
        <CommandPalette overrides={{}} contextTags={new Set()} />
      </QueryClientProvider>,
    );
    fireEvent.click(await screen.findByText(registration.title));
    await waitFor(() =>
      expect(pluginInvokeCommand).toHaveBeenCalledWith(
        registration.pluginId,
        registration.id,
        '{}',
      ),
    );
    await waitFor(() =>
      expect(
        useToastStore.getState().toasts.some((toast) => toast.detail === 'feat(core): test'),
      ).toBe(true),
    );
    expect(useToastStore.getState().toasts.at(-1)?.actions?.[0]?.label).toBe('复制结果');
  });

  it('注册表变化移除旧插件命令但保留应用命令，卸载组件清理监听', async () => {
    useCommandRegistry.getState().register([
      {
        id: 'app.keep',
        titleKey: 'commands.palette.title',
        category: 'app',
        defaultKey: null,
        when: [],
        run: vi.fn(),
      },
    ]);
    const view = render(
      <QueryClientProvider client={createTestQueryClient()}>
        <PluginRuntimeBridge />
      </QueryClientProvider>,
    );
    await waitFor(() => expect(useCommandRegistry.getState().commands).toHaveLength(2));
    vi.mocked(pluginRegistrations).mockResolvedValue([]);
    act(() => callbacks.get('plugin-registrations-changed')?.({}));
    await waitFor(() =>
      expect(useCommandRegistry.getState().commands.map((command) => command.id)).toEqual([
        'app.keep',
      ]),
    );
    act(() => callbacks.get('plugin-toast')?.({ level: 'warning', message: 'plugin notice' }));
    expect(useToastStore.getState().toasts.at(-1)?.tone).toBe('warning');
    view.unmount();
    expect(unlisten).toHaveBeenCalledTimes(2);
  });

  it('插件返回错误时显示失败，不能提示执行成功', async () => {
    vi.mocked(pluginInvokeCommand).mockResolvedValue(
      JSON.stringify({ error: 'no repository open' }),
    );
    render(
      <QueryClientProvider client={createTestQueryClient()}>
        <PluginRuntimeBridge />
      </QueryClientProvider>,
    );
    await waitFor(() => expect(useCommandRegistry.getState().commands).toHaveLength(1));
    act(() => useCommandRegistry.getState().commands[0]?.run());
    await waitFor(() => expect(useToastStore.getState().toasts.at(-1)?.tone).toBe('danger'));
    expect(useToastStore.getState().toasts.at(-1)?.description).toBe('no repository open');
  });
});
