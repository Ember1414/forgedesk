/**
 * SSH 密钥面板（T2.7）。
 *
 * # 它回答哪一半问题
 *
 * `Permission denied (publickey)` 之后有两件事要查清：
 *   1. **本地**用没用上我配的那把 key —— 这个面板回答（有哪些密钥、是否配对、
 *      agent 里加载了哪几把、agent 在不在）；
 *   2. 服务端是否接受这把公钥 —— 只有实际连一次才知道，因此面板底部提供
 *      "测试连接"（`git ls-remote`）。
 *
 * # 红线 R8
 *
 * 后端**不读私钥内容**（只判存在性），因此这里也拿不到任何私钥材料；
 * 显示出来的只有路径、密钥类型与注释（公钥信息）。
 */
import { useState } from 'react';

import { useMutation, useQuery } from '@tanstack/react-query';
import { useTranslation } from 'react-i18next';

import { useAppError } from '@/lib/errors';
import { credentialTestRemote, credentialsSshInventory } from '@/lib/ipc';
import { SSH_INVENTORY_QUERY_KEY } from '@/lib/queryKeys';
import { pushToast } from '@/stores/toastStore';

import { agentKeys, agentTone, keyName, keyState, keyStateTone } from '@/features/settings/sshKeys';
import { Button } from '@/ui/components/button';
import { Input } from '@/ui/components/input';

/** 语气 → 边框配色（只有警告用强调色：把普通事说成故障会让人白折腾）。 */
function toneClass(tone: 'ok' | 'info' | 'warning'): string {
  return tone === 'warning' ? 'border-warning' : 'border-line';
}

export function SshKeysPanel() {
  const { t } = useTranslation('shell');
  const { show } = useAppError();
  const [target, setTarget] = useState('');

  const query = useQuery({
    queryKey: [SSH_INVENTORY_QUERY_KEY],
    queryFn: credentialsSshInventory,
  });

  const probe = useMutation({
    mutationFn: credentialTestRemote,
    onSuccess: (result) => {
      pushToast({ tone: 'success', title: t('settings.ssh.test.ok', { refs: result.refs }) });
    },
    onError: show,
  });

  // IPC 边界上的 `null` 必须当成"没有数据"：真实命令要么给对象要么报错，
  // 但替身/mock（e2e 的兜底分支）会返回 `null`——那不是"有数据但字段为空"，
  // 直接在渲染里读它的字段会让整页崩掉
  const inventory = query.data ?? undefined;

  return (
    <section className="flex flex-col gap-3" data-testid="ssh-panel">
      <header className="flex flex-col gap-1">
        <h2 className="text-16 font-semibold tracking-tight">{t('settings.ssh.title')}</h2>
        <p className="text-13 text-fg-muted">{t('settings.ssh.description')}</p>
      </header>

      {query.isError ? (
        <p className="text-13 text-fg-subtle" data-testid="ssh-unavailable">
          {t('settings.ssh.unavailable')}
        </p>
      ) : null}

      {inventory !== undefined ? (
        <>
          {/* agent：最常见的困惑来源（agent 优先于文件） */}
          <div
            className={`flex flex-col gap-2 rounded-md border bg-surface p-3 ${toneClass(agentTone(inventory.agent))}`}
            data-testid="ssh-agent"
          >
            <div className="flex flex-wrap items-center gap-2">
              <span className="text-13 font-medium">{t('settings.ssh.agent.title')}</span>
              <span className="text-13 text-fg-muted" data-testid="ssh-agent-status">
                {t(`settings.ssh.agent.${inventory.agent.kind}`, {
                  count: agentKeys(inventory.agent).length,
                })}
              </span>
            </div>

            {inventory.agent.kind === 'noIdentities' ? (
              <p className="text-12 text-fg-subtle">{t('settings.ssh.agent.noIdentitiesHint')}</p>
            ) : null}
            {inventory.agent.kind === 'notRunning' ? (
              <p className="text-12 text-fg-subtle">{t('settings.ssh.agent.notRunningHint')}</p>
            ) : null}
            {inventory.agent.kind === 'unknown' ? (
              // 原因来自平台（数据），照实显示：它决定了用户该去查什么
              <p className="font-mono text-12 text-fg-subtle" data-testid="ssh-agent-reason">
                {inventory.agent.reason}
              </p>
            ) : null}

            {agentKeys(inventory.agent).length > 0 ? (
              <ul className="flex flex-col gap-1" data-testid="ssh-agent-keys">
                {agentKeys(inventory.agent).map((key) => (
                  <li key={key.fingerprint} className="flex flex-wrap items-center gap-2 text-12">
                    <span className="font-mono">{key.fingerprint}</span>
                    {key.bits !== undefined ? (
                      <span className="text-fg-subtle">
                        {t('settings.ssh.agent.bits', { bits: key.bits })}
                      </span>
                    ) : null}
                    {key.comment !== undefined ? (
                      <span className="text-fg-muted">{key.comment}</span>
                    ) : null}
                  </li>
                ))}
              </ul>
            ) : null}
          </div>

          {/* 目录与密钥清单 */}
          <div className="flex flex-col gap-2">
            <p className="font-mono text-12 text-fg-subtle" data-testid="ssh-directory">
              {inventory.directory === undefined
                ? t('settings.ssh.noDirectory')
                : t('settings.ssh.directory', { directory: inventory.directory })}
            </p>

            {inventory.keys.length === 0 ? (
              <p className="text-13 text-fg-subtle" data-testid="ssh-empty">
                {t('settings.ssh.empty')}
              </p>
            ) : (
              <ul className="flex flex-col gap-1" data-testid="ssh-keys">
                {inventory.keys.map((key) => {
                  const state = keyState(key);
                  return (
                    <li
                      key={key.publicPath ?? key.privatePath ?? keyName(key)}
                      className="flex flex-wrap items-center gap-2 rounded-md border border-line bg-surface px-3 py-2"
                    >
                      <span className="font-mono text-13">{keyName(key)}</span>
                      {key.keyType !== undefined ? (
                        <span className="text-12 text-fg-muted">{key.keyType}</span>
                      ) : null}
                      {key.comment !== undefined ? (
                        <span className="text-12 text-fg-subtle">{key.comment}</span>
                      ) : null}
                      <span
                        className={`ml-auto rounded-sm border px-1.5 text-12 ${toneClass(keyStateTone(state))}`}
                        data-testid={`ssh-key-state-${keyName(key)}`}
                      >
                        {t(`settings.ssh.state.${state}`)}
                      </span>
                      {/* 缺公钥时给出可执行的做法，而不是只报"缺少" */}
                      {state === 'privateOnly' ? (
                        <span className="w-full text-12 text-fg-subtle">
                          {t('settings.ssh.privateOnlyHint')}
                        </span>
                      ) : null}
                    </li>
                  );
                })}
              </ul>
            )}
          </div>

          {/* 服务端是否接受公钥：只能连一次才知道 */}
          <form
            className="flex flex-wrap items-end gap-2 rounded-md border border-line bg-surface p-3"
            data-testid="ssh-test-form"
            onSubmit={(event) => {
              event.preventDefault();
              if (target.trim() === '') {
                return;
              }
              probe.mutate({ url: target.trim() });
            }}
          >
            <label className="flex flex-1 flex-col gap-1">
              <span className="text-12 text-fg-muted">{t('settings.ssh.test.title')}</span>
              <Input
                value={target}
                placeholder={t('settings.ssh.test.placeholder')}
                onChange={(event) => {
                  setTarget(event.target.value);
                }}
                data-testid="ssh-test-url"
              />
            </label>
            <Button
              type="submit"
              size="sm"
              loading={probe.isPending}
              disabled={target.trim() === ''}
              data-testid="ssh-test-submit"
            >
              {t('settings.ssh.test.button')}
            </Button>
            <p className="w-full text-12 text-fg-subtle">{t('settings.ssh.test.hint')}</p>
          </form>
        </>
      ) : null}
    </section>
  );
}
