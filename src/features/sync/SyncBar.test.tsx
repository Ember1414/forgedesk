import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { QueryClientProvider } from '@tanstack/react-query';
import { MemoryRouter, Route, Routes } from 'react-router-dom';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { SyncBar } from '@/features/sync/SyncBar';
import {
  cancelJob,
  gitBranchCompare,
  gitBranchList,
  gitFetch,
  gitPull,
  gitPush,
  settingsGet,
  settingsSet,
} from '@/lib/ipc';
import type { Branch } from '@/lib/ipc';
import { initialJobState, useJobStore } from '@/stores/jobStore';
import { initialToastState, useToastStore } from '@/stores/toastStore';
import { createTestQueryClient } from '@/test/queryClient';

/**
 * 三个 `job:*` 事件通道的处理器（组件挂载后注册进来）。
 *
 * 为什么要在测试里自己驱动事件：同步的成败**不在** invoke 的返回值里——
 * 后端立即返回 `jobId`，结果经事件到达。只断言"命令被调用过"会漏掉整个
 * 结果处理链路（进度、冲突、被拒），而那正是本任务最容易出错的部分。
 */
const bus = vi.hoisted(() => ({
  progress: [] as ((payload: unknown) => void)[],
  done: [] as ((payload: unknown) => void)[],
  failed: [] as ((payload: unknown) => void)[],
}));

vi.mock('@/lib/ipc', async (importOriginal) => {
  // 只替换"会碰 Tauri"的那几个函数：`progressPercent` / `readSyncResult` /
  // `pullConflicts` 这些纯函数必须用真实现，否则测的就不是产品代码了。
  const actual = (await importOriginal()) as Record<string, unknown>;
  return {
    ...actual,
    gitBranchList: vi.fn(),
    gitBranchCompare: vi.fn(),
    gitFetch: vi.fn(),
    gitPull: vi.fn(),
    gitPush: vi.fn(),
    cancelJob: vi.fn(),
    settingsGet: vi.fn(),
    settingsSet: vi.fn(),
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

const listMock = vi.mocked(gitBranchList);
const compareMock = vi.mocked(gitBranchCompare);
const fetchMock = vi.mocked(gitFetch);
const pullMock = vi.mocked(gitPull);
const pushMock = vi.mocked(gitPush);
const cancelMock = vi.mocked(cancelJob);
const settingsGetMock = vi.mocked(settingsGet);
const settingsSetMock = vi.mocked(settingsSet);

/** 当前分支：已配置上游且领先 2 / 落后 1。 */
function headBranch(overrides: Partial<Branch> = {}): Branch {
  return {
    name: 'main',
    isRemote: false,
    isHead: true,
    target: 'abc',
    upstream: 'origin/main',
    ahead: 2,
    behind: 1,
    upstreamGone: false,
    ...overrides,
  };
}

/** 后端被拒时给出的三条修复动作（形状由 `services/sync.rs` 决定）。 */
const REJECTED_ACTIONS = [
  {
    id: 'fetch-first',
    labelKey: 'errors:actions.pushFetchFirst',
    command: 'git_fetch',
  },
  {
    id: 'force-with-lease',
    labelKey: 'errors:actions.pushForceWithLease',
    command: 'noop',
  },
  { id: 'cancel', labelKey: 'errors:actions.cancel', command: 'noop' },
];

function renderBar() {
  const queryClient = createTestQueryClient();
  return render(
    <QueryClientProvider client={queryClient}>
      <MemoryRouter initialEntries={['/repo/7']}>
        <Routes>
          <Route path="/repo/:repoId/conflict" element={<p>冲突页占位</p>} />
          <Route path="/repo/:repoId" element={<SyncBar />} />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

/** 派发一个事件，并等 React 消化掉它带来的状态更新。 */
async function emit(handlers: ((payload: unknown) => void)[], payload: unknown): Promise<void> {
  await act(async () => {
    for (const handler of handlers) {
      handler(payload);
    }
  });
}

beforeEach(() => {
  vi.clearAllMocks();
  bus.progress = [];
  bus.done = [];
  bus.failed = [];
  useJobStore.setState(initialJobState);
  useToastStore.setState(initialToastState);

  listMock.mockResolvedValue([headBranch()]);
  compareMock.mockResolvedValue({ ahead: 2, behind: 1, onlyInA: [] });
  settingsGetMock.mockResolvedValue(null);
  settingsSetMock.mockResolvedValue(undefined);
  fetchMock.mockResolvedValue({ jobId: 'job-fetch' });
  pullMock.mockResolvedValue({ jobId: 'job-pull' });
  pushMock.mockResolvedValue({ jobId: 'job-push' });
  cancelMock.mockResolvedValue(true);
});

afterEach(() => {
  useJobStore.setState(initialJobState);
  useToastStore.setState(initialToastState);
});

describe('SyncBar', () => {
  it('展示三个同步入口与上游的领先/落后', async () => {
    renderBar();

    expect(screen.getByTestId('sync-fetch')).toHaveTextContent('获取');
    expect(screen.getByTestId('sync-pull')).toHaveTextContent('拉取');
    expect(screen.getByTestId('sync-push')).toHaveTextContent('推送');

    const status = await screen.findByTestId('sync-status');
    expect(status.textContent).toContain('origin/main');
    expect(screen.getByTestId('sync-ahead')).toHaveTextContent('↑2');
    expect(screen.getByTestId('sync-behind')).toHaveTextContent('↓1');
  });

  it('有上游时推送不带额外标志（git 推到上游）', async () => {
    renderBar();
    await screen.findByTestId('sync-status');

    fireEvent.click(screen.getByTestId('sync-push'));

    await waitFor(() => {
      expect(pushMock).toHaveBeenCalledWith(7, {});
    });
  });

  it('没有上游时带 --set-upstream（否则新分支无法推送）', async () => {
    listMock.mockResolvedValue([headBranch({ upstream: null })]);
    renderBar();
    await screen.findByTestId('sync-no-upstream');

    fireEvent.click(screen.getByTestId('sync-push'));

    await waitFor(() => {
      expect(pushMock).toHaveBeenCalledWith(7, { setUpstream: true });
    });
  });

  it('拉取用选中的策略（缺省仅快进）', async () => {
    renderBar();
    await screen.findByTestId('sync-status');

    fireEvent.click(screen.getByTestId('sync-pull'));

    await waitFor(() => {
      expect(pullMock).toHaveBeenCalledWith(7, { strategy: 'fastForwardOnly' });
    });
  });

  it('进度事件驱动进度条，并可展开 git 的原始输出', async () => {
    renderBar();
    await screen.findByTestId('sync-status');

    fireEvent.click(screen.getByTestId('sync-fetch'));
    await waitFor(() => {
      expect(useJobStore.getState().jobs).toHaveLength(1);
    });

    await emit(bus.progress, {
      jobId: 'job-fetch',
      phase: 'receiving',
      current: 3,
      total: 9,
      message: 'Receiving objects: 33% (3/9)',
    });

    const strip = await screen.findByTestId('sync-progress');
    expect(strip).toHaveTextContent('接收对象');
    expect(strip).toHaveTextContent('33%');

    fireEvent.click(screen.getByTestId('sync-detail-toggle'));
    expect(await screen.findByTestId('sync-detail')).toHaveTextContent('Receiving objects');

    // jobStore 是状态栏的数据源：进度必须同步过去
    await waitFor(() => {
      expect(useJobStore.getState().jobs[0]?.progress).toBeCloseTo(0.33, 2);
    });

    fireEvent.click(screen.getByTestId('sync-cancel'));
    await waitFor(() => {
      expect(cancelMock).toHaveBeenCalledWith('job-fetch');
    });
  });

  it('推送被拒：三条修复路径可点，覆盖前先拉取并由用户确认（红线 R7）', async () => {
    renderBar();
    await screen.findByTestId('sync-status');

    fireEvent.click(screen.getByTestId('sync-push'));
    await waitFor(() => {
      expect(useJobStore.getState().jobs).toHaveLength(1);
    });

    await emit(bus.failed, {
      jobId: 'job-push',
      error: {
        code: 'PUSH_REJECTED',
        message: 'the push was rejected because the remote has commits you do not have',
        detail: 'fetch first',
        actions: REJECTED_ACTIONS,
      },
    });

    const dialog = await screen.findByTestId('sync-rejected-dialog');
    expect(dialog).toHaveTextContent('推送被拒绝（远端有新提交）');
    expect(screen.getByTestId('sync-rejected-fetch-first')).toHaveTextContent('先拉取');
    expect(screen.getByTestId('sync-rejected-force')).toHaveTextContent('覆盖远端');

    // 点"覆盖"不会立刻强推：先拉取，让 lease 的比较基准是**新鲜的**远端状态
    fireEvent.click(screen.getByTestId('sync-rejected-force'));
    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledTimes(1);
    });
    expect(pushMock).toHaveBeenCalledTimes(1);
    await waitFor(() => {
      expect(screen.queryByTestId('sync-rejected-dialog')).not.toBeInTheDocument();
    });

    // 拉取完成 → 弹出确认对话框，并把"会被覆盖多少"摊开给用户看
    await waitFor(() => {
      expect(useJobStore.getState().jobs).toHaveLength(2);
    });
    await emit(bus.done, {
      jobId: 'job-fetch',
      result: { remote: 'origin', fetch: { remote: 'origin', updates: [] } },
    });

    const confirm = await screen.findByTestId('sync-lease-dialog');
    expect(confirm).toHaveTextContent('确认覆盖远端？');
    expect(confirm).toHaveTextContent('领先 2 个提交、落后 1 个');

    fireEvent.click(screen.getByTestId('sync-lease-confirm'));

    await waitFor(() => {
      expect(pushMock).toHaveBeenCalledTimes(2);
    });
    expect(pushMock.mock.calls[1]?.[1]).toEqual({ forceWithLease: true });
  });

  it('拉取冲突：列出冲突文件并引导到冲突页', async () => {
    renderBar();
    await screen.findByTestId('sync-status');

    fireEvent.click(screen.getByTestId('sync-pull'));
    await waitFor(() => {
      expect(useJobStore.getState().jobs).toHaveLength(1);
    });

    await emit(bus.done, {
      jobId: 'job-pull',
      result: {
        remote: 'origin',
        pull: {
          fetch: { remote: 'origin', updates: [] },
          strategy: 'merge',
          upToDate: false,
          merge: { kind: 'conflicted', oid: null, conflicts: ['a.txt', 'b.txt'] },
        },
      },
    });

    const dialog = await screen.findByTestId('sync-conflict-dialog');
    expect(dialog).toHaveTextContent('拉取产生了冲突');
    expect(screen.getByTestId('sync-conflict-files')).toHaveTextContent('a.txt');
    expect(screen.getByTestId('sync-conflict-files')).toHaveTextContent('b.txt');

    fireEvent.click(screen.getByTestId('sync-conflict-guide'));

    expect(await screen.findByText('冲突页占位')).toBeInTheDocument();
  });

  it('用户取消的任务不再弹错误提示（取消是用户的决定，不是故障）', async () => {
    renderBar();
    await screen.findByTestId('sync-status');

    fireEvent.click(screen.getByTestId('sync-fetch'));
    await waitFor(() => {
      expect(useJobStore.getState().jobs).toHaveLength(1);
    });

    await emit(bus.failed, {
      jobId: 'job-fetch',
      error: { code: 'CANCELLED', message: 'git command was cancelled before it finished' },
    });

    expect(useToastStore.getState().toasts).toHaveLength(0);
    expect(screen.queryByTestId('sync-progress')).not.toBeInTheDocument();
  });
});
