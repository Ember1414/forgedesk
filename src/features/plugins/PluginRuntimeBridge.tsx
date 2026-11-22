/** 宿主贡献点到应用命令面板/通知的接线；卸载监听和旧命令随组件生命周期清理。 */
import { useEffect } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';

import { useCommandRegistry } from '@/features/commands/commandRegistry';
import { usePluginCommand } from '@/features/plugins/usePluginCommand';
import { pluginRegistrations } from '@/lib/ipc';
import { isTauriRuntime, listenEvent } from '@/lib/ipc/client';
import { PLUGIN_REGISTRATIONS_QUERY_KEY, PLUGINS_QUERY_KEY } from '@/lib/queryKeys';
import { pushToast } from '@/stores/toastStore';

export function PluginRuntimeBridge(): null {
  const client = useQueryClient();
  const inTauri = isTauriRuntime();
  const registrations = useQuery({
    queryKey: [PLUGIN_REGISTRATIONS_QUERY_KEY],
    queryFn: pluginRegistrations,
    enabled: inTauri,
  });
  const { mutate: run } = usePluginCommand();
  const register = useCommandRegistry((state) => state.register);
  const unregister = useCommandRegistry((state) => state.unregister);
  useEffect(() => {
    if (!inTauri) return;
    const commands = (registrations.data ?? [])
      .filter((entry) => entry.kind === 'command')
      .map((entry) => ({
        id: `plugin:${entry.pluginId}:${entry.id}`,
        titleKey: 'plugins.commandDone',
        title: entry.title,
        category: 'app' as const,
        defaultKey: null,
        when: [],
        run: () => run({ pluginId: entry.pluginId, command: entry.id }),
      }));
    register(commands);
    return () => unregister(commands.map((command) => command.id));
  }, [inTauri, registrations.data, register, unregister, run]);
  useEffect(() => {
    if (!inTauri) return;
    let disposed = false;
    const unsubscribers: (() => void)[] = [];
    const keep = (unlisten: () => void) => {
      if (disposed) unlisten();
      else unsubscribers.push(unlisten);
    };
    const registrationsReady = listenEvent('plugin-registrations-changed', () => {
      void client.invalidateQueries({ queryKey: [PLUGIN_REGISTRATIONS_QUERY_KEY] });
      void client.invalidateQueries({ queryKey: [PLUGINS_QUERY_KEY] });
      void client.invalidateQueries({ queryKey: ['pluginPanel'] });
    })
      .then(keep)
      .catch(() => undefined);
    const toastsReady = listenEvent<{ level: string; message: string }>('plugin-toast', (event) => {
      if (disposed || typeof event.message !== 'string') return;
      const tone =
        event.level === 'error'
          ? 'danger'
          : event.level === 'success'
            ? 'success'
            : event.level === 'warning'
              ? 'warning'
              : 'info';
      pushToast({ title: event.message, tone });
    })
      .then(keep)
      .catch(() => undefined);
    // 先订阅再读取，避免激活事件落在第一次读取与监听建立之间。
    void Promise.all([registrationsReady, toastsReady]).then(() => {
      if (!disposed) void client.invalidateQueries({ queryKey: [PLUGIN_REGISTRATIONS_QUERY_KEY] });
    });
    return () => {
      disposed = true;
      unsubscribers.forEach((unlisten) => unlisten());
    };
  }, [client, inTauri]);
  return null;
}
