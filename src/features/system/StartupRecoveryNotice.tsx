import { useState } from 'react';

import { useMutation, useQuery } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';

import { appRestart, appStartupReport, isTauriRuntime } from '@/lib/ipc';
import { useAppError } from '@/lib/errors';
import { formatDateTime } from '@/lib/i18n/intl';
import { Button } from '@/ui/components/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/ui/components/dialog';

/**
 * 启动恢复提示与安全模式标识（T7.5）。
 *
 * # 两种状态，两种呈现
 *
 * 1. **上次异常退出**（`abnormalExit`）：弹一次模态对话框，说明"这不是你的数据坏了"，
 *    并给两条出路——「继续使用」与「以安全模式重启」。用模态而不是角落提示，
 *    是因为"崩溃过"是用户此刻最需要知道的事，不该被忽略。
 * 2. **本次安全模式**（`safeMode`）：在顶栏下方给一条**常驻**横幅，
 *    提示插件与终端已被禁用，并提供「退出安全模式」（正常重启）。
 *    常驻是必要的：否则用户会以为"插件没了"。
 *
 * # 关闭即不再弹
 *
 * 对话框只弹一次：用户关掉（继续使用）后本会话不再打扰——反复弹同一个已知事实
 * 只会让人学会无视它。刷新（重启）后 `app_startup_report` 会重新给出事实。
 */
export function StartupRecoveryNotice() {
  const { t } = useTranslation('shell');
  const { show } = useAppError();
  const inTauri = isTauriRuntime();
  const [dismissed, setDismissed] = useState(false);

  const report = useQuery({
    queryKey: ['app', 'startup'],
    queryFn: appStartupReport,
    enabled: inTauri,
    staleTime: Number.POSITIVE_INFINITY,
    retry: 0,
  });

  const restart = useMutation({
    mutationFn: (safeMode: boolean) => appRestart(safeMode),
    onError: (error) => show(error),
  });

  if (!inTauri || report.data === undefined) {
    return null;
  }

  const data = report.data;
  const time = formatDateTime(data.lastExit?.detectedAtMs ?? null);

  return (
    <>
      {data.safeMode ? (
        <div
          role="status"
          className="flex flex-wrap items-center justify-between gap-2 border-b border-warning bg-surface px-3 py-1.5 text-12 text-warning"
        >
          <span>{t('startupRecovery.safeModeBadge')}</span>
          <Button
            size="sm"
            variant="secondary"
            disabled={restart.isPending}
            onClick={() => {
              restart.mutate(false);
            }}
          >
            {t('startupRecovery.exitSafeMode')}
          </Button>
        </div>
      ) : null}

      {data.abnormalExit && !dismissed ? (
        <Dialog
          open
          onOpenChange={(open) => {
            if (!open) {
              setDismissed(true);
            }
          }}
        >
          <DialogContent closeLabel={t('startupRecovery.close')}>
            <DialogHeader>
              <DialogTitle>{t('startupRecovery.abnormalTitle')}</DialogTitle>
              <DialogDescription>{t('startupRecovery.abnormalDescription')}</DialogDescription>
            </DialogHeader>

            <ul className="flex flex-col gap-1 text-12 text-fg-muted">
              <li>
                {data.lastExit?.version != null
                  ? t('startupRecovery.detailsVersion', { version: data.lastExit.version })
                  : t('startupRecovery.detailsUnknown')}
              </li>
              {time !== null ? <li>{t('startupRecovery.detailsTime', { time })}</li> : null}
              <li>{t('startupRecovery.safeModeHint')}</li>
            </ul>

            <DialogFooter>
              <Button
                variant="secondary"
                onClick={() => {
                  setDismissed(true);
                }}
              >
                {t('startupRecovery.continue')}
              </Button>
              <Button
                disabled={restart.isPending}
                onClick={() => {
                  restart.mutate(true);
                }}
              >
                {t('startupRecovery.restartSafeMode')}
              </Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      ) : null}
    </>
  );
}
