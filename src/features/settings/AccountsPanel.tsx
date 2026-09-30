/**
 * 托管平台账号面板（T4.4）。
 *
 * # 与 CredentialsPanel 的分工
 *
 * 凭据面板回答"这个 host 用哪条凭据走 git"（自建服务、密码都在那里）；
 * 本面板回答"我以谁的托管平台身份登录"（账号模型 + Device Flow）。
 * 两者共享同一个凭据库后端：删除账号会同时删掉它名下的 keyring 条目，
 * 因此删除必须确认（AlertDialog），且不可撤销要写进确认文案。
 *
 * # 列表为什么是 Query 而不是 store
 *
 * 账号是"服务端状态"（SQLite + keyring 的事实），归 TanStack Query
 * （单一真相源约定）；登录/删除成功后只需失效 `ACCOUNTS_QUERY_KEY`。
 */
import { useState } from 'react';

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';

import { AccountLoginDialog } from '@/features/settings/AccountLoginDialog';
import { useAppError } from '@/lib/errors';
import { accountList, accountRemove } from '@/lib/ipc/accounts';
import type { Account } from '@/lib/ipc/accounts';
import { ACCOUNTS_QUERY_KEY } from '@/lib/queryKeys';
import { pushToast } from '@/stores/toastStore';

import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/ui/components/alert-dialog';
import { Button } from '@/ui/components/button';

/** 待删除确认中的账号。 */
type PendingRemoval = Account | null;

export function AccountsPanel() {
  const { t } = useTranslation('shell');
  const { show } = useAppError();
  const queryClient = useQueryClient();
  const [loginOpen, setLoginOpen] = useState(false);
  const [pendingRemoval, setPendingRemoval] = useState<PendingRemoval>(null);

  // 后端不传网络时 list 也会失败：错误交给统一展示，列表区不渲染假空态
  const accountsQuery = useQuery({
    queryKey: [ACCOUNTS_QUERY_KEY],
    queryFn: accountList,
  });

  const removeMutation = useMutation({
    mutationFn: (account: Account) => accountRemove(account.id),
    onSuccess: (_data, account) => {
      void queryClient.invalidateQueries({ queryKey: [ACCOUNTS_QUERY_KEY] });
      pushToast({
        tone: 'success',
        title: t('settings.accounts.removedToast', { login: account.login }),
      });
      setPendingRemoval(null);
    },
    onError: (error) => {
      show(error);
      setPendingRemoval(null);
    },
  });

  const accounts = accountsQuery.data ?? [];

  return (
    <section
      aria-labelledby="accounts-panel-title"
      className="flex flex-col gap-3 rounded-md border border-line bg-surface p-4"
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h2 id="accounts-panel-title" className="text-16 font-semibold">
          {t('settings.accounts.title')}
        </h2>
        <Button type="button" onClick={() => setLoginOpen(true)} data-testid="account-add">
          {t('settings.accounts.addAccount')}
        </Button>
      </div>
      <p className="text-12 text-fg-muted">{t('settings.accounts.description')}</p>

      {accountsQuery.isError ? (
        <p className="text-12 text-danger" data-testid="account-list-error">
          {t('settings.accounts.listError')}
        </p>
      ) : null}

      {accounts.length === 0 && !accountsQuery.isError ? (
        <p className="text-12 text-fg-subtle" data-testid="account-empty">
          {t('settings.accounts.empty')}
        </p>
      ) : null}

      <ul className="flex flex-col gap-2" data-testid="account-items">
        {accounts.map((account) => (
          <li
            key={account.id}
            className="flex flex-wrap items-center justify-between gap-2 rounded-sm border border-line px-3 py-2"
            data-testid="account-item"
          >
            <div className="flex min-w-0 flex-col">
              <span className="text-13 font-medium">
                {account.login}
                <span className="ml-2 font-mono text-12 text-fg-subtle">{account.host}</span>
              </span>
              <span className="truncate text-12 text-fg-subtle">
                {account.scopes.length > 0
                  ? t('settings.accounts.scopes', { scopes: account.scopes.join(', ') })
                  : t('settings.accounts.noScopes')}
              </span>
            </div>
            <Button
              type="button"
              variant="secondary"
              onClick={() => setPendingRemoval(account)}
              data-testid={`account-remove-${account.login}`}
            >
              {t('settings.accounts.remove')}
            </Button>
          </li>
        ))}
      </ul>

      <AccountLoginDialog open={loginOpen} onOpenChange={setLoginOpen} />

      <AlertDialog
        open={pendingRemoval !== null}
        onOpenChange={(open) => setPendingRemoval(open ? pendingRemoval : null)}
      >
        <AlertDialogContent impact={t('settings.accounts.removeImpact')}>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t('settings.accounts.removeTitle', { login: pendingRemoval?.login ?? '' })}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t('settings.accounts.removeDescription', { host: pendingRemoval?.host ?? '' })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel data-testid="account-remove-cancel">
              {t('common:actions.cancel')}
            </AlertDialogCancel>
            <AlertDialogAction
              data-testid="account-remove-confirm"
              onClick={(event) => {
                event.preventDefault();
                if (pendingRemoval !== null) {
                  removeMutation.mutate(pendingRemoval);
                }
              }}
            >
              {t('settings.accounts.removeConfirm')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </section>
  );
}
