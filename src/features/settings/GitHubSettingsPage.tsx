import { useTranslation } from 'react-i18next';

import { CredentialsPanel } from '@/features/settings/CredentialsPanel';
import { SshKeysPanel } from '@/features/settings/SshKeysPanel';

/**
 * 代码托管账号设置（T2.7 起有真实内容）。
 *
 * # 为什么不再是骨架页
 *
 * T2.7 交付了凭据通道（系统凭据库 + git 注入 + 测试连接），账号面板因此有了
 * 可用的实体逻辑。仍然**未做**的是 T4.4：OAuth 设备码登录、多账号切换、
 * 与托管平台同步身份信息——这些会在本面板之上加，而不是重写它。
 *
 * 把"未做"明确写在页面上（`plannedTask` 徽标）是刻意保留的：用户找不到
 * "用 GitHub 登录"的按钮时，需要知道那是还没做，而不是坏了。
 */
export function GitHubSettingsPage() {
  const { t } = useTranslation('shell');

  return (
    <section className="flex h-full flex-col gap-4">
      <header className="flex flex-col gap-1">
        <div className="flex flex-wrap items-center gap-2">
          <h1 className="text-20 font-semibold tracking-tight">
            {t('pages.settingsGithub.title')}
          </h1>
          <span className="rounded-sm border border-line bg-surface-sunken px-2 py-0.5 font-mono text-12 text-fg-subtle">
            {t('placeholder.planned')} T4.4
          </span>
        </div>
        <p className="text-13 text-fg-muted">{t('pages.settingsGithub.description')}</p>
      </header>

      <CredentialsPanel />

      {/* SSH 是另一条认证路径：令牌对 git@host 远端毫无作用，因此单独一组 */}
      <SshKeysPanel />

      <p className="rounded-md border border-dashed border-line bg-surface p-4 text-12 text-fg-subtle">
        {t('settings.credentials.todoT44')}
      </p>
    </section>
  );
}
