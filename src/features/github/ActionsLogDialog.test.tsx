/** 日志视图的资源边界：事件突发、超大输出与 React 重渲染都不应重启下载。 */
import { Profiler } from 'react';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { ActionsLogDialog } from '@/features/github/ActionsLogDialog';
import { cancelJob, listenActionsLogChunks, repoActionsJobLogs } from '@/lib/ipc';
import type { ActionsLogChunkPayload } from '@/lib/ipc';
import { listenEvent } from '@/lib/ipc/client';

vi.mock('@/lib/ipc', () => ({
  cancelJob: vi.fn(async () => true),
  listenActionsLogChunks: vi.fn(),
  repoActionsJobLogs: vi.fn(),
  JOB_DONE_EVENT: 'job:done',
  JOB_FAILED_EVENT: 'job:failed',
}));
vi.mock('@/lib/ipc/client', () => ({ listenEvent: vi.fn() }));

const target = { owner: 'test', repo: 'repo', jobId: 123, jobName: 'build' };
let receiveChunk: (payload: ActionsLogChunkPayload) => void;
const events = new Map<string, (payload: { jobId: string }) => void>();
const unlisten = vi.fn();

beforeEach(() => {
  vi.clearAllMocks();
  events.clear();
  vi.mocked(repoActionsJobLogs).mockResolvedValue({ jobId: 'stream-1' });
  vi.mocked(listenActionsLogChunks).mockImplementation(async (handler) => {
    receiveChunk = handler;
    return unlisten;
  });
  vi.mocked(listenEvent).mockImplementation(async (name, handler) => {
    events.set(name, handler as (payload: { jobId: string }) => void);
    return unlisten;
  });
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

async function started(): Promise<void> {
  await waitFor(() => expect(repoActionsJobLogs).toHaveBeenCalledTimes(1));
  await act(async () => undefined);
}

describe('ActionsLogDialog', () => {
  it('同一任务的目标对象重新创建时保留日志并继续原下载', async () => {
    const view = render(<ActionsLogDialog target={target} onOpenChange={vi.fn()} />);
    await started();
    act(() => receiveChunk({ jobId: 'stream-1', text: 'retained output\n', totalLines: 1 }));
    expect(await screen.findByText('retained output')).toBeInTheDocument();

    view.rerender(<ActionsLogDialog target={{ ...target }} onOpenChange={vi.fn()} />);
    await act(async () => undefined);
    expect(repoActionsJobLogs).toHaveBeenCalledTimes(1);
    expect(cancelJob).not.toHaveBeenCalled();
    expect(screen.getByText('retained output')).toBeInTheDocument();
  });

  it('事件连续到达时批量刷新，完成事件立即呈现最后一块', async () => {
    const commits: number[] = [];
    render(
      <Profiler id="logs" onRender={() => commits.push(1)}>
        <ActionsLogDialog target={target} onOpenChange={vi.fn()} />
      </Profiler>,
    );
    await started();
    vi.useFakeTimers();
    commits.length = 0;
    for (let index = 0; index < 30; index += 1) {
      act(() =>
        receiveChunk({ jobId: 'stream-1', text: `line-${index}\n`, totalLines: index + 1 }),
      );
    }
    expect(commits).toHaveLength(0);
    act(() => vi.advanceTimersByTime(50));
    expect(commits).toHaveLength(1);

    act(() => {
      receiveChunk({ jobId: 'stream-1', text: 'final line\n', totalLines: 31 });
      events.get('job:done')?.({ jobId: 'stream-1' });
    });
    expect(screen.getByTestId('actions-log-status')).toHaveTextContent('共 31 行');
    const viewport = screen.getByRole('list', { name: /日志/ });
    fireEvent.scroll(viewport, { target: { scrollTop: 30 * 24 } });
    expect(screen.getByText('final line')).toBeInTheDocument();
  });

  it('大日志只保留有界的最近行，明确提示省略并保留总行数', async () => {
    render(<ActionsLogDialog target={target} onOpenChange={vi.fn()} />);
    await started();
    act(() => {
      receiveChunk({
        jobId: 'stream-1',
        text: Array.from({ length: 20_005 }, (_, index) => `line-${index}`).join('\n') + '\n',
        totalLines: 20_005,
      });
      events.get('job:done')?.({ jobId: 'stream-1' });
    });
    expect(await screen.findByTestId('actions-log-truncated')).toBeVisible();
    expect(screen.getByTestId('actions-log-status')).toHaveTextContent('20005');
    const rows = screen.getAllByRole('listitem');
    expect(rows[0]).toHaveAttribute('aria-setsize', '20000');
    expect(screen.queryByText('line-0')).not.toBeInTheDocument();
    fireEvent.scroll(screen.getByRole('list', { name: /日志/ }), {
      target: { scrollTop: 19_999 * 24 },
    });
    expect(screen.getByText('line-20004')).toBeInTheDocument();
  });

  it('关闭后不接收事件，并取消尚未返回身份的下载', async () => {
    let resolveJob!: (job: { jobId: string }) => void;
    vi.mocked(repoActionsJobLogs).mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveJob = resolve;
        }),
    );
    const view = render(<ActionsLogDialog target={target} onOpenChange={vi.fn()} />);
    await started();
    view.rerender(<ActionsLogDialog target={null} onOpenChange={vi.fn()} />);
    await act(async () => resolveJob({ jobId: 'late-stream' }));
    expect(cancelJob).toHaveBeenCalledWith('late-stream');
    expect(unlisten).toHaveBeenCalledTimes(3);
  });

  it('任务身份返回前的大量日志也会截断，并保留完成状态和末尾内容', async () => {
    vi.mocked(repoActionsJobLogs).mockImplementationOnce(async () => {
      for (let index = 0; index < 520; index += 1) {
        receiveChunk({ jobId: 'stream-1', text: `early-${index}\n`, totalLines: index + 1 });
      }
      events.get('job:done')?.({ jobId: 'stream-1' });
      return { jobId: 'stream-1' };
    });
    render(<ActionsLogDialog target={target} onOpenChange={vi.fn()} />);
    await started();
    expect(screen.getByTestId('actions-log-truncated')).toBeVisible();
    expect(screen.getByTestId('actions-log-status')).toHaveTextContent('共 520 行');
    const list = screen.getByRole('list', { name: /日志/ });
    fireEvent.scroll(list, { target: { scrollTop: 510 * 24 } });
    expect(screen.getByText('early-519')).toBeInTheDocument();
  });
});
