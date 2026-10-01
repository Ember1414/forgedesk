import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { GitHubActionsPage } from '@/features/github/GitHubActionsPage';
import {
  JOB_FAILED_EVENT,
  JOB_DONE_EVENT,
  listenActionsLogChunks,
  repoActionsJobLogs,
  repoActionsRunCancel,
  repoActionsRunJobs,
  repoActionsRunRerun,
  repoActionsRunsList,
} from '@/lib/ipc';
import type { ActionsLogChunkPayload } from '@/lib/ipc';
import type { WorkflowRunSummary } from '@/lib/ipc';
import { initialToastState, useToastStore } from '@/stores/toastStore';

/**
 * Actions 页（T4.9 UI）。
 *
 * 钉住：run 列表渲染与详情打开、取消/重跑的判定（completed 才能重跑、
 * 未完成才能取消）、日志对话框按 jobId 过滤事件流并把块拼成行、
 * job:done 结束加载态。
 */
vi.mock('@/lib/ipc', () => ({
  repoActionsRunsList: vi.fn(),
  repoActionsRunJobs: vi.fn(),
  repoActionsRunCancel: vi.fn(),
  repoActionsRunRerun: vi.fn(),
  repoActionsJobLogs: vi.fn(),
  listenActionsLogChunks: vi.fn(),
  JOB_DONE_EVENT: 'job:done',
  JOB_FAILED_EVENT: 'job:failed',
}));
vi.mock('@/lib/ipc/client', () => ({
  listenEvent: vi.fn(),
}));

const listMock = vi.mocked(repoActionsRunsList);
const jobsMock = vi.mocked(repoActionsRunJobs);
const cancelMock = vi.mocked(repoActionsRunCancel);
const rerunMock = vi.mocked(repoActionsRunRerun);
const logsMock = vi.mocked(repoActionsJobLogs);
const logChunksMock = vi.mocked(listenActionsLogChunks);
const listenEventMock = vi.mocked((await import('@/lib/ipc/client')).listenEvent);

function run(id: number, overrides: Partial<WorkflowRunSummary> = {}): WorkflowRunSummary {
  return {
    id,
    name: `Run ${id}`,
    headBranch: 'main',
    headSha: 'abc123',
    status: 'completed',
    conclusion: 'success',
    event: 'push',
    actor: 'octocat',
    runNumber: id,
    createdAt: '2026-10-01T00:00:00Z',
    updatedAt: '2026-10-01T00:05:00Z',
    htmlUrl: `https://github.com/octocat/x/actions/runs/${id}`,
    ...overrides,
  };
}

type ChunkHandler = (payload: ActionsLogChunkPayload) => void;
type DoneHandler = (payload: { jobId: string }) => void;
type FailedHandler = (payload: { jobId: string; error?: { code?: string } }) => void;

/** 测试内的事件总线：捕获 chunk/done/failed 处理器后手动派发。 */
const bus: {
  chunks: ChunkHandler[];
  done: DoneHandler[];
  failed: FailedHandler[];
} = { chunks: [], done: [], failed: [] };

function renderPage(): void {
  render(<GitHubActionsPage />);
}

async function openRun(id: number): Promise<void> {
  renderPage();
  const input = screen.getByLabelText('仓库（owner/repo，如 octocat/Hello-World）');
  fireEvent.change(input, { target: { value: 'octocat/x' } });
  fireEvent.submit(screen.getByTestId('actions-repo-go'));
  await waitFor(() => expect(screen.getByTestId(`actions-item-${id}`)).toBeInTheDocument());
  fireEvent.click(screen.getByTestId(`actions-item-${id}`));
  await waitFor(() => expect(jobsMock).toHaveBeenCalled());
}

beforeEach(() => {
  vi.clearAllMocks();
  bus.chunks = [];
  bus.done = [];
  bus.failed = [];
  useToastStore.setState(initialToastState);
  listMock.mockResolvedValue({ items: [run(10), run(11)], nextPage: null });
  jobsMock.mockResolvedValue([
    {
      id: 100,
      name: 'build',
      status: 'completed',
      conclusion: 'failure',
      startedAt: '2026-10-01T00:00:00Z',
      completedAt: '2026-10-01T00:03:00Z',
    },
  ]);
  cancelMock.mockResolvedValue(undefined);
  rerunMock.mockResolvedValue(undefined);
  logsMock.mockResolvedValue({ jobId: 'job-stream-1' });
  logChunksMock.mockImplementation((handler: ChunkHandler) => {
    bus.chunks.push(handler);
    return Promise.resolve(() => undefined);
  });
  listenEventMock.mockImplementation(
    <TPayload,>(name: string, handler: (payload: TPayload) => void): Promise<() => void> => {
      if (name === JOB_DONE_EVENT) {
        bus.done.push(handler as unknown as DoneHandler);
      } else if (name === JOB_FAILED_EVENT) {
        bus.failed.push(handler as unknown as FailedHandler);
      }
      return Promise.resolve(() => undefined);
    },
  );
});

afterEach(() => {
  useToastStore.setState(initialToastState);
});

describe('GitHubActionsPage — 运行列表', () => {
  it('输入仓库后加载 run 列表并展示状态徽标', async () => {
    renderPage();
    const input = screen.getByLabelText('仓库（owner/repo，如 octocat/Hello-World）');
    fireEvent.change(input, { target: { value: 'octocat/x' } });
    fireEvent.submit(screen.getByTestId('actions-repo-go'));

    await waitFor(() => expect(screen.getByTestId('actions-item-10')).toBeInTheDocument());
    expect(screen.getByText('Run 10')).toBeInTheDocument();
    expect(screen.getByTestId('actions-run-status-10')).toHaveTextContent('成功');
    expect(screen.getByTestId('actions-run-status-11')).toHaveTextContent('成功');
  });

  it('未完成与已完成分别显示取消/重跑按钮', async () => {
    listMock.mockResolvedValue({
      items: [run(10, { status: 'in_progress', conclusion: null }), run(11)],
      nextPage: null,
    });
    await openRun(10);

    expect(screen.getByTestId('actions-run-cancel')).toBeInTheDocument();
    expect(screen.queryByTestId('actions-run-rerun')).not.toBeInTheDocument();
  });

  it('已完成的 run 显示重跑并携带 runId 调用', async () => {
    await openRun(10);
    fireEvent.click(screen.getByTestId('actions-run-rerun'));

    await waitFor(() => expect(rerunMock).toHaveBeenCalledWith('github.com', 'octocat', 'x', 10));
  });

  it('未登录时给出登录引导', async () => {
    listMock.mockRejectedValue({ code: 'AUTH_REQUIRED' });
    renderPage();
    const input = screen.getByLabelText('仓库（owner/repo，如 octocat/Hello-World）');
    fireEvent.change(input, { target: { value: 'octocat/x' } });
    fireEvent.submit(screen.getByTestId('actions-repo-go'));

    await waitFor(() => expect(screen.getByText('还没有登录账号')).toBeInTheDocument());
  });
});

describe('GitHubActionsPage — 日志流式加载', () => {
  it('分块事件按 jobId 过滤并拼成行，job:done 结束加载', async () => {
    await openRun(10);
    fireEvent.click(screen.getByTestId('actions-log-open-100'));
    await waitFor(() => expect(logsMock).toHaveBeenCalledWith('github.com', 'octocat', 'x', 100));
    await waitFor(() => expect(logChunksMock).toHaveBeenCalled());

    // 同 jobId 的块会被接收
    bus.chunks.forEach((handler) =>
      handler({ jobId: 'job-stream-1', text: 'line one\n', totalLines: 1 }),
    );
    bus.chunks.forEach((handler) =>
      handler({ jobId: 'job-stream-1', text: 'line two\n', totalLines: 2 }),
    );
    // 别的 jobId（其他窗口）不能串流
    bus.chunks.forEach((handler) => handler({ jobId: 'other', text: 'alien\n', totalLines: 1 }));

    expect(await screen.findByText('line one')).toBeInTheDocument();
    expect(screen.getByText('line two')).toBeInTheDocument();
    expect(screen.queryByText('alien')).not.toBeInTheDocument();

    bus.done.forEach((handler) => handler({ jobId: 'job-stream-1' }));
    await waitFor(() =>
      expect(screen.getByTestId('actions-log-status')).toHaveTextContent('共 2 行'),
    );
  });

  it('取消任务显示已取消而不是错误', async () => {
    await openRun(10);
    fireEvent.click(screen.getByTestId('actions-log-open-100'));
    // jobId 分配后（与真实时序一致）job:failed 才可能到来
    await waitFor(() => expect(logsMock).toHaveBeenCalledWith('github.com', 'octocat', 'x', 100));

    bus.failed.forEach((handler) =>
      handler({ jobId: 'job-stream-1', error: { code: 'CANCELLED' } }),
    );
    await waitFor(() =>
      expect(screen.getByTestId('actions-log-status')).toHaveTextContent('已取消'),
    );
  });
});
