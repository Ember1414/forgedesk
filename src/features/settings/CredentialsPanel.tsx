/**
 * 凭据（账号与令牌）面板（T2.7）。
 *
 * # 这一版能做什么、不能做什么
 *
 * 能做：把访问令牌/密码保存进系统凭据库、列出已保存的账号（**不含密文**）、删除、
 * 查看"密文存在哪里"，以及用 `git ls-remote` 测试某个地址现在能不能连上。
 *
 * 不能做（属于 T4.4）：OAuth 设备码登录、多账号切换、README 里的身份信息同步。
 * 面板头部把这件事写出来，避免用户以为"没有登录按钮是坏了"。
 *
 * # 明文只经过一次
 *
 * 令牌只在提交那一次从输入框流向 `credentials_save`。之后：
 * Query 缓存里**没有**它（列表 DTO 不含密文字段）、store 里没有它、
 * 任何提示文案里都不引用它（红线 R8）。
 */
import { useState } from 'react';

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';

import { useAppError } from '@/lib/errors';
import {
  credentialTestRemote,
  credentialsDelete,
  credentialsList,
  credentialsSave,
  credentialsStatus,
  probeUrlFor,
} from '@/lib/ipc/credentials';
import type { CredentialKind, CredentialMeta } from '@/lib/ipc/credentials';
import { CREDENTIALS_QUERY_KEY, credentialsStatusKey } from '@/lib/queryKeys';
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
import { Input } from '@/ui/components/input';

/** 新增凭据表单的状态。 */
interface CredentialForm {
  readonly provider: string;
  readonly host: string;
  readonly login: string;
  readonly kind: CredentialKind;
  readonly secret: string;
}

function emptyForm(): CredentialForm {
  return { provider: 'github', host: 'github.com', login: '', kind: 'pat', secret: '' };
}

/** 表单是否填全（只做"是否为空"这一级判断，格式由后端收敛）。 */
function isComplete(form: CredentialForm): boolean {
  return (
    form.provider.trim() !== '' &&
    form.host.trim() !== '' &&
    form.login.trim() !== '' &&
    form.secret.trim() !== ''
  );
}

export function CredentialsPanel() {
  const { t } = useTranslation('shell');
  const { show } = useAppError();
  const queryClient = useQueryClient();
  const [form, setForm] = useState<CredentialForm>(emptyForm);
  const [pendingDelete, setPendingDelete] = useState<CredentialMeta | null>(null);

  const listQuery = useQuery({
    queryKey: [CREDENTIALS_QUERY_KEY],
    queryFn: credentialsList,
  });
  const statusQuery = useQuery({
    queryKey: credentialsStatusKey(),
    queryFn: credentialsStatus,
  });

  const invalidate = (): void => {
    void queryClient.invalidateQueries({ queryKey: [CREDENTIALS_QUERY_KEY] });
    void queryClient.invalidateQueries({ queryKey: credentialsStatusKey() });
  };

  const save = useMutation({
    mutationFn: credentialsSave,
    onSuccess: () => {
      setForm(emptyForm());
      invalidate();
      pushToast({ tone: 'success', title: t('settings.credentials.saved') });
    },
    onError: show,
  });

  const remove = useMutation({
    mutationFn: credentialsDelete,
    onSuccess: () => {
      setPendingDelete(null);
      invalidate();
    },
    onError: (error) => {
      setPendingDelete(null);
      show(error);
    },
  });

  const probe = useMutation({
    mutationFn: credentialTestRemote,
    onSuccess: (result) => {
      pushToast({
        tone: 'success',
        title: t('settings.credentials.probeOk', { refs: result.refs }),
      });
    },
    onError: show,
  });

  const status = statusQuery.data;
  const credentials = listQuery.data ?? [];

  return (
    <section className="flex flex-col gap-4" data-testid="credentials-panel">
      <header className="flex flex-col gap-1">
        <div className="flex flex-wrap items-center gap-2">
          <h2 className="text-16 font-semibold tracking-tight">
            {t('settings.credentials.title')}
          </h2>
          {status !== undefined ? (
            <span
              className="rounded-sm border border-line bg-surface-sunken px-2 py-0.5 text-12 text-fg-subtle"
              data-testid="credentials-status"
            >
              {t('settings.credentials.status', {
                backend: t(`settings.credentials.backend.${status.backend}`),
                count: status.count,
              })}
            </span>
          ) : null}
        </div>
        <p className="text-13 text-fg-muted">{t('settings.credentials.description')}</p>
      </header>

      {status?.keyringUnavailableReason !== undefined ? (
        <p
          className="rounded-md border border-warning bg-surface p-3 text-12 leading-relaxed"
          data-testid="credentials-keyring-warning"
        >
          {t('settings.credentials.keyringUnavailable')}
          {`: ${status.keyringUnavailableReason}`}
        </p>
      ) : null}

      {/* 已保存的凭据 */}
      {credentials.length === 0 ? (
        <p className="text-13 text-fg-subtle" data-testid="credentials-empty">
          {t('settings.credentials.empty')}
        </p>
      ) : (
        <ul className="flex flex-col gap-1" data-testid="credentials-list">
          {credentials.map((meta) => (
            <li
              key={`${meta.key.provider}:${meta.key.host}:${meta.key.login}`}
              className="flex flex-wrap items-center gap-2 rounded-md border border-line bg-surface px-3 py-2"
            >
              <span className="font-mono text-13">
                {meta.key.provider}:{meta.key.host}
              </span>
              <span className="text-13 text-fg-muted">{meta.key.login}</span>
              <span className="rounded-sm bg-surface-sunken px-1.5 text-12 text-fg-subtle">
                {t(`settings.credentials.kind.${meta.kind}`)}
              </span>
              <span className="ml-auto flex items-center gap-2">
                <Button
                  size="sm"
                  variant="secondary"
                  disabled={probe.isPending}
                  onClick={() => {
                    probe.mutate({ url: probeUrlFor(meta.key.host) });
                  }}
                  data-testid={`credentials-probe-${meta.key.login}`}
                >
                  {t('settings.credentials.testConnection')}
                </Button>
                <Button
                  size="sm"
                  variant="danger"
                  onClick={() => {
                    setPendingDelete(meta);
                  }}
                  data-testid={`credentials-delete-${meta.key.login}`}
                >
                  {t('common:actions.delete')}
                </Button>
              </span>
            </li>
          ))}
        </ul>
      )}

      {/* 新增 */}
      <form
        className="flex flex-col gap-2 rounded-md border border-line bg-surface p-3"
        onSubmit={(event) => {
          event.preventDefault();
          if (!isComplete(form)) {
            return;
          }
          save.mutate({
            provider: form.provider.trim(),
            host: form.host.trim(),
            login: form.login.trim(),
            kind: form.kind,
            secret: form.secret,
          });
        }}
      >
        <h3 className="text-13 font-medium">{t('settings.credentials.addTitle')}</h3>
        <div className="flex flex-wrap items-end gap-2">
          <label className="flex flex-col gap-1">
            <span className="text-12 text-fg-muted">{t('settings.credentials.provider')}</span>
            <Input
              className="w-32"
              value={form.provider}
              onChange={(event) => {
                setForm((current) => ({ ...current, provider: event.target.value }));
              }}
              data-testid="credentials-provider"
            />
          </label>
          <label className="flex flex-col gap-1">
            <span className="text-12 text-fg-muted">{t('settings.credentials.host')}</span>
            <Input
              className="w-44"
              value={form.host}
              onChange={(event) => {
                setForm((current) => ({ ...current, host: event.target.value }));
              }}
              data-testid="credentials-host"
            />
          </label>
          <label className="flex flex-col gap-1">
            <span className="text-12 text-fg-muted">{t('settings.credentials.login')}</span>
            <Input
              className="w-40"
              value={form.login}
              onChange={(event) => {
                setForm((current) => ({ ...current, login: event.target.value }));
              }}
              data-testid="credentials-login"
            />
          </label>
          <label className="flex flex-col gap-1">
            <span className="text-12 text-fg-muted">{t('settings.credentials.kindLabel')}</span>
            <select
              className="h-8 rounded-md border border-line bg-surface px-2 text-13"
              value={form.kind}
              onChange={(event) => {
                const next = event.target.value as CredentialKind;
                setForm((current) => ({ ...current, kind: next }));
              }}
              data-testid="credentials-kind"
            >
              <option value="pat">{t('settings.credentials.kind.pat')}</option>
              <option value="oauth">{t('settings.credentials.kind.oauth')}</option>
              <option value="password">{t('settings.credentials.kind.password')}</option>
            </select>
          </label>
          <label className="flex flex-1 flex-col gap-1">
            <span className="text-12 text-fg-muted">{t('settings.credentials.secret')}</span>
            {/* type=password：只防肩窥；真正的保护是"只存 keyring、不进日志" */}
            <Input
              type="password"
              autoComplete="off"
              value={form.secret}
              onChange={(event) => {
                setForm((current) => ({ ...current, secret: event.target.value }));
              }}
              data-testid="credentials-secret"
            />
          </label>
          <Button
            type="submit"
            size="sm"
            loading={save.isPending}
            disabled={!isComplete(form)}
            data-testid="credentials-save"
          >
            {t('settings.credentials.save')}
          </Button>
        </div>
        <p className="text-12 text-fg-subtle">{t('settings.credentials.secretHint')}</p>
      </form>

      <AlertDialog
        open={pendingDelete !== null}
        onOpenChange={(open) => {
          if (!open) {
            setPendingDelete(null);
          }
        }}
      >
        <AlertDialogContent
          impact={t('settings.credentials.deleteImpact')}
          impactLabel={t('settings.credentials.deleteImpactLabel')}
          data-testid="credentials-delete-dialog"
        >
          <AlertDialogHeader>
            <AlertDialogTitle>{t('settings.credentials.deleteTitle')}</AlertDialogTitle>
            <AlertDialogDescription>
              {pendingDelete === null
                ? ''
                : `${pendingDelete.key.provider}:${pendingDelete.key.host}:${pendingDelete.key.login}`}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel
              onClick={() => {
                setPendingDelete(null);
              }}
            >
              {t('common:actions.cancel')}
            </AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                if (pendingDelete !== null) {
                  remove.mutate(pendingDelete.key);
                }
              }}
              data-testid="credentials-delete-confirm"
            >
              {t('common:actions.delete')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </section>
  );
}
