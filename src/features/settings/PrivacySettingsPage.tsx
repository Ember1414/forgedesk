import { useTranslation } from 'react-i18next';

import { systemOpenUrl } from '@/lib/ipc';
import { useAppError } from '@/lib/errors';
import { Button } from '@/ui/components/button';

/**
 * 隐私说明页（T7.6）。
 *
 * # 为什么应用内要有这一页
 *
 * 隐私承诺写在 `docs/PRIVACY.md` 里，但**打包后的用户不会去翻仓库**。
 * 把"收集了什么、发往哪里、怎么删除"放在设置里，是让承诺可被随时核对的唯一方式。
 * 内容与 `docs/PRIVACY.md` 保持一致（后者是完整版，本页是速览 + 入口）。
 *
 * # 为什么这些文案不是"法律文本"
 *
 * 逐条写清**具体行为**（不收集代码内容、令牌存系统钥匙串、日志保留 7 天），
 * 而不是抽象承诺——用户能据此验证，我们也不会在实现变化时忘了改文档。
 */

/** 完整隐私政策（仓库内 `docs/PRIVACY.md` 的线上地址）。 */
const PRIVACY_POLICY_URL = 'https://github.com/Ember1414/forgedesk/blob/main/docs/PRIVACY.md';

/** 速览条目（标题 key, 正文 key）。 */
const SECTIONS = [
  ['settings.privacy.noTelemetryTitle', 'settings.privacy.noTelemetryBody'],
  ['settings.privacy.noAiTitle', 'settings.privacy.noAiBody'],
  ['settings.privacy.credentialsTitle', 'settings.privacy.credentialsBody'],
  ['settings.privacy.logsTitle', 'settings.privacy.logsBody'],
  ['settings.privacy.requestsTitle', 'settings.privacy.requestsBody'],
  ['settings.privacy.dataTitle', 'settings.privacy.dataBody'],
] as const;

export function PrivacySettingsPage() {
  const { t } = useTranslation('shell');
  const { show } = useAppError();

  return (
    <section className="flex flex-col gap-4">
      <header className="flex flex-col gap-1">
        <h1 className="text-20 font-semibold tracking-tight">{t('pages.settingsPrivacy.title')}</h1>
        <p className="text-13 text-fg-muted">{t('pages.settingsPrivacy.description')}</p>
      </header>

      <p className="rounded-lg border border-line bg-surface p-4 text-13 text-fg-muted">
        {t('settings.privacy.summary')}
      </p>

      <div className="flex flex-col gap-3">
        {SECTIONS.map(([titleKey, bodyKey]) => (
          <div key={titleKey} className="rounded-lg border border-line bg-surface p-4">
            <h2 className="text-14 font-medium">{t(titleKey)}</h2>
            <p className="mt-1 text-13 text-fg-muted">{t(bodyKey)}</p>
          </div>
        ))}
      </div>

      <div>
        <Button
          variant="secondary"
          onClick={() => {
            void systemOpenUrl(PRIVACY_POLICY_URL).catch(show);
          }}
        >
          {t('settings.privacy.openPolicy')}
        </Button>
      </div>
    </section>
  );
}
