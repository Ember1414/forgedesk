/**
 * 限流状态横幅（T4.10 降级 UI）。
 *
 * # 只读本地快照，零网络成本
 *
 * 快照由 HTTP 底座逐响应捕获，命令读取是纯本地操作——挂载时取一次、
 * 之后每 30 秒轮询即可跟随最新状态。主动刷新（`GET /rate_limit`）由
 * 用户点按钮触发，不自动轮询网络。
 *
 * # 判定与文案
 *
 * `remaining === 0` → 耗尽档：告知"正在展示缓存数据 + 重置时间"
 * （缓存回退本身发生在 HTTP 底座，页面上看到的数据就是降级结果）；
 * 剩余 ≤ 50 → 提醒档：只报数字，不打扰。快照不存在（会话内还没发过
 * 请求）或额度充足时，横幅一个像素都不占。
 */
import { useCallback, useEffect, useState } from 'react';

import { useTranslation } from 'react-i18next';

import { useAppError } from '@/lib/errors';
import { repoRateLimitRefresh, repoRateLimitState } from '@/lib/ipc';
import type { RateLimitSnapshot } from '@/lib/ipc';

import { Button } from '@/ui/components/button';

/** 与其他 GitHub 页一致的站点常量。 */
const HOST = 'github.com';

/** 轮询间隔（ms）：本地命令，便宜。 */
const POLL_INTERVAL_MS = 30_000;

/** 剩余额度低于该值时显示提醒档横幅。 */
const NEAR_LIMIT_THRESHOLD = 50;

export type RateLimitTone = 'exhausted' | 'nearLimit';

export function RateLimitBanner() {
  const { t } = useTranslation('shell');
  const { show } = useAppError();
  const [snapshot, setSnapshot] = useState<RateLimitSnapshot | null>(null);
  const [refreshing, setRefreshing] = useState(false);

  const poll = useCallback(() => {
    void repoRateLimitState()
      .then((state) => setSnapshot(state))
      .catch(() => undefined);
  }, []);

  useEffect(() => {
    poll();
    const timer = window.setInterval(poll, POLL_INTERVAL_MS);
    return () => window.clearInterval(timer);
  }, [poll]);

  const refresh = () => {
    setRefreshing(true);
    void repoRateLimitRefresh(HOST)
      .then((state) => setSnapshot(state))
      .catch((raw: unknown) => show(raw))
      .finally(() => setRefreshing(false));
  };

  if (snapshot === null) {
    return null;
  }
  const tone: RateLimitTone | null =
    snapshot.remaining === 0
      ? 'exhausted'
      : snapshot.remaining <= NEAR_LIMIT_THRESHOLD
        ? 'nearLimit'
        : null;
  if (tone === null) {
    return null;
  }

  const resetAt = new Date(snapshot.resetUnixSecs * 1000).toLocaleTimeString();

  return (
    <div
      role="status"
      className={`flex flex-wrap items-center gap-2 rounded-md border px-3 py-2 text-13 ${
        tone === 'exhausted' ? 'border-danger bg-danger/10' : 'border-line bg-surface'
      }`}
      data-testid="rate-limit-banner"
    >
      <span className={tone === 'exhausted' ? 'text-fg' : 'text-fg-muted'}>
        {tone === 'exhausted'
          ? t('github.rateLimit.exhausted')
          : t('github.rateLimit.nearLimit', {
              remaining: snapshot.remaining,
              limit: snapshot.limit,
            })}
      </span>
      {tone === 'exhausted' ? (
        <span className="text-fg-muted" data-testid="rate-limit-reset">
          {t('github.rateLimit.resetAt', { time: resetAt })}
        </span>
      ) : null}
      <Button
        type="button"
        variant="secondary"
        className="ml-auto"
        disabled={refreshing}
        onClick={refresh}
        data-testid="rate-limit-refresh"
      >
        {t('github.rateLimit.refresh')}
      </Button>
    </div>
  );
}
