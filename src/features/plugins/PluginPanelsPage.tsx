import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';

import { PanelRenderer } from '@/features/plugins/PanelRenderer';
import { pluginInvokeCommand, pluginList, pluginRegistrations, pluginRenderPanel } from '@/lib/ipc';
import type { PluginRegistration } from '@/lib/ipc';
import { PLUGIN_REGISTRATIONS_QUERY_KEY, PLUGINS_QUERY_KEY } from '@/lib/queryKeys';
import { normalizeError, useAppError } from '@/lib/errors';
import { pushToast } from '@/stores/toastStore';
import { Badge } from '@/ui/components/badge';
import { Button } from '@/ui/components/button';
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/ui/components/tabs';

/**
 * 插件面板页（T6.3 挂载点）。
 *
 * 应用外壳是"左侧导航 + Outlet"模型，没有通用的底部坞——所以三个语义位置
 * （sidebar / bottom / repo-tab）统一收敛为**仓库级导航项 + 页内 Tabs**：
 * 每个已注册面板一个 Tab，语义位置以徽章标注（信息保留，布局不虚构）。
 * 没有任何面板时给引导空态；面板内容渲染失败由 PanelRenderer 的兜底卡片
 * 接住（T6.3 验收），本页不再重复处理。
 */
export function PluginPanelsPage(): React.JSX.Element {
  const { t } = useTranslation('shell');
  const { show } = useAppError();
  const queryClient = useQueryClient();
  const navigate = useNavigate();

  const registrations = useQuery({
    queryKey: [PLUGIN_REGISTRATIONS_QUERY_KEY],
    queryFn: pluginRegistrations,
  });

  // 已安装的插件清单：用来区分"没有面板"与"声明了面板但还没启用"。
  // 只读注册表会让这两种情况都显示成空列表——用户看到的就是"插件面板一无所有"。
  const installed = useQuery({ queryKey: [PLUGINS_QUERY_KEY], queryFn: pluginList });

  const panels = (registrations.data ?? []).filter(
    (registration): registration is PluginRegistration & { readonly location: string } =>
      registration.kind === 'panel',
  );

  // 声明了面板但当前不在运行的插件（禁用 / 崩溃）
  const pendingPanels = (installed.data ?? [])
    .filter((plugin) => plugin.state !== 'enabled' && (plugin.declaredPanels?.length ?? 0) > 0)
    .map((plugin) => ({
      id: plugin.id,
      name: plugin.name,
      state: plugin.state,
      titles: (plugin.declaredPanels ?? []).map((panel) => panel.title),
    }));

  const pendingSection =
    pendingPanels.length === 0 ? null : (
      <section className="border-line bg-surface flex flex-col gap-2 rounded-md border p-3">
        <h2 className="text-14 font-medium">{t('plugins.panelsPendingTitle')}</h2>
        <p className="text-12 text-fg-subtle">{t('plugins.panelsPendingHint')}</p>
        <ul className="flex flex-col gap-2">
          {pendingPanels.map((plugin) => (
            <li key={plugin.id} className="flex flex-wrap items-center gap-2 text-13">
              <span className="font-medium">{plugin.name}</span>
              <Badge tone={plugin.state === 'crashed' ? 'danger' : 'neutral'}>
                {plugin.state === 'crashed'
                  ? t('plugins.stateCrashed')
                  : t('plugins.stateDisabled')}
              </Badge>
              <span className="text-fg-muted min-w-0 flex-1 truncate">
                {plugin.titles.join(' · ')}
              </span>
              <Button
                size="sm"
                variant="secondary"
                onClick={() => void navigate('/settings/plugins')}
              >
                {t('plugins.panelsPendingAction')}
              </Button>
            </li>
          ))}
        </ul>
      </section>
    );

  const runCommand = useMutation({
    mutationFn: ({ pluginId, command }: { pluginId: string; command: string }) =>
      pluginInvokeCommand(pluginId, command, '{}'),
    onSuccess: () => {
      // 命令可能改了插件状态/面板内容：刷新注册表与面板缓存
      void queryClient.invalidateQueries({ queryKey: [PLUGIN_REGISTRATIONS_QUERY_KEY] });
      void queryClient.invalidateQueries({ queryKey: ['pluginPanel'] });
      pushToast({ title: t('plugins.commandDone'), tone: 'success' });
    },
    onError: show,
  });

  if (registrations.isLoading) {
    return (
      <section className="flex flex-col gap-2 p-4" aria-busy="true">
        <p className="text-13 text-fg-subtle">{t('plugins.panelsLoading')}</p>
      </section>
    );
  }

  if (panels.length === 0) {
    return (
      <section className="flex flex-col gap-3 p-4">
        <h1 className="text-20 font-semibold tracking-tight">{t('pages.pluginPanels.title')}</h1>
        <p className="text-13 text-fg-muted">{t('plugins.panelsEmpty')}</p>
        {installed.isError ? (
          // 查询失败必须可见：静默空列表曾让"注册表里有 3 个禁用插件"显示成
          // "什么都没装"，用户无从分辨是没装还是坏了
          <p className="text-danger text-12">
            {t('plugins.panelsListError', {
              message: normalizeError(installed.error).message,
            })}
          </p>
        ) : null}
        {pendingSection}
        {pendingPanels.length === 0 && !installed.isError ? (
          <p className="text-12 text-fg-subtle">{t('plugins.panelsEmptyHint')}</p>
        ) : null}
      </section>
    );
  }

  return (
    <section className="flex h-full flex-col gap-3 p-4">
      <h1 className="text-20 font-semibold tracking-tight">{t('pages.pluginPanels.title')}</h1>
      <Tabs
        {...(panels[0] === undefined
          ? {}
          : { defaultValue: `${panels[0].pluginId}.${panels[0].id}` })}
        className="flex min-h-0 flex-1 flex-col"
      >
        <TabsList>
          {panels.map((panel) => (
            <TabsTrigger
              key={`${panel.pluginId}.${panel.id}`}
              value={`${panel.pluginId}.${panel.id}`}
            >
              {panel.title}
              <span className="ml-1 text-11 text-fg-subtle">({panel.location})</span>
            </TabsTrigger>
          ))}
        </TabsList>
        {panels.map((panel) => (
          <TabsContent
            key={`${panel.pluginId}.${panel.id}`}
            value={`${panel.pluginId}.${panel.id}`}
            className="min-h-0 flex-1 overflow-auto"
          >
            <PanelContent
              pluginId={panel.pluginId}
              panelId={panel.id}
              onCommand={(command) => {
                runCommand.mutate({ pluginId: panel.pluginId, command });
              }}
            />
          </TabsContent>
        ))}
      </Tabs>
      {pendingSection}
    </section>
  );
}

/** 单个面板内容：拉 DSL → 渲染。按钮点击经命令链路执行（可执行插件命令）。 */
function PanelContent({
  pluginId,
  panelId,
  onCommand,
}: {
  readonly pluginId: string;
  readonly panelId: string;
  readonly onCommand: (command: string) => void;
}): React.JSX.Element {
  const { t } = useTranslation('shell');
  const dsl = useQuery({
    queryKey: ['pluginPanel', pluginId, panelId],
    queryFn: () => pluginRenderPanel(pluginId, panelId),
  });

  if (dsl.isError) {
    return (
      <div className="flex flex-col gap-2">
        <p className="text-13 text-danger">{t('plugins.panelRenderFailedTitle')}</p>
        <Button size="sm" variant="secondary" onClick={() => void dsl.refetch()}>
          {t('plugins.panelRetry')}
        </Button>
      </div>
    );
  }
  if (dsl.isLoading) {
    return (
      <p className="text-13 text-fg-subtle" aria-busy="true">
        {t('plugins.panelsLoading')}
      </p>
    );
  }
  return <PanelRenderer dslJson={dsl.data ?? '[]'} onCommand={onCommand} />;
}
