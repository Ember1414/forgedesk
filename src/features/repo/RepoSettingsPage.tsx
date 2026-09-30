/**
 * 仓库设置页（T4.5）：当前是"绑定账号"面板。
 *
 * # 绑定解决什么问题
 *
 * 同一个 host 登录了多个账号时（工作号 + 个人号），fetch/pull/push 与
 * 平台 API 用**哪一个**身份需要按仓库记住——这就是 `repo_account_binding_*`
 * 的仓库级设置。解析顺序：绑定的账号 → URL 里的用户名 → 最早登录的账号
 * （后端 `HostRepoService`，docs/API.md「远端仓库与账号绑定」节）。
 *
 * # 即点即生效
 *
 * 绑定可逆且无远端副作用，因此不做"编辑-保存"两段式：点账号即绑定，
 * 点"不绑定"即解除。成功/失败都给出明确反馈；列表为空时引导去全局
 * 设置登录账号（绑定没有可选对象）。
 */
import { useCallback, useEffect, useRef, useState } from 'react';

import { useTranslation } from 'react-i18next';
import { useNavigate, useParams } from 'react-router-dom';

import { useAppError, type NormalizedError } from '@/lib/errors';
import { accountList, repoAccountBindingGet, repoAccountBindingSet } from '@/lib/ipc';
import type { Account } from '@/lib/ipc';
import { pushToast } from '@/stores/toastStore';

import { Button } from '@/ui/components/button';
import { ErrorState } from '@/ui/components/error-state';

export function RepoSettingsPage() {
  const { t } = useTranslation('shell');
  // 仓库 id 来自路由段（后端按它读写仓库级设置）；非法段落直接报错
  const params = useParams();
  const repoId = Number(params.repoId);
  const { show } = useAppError();
  const navigate = useNavigate();

  const [accounts, setAccounts] = useState<readonly Account[]>([]);
  const repoIdValid = Number.isInteger(repoId) && params.repoId !== undefined;
  const [accountsError, setAccountsError] = useState<NormalizedError | null>(null);
  const [bound, setBound] = useState<Account | null>(null);
  const [busy, setBusy] = useState(false);
  // 只认最后一次加载的结果（repoId 切换时旧响应作废）
  const seqRef = useRef(0);

  const load = useCallback(async () => {
    const seq = ++seqRef.current;
    setAccountsError(null);
    try {
      const [list, binding] = await Promise.all([accountList(), repoAccountBindingGet(repoId)]);
      if (seq !== seqRef.current) {
        return;
      }
      setAccounts(list);
      setBound(binding);
    } catch (raw) {
      if (seq !== seqRef.current) {
        return;
      }
      setAccountsError(show(raw));
    }
  }, [repoId, show]);

  useEffect(() => {
    if (!repoIdValid) {
      return;
    }
    void Promise.resolve().then(() => load());
  }, [load, repoIdValid]);

  const bind = async (account: Account | null) => {
    setBusy(true);
    try {
      const next = await repoAccountBindingSet(repoId, account?.id ?? null);
      setBound(next);
      pushToast({
        tone: 'success',
        title:
          account === null
            ? t('repo.accountBinding.unboundToast')
            : t('repo.accountBinding.boundToast', {
                login: account.login,
                host: account.host,
              }),
      });
    } catch (raw) {
      show(raw);
    } finally {
      setBusy(false);
    }
  };

  if (!repoIdValid) {
    return (
      <section className="flex flex-col gap-3" data-testid="repo-settings-page">
        <ErrorState title={t('repo.settings.invalidRepo')} />
      </section>
    );
  }

  return (
    <section className="flex flex-col gap-3" data-testid="repo-settings-page">
      <header className="flex flex-col gap-1">
        <h1 className="text-20 font-semibold tracking-tight">{t('repo.settings.title')}</h1>
        <p className="text-13 text-fg-muted">{t('repo.settings.description')}</p>
      </header>

      <div
        aria-labelledby="account-binding-title"
        className="flex flex-col gap-3 rounded-md border border-line bg-surface p-4"
      >
        <h2 id="account-binding-title" className="text-16 font-semibold">
          {t('repo.accountBinding.title')}
        </h2>
        <p className="text-12 text-fg-muted">{t('repo.accountBinding.description')}</p>

        {accountsError !== null ? (
          <ErrorState
            title={t(`errors.${accountsError.code}.title`)}
            hint={t('repo.accountBinding.loadErrorHint')}
            onRetry={() => void load()}
            retryLabel={t('repo.accountBinding.retry')}
          />
        ) : null}

        {accountsError === null && accounts.length === 0 ? (
          <div className="flex flex-col gap-2">
            <p className="text-12 text-fg-subtle" data-testid="account-binding-empty">
              {t('repo.accountBinding.empty')}
            </p>
            <Button
              type="button"
              variant="secondary"
              onClick={() => void navigate('/settings/github')}
              data-testid="account-binding-sign-in-go"
            >
              {t('repo.accountBinding.signInGo')}
            </Button>
          </div>
        ) : null}

        <div
          role="radiogroup"
          aria-label={t('repo.accountBinding.title')}
          className="flex flex-col gap-1"
          data-testid="account-binding-options"
        >
          <Button
            type="button"
            variant={bound === null ? 'primary' : 'secondary'}
            aria-pressed={bound === null}
            disabled={busy}
            onClick={() => void bind(null)}
            data-testid="account-binding-none"
          >
            {t('repo.accountBinding.none')}
          </Button>
          {accounts.map((account) => (
            <Button
              key={account.id}
              type="button"
              variant={bound?.id === account.id ? 'primary' : 'secondary'}
              aria-pressed={bound?.id === account.id}
              disabled={busy}
              onClick={() => void bind(account)}
              data-testid={`account-binding-${account.login}`}
            >
              {t('repo.accountBinding.option', {
                login: account.login,
                host: account.host,
              })}
            </Button>
          ))}
        </div>
      </div>
    </section>
  );
}
