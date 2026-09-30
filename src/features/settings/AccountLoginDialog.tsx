/**
 * 登录对话框（T4.4）：PAT 粘贴与 OAuth Device Flow 三步引导。
 *
 * # 为什么默认是 Device Flow
 *
 * PLAN M4 风险表要求的引导顺序是"复制码 → 打开浏览器 → 自动轮询"，
 * 它不需要用户理解"令牌作用域"是什么；PAT 是高级路径。两者后端落地
 * 完全一致（keyring + accounts 表），切换只在输入形态。
 *
 * # device_code 永远不经过前端
 *
 * `account_device_flow_start` 返回的会话里没有 device_code（后端会话表
 * 持有，类型上不可序列化）；前端只拿 user_code 与验证链接，轮询结果经
 * `job:done` 只带回账号信息。因此本对话框对秘密的处理只有一条：
 * PAT 输入框在提交成功后立即清空（与 CredentialsPanel 同一纪律）。
 *
 * # 链接为什么是"复制"而不是点击
 *
 * 应用内没有接入 opener 插件，`<a href>` 会把整个 webview 导航走。
 * 复制链接 + 用户自己粘贴是当前零依赖的正确做法；接入 opener 后
 * 在这里换成"打开浏览器"按钮（TODO 标记在验收清单里）。
 */
import { useEffect, useRef, useState } from 'react';

import { useMutation, useQueryClient } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';

import { useAppError } from '@/lib/errors';
import {
  accountDeviceFlowStart,
  accountDeviceFlowWait,
  accountLoginWithPat,
} from '@/lib/ipc/accounts';
import type { Account, DeviceFlowSession } from '@/lib/ipc/accounts';
import { cancelJob, onJobDone, onJobFailed } from '@/lib/ipc/jobs';
import { ACCOUNTS_QUERY_KEY } from '@/lib/queryKeys';
import { pushToast } from '@/stores/toastStore';

import { Button } from '@/ui/components/button';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/ui/components/dialog';
import { Input } from '@/ui/components/input';

/** 登录方式。 */
type LoginMode = 'device-flow' | 'pat';

/** 对话框的阶段：表单 → （Device Flow）等待授权。 */
type LoginStage =
  | { readonly kind: 'form' }
  | { readonly kind: 'waiting'; readonly session: DeviceFlowSession; readonly jobId: string };

interface AccountLoginDialogProps {
  /** 打开时重置为表单阶段（关闭再开不残留上一次的会话与输入）。 */
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
}

function normalizeResult(payload: unknown): Account {
  // job:done 的 result 形状由后端 `device_flow_done_payload` 保证：{ account }
  const account = (payload as { account?: Account }).account;
  if (account === undefined) {
    throw new Error('device flow job result did not contain an account');
  }
  return account;
}

export function AccountLoginDialog({ open, onOpenChange }: AccountLoginDialogProps) {
  const { t } = useTranslation('shell');
  const { show } = useAppError();
  const queryClient = useQueryClient();

  const [mode, setMode] = useState<LoginMode>('device-flow');
  const [host, setHost] = useState('github.com');
  const [token, setToken] = useState('');
  const [stage, setStage] = useState<LoginStage>({ kind: 'form' });
  const [busy, setBusy] = useState(false);
  const [waitingFailed, setWaitingFailed] = useState(false);
  const [copied, setCopied] = useState<'code' | 'link' | null>(null);
  // 事件回调里要读当前 jobId：放进 ref 才能在只挂一次的订阅里读到
  const jobIdRef = useRef<string | null>(null);

  /** 成功：失效列表、提示、清令牌、回表单并关闭。 */
  const finishWith = (account: Account) => {
    void queryClient.invalidateQueries({ queryKey: [ACCOUNTS_QUERY_KEY] });
    pushToast({
      tone: 'success',
      title: t('settings.accounts.loginSuccess', { login: account.login }),
    });
    setToken('');
    setStage({ kind: 'form' });
    setWaitingFailed(false);
    onOpenChange(false);
  };

  // 只挂一次的 job 事件订阅：按 jobIdRef 过滤别人的任务
  useEffect(() => {
    const donePromise = onJobDone((payload) => {
      if (jobIdRef.current === null || payload.jobId !== jobIdRef.current) {
        return;
      }
      jobIdRef.current = null;
      try {
        finishWith(normalizeResult(payload.result));
      } catch (error) {
        show(error instanceof Error ? error : new Error(String(error)));
      }
    });
    const failedPromise = onJobFailed((payload) => {
      if (jobIdRef.current === null || payload.jobId !== jobIdRef.current) {
        return;
      }
      jobIdRef.current = null;
      setBusy(false);
      setWaitingFailed(true);
      // 错误详情走统一 toast（按 code 走 i18n）；向导停在等待页供重试
      show(payload.error);
    });
    return () => {
      void donePromise.then((unlisten) => unlisten());
      void failedPromise.then((unlisten) => unlisten());
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- 只在挂载时订阅一次；回调经 ref/state 读最新值
  }, []);

  const startDeviceFlow = useMutation({
    mutationFn: () => accountDeviceFlowStart(host.trim()),
    onMutate: () => {
      setBusy(true);
      setWaitingFailed(false);
    },
    onSuccess: async (session) => {
      const { jobId } = await accountDeviceFlowWait(session.flowId);
      jobIdRef.current = jobId;
      setBusy(false);
      setStage({ kind: 'waiting', session, jobId });
    },
    onError: (error) => {
      setBusy(false);
      show(error);
    },
  });

  const loginWithPat = useMutation({
    mutationFn: () => accountLoginWithPat(host.trim(), token),
    onMutate: () => {
      setBusy(true);
    },
    onSuccess: (account) => {
      setBusy(false);
      finishWith(account);
    },
    onError: (error) => {
      setBusy(false);
      show(error);
    },
  });

  const cancelWaiting = async () => {
    if (stage.kind === 'waiting' && stage.jobId !== '') {
      await cancelJob(stage.jobId).catch(() => undefined);
    }
    jobIdRef.current = null;
    setStage({ kind: 'form' });
    setWaitingFailed(false);
  };

  const copy = (text: string, kind: 'code' | 'link') => {
    void navigator.clipboard?.writeText(text).catch(() => undefined);
    setCopied(kind);
    window.setTimeout(() => setCopied(null), 1500);
  };

  const hostInvalid = host.trim() === '';
  const patInvalid = hostInvalid || token.trim() === '';

  const closeDialog = (next: boolean) => {
    if (!next && stage.kind === 'waiting') {
      // Esc / 点遮罩关闭 = 放弃等待：停掉后端轮询，不留孤儿任务
      void cancelWaiting();
    }
    onOpenChange(next);
  };

  return (
    <Dialog open={open} onOpenChange={closeDialog}>
      <DialogContent className="max-w-md" closeLabel={t('common:actions.close')}>
        <DialogHeader>
          <DialogTitle>{t('settings.accounts.loginTitle')}</DialogTitle>
          <DialogDescription>{t('settings.accounts.loginDescription')}</DialogDescription>
        </DialogHeader>

        {stage.kind === 'form' ? (
          <div className="flex flex-col gap-3" data-testid="account-login-form">
            <div
              role="radiogroup"
              aria-label={t('settings.accounts.modeLabel')}
              className="flex gap-2"
            >
              <Button
                type="button"
                variant={mode === 'device-flow' ? 'primary' : 'secondary'}
                aria-pressed={mode === 'device-flow'}
                onClick={() => setMode('device-flow')}
                data-testid="account-mode-device"
              >
                {t('settings.accounts.modeDeviceFlow')}
              </Button>
              <Button
                type="button"
                variant={mode === 'pat' ? 'primary' : 'secondary'}
                aria-pressed={mode === 'pat'}
                onClick={() => setMode('pat')}
                data-testid="account-mode-pat"
              >
                {t('settings.accounts.modePat')}
              </Button>
            </div>

            <label className="flex flex-col gap-1 text-13">
              <span className="text-fg-muted">{t('settings.accounts.hostLabel')}</span>
              <Input
                value={host}
                onChange={(event) => setHost(event.target.value)}
                data-testid="account-host"
                placeholder="github.com"
              />
            </label>

            {mode === 'pat' ? (
              <label className="flex flex-col gap-1 text-13">
                <span className="text-fg-muted">{t('settings.accounts.tokenLabel')}</span>
                <Input
                  type="password"
                  value={token}
                  onChange={(event) => setToken(event.target.value)}
                  data-testid="account-token"
                  autoComplete="off"
                />
              </label>
            ) : (
              <p className="text-12 text-fg-muted">{t('settings.accounts.deviceIntro')}</p>
            )}
          </div>
        ) : (
          <div className="flex flex-col gap-3" data-testid="account-waiting">
            <p className="text-13 text-fg-muted">{t('settings.accounts.stepCopyCode')}</p>
            <div className="flex items-center justify-between gap-2">
              <span className="font-mono text-20 tracking-widest" data-testid="account-user-code">
                {stage.session.userCode}
              </span>
              <Button
                type="button"
                variant="secondary"
                onClick={() => copy(stage.session.userCode, 'code')}
                data-testid="account-copy-code"
              >
                {copied === 'code'
                  ? t('settings.accounts.copied')
                  : t('settings.accounts.copyCode')}
              </Button>
            </div>
            <p className="text-13 text-fg-muted">{t('settings.accounts.stepOpenLink')}</p>
            <div className="flex items-center justify-between gap-2">
              <span className="truncate font-mono text-12" data-testid="account-verification-uri">
                {stage.session.verificationUriComplete ?? stage.session.verificationUri}
              </span>
              <Button
                type="button"
                variant="secondary"
                onClick={() =>
                  copy(
                    stage.session.verificationUriComplete ?? stage.session.verificationUri,
                    'link',
                  )
                }
                data-testid="account-copy-link"
              >
                {copied === 'link'
                  ? t('settings.accounts.copied')
                  : t('settings.accounts.copyLink')}
              </Button>
            </div>
            <p className="text-12 text-fg-subtle" data-testid="account-waiting-hint">
              {t('settings.accounts.waitingHint', { interval: stage.session.intervalSecs })}
            </p>
            {waitingFailed ? (
              <p className="text-12 text-danger" data-testid="account-waiting-error">
                {t('settings.accounts.waitingFailed')}
              </p>
            ) : null}
          </div>
        )}

        <DialogFooter>
          {stage.kind === 'waiting' ? (
            <Button
              type="button"
              variant="secondary"
              onClick={() => void cancelWaiting()}
              data-testid="account-cancel"
            >
              {t('settings.accounts.cancel')}
            </Button>
          ) : (
            <Button type="button" variant="secondary" onClick={() => onOpenChange(false)}>
              {t('common:actions.cancel')}
            </Button>
          )}
          {stage.kind === 'form' && mode === 'pat' ? (
            <Button
              type="button"
              disabled={busy || patInvalid}
              onClick={() => loginWithPat.mutate()}
              data-testid="account-pat-submit"
            >
              {t('settings.accounts.patSubmit')}
            </Button>
          ) : null}
          {stage.kind === 'form' && mode === 'device-flow' ? (
            <Button
              type="button"
              disabled={busy || hostInvalid}
              onClick={() => startDeviceFlow.mutate()}
              data-testid="account-device-submit"
            >
              {t('settings.accounts.deviceStart')}
            </Button>
          ) : null}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
