import { useTranslation } from 'react-i18next';

import { AccountsPanel } from '@/features/settings/AccountsPanel';
import { CredentialsPanel } from '@/features/settings/CredentialsPanel';
import { SshKeysPanel } from '@/features/settings/SshKeysPanel';

/**
 * 代码托管账号设置。
 *
 * # 三块内容的分工
 *
 * - 账号面板（T4.4）：托管平台身份——Device Flow / PAT 登录、多账号管理；
 * - 凭据面板（T2.7）：git 网络操作用哪条凭据（自建服务、密码也在那里）；
 * - SSH 面板：另一条认证路径，令牌对 git@host 远端毫无作用。
 *
 * 三者共享同一个凭据库后端，因此"删除账号"会连带删掉它名下的 keyring
 * 条目（确认文案里写明不可撤销）。
 */
export function GitHubSettingsPage() {
  const { t } = useTranslation('shell');

  return (
    <section className="flex h-full flex-col gap-4">
      <header className="flex flex-col gap-1">
        <h1 className="text-20 font-semibold tracking-tight">{t('pages.settingsGithub.title')}</h1>
        <p className="text-13 text-fg-muted">{t('pages.settingsGithub.description')}</p>
      </header>

      <AccountsPanel />

      <CredentialsPanel />

      {/* SSH 是另一条认证路径：令牌对 git@host 远端毫无作用，因此单独一组 */}
      <SshKeysPanel />
    </section>
  );
}
