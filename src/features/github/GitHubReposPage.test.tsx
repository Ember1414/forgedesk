import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter, Route, Routes } from 'react-router-dom';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { GitHubReposPage } from '@/features/github/GitHubReposPage';
import {
  repoRemoteFork,
  repoRemoteList,
  repoRemoteSearch,
  repoRemoteStar,
  repoRemoteStarred,
} from '@/lib/ipc';
import type { RemoteRepo, RemoteRepoPage } from '@/lib/ipc';
import { initialToastState, useToastStore } from '@/stores/toastStore';

/**
 * 远程仓库页（T4.5）。
 *
 * 钉住的是分页契约（nextPage 为 null 才隐藏"加载更多"）、
 * 标签语义（星标页里是"取消星标"且成功后移出列表）、
 * 以及 AUTH_REQUIRED 的"去登录"直达入口——这三点错一个，
 * 用户看到的就是错误的数据或一条死胡同。
 */
vi.mock('@/lib/ipc', () => ({
  repoRemoteList: vi.fn(),
  repoRemoteStarred: vi.fn(),
  repoRemoteSearch: vi.fn(),
  repoRemoteStar: vi.fn(),
  repoRemoteFork: vi.fn(),
}));

const listMock = vi.mocked(repoRemoteList);
const starredMock = vi.mocked(repoRemoteStarred);
const searchMock = vi.mocked(repoRemoteSearch);
const starMock = vi.mocked(repoRemoteStar);
const forkMock = vi.mocked(repoRemoteFork);

function repo(name: string, overrides: Partial<RemoteRepo> = {}): RemoteRepo {
  return {
    id: name.length,
    owner: 'octocat',
    name,
    fullName: `octocat/${name}`,
    description: `${name} 的描述`,
    htmlUrl: `https://github.com/octocat/${name}`,
    defaultBranch: 'main',
    private: false,
    fork: false,
    stars: 3,
    ...overrides,
  };
}

function page(items: readonly RemoteRepo[], nextPage: number | null): RemoteRepoPage {
  return { items, nextPage };
}

function renderPage(): void {
  render(
    <MemoryRouter initialEntries={['/github/repos']}>
      <Routes>
        <Route path="/github/repos" element={<GitHubReposPage />} />
        <Route path="/settings/github" element={<p data-testid="settings-target">设置页</p>} />
      </Routes>
    </MemoryRouter>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  useToastStore.setState(initialToastState);
  listMock.mockResolvedValue(page([repo('Hello-World')], null));
  starredMock.mockResolvedValue(page([], null));
  searchMock.mockResolvedValue(page([], null));
  starMock.mockResolvedValue(undefined);
  forkMock.mockResolvedValue(repo('Hello-World', { owner: 'me', fullName: 'me/Hello-World' }));
});

afterEach(() => {
  useToastStore.setState(initialToastState);
});

describe('GitHubReposPage — 列表与分页', () => {
  it('默认加载"我的"标签并渲染仓库条目', async () => {
    renderPage();

    await waitFor(() => {
      expect(listMock).toHaveBeenCalledWith('github.com', { scope: 'owned', page: 1 });
    });
    await waitFor(() => {
      expect(screen.getAllByTestId('repos-item')).toHaveLength(1);
    });
    expect(screen.getByText('octocat/Hello-World')).toBeVisible();
  });

  it('nextPage 存在时显示加载更多，点击追加下一页', async () => {
    listMock
      .mockResolvedValueOnce(page([repo('one')], 2))
      .mockResolvedValueOnce(page([repo('two')], null));
    renderPage();

    await waitFor(() => {
      expect(screen.getByTestId('repos-load-more')).toBeVisible();
    });
    fireEvent.click(screen.getByTestId('repos-load-more'));

    await waitFor(() => {
      expect(listMock).toHaveBeenCalledWith('github.com', { scope: 'owned', page: 2 });
    });
    await waitFor(() => {
      expect(screen.getAllByTestId('repos-item')).toHaveLength(2);
    });
    // 末页：按钮消失
    expect(screen.queryByTestId('repos-load-more')).toBeNull();
  });

  it('列表为空时显示空态', async () => {
    listMock.mockResolvedValue(page([], null));
    renderPage();

    await waitFor(() => {
      expect(screen.getByTestId('repos-empty')).toBeVisible();
    });
  });
});

describe('GitHubReposPage — 星标语义', () => {
  it('"我的"标签里按钮是加星，成功后提示', async () => {
    renderPage();
    await waitFor(() => {
      expect(screen.getByTestId('repos-star-Hello-World')).toBeVisible();
    });

    fireEvent.click(screen.getByTestId('repos-star-Hello-World'));
    await waitFor(() => {
      expect(starMock).toHaveBeenCalledWith('github.com', 'octocat', 'Hello-World', true);
    });
    await waitFor(() => {
      expect(useToastStore.getState().toasts.length).toBeGreaterThan(0);
    });
    // 非星标页：条目保持原位
    expect(screen.getAllByTestId('repos-item')).toHaveLength(1);
  });

  it('"星标"标签里按钮是取消星标，成功后条目移出列表', async () => {
    starredMock.mockResolvedValue(page([repo('liked')], null));
    renderPage();

    fireEvent.click(screen.getByTestId('repos-tab-starred'));
    await waitFor(() => {
      expect(starredMock).toHaveBeenCalledWith('github.com', { page: 1 });
    });
    await waitFor(() => {
      expect(screen.getByTestId('repos-star-liked')).toBeVisible();
    });

    fireEvent.click(screen.getByTestId('repos-star-liked'));
    await waitFor(() => {
      expect(starMock).toHaveBeenCalledWith('github.com', 'octocat', 'liked', false);
    });
    await waitFor(() => {
      expect(screen.queryByTestId('repos-item')).toBeNull();
    });
  });
});

describe('GitHubReposPage — 搜索与 fork', () => {
  it('搜索标签提交后按关键词查询', async () => {
    renderPage();

    fireEvent.click(screen.getByTestId('repos-tab-search'));
    fireEvent.change(screen.getByTestId('repos-search-input'), {
      target: { value: 'forgedesk' },
    });
    fireEvent.click(screen.getByTestId('repos-search-go'));

    await waitFor(() => {
      expect(searchMock).toHaveBeenCalledWith('github.com', 'forgedesk', { page: 1 });
    });
  });

  it('fork 成功后提示新副本', async () => {
    renderPage();
    await waitFor(() => {
      expect(screen.getByTestId('repos-fork-Hello-World')).toBeVisible();
    });

    fireEvent.click(screen.getByTestId('repos-fork-Hello-World'));
    await waitFor(() => {
      expect(forkMock).toHaveBeenCalledWith('github.com', 'octocat', 'Hello-World');
    });
    await waitFor(() => {
      expect(useToastStore.getState().toasts.length).toBeGreaterThan(0);
    });
  });
});

describe('GitHubReposPage — 未登录引导', () => {
  it('AUTH_REQUIRED 时给出"去登录"入口并可跳转设置页', async () => {
    listMock.mockRejectedValue({ code: 'AUTH_REQUIRED', message: 'no account' });
    renderPage();

    await waitFor(() => {
      expect(screen.getByTestId('repos-sign-in-go')).toBeVisible();
    });
    expect(screen.queryByTestId('repos-load-more')).toBeNull();

    fireEvent.click(screen.getByTestId('repos-sign-in-go'));
    await waitFor(() => {
      expect(screen.getByTestId('settings-target')).toBeVisible();
    });
  });
});
