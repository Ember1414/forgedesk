/**
 * "添加仓库"卡片的单测（GIT-01/02/03 UI）。
 *
 * 三个事件通道（progress/done/failed）的处理器由测试捕获后手动驱动——
 * 克隆的结果不在 invoke 返回值里（那里只有 jobId），断言"命令被调过"
 * 会漏掉 `job:done → 打开新仓库` 这条结果链路。
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { QueryClientProvider } from '@tanstack/react-query';
import { MemoryRouter, Route, Routes, useLocation } from 'react-router-dom';
import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';

import { createTestQueryClient } from '@/test/queryClient';
import { useUiStore } from '@/stores/uiStore';
import { useJobStore, initialJobState } from '@/stores/jobStore';
import { useToastStore, initialToastState } from '@/stores/toastStore';

import { AddRepoCard } from '@/features/dashboard/AddRepoCard';

const bus = vi.hoisted(() => ({
  progress: [] as ((payload: unknown) => void)[],
  done: [] as ((payload: unknown) => void)[],
  failed: [] as ((payload: unknown) => void)[],
}));

vi.mock('@/lib/ipc', async (importOriginal) => {
  const actual = (await importOriginal()) as Record<string, unknown>;
  return {
    ...actual,
    repoOpen: vi.fn(),
    repoClone: vi.fn(),
    repoInit: vi.fn(),
    pickFolder: vi.fn(),
    repoRecentList: vi.fn().mockResolvedValue([]),
    onJobProgress: (handler: (payload: unknown) => void) => {
      bus.progress.push(handler);
      return Promise.resolve(() => undefined);
    },
    onJobDone: (handler: (payload: unknown) => void) => {
      bus.done.push(handler);
      return Promise.resolve(() => undefined);
    },
    onJobFailed: (handler: (payload: unknown) => void) => {
      bus.failed.push(handler);
      return Promise.resolve(() => undefined);
    },
  };
});

import { repoClone, repoInit, repoOpen } from '@/lib/ipc';
import type { OpenedRepository } from '@/lib/ipc';
const repoOpenMock = vi.mocked(repoOpen);
const repoCloneMock = vi.mocked(repoClone);
const repoInitMock = vi.mocked(repoInit);

/** 显示当前位置：断言成功后的跳转目标。 */
function LocationProbe() {
  const location = useLocation();
  return <span data-testid="location">{location.pathname}</span>;
}

function renderCard(): void {
  const queryClient = createTestQueryClient();
  render(
    <QueryClientProvider client={queryClient}>
      <MemoryRouter initialEntries={['/']}>
        <Routes>
          <Route
            path="/"
            element={
              <>
                <AddRepoCard />
                <LocationProbe />
              </>
            }
          />
          <Route path="/repo/:repoId/status" element={<LocationProbe />} />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  useUiStore.setState({ ...useUiStore.getState(), currentRepoId: null });
  useJobStore.setState(initialJobState);
  useToastStore.setState(initialToastState);
  repoOpenMock.mockReset();
  repoCloneMock.mockReset();
  repoInitMock.mockReset();
});

afterEach(() => {
  useToastStore.setState(initialToastState);
});

// 显式注成 IPC 契约形状：fixture 漏字段/错字面量由 typecheck 当场拦住，
// 而不是等 CI（本机漏跑 typecheck 被抓过一次）
const openRepoResult: OpenedRepository = {
  recordId: 7,
  repository: {
    workdir: 'D:/demo',
    gitDir: 'D:/demo/.git',
    isBare: false,
    isEmpty: false,
    head: 'main',
    detached: false,
    upstream: null,
    defaultBranch: 'main',
    isShallow: false,
    isLfs: false,
    worktrees: [],
    branchLabel: { kind: 'named', name: 'main' },
  },
  audit: { findings: [], hasDanger: false, maxSeverity: null },
  gitVersion: '2.50.0',
  gitVersionSupported: true,
  needsGitUpgrade: false,
};

describe('AddRepoCard — 模式切换', () => {
  it('默认是"打开"，三个模式各有自己的表单', async () => {
    renderCard();
    expect(screen.getByTestId('add-repo-open')).toBeVisible();
    expect(screen.queryByTestId('add-repo-clone')).toBeNull();

    fireEvent.click(screen.getByRole('radio', { name: '克隆' }));
    expect(screen.getByTestId('add-repo-clone')).toBeVisible();
    expect(screen.queryByTestId('add-repo-open')).toBeNull();

    fireEvent.click(screen.getByRole('radio', { name: '初始化' }));
    expect(screen.getByTestId('add-repo-init')).toBeVisible();
  });

  it('路径为空时提交按钮禁用', () => {
    renderCard();
    expect(screen.getByTestId('add-repo-open-submit')).toBeDisabled();
  });
});

describe('AddRepoCard — 打开', () => {
  it('repoOpen 成功后：登记当前仓库并跳到工作区', async () => {
    repoOpenMock.mockResolvedValue(openRepoResult);
    renderCard();

    fireEvent.change(screen.getByTestId('add-repo-path'), {
      target: { value: 'D:/demo' },
    });
    fireEvent.click(screen.getByTestId('add-repo-open-submit'));

    await waitFor(() => {
      expect(repoOpenMock).toHaveBeenCalledWith('D:/demo');
    });
    await waitFor(() => {
      expect(screen.getByTestId('location')).toHaveTextContent('/repo/7/status');
    });
    expect(useUiStore.getState().currentRepoId).toBe('7');
  });

  it('打开失败时弹 danger toast，不跳转', async () => {
    repoOpenMock.mockRejectedValue({
      code: 'PATH_NOT_REPO',
      message: 'not a git repository',
    });
    renderCard();

    fireEvent.change(screen.getByTestId('add-repo-path'), { target: { value: 'D:/nope' } });
    fireEvent.click(screen.getByTestId('add-repo-open-submit'));

    await waitFor(() => {
      const toasts = useToastStore.getState().toasts;
      expect(toasts.length).toBeGreaterThan(0);
      expect(toasts[0]?.tone).toBe('danger');
    });
    expect(screen.getByTestId('location')).toHaveTextContent('/');
  });
});

describe('AddRepoCard — 克隆', () => {
  it('repoClone 返回 jobId 后进入"克隆中"，job:done 携带 recordId 自动打开', async () => {
    repoCloneMock.mockResolvedValue({ jobId: 'job-clone-1' });
    renderCard();

    fireEvent.click(screen.getByRole('radio', { name: '克隆' }));
    fireEvent.change(screen.getByTestId('add-repo-url'), {
      target: { value: 'https://github.com/u/r.git' },
    });
    fireEvent.change(screen.getByTestId('add-repo-clone-target'), {
      target: { value: 'D:/demo' },
    });
    fireEvent.click(screen.getByTestId('add-repo-clone-submit'));

    await waitFor(() => {
      expect(repoCloneMock).toHaveBeenCalledWith({
        url: 'https://github.com/u/r.git',
        into: 'D:/demo',
      });
    });
    // 任务进了全局投影（仪表盘任务卡片）
    await waitFor(() => {
      expect(useJobStore.getState().jobs).toHaveLength(1);
      expect(useJobStore.getState().jobs[0]?.state).toBe('running');
    });
    // 界面显示"克隆中"
    expect(screen.getByTestId('add-repo-clone-status')).toBeVisible();

    // 结果经事件到达：recordId 在 job:done 的 result 里
    for (const handler of bus.done) {
      handler({ jobId: 'job-clone-1', result: { recordId: 9 } });
    }
    await waitFor(() => {
      expect(screen.getByTestId('location')).toHaveTextContent('/repo/9/status');
    });
    expect(useUiStore.getState().currentRepoId).toBe('9');
  });

  it('job:failed 时解除克隆中状态且不跳转', async () => {
    repoCloneMock.mockResolvedValue({ jobId: 'job-clone-2' });
    renderCard();

    fireEvent.click(screen.getByRole('radio', { name: '克隆' }));
    fireEvent.change(screen.getByTestId('add-repo-url'), { target: { value: 'x' } });
    fireEvent.change(screen.getByTestId('add-repo-clone-target'), { target: { value: 'y' } });
    fireEvent.click(screen.getByTestId('add-repo-clone-submit'));

    await waitFor(() => {
      expect(useJobStore.getState().jobs).toHaveLength(1);
    });
    for (const handler of bus.failed) {
      handler({ jobId: 'job-clone-2', error: { code: 'NETWORK', message: 'offline' } });
    }
    await waitFor(() => {
      expect(screen.queryByTestId('add-repo-clone-status')).toBeNull();
    });
    expect(screen.getByTestId('location')).toHaveTextContent('/');
  });
});

describe('AddRepoCard — 初始化', () => {
  it('repoInit 成功后登记并跳转；分支名留空时不传该字段', async () => {
    repoInitMock.mockResolvedValue(openRepoResult);
    renderCard();

    fireEvent.click(screen.getByRole('radio', { name: '初始化' }));
    fireEvent.change(screen.getByTestId('add-repo-init-path'), {
      target: { value: 'D:/fresh' },
    });
    fireEvent.click(screen.getByTestId('add-repo-init-submit'));

    await waitFor(() => {
      expect(repoInitMock).toHaveBeenCalledWith({ path: 'D:/fresh' });
    });
    await waitFor(() => {
      expect(screen.getByTestId('location')).toHaveTextContent('/repo/7/status');
    });
  });
});
