import { useEffect, useState } from 'react';

import { useMutation, useQuery } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';

import { isTauriRuntime, listenUpdateProgress, updateCheck, updateInstall } from '@/lib/ipc';
import type { UpdateProgressPayload } from '@/lib/ipc';
import { useAppError } from '@/lib/errors';
import {
  DEFAULT_UPDATE_AUTO_CHECK,
  UPDATE_AUTO_CHECK_KEY,
  UPDATE_SKIPPED_VERSION_KEY,
  useSettingsStore,
} from '@/stores/settingsStore';
import { Button } from '@/ui/components/button';

/**
 * 新版本提示横幅（T7.1）。
 *
 * # 什么时候**不**出现
 *
 * 未运行在 Tauri 宿主、设置尚未加载、用户在设置里关闭了自动检查、本构建未配置更新源、
 * 没有新版本、该版本被用户"跳过"、本次会话已"稍后提醒"——任何一条成立就不渲染。
 * 提示条的价值来自它稀少：一条永远消不掉的提示会被学会无视。
 *
 * # 三个动作的分工
 *
 * | 动作 | 作用范围 | 存哪 |
 * | --- | --- | --- |
 * | 稍后提醒 | 仅本次会话 | 组件状态（不落盘） |
 * | 跳过此版本 | 只跳过这**一个**版本号，发下一个版本仍提示 | 设置 `update.skippedVersion` |
 * | 立即更新 | 交给后端下载安装 | — |
 *
 * "跳过"刻意不做成"永久忽略更新"：那会让用户再也收不到任何提示，属于另一种产品决策。
 *
 * # 为什么"立即更新"要把版本号带上
 *
 * 后端会校验"要装的版本 == 本次检查到的版本"（见 `docs/API.md`）。界面上的信息可能已经过期
 * （期间又发布了新版本），带上版本号让后端能拒绝并让界面重新检查，而不是装上一个用户没看过的版本。
 */
export function UpdateBanner() {
  const { t } = useTranslation('shell');
  const { show } = useAppError();
  const inTauri = isTauriRuntime();
  const [dismissed, setDismissed] = useState(false);
  const [progress, setProgress] = useState<UpdateProgressPayload | null>(null);

  // 选择器里调 `getJson`（而不是拿函数引用）：zustand 只在选择器结果变化时重渲染，
  // 因此在设置页改开关/清跳过时，这条横幅能立刻跟上（见 settingsStore 的说明）。
  const settingsLoaded = useSettingsStore((state) => state.loaded);
  const setJson = useSettingsStore((state) => state.setJson);
  const autoCheck = useSettingsStore((state) =>
    state.getJson<boolean>(UPDATE_AUTO_CHECK_KEY, DEFAULT_UPDATE_AUTO_CHECK),
  );
  const skippedVersion = useSettingsStore((state) =>
    state.getJson<string | null>(UPDATE_SKIPPED_VERSION_KEY, null),
  );

  // 设置没加载完就不查：否则用户明明关掉了自动检查，仍会在每次启动时发一次请求
  const canCheck = inTauri && settingsLoaded && autoCheck;

  const check = useQuery({
    queryKey: ['app', 'update'],
    queryFn: updateCheck,
    enabled: canCheck,
    staleTime: Number.POSITIVE_INFINITY,
    retry: 0,
  });

  // 进度事件订阅：卸载时必须 unlisten（否则监听器泄漏、进度回调叠加多次）
  useEffect(() => {
    if (!inTauri) {
      return;
    }
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    void listenUpdateProgress((payload) => {
      setProgress(payload);
    })
      .then((off) => {
        if (cancelled) {
          off();
        } else {
          unlisten = off;
        }
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [inTauri]);

  const install = useMutation({
    mutationFn: (version: string) => updateInstall(version),
    onError: (error) => {
      // 失败后清掉进度，让按钮回到"立即更新"可再试（错误详情由统一 Toast 呈现）
      setProgress(null);
      show(error);
    },
  });

  const available = check.data?.update ?? null;
  if (
    !canCheck ||
    available === null ||
    dismissed ||
    // 用户跳过的是"这一个版本"：发下一个版本时仍要提示
    available.version === skippedVersion
  ) {
    return null;
  }

  const percent =
    progress?.total != null && progress.total > 0
      ? Math.min(100, Math.round((progress.received / progress.total) * 100))
      : null;
  const statusText = install.isPending
    ? progress?.phase === 'installing'
      ? t('updateBanner.installingPhase')
      : percent !== null
        ? t('updateBanner.downloading', { percent })
        : t('updateBanner.downloadingUnknown')
    : null;

  return (
    <div
      role="status"
      className="border-line bg-brand-subtle text-brand flex flex-wrap items-center justify-between gap-2 border-b px-3 py-1.5 text-12"
    >
      <span>{t('updateBanner.available', { version: available.version })}</span>

      {statusText !== null ? <span className="opacity-80">{statusText}</span> : null}

      <div className="flex items-center gap-1.5">
        <Button
          size="sm"
          variant="ghost"
          disabled={install.isPending}
          onClick={() => {
            setDismissed(true);
          }}
        >
          {t('updateBanner.later')}
        </Button>
        <Button
          size="sm"
          variant="ghost"
          disabled={install.isPending}
          onClick={() => {
            void setJson(UPDATE_SKIPPED_VERSION_KEY, available.version).catch(show);
          }}
        >
          {t('updateBanner.skip')}
        </Button>
        <Button
          size="sm"
          disabled={install.isPending}
          onClick={() => {
            install.mutate(available.version);
          }}
        >
          {install.isPending ? t('updateBanner.installing') : t('updateBanner.install')}
        </Button>
      </div>
    </div>
  );
}
