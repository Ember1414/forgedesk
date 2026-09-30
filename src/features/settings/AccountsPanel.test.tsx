import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { QueryClientProvider } from '@tanstack/react-query';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { AccountsPanel } from '@/features/settings/AccountsPanel';
import {
  accountDeviceFlowStart,
  accountDeviceFlowWait,
  accountList,
  accountLoginWithPat,
  accountRemove,
} from '@/lib/ipc/accounts';
import { cancelJob } from '@/lib/ipc/jobs';
import type { Account, DeviceFlowSession } from '@/lib/ipc/accounts';
import { initialToastState, useToastStore } from '@/stores/toastStore';
import { createTestQueryClient } from '@/test/queryClient';

/**
 * 账号面板（T4.4）。
 *
 * 这一层钉住的是**秘密的路径**：PAT 只在提交那一次离开输入框；
 * Device Flow 的会话数据里没有 device_code（后端就不给），job 结果
 * 只带账号信息。断言集中在"谁被调了、参数是什么、界面进入哪个阶段"，
 * 文案与样式交给 i18n 与设计 token。
 */
const bus = {
  done: [] as Array<(payload: unknown) => void>,
  failed: [] as Array<(payload: unknown) => void>,
};

vi.mock('@/lib/ipc/accounts', () => ({
  accountList: vi.fn(),
  accountLoginWithPat: vi.fn(),
  accountDeviceFlowStart: vi.fn(),
  accountDeviceFlowWait: vi.fn(),
  accountRemove: vi.fn(),
}));

vi.mock('@/lib/ipc/jobs', () => ({
  cancelJob: vi.fn().mockResolvedValue(true),
  onJobDone: (handler: (payload: unknown) => void) => {
    bus.done.push(handler);
    return Promise.resolve(() => undefined);
  },
  onJobFailed: (handler: (payload: unknown) => void) => {
    bus.failed.push(handler);
    return Promise.resolve(() => undefined);
  },
}));

const listMock = vi.mocked(accountList);
const patMock = vi.mocked(accountLoginWithPat);
const startMock = vi.mocked(accountDeviceFlowStart);
const waitMock = vi.mocked(accountDeviceFlowWait);
const removeMock = vi.mocked(accountRemove);

function account(login: string): Account {
  return {
    id: `id-${login}`,
    provider: 'github',
    host: 'github.com',
    login,
    scopes: ['repo', 'read:org'],
    createdAt: 1_790_000_000_000,
  };
}

function session(): DeviceFlowSession {
  return {
    flowId: 'flow-1',
    userCode: 'WDJB-MJTK',
    verificationUri: 'https://github.com/login/device',
    expiresInSecs: 900,
    intervalSecs: 5,
  };
}

function renderPanel() {
  const queryClient = createTestQueryClient();
  const view = render(
    <QueryClientProvider client={queryClient}>
      <AccountsPanel />
    </QueryClientProvider>,
  );
  return { queryClient, ...view };
}

beforeEach(() => {
  vi.clearAllMocks();
  bus.done = [];
  bus.failed = [];
  useToastStore.setState(initialToastState);
  listMock.mockResolvedValue([]);
  patMock.mockResolvedValue(account('octocat'));
  startMock.mockResolvedValue(session());
  waitMock.mockResolvedValue({ jobId: 'job-flow-1' });
  removeMock.mockResolvedValue(undefined);
});

afterEach(() => {
  cleanupAccounts();
});

function cleanupAccounts(): void {
  // 屏幕清理由 setup 的 afterEach 统一做；这里只复位 store
  useToastStore.setState(initialToastState);
}

describe('AccountsPanel — 账号列表', () => {
  it('已登录的账号显示登录名、站点与移除按钮', async () => {
    listMock.mockResolvedValue([account('octocat'), account('hubot')]);
    renderPanel();

    await waitFor(() => {
      expect(screen.getAllByTestId('account-item')).toHaveLength(2);
    });
    expect(screen.getByText('octocat')).toBeVisible();
    expect(screen.getByText('hubot')).toBeVisible();
    expect(screen.getByTestId('account-remove-octocat')).toBeVisible();
  });

  it('没有账号时显示空态而不是报错', async () => {
    renderPanel();

    await waitFor(() => {
      expect(screen.getByTestId('account-empty')).toBeVisible();
    });
    expect(screen.queryByTestId('account-item')).toBeNull();
  });
});

describe('AccountsPanel — PAT 登录', () => {
  it('提交后调用 accountLoginWithPat，成功即提示并刷新列表', async () => {
    renderPanel();

    fireEvent.click(screen.getByTestId('account-add'));
    await waitFor(() => {
      expect(screen.getByTestId('account-login-form')).toBeVisible();
    });
    fireEvent.click(screen.getByTestId('account-mode-pat'));
    fireEvent.change(screen.getByTestId('account-host'), { target: { value: 'github.com' } });
    fireEvent.change(screen.getByTestId('account-token'), { target: { value: 'ghp_secret' } });
    fireEvent.click(screen.getByTestId('account-pat-submit'));

    await waitFor(() => {
      expect(patMock).toHaveBeenCalledWith('github.com', 'ghp_secret');
    });
    // 成功提示与列表刷新
    await waitFor(() => {
      expect(useToastStore.getState().toasts.length).toBeGreaterThan(0);
      expect(listMock).toHaveBeenCalledTimes(2);
    });
    // 令牌不得留在输入框里（红线 R8 的界面侧）
    await waitFor(() => {
      expect(screen.queryByTestId('account-login-form')).toBeNull();
    });
  });

  it('令牌为空时提交按钮不可用', async () => {
    renderPanel();

    fireEvent.click(screen.getByTestId('account-add'));
    fireEvent.click(screen.getByTestId('account-mode-pat'));
    await waitFor(() => {
      expect(screen.getByTestId('account-pat-submit')).toBeDisabled();
    });
  });
});

describe('AccountsPanel — 设备码登录', () => {
  it('获取设备码后显示 user_code 并启动等待任务，授权完成经 job:done 关闭', async () => {
    renderPanel();

    fireEvent.click(screen.getByTestId('account-add'));
    await waitFor(() => {
      expect(screen.getByTestId('account-login-form')).toBeVisible();
    });
    fireEvent.change(screen.getByTestId('account-host'), { target: { value: 'github.com' } });
    fireEvent.click(screen.getByTestId('account-device-submit'));

    await waitFor(() => {
      expect(startMock).toHaveBeenCalledWith('github.com');
    });
    await waitFor(() => {
      expect(waitMock).toHaveBeenCalledWith('flow-1');
    });
    // 三步引导：码 + 链接 + 复制按钮可见
    expect(screen.getByTestId('account-user-code')).toHaveTextContent('WDJB-MJTK');
    expect(screen.getByTestId('account-verification-uri')).toHaveTextContent(
      'https://github.com/login/device',
    );
    expect(screen.getByTestId('account-copy-code')).toBeVisible();

    // 后端轮询完成：job:done 只携带账号信息（没有令牌材料）
    for (const handler of bus.done) {
      handler({ jobId: 'job-flow-1', result: { account: account('octocat') } });
    }
    await waitFor(() => {
      expect(useToastStore.getState().toasts.length).toBeGreaterThan(0);
      expect(screen.queryByTestId('account-waiting')).toBeNull();
      expect(listMock).toHaveBeenCalledTimes(2);
    });
  });

  it('job:failed 时向导留在等待页并给出重试提示', async () => {
    renderPanel();

    fireEvent.click(screen.getByTestId('account-add'));
    await waitFor(() => {
      expect(screen.getByTestId('account-login-form')).toBeVisible();
    });
    fireEvent.click(screen.getByTestId('account-device-submit'));
    await waitFor(() => {
      expect(screen.getByTestId('account-waiting')).toBeVisible();
    });

    for (const handler of bus.failed) {
      handler({ jobId: 'job-flow-1', error: { code: 'AUTH_EXPIRED', message: 'expired' } });
    }
    await waitFor(() => {
      expect(screen.getByTestId('account-waiting-error')).toBeVisible();
    });
    expect(screen.getByTestId('account-waiting')).toBeVisible();
  });

  it('取消等待会调用 job_cancel 并回到表单', async () => {
    renderPanel();

    fireEvent.click(screen.getByTestId('account-add'));
    await waitFor(() => {
      expect(screen.getByTestId('account-login-form')).toBeVisible();
    });
    fireEvent.click(screen.getByTestId('account-device-submit'));
    await waitFor(() => {
      expect(screen.getByTestId('account-waiting')).toBeVisible();
    });

    fireEvent.click(screen.getByTestId('account-cancel'));
    await waitFor(() => {
      expect(cancelJob).toHaveBeenCalledWith('job-flow-1');
    });
    expect(screen.queryByTestId('account-waiting')).toBeNull();
  });
});

describe('AccountsPanel — 移除账号', () => {
  it('删除需要确认，确认后调用 accountRemove 并刷新', async () => {
    listMock.mockResolvedValue([account('octocat')]);
    renderPanel();

    await waitFor(() => {
      expect(screen.getByTestId('account-remove-octocat')).toBeVisible();
    });
    fireEvent.click(screen.getByTestId('account-remove-octocat'));
    // 先出现确认框，还没有真正删除
    expect(screen.getByTestId('account-remove-confirm')).toBeVisible();
    expect(removeMock).not.toHaveBeenCalled();

    fireEvent.click(screen.getByTestId('account-remove-confirm'));
    await waitFor(() => {
      expect(removeMock).toHaveBeenCalledWith('id-octocat');
    });
    await waitFor(() => {
      expect(useToastStore.getState().toasts.length).toBeGreaterThan(0);
    });
  });
});
