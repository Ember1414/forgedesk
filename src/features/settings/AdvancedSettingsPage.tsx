import { useTranslation } from 'react-i18next';

import { LogViewer } from '@/features/logs/LogViewer';
import { AuditHistoryPanel } from '@/features/settings/AuditHistoryPanel';
import { UpdateSettingsSection } from '@/features/settings/UpdateSettingsSection';
import { Button } from '@/ui/components/button';
import { logsOpen } from '@/lib/ipc';
import { useAppError } from '@/lib/errors';

/**
 * 高级设置页。
 *
 * T0.8 起它不是骨架页：这里是"日志"的落点——
 * 用户可以打开日志目录，或就地看到最近若干行（**已脱敏**），
 * 出问题时不必去翻隐藏目录，也不必复制一大段控制台输出。
 *
 * 保留策略的说明也放在这里：用户有权知道"日志会占多少空间、留多久、会不会上传"，
 * 这属于隐私承诺的一部分（本地生成、不上传）。
 */
export function AdvancedSettingsPage() {
  const { t } = useTranslation('shell');
  const { show } = useAppError();

  return (
    <section className="flex flex-col gap-4">
      <header className="flex flex-col gap-1">
        <h1 className="text-20 font-semibold tracking-tight">
          {t('pages.settingsAdvanced.title')}
        </h1>
        <p className="text-13 text-fg-muted">{t('pages.settingsAdvanced.description')}</p>
      </header>

      <div className="flex min-h-0 flex-col gap-3 rounded-lg border border-line bg-surface p-4">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="flex max-w-xl flex-col gap-0.5">
            <span className="text-14 font-medium">{t('settings.advanced.logsTitle')}</span>
            <span className="text-12 text-fg-subtle">{t('settings.advanced.logsPolicy')}</span>
          </div>
          <Button
            variant="secondary"
            onClick={() => {
              void logsOpen().catch(show);
            }}
          >
            {t('settings.advanced.openLogDirectory')}
          </Button>
        </div>

        {/* 就地查看最近日志：高度固定，避免长日志把设置页撑成一篇文档 */}
        <LogViewer className="h-72" />
      </div>

      {/* 更新（T7.1）：自动检查开关 + 被跳过的版本 */}
      <UpdateSettingsSection />

      {/* 操作历史（T1.11）：每一次写操作都留了记录，这里让用户真的看得到 */}
      <AuditHistoryPanel />
    </section>
  );
}
