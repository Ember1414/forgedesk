/** 插件命令的共用执行入口：命令面板与 DSL 按钮显示同一份结果和错误。 */
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';

import { useAppError } from '@/lib/errors';
import { pluginInvokeCommand } from '@/lib/ipc';
import { PLUGIN_REGISTRATIONS_QUERY_KEY } from '@/lib/queryKeys';
import { pushToast } from '@/stores/toastStore';

export function usePluginCommand() {
  const { t } = useTranslation('shell');
  const { show } = useAppError();
  const client = useQueryClient();
  return useMutation({
    mutationFn: ({ pluginId, command }: { pluginId: string; command: string }) =>
      pluginInvokeCommand(pluginId, command, '{}'),
    onSuccess: (raw) => {
      void client.invalidateQueries({ queryKey: [PLUGIN_REGISTRATIONS_QUERY_KEY] });
      void client.invalidateQueries({ queryKey: ['pluginPanel'] });
      let output: unknown;
      try {
        output = JSON.parse(raw);
      } catch {
        output = raw;
      }
      const result =
        output !== null && typeof output === 'object' ? (output as Record<string, unknown>) : {};
      if (typeof result['error'] === 'string') {
        pushToast({
          title: t('plugins.commandFailed'),
          description: result['error'],
          tone: 'danger',
          duration: 0,
        });
        return;
      }
      const text =
        typeof result['message'] === 'string'
          ? result['message']
          : typeof result['markdown'] === 'string'
            ? result['markdown']
            : null;
      pushToast({
        title: t('plugins.commandDone'),
        tone: 'success',
        ...(text === null
          ? {}
          : {
              detail: text,
              duration: 0,
              actions: [
                {
                  id: 'copy-plugin-result',
                  label: t('plugins.copyResult'),
                  onClick: () => {
                    void navigator.clipboard
                      .writeText(text)
                      .then(
                        () => pushToast({ title: t('plugins.resultCopied'), tone: 'success' }),
                        show,
                      );
                  },
                },
              ],
            }),
      });
    },
    onError: show,
  });
}
