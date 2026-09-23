import { useQuery } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';

import { appVersion, isTauriRuntime } from '@/lib/ipc';
import { cn } from '@/lib/utils';

/**
 * 运行时信息徽标 —— M0/T0.1 的 IPC 通路验证组件。
 *
 * 它同时验证了三件事：
 *   1. 前端 → Tauri 命令 → 前端 的完整链路可用（`app_version` 能返回数据）。
 *   2. TanStack Query 的接入正确（加载态、错误态、成功态都能渲染）。
 *   3. 设计 token 在真实业务组件中可用（不写任何硬编码色值）。
 *
 * 在普通浏览器（`pnpm dev` 未套壳 Tauri）中运行时，会明确提示 IPC 不可用，
 * 而不是抛错或显示空白。
 *
 * 文案全部走 i18n（T0.6 起）：本组件会出现在设置页的"高级"里，
 * 是用户可见界面，不能有硬编码中文。
 */
export function VersionBadge() {
  const { t } = useTranslation('shell');
  const inTauri = isTauriRuntime();

  const { data, isPending, isError } = useQuery({
    queryKey: ['app', 'version'],
    queryFn: appVersion,
    enabled: inTauri,
    staleTime: Number.POSITIVE_INFINITY,
    retry: 0,
  });

  const state: 'loading' | 'ready' | 'error' | 'browser' = !inTauri
    ? 'browser'
    : isPending
      ? 'loading'
      : isError
        ? 'error'
        : 'ready';

  return (
    <div
      className={cn(
        'flex flex-wrap items-center gap-x-3 gap-y-1 rounded-md border px-3 py-2 text-12',
        state === 'ready' && 'border-line bg-surface text-fg-muted',
        state === 'loading' && 'border-line bg-surface text-fg-subtle',
        state === 'error' && 'border-danger bg-surface text-danger',
        state === 'browser' && 'border-warning bg-surface text-warning',
      )}
      role="status"
      aria-live="polite"
    >
      <span className="font-medium">{t('runtime.label')}</span>

      {state === 'ready' && data !== undefined ? (
        <>
          <span className="font-mono">
            v{data.version}
            <span className="text-fg-subtle"> · </span>
            {data.target}
            <span className="text-fg-subtle"> · </span>
            {data.profile}
          </span>
          <span className="font-mono text-fg-subtle" title={t('runtime.commitTitle')}>
            {data.gitSha}
          </span>
          <span className="text-success">{t('runtime.ipcOk')}</span>
        </>
      ) : null}

      {state === 'loading' ? <span>{t('runtime.loading')}</span> : null}

      {state === 'error' ? <span>{t('runtime.error')}</span> : null}

      {state === 'browser' ? <span>{t('runtime.browserOnly')}</span> : null}
    </div>
  );
}
