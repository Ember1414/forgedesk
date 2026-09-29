import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { QueryClientProvider } from '@tanstack/react-query';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { SshKeysPanel } from '@/features/settings/SshKeysPanel';
import { credentialTestRemote, credentialsSshInventory } from '@/lib/ipc';
import type { SshInventory } from '@/lib/ipc';
import { initialToastState, useToastStore } from '@/stores/toastStore';
import { createTestQueryClient } from '@/test/queryClient';

/**
 * SSH 密钥面板（T2.7）。
 *
 * 要钉住的不是"渲染出来了"，而是**结论与语气**：agent 没运行必须给出"去启动 agent"，
 * "缺公钥"必须等到用户能照做的那一步（导出公钥的命令），以及"这里不读私钥"这件事
 * 在界面上有明确交代——用户看不到私钥内容是设计，而不是功能缺失。
 */
vi.mock('@/lib/ipc', () => ({
  credentialsSshInventory: vi.fn(),
  credentialTestRemote: vi.fn(),
}));

const inventoryMock = vi.mocked(credentialsSshInventory);
const probeMock = vi.mocked(credentialTestRemote);

function inventory(overrides: Partial<SshInventory> = {}): SshInventory {
  return {
    directory: 'C:\\Users\\octocat\\.ssh',
    keys: [
      {
        publicPath: 'C:\\Users\\octocat\\.ssh\\id_ed25519.pub',
        privatePath: 'C:\\Users\\octocat\\.ssh\\id_ed25519',
        keyType: 'ssh-ed25519',
        comment: 'octocat@example.com',
      },
    ],
    agent: {
      kind: 'ready',
      keys: [{ bits: 256, fingerprint: 'SHA256:abc', comment: 'id_ed25519' }],
    },
    ...overrides,
  };
}

function renderPanel() {
  const queryClient = createTestQueryClient();
  return render(
    <QueryClientProvider client={queryClient}>
      <SshKeysPanel />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  useToastStore.setState(initialToastState);
  inventoryMock.mockResolvedValue(inventory());
  probeMock.mockResolvedValue({ refs: 4 });
});

afterEach(() => {
  useToastStore.setState(initialToastState);
});

describe('SSH 密钥面板', () => {
  it('列出密钥、目录与 agent 里已加载的指纹', async () => {
    renderPanel();

    const keys = await screen.findByTestId('ssh-keys');
    // 显示的是文件名而不是整条路径（路径在目录那一行给出）
    expect(keys).toHaveTextContent('id_ed25519.pub');
    expect(keys).not.toHaveTextContent('C:\\Users');
    expect(keys).toHaveTextContent('ssh-ed25519');
    expect(keys).toHaveTextContent('octocat@example.com');
    expect(screen.getByTestId('ssh-key-state-id_ed25519.pub')).toHaveTextContent('公私钥齐全');
    expect(screen.getByTestId('ssh-directory')).toHaveTextContent('C:\\Users\\octocat\\.ssh');

    const agent = screen.getByTestId('ssh-agent-keys');
    expect(agent).toHaveTextContent('SHA256:abc');
    expect(agent).toHaveTextContent('256 位');
    expect(screen.getByTestId('ssh-agent-status')).toHaveTextContent('已加载 1 把密钥');
  });

  it('agent 没运行时给出启动建议（而不是只说状态）', async () => {
    inventoryMock.mockResolvedValue(inventory({ agent: { kind: 'notRunning' } }));

    renderPanel();

    const agent = await screen.findByTestId('ssh-agent-status');
    expect(agent).toHaveTextContent('agent 没有运行');
    expect(screen.getByTestId('ssh-agent')).toHaveTextContent('先启动 agent 并加载密钥');
    expect(screen.queryByTestId('ssh-agent-keys')).not.toBeInTheDocument();
  });

  it('agent 在跑但没加载密钥时给出 ssh-add 的做法', async () => {
    inventoryMock.mockResolvedValue(inventory({ agent: { kind: 'noIdentities' } }));

    renderPanel();

    expect(await screen.findByTestId('ssh-agent')).toHaveTextContent('ssh-add ~/.ssh/id_ed25519');
  });

  it('缺公钥时给出从私钥导出公钥的做法', async () => {
    inventoryMock.mockResolvedValue(
      inventory({
        keys: [{ privatePath: 'C:\\Users\\octocat\\.ssh\\id_rsa' }],
      }),
    );

    renderPanel();

    const state = await screen.findByTestId('ssh-key-state-id_rsa');
    expect(state).toHaveTextContent('缺少公钥');
    expect(screen.getByTestId('ssh-keys')).toHaveTextContent('ssh-keygen -y -f');
  });

  it('没有 ~/.ssh 时说明这是正常状态，并提示怎么生成', async () => {
    // 刻意**不带** `directory`（而不是写成 undefined）：后端的 `Option<String>`
    // 序列化后就是缺字段，测试要覆盖的是真实形状
    inventoryMock.mockResolvedValue({ keys: [], agent: { kind: 'notRunning' } });

    renderPanel();

    expect(await screen.findByTestId('ssh-empty')).toHaveTextContent('ssh-keygen');
    expect(screen.getByTestId('ssh-directory')).toHaveTextContent('找不到 ~/.ssh 目录');
  });

  it('测试连接用输入的远端地址，并展示引用条数', async () => {
    renderPanel();
    await screen.findByTestId('ssh-keys');

    // 空地址时按钮不可点：避免一次注定失败的调用
    expect(screen.getByTestId('ssh-test-submit')).toBeDisabled();

    fireEvent.change(screen.getByTestId('ssh-test-url'), {
      target: { value: '  git@github.com:octocat/repo.git  ' },
    });
    fireEvent.click(screen.getByTestId('ssh-test-submit'));

    await waitFor(() => {
      // 前后空格要去掉：用户从终端复制时经常带上
      expect(probeMock.mock.calls[0]?.[0]).toEqual({
        url: 'git@github.com:octocat/repo.git',
      });
    });
    await waitFor(() => {
      expect(useToastStore.getState().toasts[0]?.title).toContain('4');
    });
  });

  it('盘点失败时如实说明（而不是显示成"没有密钥"）', async () => {
    inventoryMock.mockRejectedValue(new Error('EACCES'));

    renderPanel();

    expect(await screen.findByTestId('ssh-unavailable')).toBeInTheDocument();
    expect(screen.queryByTestId('ssh-empty')).not.toBeInTheDocument();
  });
});
