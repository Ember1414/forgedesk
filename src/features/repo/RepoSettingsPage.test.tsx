import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter, Route, Routes } from 'react-router-dom';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { RepoSettingsPage } from '@/features/repo/RepoSettingsPage';
import { accountList, repoAccountBindingGet, repoAccountBindingSet } from '@/lib/ipc';
import type { Account } from '@/lib/ipc';
import { initialToastState, useToastStore } from '@/stores/toastStore';

/**
 * 仓库设置页（T4.5）——绑定账号面板。
 *
 * 钉住三件事：绑定按"点击即生效"落库（repo_account_binding_set 的参数
 * 就是后端契约）；解除绑定传 null；同 host 多账号时用户必须能看出
 * "现在绑定的是谁"。
 */
vi.mock('@/lib/ipc', () => ({
  accountList: vi.fn(),
  repoAccountBindingGet: vi.fn(),
  repoAccountBindingSet: vi.fn(),
}));

const listMock = vi.mocked(accountList);
const getMock = vi.mocked(repoAccountBindingGet);
const setMock = vi.mocked(repoAccountBindingSet);

function account(id: string, login: string, host = 'github.com'): Account {
  return { id, provider: 'github', host, login, scopes: ['repo'], createdAt: 1 };
}

function renderAt(repoId = '7'): void {
  render(
    <MemoryRouter initialEntries={[`/repo/${repoId}/settings`]}>
      <Routes>
        <Route path="/repo/:repoId/settings" element={<RepoSettingsPage />} />
      </Routes>
    </MemoryRouter>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  useToastStore.setState(initialToastState);
  listMock.mockResolvedValue([account('a1', 'octocat'), account('a2', 'hubot')]);
  getMock.mockResolvedValue(null);
  setMock.mockImplementation(async (_repoId, accountId) => {
    if (accountId === null) {
      return null;
    }
    const found = [account('a1', 'octocat'), account('a2', 'hubot')].find(
      (item) => item.id === accountId,
    );
    return found ?? null;
  });
});

afterEach(() => {
  useToastStore.setState(initialToastState);
});

describe('RepoSettingsPage — 账号绑定', () => {
  it('展示账号列表并标出当前绑定的账号', async () => {
    getMock.mockResolvedValue(account('a2', 'hubot'));
    renderAt();

    await waitFor(() => {
      expect(getMock).toHaveBeenCalledWith(7);
    });
    await waitFor(() => {
      expect(screen.getByTestId('account-binding-octocat')).toBeVisible();
    });
    expect(screen.getByTestId('account-binding-hubot')).toBeVisible();
    // 当前绑定者处于选中态
    expect(screen.getByTestId('account-binding-hubot')).toHaveAttribute('aria-pressed', 'true');
    expect(screen.getByTestId('account-binding-none')).toHaveAttribute('aria-pressed', 'false');
  });

  it('点击账号即绑定并提示', async () => {
    renderAt();

    await waitFor(() => {
      expect(screen.getByTestId('account-binding-octocat')).toBeVisible();
    });
    fireEvent.click(screen.getByTestId('account-binding-octocat'));

    await waitFor(() => {
      expect(setMock).toHaveBeenCalledWith(7, 'a1');
    });
    await waitFor(() => {
      expect(useToastStore.getState().toasts.length).toBeGreaterThan(0);
    });
  });

  it('点"不绑定"解除绑定（后端收到 null）', async () => {
    getMock.mockResolvedValue(account('a1', 'octocat'));
    renderAt();

    await waitFor(() => {
      expect(screen.getByTestId('account-binding-none')).toBeVisible();
    });
    fireEvent.click(screen.getByTestId('account-binding-none'));

    await waitFor(() => {
      expect(setMock).toHaveBeenCalledWith(7, null);
    });
  });

  it('没有已登录账号时给出去登录的入口', async () => {
    listMock.mockResolvedValue([]);
    renderAt();

    await waitFor(() => {
      expect(screen.getByTestId('account-binding-empty')).toBeVisible();
    });
    expect(screen.getByTestId('account-binding-sign-in-go')).toBeVisible();
  });

  it('非法仓库 id 不发起任何请求', async () => {
    renderAt('not-a-number');

    await waitFor(() => {
      expect(screen.getByTestId('repo-settings-page')).toBeVisible();
    });
    expect(listMock).not.toHaveBeenCalled();
    expect(getMock).not.toHaveBeenCalled();
  });
});
