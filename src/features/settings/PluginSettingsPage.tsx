import { useState } from 'react';

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';

import { useAppError } from '@/lib/errors';
import {
  pluginGrant,
  pluginInstallFromDir,
  pluginList,
  pluginLogs,
  pluginReload,
  pluginRevoke,
  pluginSetEnabled,
  pluginUninstall,
} from '@/lib/ipc';
import type { PluginSummary } from '@/lib/ipc';
import { PLUGINS_QUERY_KEY } from '@/lib/queryKeys';
import { pushToast } from '@/stores/toastStore';
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/ui/components/alert-dialog';
import { Badge } from '@/ui/components/badge';
import { Button } from '@/ui/components/button';
import { Checkbox } from '@/ui/components/checkbox';
import { Input } from '@/ui/components/input';

/**
 * 插件管理页（T6.4）。
 *
 * # 授权对话框是安全模型的 UI 面
 *
 * 启用前如果清单声明的权限未被全部授予，先弹出逐项授权对话框：
 * 勾选项即授予权限，危险权限（fs:write / git:write / net:github，与后端
 * `Permission::is_dangerous` 同表）醒目标注，且**必须**勾选独立的理解确认
 * 才能提交。撤销是立即生效的反向操作（后端同步收缩运行中实例）。
 *
 * # 卸载的目录语义与后端对齐
 *
 * 后端返回"是否删除了目录"：正常安装在 plugins_root 内会删；开发者模式的
 * 外部目录保留（那是开发者自己的工作副本）。两种结果都如实 toast。
 */

/** 权限的人类可读说明与危险级（与后端 Permission::is_dangerous 同表）。 */
const PERMISSION_META: Record<string, { key: string; dangerous: boolean }> = {
  'fs:read': { key: 'permFsRead', dangerous: false },
  'fs:write': { key: 'permFsWrite', dangerous: true },
  'git:read': { key: 'permGitRead', dangerous: false },
  'git:write': { key: 'permGitWrite', dangerous: true },
  'net:github': { key: 'permNetGithub', dangerous: true },
  'ui:panel': { key: 'permUiPanel', dangerous: false },
  'ui:command': { key: 'permUiCommand', dangerous: false },
  'ui:toast': { key: 'permUiToast', dangerous: false },
  'settings:read': { key: 'permSettingsRead', dangerous: false },
  'settings:write': { key: 'permSettingsWrite', dangerous: false },
};

function StateBadge({ state }: { readonly state: PluginSummary['state'] }): React.JSX.Element {
  const { t } = useTranslation('shell');
  const key =
    state === 'enabled' ? 'stateEnabled' : state === 'crashed' ? 'stateCrashed' : 'stateDisabled';
  const tone = state === 'enabled' ? 'success' : state === 'crashed' ? 'danger' : 'neutral';
  return <Badge tone={tone}>{t(`plugins.${key}`)}</Badge>;
}

function PermissionRow({
  permission,
  granted,
  usage,
  onRevoke,
}: {
  readonly permission: string;
  readonly granted: boolean;
  readonly usage?: number;
  readonly onRevoke?: (permission: string) => void;
}): React.JSX.Element {
  const { t } = useTranslation('shell');
  const meta = PERMISSION_META[permission];
  return (
    <div className="flex items-center justify-between gap-2 text-13">
      <div className="flex flex-col gap-0.5">
        <span className="flex items-center gap-1.5">
          <span className="font-mono">{permission}</span>
          {meta?.dangerous ? <Badge tone="warning">{t('plugins.dangerousBadge')}</Badge> : null}
          {!granted ? <span className="text-fg-subtle">{t('plugins.permNotGranted')}</span> : null}
        </span>
        <span className="text-12 text-fg-subtle">
          {t(`plugins.${meta?.key ?? 'permUnknown'}`)}
          {granted && usage !== undefined && usage > 0
            ? ` · ${t('plugins.permUsage', { n: usage })}`
            : ''}
        </span>
      </div>
      {granted && onRevoke !== undefined ? (
        <Button size="sm" variant="ghost" onClick={() => onRevoke(permission)}>
          {t('plugins.permRevoke')}
        </Button>
      ) : null}
    </div>
  );
}

/** 启用前的逐项授权对话框（缺失权限的授予 + 危险权限的额外确认）。 */
function GrantDialog({
  plugin,
  onClose,
}: {
  readonly plugin: PluginSummary;
  readonly onClose: (granted: boolean) => void;
}): React.JSX.Element {
  const { t } = useTranslation('shell');
  const { show } = useAppError();
  const queryClient = useQueryClient();
  const missing = plugin.declaredPermissions.filter(
    (permission) => !plugin.grantedPermissions.includes(permission),
  );
  const [checked, setChecked] = useState<ReadonlySet<string>>(new Set(missing));
  const [dangerUnderstood, setDangerUnderstood] = useState(false);
  const dangerousChecked = [...checked].some(
    (permission) => PERMISSION_META[permission]?.dangerous,
  );
  const canConfirm = checked.size > 0 && (!dangerousChecked || dangerUnderstood);

  const confirm = useMutation({
    mutationFn: async () => {
      await pluginGrant(plugin.id, [...checked]);
      await pluginSetEnabled(plugin.id, true);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: [PLUGINS_QUERY_KEY] });
      pushToast({ title: t('plugins.grantDone', { name: plugin.name }), tone: 'success' });
      onClose(true);
    },
    onError: show,
  });

  return (
    <AlertDialog open onOpenChange={(open) => !open && onClose(false)}>
      <AlertDialogContent impact={t('plugins.grantImpact', { name: plugin.name })}>
        <AlertDialogHeader>
          <AlertDialogTitle>{t('plugins.grantTitle', { name: plugin.name })}</AlertDialogTitle>
        </AlertDialogHeader>
        <p className="text-12 text-fg-subtle">{t('plugins.grantHint')}</p>
        <div
          className="flex flex-col gap-2"
          role="group"
          aria-label={t('plugins.permissionsTitle')}
        >
          {missing.map((permission) => {
            const meta = PERMISSION_META[permission];
            const isChecked = checked.has(permission);
            return (
              <label
                key={permission}
                className={`flex flex-col gap-0.5 rounded border p-2 ${
                  meta?.dangerous ? 'border-warning' : 'border-line'
                } ${isChecked ? 'bg-surface' : 'bg-surface-sunken'}`}
              >
                <span className="flex items-center gap-2">
                  <Checkbox
                    checked={isChecked}
                    onCheckedChange={(next) => {
                      const nextSet = new Set(checked);
                      if (next === true) {
                        nextSet.add(permission);
                      } else {
                        nextSet.delete(permission);
                      }
                      setChecked(nextSet);
                    }}
                    aria-label={permission}
                  />
                  <span className="font-mono text-13">{permission}</span>
                  {meta?.dangerous ? (
                    <Badge tone="warning">{t('plugins.dangerousBadge')}</Badge>
                  ) : null}
                </span>
                <span className="pl-6 text-12 text-fg-subtle">
                  {t(`plugins.${meta?.key ?? 'permUnknown'}`)}
                </span>
              </label>
            );
          })}
        </div>
        {dangerousChecked ? (
          <label className="flex items-center gap-2 text-13 text-warning">
            <Checkbox
              checked={dangerUnderstood}
              onCheckedChange={(next) => setDangerUnderstood(next === true)}
              aria-label={t('plugins.grantDangerousConfirm')}
            />
            {t('plugins.grantDangerousConfirm')}
          </label>
        ) : null}
        <AlertDialogFooter>
          <AlertDialogCancel onClick={() => onClose(false)}>
            {t('plugins.grantCancel')}
          </AlertDialogCancel>
          <AlertDialogAction
            disabled={!canConfirm || confirm.isPending}
            onClick={(event) => {
              event.preventDefault();
              confirm.mutate();
            }}
          >
            {t('plugins.grantConfirm')}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

/** 单个插件的日志区（展开时才拉取）。 */
function PluginLogs({ id }: { readonly id: string }): React.JSX.Element {
  const { t } = useTranslation('shell');
  const logs = useQuery({
    queryKey: [PLUGINS_QUERY_KEY, 'logs', id],
    queryFn: () => pluginLogs(id, 100),
  });
  if ((logs.data?.length ?? 0) === 0) {
    return <p className="text-12 text-fg-subtle">{t('plugins.logsEmpty')}</p>;
  }
  const formatter = new Intl.DateTimeFormat(undefined, {
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  });
  return (
    <ul className="flex flex-col gap-1 font-mono text-12">
      {logs.data?.map((entry, index) => (
        <li key={`${entry.timeMs}-${index}`} className="flex gap-2">
          <span className="text-fg-subtle">{formatter.format(new Date(entry.timeMs))}</span>
          <span
            className={
              entry.level === 3
                ? 'text-danger'
                : entry.level === 2
                  ? 'text-warning'
                  : 'text-fg-muted'
            }
          >
            {entry.message}
          </span>
        </li>
      ))}
    </ul>
  );
}

export function PluginSettingsPage() {
  const { t } = useTranslation('shell');
  const { show } = useAppError();
  const queryClient = useQueryClient();
  const plugins = useQuery({
    queryKey: [PLUGINS_QUERY_KEY],
    queryFn: pluginList,
  });
  const [grantTarget, setGrantTarget] = useState<PluginSummary | null>(null);
  const [pendingRemoval, setPendingRemoval] = useState<PluginSummary | null>(null);
  const [logsOpen, setLogsOpen] = useState<string | null>(null);
  const [devDir, setDevDir] = useState('');

  const invalidate = () => {
    void queryClient.invalidateQueries({ queryKey: [PLUGINS_QUERY_KEY] });
  };

  const setEnabled = useMutation({
    mutationFn: ({ id, enabled }: { id: string; enabled: boolean }) =>
      pluginSetEnabled(id, enabled),
    onSuccess: (_data, variables) => {
      invalidate();
      pushToast({
        title: variables.enabled ? t('plugins.enabledToast') : t('plugins.disabledToast'),
        tone: 'success',
      });
    },
    onError: show,
  });

  const revoke = useMutation({
    mutationFn: ({ id, permission }: { id: string; permission: string }) =>
      pluginRevoke(id, permission),
    onSuccess: () => {
      invalidate();
      pushToast({ title: t('plugins.revokedToast'), tone: 'success' });
    },
    onError: show,
  });

  const uninstall = useMutation({
    mutationFn: (plugin: PluginSummary) => pluginUninstall(plugin.id),
    onSuccess: (dirDeleted) => {
      invalidate();
      pushToast({
        title: dirDeleted ? t('plugins.uninstalledWithDir') : t('plugins.uninstalledKept'),
        tone: 'success',
      });
    },
    onError: show,
  });

  const reload = useMutation({
    mutationFn: (id: string) => pluginReload(id),
    onSuccess: () => {
      invalidate();
      pushToast({ title: t('plugins.reloadedToast'), tone: 'success' });
    },
    onError: show,
  });

  const install = useMutation({
    mutationFn: () => pluginInstallFromDir(devDir),
    onSuccess: (report) => {
      invalidate();
      setDevDir('');
      pushToast({
        title: t('plugins.installOk', { sha: report.sha256.slice(0, 12) }),
        tone: 'success',
      });
    },
    onError: show,
  });

  /** 启用入口：缺失授权才走对话框，否则直接启用。 */
  const handleEnable = (plugin: PluginSummary): void => {
    const missing = plugin.declaredPermissions.filter(
      (permission) => !plugin.grantedPermissions.includes(permission),
    );
    if (missing.length === 0) {
      setEnabled.mutate({ id: plugin.id, enabled: true });
      return;
    }
    setGrantTarget(plugin);
  };

  return (
    <section className="flex flex-col gap-4">
      <header className="flex flex-col gap-1">
        <h1 className="text-20 font-semibold tracking-tight">{t('pages.settingsPlugins.title')}</h1>
        <p className="text-13 text-fg-muted">{t('pages.settingsPlugins.description')}</p>
        <p className="rounded-lg border border-warning/40 bg-surface p-3 text-12 text-fg-muted">
          {t('plugins.securityNotice')}
        </p>
      </header>

      {plugins.data !== undefined && plugins.data.length === 0 ? (
        <div className="flex flex-col gap-1 rounded-lg border border-line bg-surface p-4">
          <p className="text-14 font-medium">{t('plugins.emptyTitle')}</p>
          <p className="text-13 text-fg-muted">{t('plugins.emptyHint')}</p>
        </div>
      ) : null}

      <div className="flex flex-col gap-3">
        {plugins.data?.map((plugin) => {
          const usage = new Map(plugin.permissionUsage);
          return (
            <div
              key={plugin.id}
              className="flex flex-col gap-3 rounded-lg border border-line bg-surface p-4"
              data-testid={`plugin-card-${plugin.id}`}
            >
              <div className="flex flex-wrap items-center justify-between gap-2">
                <div className="flex items-center gap-2">
                  <span className="text-14 font-medium">{plugin.name}</span>
                  <span className="text-12 text-fg-subtle">v{plugin.version}</span>
                  <StateBadge state={plugin.state} />
                </div>
                <div className="flex items-center gap-2">
                  {plugin.state === 'enabled' ? (
                    <Button
                      size="sm"
                      variant="secondary"
                      onClick={() => setEnabled.mutate({ id: plugin.id, enabled: false })}
                    >
                      {t('plugins.actionDisable')}
                    </Button>
                  ) : (
                    <Button
                      size="sm"
                      variant="secondary"
                      disabled={plugin.state === 'crashed'}
                      onClick={() => handleEnable(plugin)}
                    >
                      {t('plugins.actionEnable')}
                    </Button>
                  )}
                  {plugin.state === 'enabled' ? (
                    <Button size="sm" variant="ghost" onClick={() => reload.mutate(plugin.id)}>
                      {t('plugins.actionReload')}
                    </Button>
                  ) : null}
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={() => setLogsOpen(logsOpen === plugin.id ? null : plugin.id)}
                  >
                    {logsOpen === plugin.id
                      ? t('plugins.actionHideLogs')
                      : t('plugins.actionShowLogs')}
                  </Button>
                  <Button size="sm" variant="ghost" onClick={() => setPendingRemoval(plugin)}>
                    {t('plugins.actionUninstall')}
                  </Button>
                </div>
              </div>
              <p className="text-13 text-fg-muted">{plugin.description}</p>
              <p className="text-12 text-fg-subtle">
                {plugin.author} · {plugin.license}
                {plugin.homepage !== undefined && plugin.homepage !== '' ? (
                  <>
                    {' · '}
                    <a
                      href={plugin.homepage}
                      target="_blank"
                      rel="noreferrer"
                      className="text-info underline"
                    >
                      {t('plugins.homepageLink')}
                    </a>
                  </>
                ) : null}
                {' · '}
                <span className="font-mono">{plugin.id}</span>
              </p>

              <div className="flex flex-col gap-2 border-t border-line pt-3">
                <span className="text-12 font-medium text-fg-muted">
                  {t('plugins.permissionsTitle')}
                </span>
                {plugin.declaredPermissions.length === 0 ? (
                  <p className="text-12 text-fg-subtle">{t('plugins.noPermissions')}</p>
                ) : (
                  plugin.declaredPermissions.map((permission) => {
                    const rowUsage = usage.get(permission);
                    const revocable = plugin.state === 'enabled';
                    return (
                      <PermissionRow
                        key={permission}
                        permission={permission}
                        granted={plugin.grantedPermissions.includes(permission)}
                        {...(rowUsage === undefined ? {} : { usage: rowUsage })}
                        {...(revocable
                          ? {
                              onRevoke: (perm: string) =>
                                revoke.mutate({ id: plugin.id, permission: perm }),
                            }
                          : {})}
                      />
                    );
                  })
                )}
              </div>

              {plugin.state === 'crashed' ? (
                <p className="text-12 text-danger">{t('plugins.crashedHint')}</p>
              ) : null}
              {logsOpen === plugin.id ? (
                <div className="border-t border-line pt-3">
                  <PluginLogs id={plugin.id} />
                </div>
              ) : null}
            </div>
          );
        })}
      </div>

      <div className="flex flex-col gap-2 rounded-lg border border-line bg-surface p-4">
        <span className="text-14 font-medium">{t('plugins.devInstallTitle')}</span>
        <span className="text-12 text-fg-subtle">{t('plugins.devInstallHint')}</span>
        <div className="flex items-center gap-2">
          <Input
            value={devDir}
            onChange={(event) => setDevDir(event.target.value)}
            placeholder={t('plugins.devInstallPathLabel')}
            aria-label={t('plugins.devInstallPathLabel')}
            className="flex-1"
          />
          <Button
            size="sm"
            variant="secondary"
            disabled={devDir.trim().length === 0 || install.isPending}
            onClick={() => install.mutate()}
          >
            {t('plugins.devInstallAction')}
          </Button>
        </div>
      </div>

      {grantTarget !== null ? (
        <GrantDialog plugin={grantTarget} onClose={() => setGrantTarget(null)} />
      ) : null}

      {pendingRemoval !== null ? (
        <AlertDialog open onOpenChange={(open) => !open && setPendingRemoval(null)}>
          <AlertDialogContent impact={t('plugins.uninstallImpactDir')}>
            <AlertDialogHeader>
              <AlertDialogTitle>
                {t('plugins.uninstallTitle', { name: pendingRemoval.name })}
              </AlertDialogTitle>
            </AlertDialogHeader>
            <p className="text-13 text-fg-muted">{t('plugins.uninstallImpactKept')}</p>
            <AlertDialogFooter>
              <AlertDialogCancel onClick={() => setPendingRemoval(null)}>
                {t('plugins.grantCancel')}
              </AlertDialogCancel>
              <AlertDialogAction
                onClick={(event) => {
                  event.preventDefault();
                  uninstall.mutate(pendingRemoval);
                  setPendingRemoval(null);
                }}
              >
                {t('plugins.uninstallConfirm')}
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      ) : null}
    </section>
  );
}
