import { CircleUser, RefreshCw } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useQuery } from '@tanstack/react-query';
import { useNavigate } from 'react-router-dom';

import { GlobalSearch } from '@/app/shell/GlobalSearch';
import { RepoSwitcher } from '@/app/shell/RepoSwitcher';
import { accountList } from '@/lib/ipc/accounts';
import { isTauriRuntime, updateCheck } from '@/lib/ipc';
import { ACCOUNTS_QUERY_KEY } from '@/lib/queryKeys';
import {
  DEFAULT_UPDATE_AUTO_CHECK,
  UPDATE_AUTO_CHECK_KEY,
  useSettingsStore,
} from '@/stores/settingsStore';
import { cn } from '@/lib/utils';

/**
 * 顶部标题栏：仓库切换器 + 全局搜索入口 + 更新状态 + 账号入口。
 *
 * 布局决策（原创，未参考任何竞品）：
 *   采用「左：身份与上下文（应用标 + 当前仓库）｜中：检索入口｜右：账号与更新」的单行结构。
 *   理由是把"我在哪个仓库"（上下文）与"我要找什么"（动作）放在注意力的左侧到中部，
 *   而把低频的账号/更新放到最右，符合从左到右递减的使用频率。
 *
 * # 右上角两枚元素各自的职责（2026-10-08 补齐，此前都是摆设）
 *
 * - **账号位**：此前是一个没有 `onClick` 的按钮——用户点它没有任何反应，
 *   看起来像坏了。现在它读与账号面板同一个缓存（`accounts` 键），登录后显示
 *   用户名首字/头像，点击进入"代码托管账号"设置页；未登录时提示去哪登录。
 * - **更新状态**：此前是写死的"已是最新版本"。现在复用更新横幅那条查询
 *   （`['app', 'update']`，键相同即共享缓存，不多发请求），并且**未配置更新源
 *   时如实说明"本地构建"**——不能对自编译的用户假装已经查过最新版。
 *
 * 应用标记目前是一个纯 CSS 的抽象几何块（品牌标导出为前端资源后替换），
 * 不使用任何第三方标识（红线 R2）。
 */
export function TitleBar() {
  const { t } = useTranslation('shell');
  const navigate = useNavigate();
  const inTauri = isTauriRuntime();

  // 与账号面板共用同一条缓存记录（键即 `accounts`）：标题栏不额外发请求，
  // 登录/删除后由那一侧 invalidate，这里自然跟着更新。
  // staleTime 给 60s 而不是默认 0：账号列举会碰系统 keyring（Windows Credential
  // Manager），而它只在用户登录/登出时变化——每次窗口聚焦都重读一遍没有收益。
  const accountQuery = useQuery({
    queryKey: [ACCOUNTS_QUERY_KEY],
    queryFn: accountList,
    // 非宿主环境（浏览器里跑 `pnpm dev`）没有后端，直接跳过
    enabled: inTauri,
    staleTime: 60_000,
  });
  const account = accountQuery.data?.[0];

  // 与 UpdateBanner 同一查询键：这里只**读**缓存/触发一次检查，不引入第二条数据通路
  const settingsLoaded = useSettingsStore((state) => state.loaded);
  const autoCheck = useSettingsStore((state) =>
    state.getJson<boolean>(UPDATE_AUTO_CHECK_KEY, DEFAULT_UPDATE_AUTO_CHECK),
  );
  const canCheckUpdate = inTauri && settingsLoaded && autoCheck;
  const updateQuery = useQuery({
    queryKey: ['app', 'update'],
    queryFn: updateCheck,
    enabled: canCheckUpdate,
    staleTime: Number.POSITIVE_INFINITY,
    retry: 0,
    // 长驻应用也要周期复查：启动那一刻还没有新版、半小时后发了新版，
    // 不复查就永远看不到（2026-10-08 "明明发了 1.1.1 却没有提示"的场景之一）
    refetchInterval: 30 * 60 * 1000,
  });

  const updateBadge = (() => {
    const data = updateQuery.data;
    // 检查失败此前是"徽标直接消失"——用户无从分辨"没新版"和"没查成"。
    // 现在显式给出失败态，并且徽标可点击重新检查
    if (updateQuery.isError) {
      return { text: t('titleBar.update.checkFailed'), tone: 'danger' as const };
    }
    if (data === undefined) {
      return null;
    }
    if (!data.configured) {
      return { text: t('titleBar.update.localBuild'), tone: 'muted' as const };
    }
    if (data.update !== null) {
      return {
        text: t('titleBar.update.available', { version: data.update.version }),
        tone: 'brand' as const,
      };
    }
    return { text: t('titleBar.update.latest'), tone: 'muted' as const };
  })();

  return (
    <header className="flex h-12 shrink-0 items-center gap-3 border-b border-line bg-surface px-3">
      <div className="flex shrink-0 items-center gap-2">
        {/* 抽象锻炉记号：方形炉体 + 右上角的火花缺口。纯几何，无第三方标识元素。 */}
        <span aria-hidden="true" className="relative size-5 rounded-sm bg-brand">
          <span className="absolute right-0 top-0 size-2 rounded-bl-sm bg-spark" />
        </span>
        <span className="hidden text-14 font-semibold tracking-tight sm:inline">
          {t('common:app.name')}
        </span>
      </div>

      <RepoSwitcher />

      <div className="flex min-w-0 flex-1 justify-center px-2">
        <GlobalSearch />
      </div>

      <div className="flex shrink-0 items-center gap-1">
        {updateBadge === null ? null : (
          <button
            type="button"
            className={cn(
              'fd-transition hidden items-center gap-1.5 rounded-sm px-2 py-1 text-12 lg:flex',
              'hover:bg-surface-sunken',
              updateBadge.tone === 'brand' && 'text-brand',
              updateBadge.tone === 'danger' && 'text-danger',
              updateBadge.tone === 'muted' && 'text-fg-subtle',
            )}
            title={t('titleBar.update.recheck')}
            data-testid="titlebar-update-badge"
            onClick={() => void updateQuery.refetch()}
          >
            <RefreshCw
              aria-hidden="true"
              className={cn('size-3.5', updateQuery.isFetching && 'animate-spin')}
            />
            {updateBadge.text}
          </button>
        )}

        <button
          type="button"
          aria-label={t('titleBar.account.label')}
          title={
            account === undefined
              ? t('titleBar.account.signedOutHint')
              : t('titleBar.account.signedInHint', { login: account.login })
          }
          data-testid="titlebar-account"
          onClick={() => {
            void navigate('/settings/github');
          }}
          className={cn(
            'fd-transition flex size-8 items-center justify-center overflow-hidden rounded-md border border-line',
            'text-fg-subtle hover:border-line-strong hover:bg-surface-sunken hover:text-fg',
          )}
        >
          {account?.avatarUrl !== undefined && account.avatarUrl !== '' ? (
            // 头像来自托管平台（外链图片）：加载失败时下面的兜底首字仍然可见
            <img
              src={account.avatarUrl}
              alt=""
              className="size-8 object-cover"
              referrerPolicy="no-referrer"
            />
          ) : account !== undefined ? (
            <span className="text-12 font-medium">{account.login.slice(0, 1).toUpperCase()}</span>
          ) : (
            <CircleUser aria-hidden="true" className="size-4" />
          )}
        </button>
      </div>
    </header>
  );
}
