import { useState } from 'react';
import { useTranslation } from 'react-i18next';

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';

import {
  gpgListSecretKeys,
  gpgTestSign,
  networkGitTest,
  networkProxyTest,
  settingsAll,
  settingsSet,
  sshTestConnection,
} from '@/lib/ipc';
import type { ConnectivityResult } from '@/lib/ipc';
import { pushToast } from '@/stores/toastStore';
import { useAppError } from '@/lib/errors';
import { Badge } from '@/ui/components/badge';
import { Button } from '@/ui/components/button';
import { Input } from '@/ui/components/input';

/**
 * 网络与密钥设置页（T6.8）：代理设置 + 连通性测试 + SSH 连接测试 + GPG 密钥/签名自检。
 *
 * # 代理生效范围（与后端对齐）
 *
 * 设置写入全局 KV（`network.*`），后端在**设置命令里即时应用**到两处消费点：
 * git CLI（`-c http.proxy` 注入，不污染用户配置）与 GitHub HTTP 客户端（换实例）。
 * 不需要重启；测试按钮走的就是当前设置的代理，所见即所得。
 *
 * # SSH 安全边界
 *
 * 连接测试用 `BatchMode` + 5 秒超时；**不**设置 `StrictHostKeyChecking=no`
 * （T6.8 规格红线）——首次连接的指纹确认由 OpenSSH 自己的流程处理，
 * 未确认主机的测试会失败并如实显示原因。
 */
export function NetworkSettingsPage(): React.JSX.Element {
  const { t } = useTranslation('shell');
  const { show } = useAppError();
  const queryClient = useQueryClient();

  // 代理设置（settings KV 直读直写；后端 set 命令里即时应用）
  const settings = useQuery({
    queryKey: ['networkSettings'],
    queryFn: () => settingsAll('global', undefined),
  });
  const mode = (parseMode(settings.data?.['network.proxyMode']) ?? 'system') as ProxyModeUi;
  const proxyUrl = parseStr(settings.data?.['network.proxyUrl']);
  const noProxy = parseStr(settings.data?.['network.noProxy']);

  const [urlDraft, setUrlDraft] = useState(proxyUrl ?? '');
  const [noProxyDraft, setNoProxyDraft] = useState(noProxy ?? '');

  const save = useMutation({
    mutationFn: async (next: { mode: ProxyModeUi; url?: string; noProxy?: string }) => {
      await settingsSet('global', 'network.proxyMode', JSON.stringify(next.mode), undefined);
      if (next.url !== undefined) {
        await settingsSet('global', 'network.proxyUrl', JSON.stringify(next.url), undefined);
      }
      if (next.noProxy !== undefined) {
        await settingsSet('global', 'network.noProxy', JSON.stringify(next.noProxy), undefined);
      }
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ['networkSettings'] });
      pushToast({ title: t('network.savedToast'), tone: 'success' });
    },
    onError: show,
  });

  const saveMode = (nextMode: ProxyModeUi): void => {
    // mode=manual 时带上当前草稿里的 URL；其他模式清 URL（避免残留生效）
    save.mutate({
      mode: nextMode,
      ...(nextMode === 'manual' ? { url: urlDraft } : {}),
      noProxy: noProxyDraft,
    });
  };

  return (
    <section className="flex flex-col gap-4">
      <header className="flex flex-col gap-1">
        <h1 className="text-20 font-semibold tracking-tight">{t('pages.settingsNetwork.title')}</h1>
        <p className="text-13 text-fg-muted">{t('pages.settingsNetwork.description')}</p>
      </header>

      {/* ---------- 代理 ---------- */}
      <div className="flex flex-col gap-3 rounded-lg border border-line bg-surface p-4">
        <span className="text-14 font-medium">{t('network.proxyTitle')}</span>
        <ModeSelector mode={mode} onSelect={saveMode} />
        {mode === 'manual' ? (
          <div className="flex flex-col gap-2">
            <label className="flex flex-col gap-1 text-13">
              <span className="text-fg-muted">{t('network.proxyUrlLabel')}</span>
              <Input
                value={urlDraft}
                onChange={(event) => setUrlDraft(event.target.value)}
                placeholder="http://127.0.0.1:7890"
                aria-label={t('network.proxyUrlLabel')}
              />
            </label>
            <label className="flex flex-col gap-1 text-13">
              <span className="text-fg-muted">{t('network.noProxyLabel')}</span>
              <Input
                value={noProxyDraft}
                onChange={(event) => setNoProxyDraft(event.target.value)}
                placeholder="localhost,127.0.0.1,.internal"
                aria-label={t('network.noProxyLabel')}
              />
            </label>
            <div>
              <Button
                size="sm"
                variant="secondary"
                disabled={save.isPending || urlDraft.trim().length === 0}
                onClick={() =>
                  save.mutate({ mode: 'manual', url: urlDraft, noProxy: noProxyDraft })
                }
              >
                {t('network.applyProxy')}
              </Button>
            </div>
          </div>
        ) : null}
        <ConnectivityRow label={t('network.testApi')} target="api" />
        <ConnectivityRow label={t('network.testRaw')} target="raw" />
        <ConnectivityRow label={t('network.testGit')} target="git" />
      </div>

      {/* ---------- SSH ---------- */}
      <div className="flex flex-col gap-3 rounded-lg border border-line bg-surface p-4">
        <span className="text-14 font-medium">{t('network.sshTitle')}</span>
        <ConnectivityRow label={t('network.testSsh')} target="ssh-github" />
        <p className="text-12 text-fg-subtle">{t('network.sshHint')}</p>
      </div>

      {/* ---------- GPG ---------- */}
      <GpgSection />
    </section>
  );
}

type ProxyModeUi = 'none' | 'system' | 'manual';

function parseMode(raw: string | undefined): ProxyModeUi | null {
  return raw === 'none' || raw === 'system' || raw === 'manual' ? raw : null;
}

function parseStr(raw: string | undefined): string | null {
  if (raw === undefined) {
    return null;
  }
  try {
    const parsed: unknown = JSON.parse(raw);
    return typeof parsed === 'string' ? parsed : null;
  } catch {
    return null;
  }
}

/** 代理模式三选一（ToggleGroup 的轻量替代，保持本页自足）。 */
function ModeSelector({
  mode,
  onSelect,
}: {
  readonly mode: ProxyModeUi;
  readonly onSelect: (mode: ProxyModeUi) => void;
}): React.JSX.Element {
  const { t } = useTranslation('shell');
  const modes: readonly { readonly value: ProxyModeUi; readonly label: string }[] = [
    { value: 'none', label: t('network.modeNone') },
    { value: 'system', label: t('network.modeSystem') },
    { value: 'manual', label: t('network.modeManual') },
  ];
  return (
    <div role="radiogroup" aria-label={t('network.proxyTitle')} className="flex gap-2">
      {modes.map((entry) => (
        <Button
          key={entry.value}
          size="sm"
          variant={mode === entry.value ? 'primary' : 'secondary'}
          aria-pressed={mode === entry.value}
          onClick={() => onSelect(entry.value)}
        >
          {entry.label}
        </Button>
      ))}
    </div>
  );
}

/** 单行连通性测试（点击测试 → 结果徽章 + 延迟 + 详情）。 */
function ConnectivityRow({
  label,
  target,
}: {
  readonly label: string;
  readonly target: string;
}): React.JSX.Element {
  const { t } = useTranslation('shell');
  const [result, setResult] = useState<ConnectivityResult | null>(null);
  const [running, setRunning] = useState(false);
  const { show } = useAppError();

  const run = async (): Promise<void> => {
    setRunning(true);
    try {
      if (target === 'api' || target === 'raw') {
        setResult(await networkProxyTest(target));
      } else if (target === 'git') {
        setResult(await networkGitTest());
      } else if (target === 'ssh-github') {
        setResult(await sshTestConnection('github.com'));
      }
    } catch (error) {
      show(error instanceof Error ? error : new Error(String(error)));
    } finally {
      setRunning(false);
    }
  };

  return (
    <div className="flex items-center justify-between gap-3">
      <div className="flex items-center gap-2">
        <span className="text-13">{label}</span>
        {result !== null ? (
          <>
            <Badge tone={result.ok ? 'success' : 'danger'}>
              {result.ok ? t('network.testOk') : t('network.testFail')}
            </Badge>
            <span className="text-12 text-fg-subtle">
              {result.latencyMs}
              {t('network.msUnit')}
            </span>
          </>
        ) : null}
        {result !== null && !result.ok ? (
          <span className="text-12 text-danger">{result.detail}</span>
        ) : null}
        {result !== null && result.ok ? (
          <span className="text-12 text-fg-subtle">{result.detail}</span>
        ) : null}
      </div>
      <Button size="sm" variant="ghost" disabled={running} onClick={() => void run()}>
        {running ? t('network.testing') : t('network.testAction')}
      </Button>
    </div>
  );
}

/** GPG 密钥列表 + 签名自检。 */
function GpgSection(): React.JSX.Element {
  const { t } = useTranslation('shell');
  const { show } = useAppError();
  const keys = useQuery({ queryKey: ['gpgKeys'], queryFn: gpgListSecretKeys });
  const [testResult, setTestResult] = useState<ConnectivityResult | null>(null);
  const [testing, setTesting] = useState(false);

  const runTest = async (): Promise<void> => {
    setTesting(true);
    try {
      setTestResult(await gpgTestSign(undefined));
    } catch (error) {
      show(error instanceof Error ? error : new Error(String(error)));
    } finally {
      setTesting(false);
    }
  };

  return (
    <div className="flex flex-col gap-3 rounded-lg border border-line bg-surface p-4">
      <div className="flex items-center justify-between gap-2">
        <span className="text-14 font-medium">{t('network.gpgTitle')}</span>
        <Button size="sm" variant="ghost" disabled={testing} onClick={() => void runTest()}>
          {testing ? t('network.testing') : t('network.gpgSelfTest')}
        </Button>
      </div>
      {testResult !== null ? (
        <p className={`text-13 ${testResult.ok ? 'text-success' : 'text-danger'}`}>
          {testResult.detail}
        </p>
      ) : null}
      {keys.isLoading ? (
        <p className="text-12 text-fg-subtle" aria-busy="true">
          {t('plugins.panelsLoading')}
        </p>
      ) : (keys.data?.length ?? 0) === 0 ? (
        <p className="text-12 text-fg-subtle">{t('network.gpgEmpty')}</p>
      ) : (
        <ul className="flex flex-col gap-1 text-13">
          {keys.data?.map((key) => (
            <li key={key.keyId} className="flex items-center justify-between gap-2">
              <span className="font-mono text-12">{key.keyId}</span>
              <span className="text-fg-muted">{key.uid}</span>
            </li>
          ))}
        </ul>
      )}
      <p className="text-12 text-fg-subtle">{t('network.gpgHint')}</p>
    </div>
  );
}
