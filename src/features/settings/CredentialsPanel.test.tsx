import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { QueryClientProvider } from '@tanstack/react-query';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { CredentialsPanel } from '@/features/settings/CredentialsPanel';
import {
  credentialTestRemote,
  credentialsDelete,
  credentialsList,
  credentialsSave,
  credentialsStatus,
} from '@/lib/ipc/credentials';
import type { CredentialMeta } from '@/lib/ipc/credentials';
import { initialToastState, useToastStore } from '@/stores/toastStore';
import { createTestQueryClient } from '@/test/queryClient';

/**
 * 凭据面板（T2.7）。
 *
 * 这一层要钉住的是**红线 R8 的界面侧**：面板只能把令牌**写出去**，
 * 任何渲染路径都不该把它读回来。因此除了"功能能跑"，还断言
 * "保存后输入框被清空"——一个把令牌留在输入框里的表单，会在下一次
 * 截图/录屏/共享屏幕时把它露出去。
 */
vi.mock('@/lib/ipc/credentials', () => ({
  credentialsList: vi.fn(),
  credentialsSave: vi.fn(),
  credentialsDelete: vi.fn(),
  credentialsStatus: vi.fn(),
  credentialTestRemote: vi.fn(),
  probeUrlFor: (host: string) => `https://${host}`,
}));

const listMock = vi.mocked(credentialsList);
const saveMock = vi.mocked(credentialsSave);
const deleteMock = vi.mocked(credentialsDelete);
const statusMock = vi.mocked(credentialsStatus);
const probeMock = vi.mocked(credentialTestRemote);

function meta(login: string): CredentialMeta {
  return {
    key: { provider: 'github', host: 'github.com', login },
    kind: 'pat',
    createdAtMs: 1_700_000_000_000,
  };
}

function renderPanel() {
  const queryClient = createTestQueryClient();
  return render(
    <QueryClientProvider client={queryClient}>
      <CredentialsPanel />
    </QueryClientProvider>,
  );
}

/** 填好表单（除了 secret 由用例自己决定）。 */
function fillForm(secret: string): void {
  fireEvent.change(screen.getByTestId('credentials-login'), { target: { value: 'octocat' } });
  fireEvent.change(screen.getByTestId('credentials-secret'), { target: { value: secret } });
}

beforeEach(() => {
  vi.clearAllMocks();
  useToastStore.setState(initialToastState);
  listMock.mockResolvedValue([meta('octocat')]);
  statusMock.mockResolvedValue({ backend: 'systemKeyring', count: 1 });
  saveMock.mockResolvedValue(meta('octocat'));
  deleteMock.mockResolvedValue(undefined);
  probeMock.mockResolvedValue({ refs: 3 });
});

afterEach(() => {
  useToastStore.setState(initialToastState);
});

describe('凭据面板', () => {
  it('列出已保存的凭据并展示存放位置与数量', async () => {
    renderPanel();

    const list = await screen.findByTestId('credentials-list');
    expect(list).toHaveTextContent('github:github.com');
    expect(list).toHaveTextContent('octocat');
    // 类型也要显示出来：排查登录问题时第一句要问的就是"这是令牌还是密码"
    expect(list).toHaveTextContent('访问令牌');
    expect(screen.getByTestId('credentials-status')).toHaveTextContent('系统凭据库');
    expect(screen.getByTestId('credentials-status')).toHaveTextContent('1 条');
  });

  it('系统凭据库不可用时给出原因（而不是等用户保存失败才知道）', async () => {
    statusMock.mockResolvedValue({
      backend: 'systemKeyring',
      count: 0,
      keyringUnavailableReason: 'platform=linux; no such service',
    });
    listMock.mockResolvedValue([]);

    renderPanel();

    const warning = await screen.findByTestId('credentials-keyring-warning');
    expect(warning).toHaveTextContent('platform=linux; no such service');
    expect(screen.getByTestId('credentials-empty')).toBeInTheDocument();
  });

  it('保存凭据后清空输入框（令牌不该留在界面上）', async () => {
    renderPanel();
    await screen.findByTestId('credentials-list');

    fillForm('ghp_supersecret');
    fireEvent.click(screen.getByTestId('credentials-save'));

    await waitFor(() => {
      // 取第 0 个实参：TanStack Query 还会传一个 mutation context 作为第二个参数，
      // 直接 toHaveBeenCalledWith 会因为它而失败（而不是因为参数真的不对）
      expect(saveMock.mock.calls[0]?.[0]).toEqual({
        provider: 'github',
        host: 'github.com',
        login: 'octocat',
        kind: 'pat',
        secret: 'ghp_supersecret',
      });
    });

    await waitFor(() => {
      expect(screen.getByTestId('credentials-secret')).toHaveValue('');
      expect(screen.getByTestId('credentials-login')).toHaveValue('');
    });
  });

  it('不填令牌时保存按钮不可用（空令牌等于"存了一坨没用的东西"）', async () => {
    renderPanel();
    await screen.findByTestId('credentials-list');

    fireEvent.change(screen.getByTestId('credentials-login'), { target: { value: 'octocat' } });

    expect(screen.getByTestId('credentials-save')).toBeDisabled();
    expect(saveMock).not.toHaveBeenCalled();
  });

  it('删除前先确认，确认后调用删除命令', async () => {
    renderPanel();
    await screen.findByTestId('credentials-list');

    fireEvent.click(screen.getByTestId('credentials-delete-octocat'));

    const dialog = await screen.findByTestId('credentials-delete-dialog');
    expect(dialog).toHaveTextContent('github:github.com:octocat');
    expect(deleteMock).not.toHaveBeenCalled();

    fireEvent.click(screen.getByTestId('credentials-delete-confirm'));

    await waitFor(() => {
      expect(deleteMock.mock.calls[0]?.[0]).toEqual({
        provider: 'github',
        host: 'github.com',
        login: 'octocat',
      });
    });
  });

  it('测试连接把主机拼成地址并展示远端引用条数', async () => {
    renderPanel();
    await screen.findByTestId('credentials-list');

    fireEvent.click(screen.getByTestId('credentials-probe-octocat'));

    await waitFor(() => {
      expect(probeMock.mock.calls[0]?.[0]).toEqual({ url: 'https://github.com' });
    });
    await waitFor(() => {
      expect(useToastStore.getState().toasts[0]?.title).toContain('3');
    });
  });
});
