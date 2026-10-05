import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { RateLimitBanner } from '@/features/github/RateLimitBanner';
import { repoRateLimitRefresh, repoRateLimitState } from '@/lib/ipc';
import type { RateLimitSnapshot } from '@/lib/ipc';

/**
 * 限流横幅（T4.10 降级 UI）。
 *
 * 钉住：无快照不显示、额度耗尽显示"缓存数据 + 重置时间"、低额度走提醒档、
 * 手动刷新调用网络命令并更新快照。
 */
vi.mock('@/lib/ipc', () => ({
  repoRateLimitState: vi.fn(),
  repoRateLimitRefresh: vi.fn(),
}));

const stateMock = vi.mocked(repoRateLimitState);
const refreshMock = vi.mocked(repoRateLimitRefresh);

function snapshot(remaining: number): RateLimitSnapshot {
  return {
    resource: 'core',
    limit: 5000,
    remaining,
    used: 5000 - remaining,
    resetUnixSecs: 1790000000,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  stateMock.mockResolvedValue(null);
  refreshMock.mockResolvedValue(snapshot(5000));
});

afterEach(() => {
  vi.useRealTimers();
});

describe('RateLimitBanner — 限流横幅', () => {
  it('没有快照（会话内还没发过请求）时不占任何位置', async () => {
    render(<RateLimitBanner />);
    await waitFor(() => expect(stateMock).toHaveBeenCalled());
    expect(screen.queryByTestId('rate-limit-banner')).not.toBeInTheDocument();
  });

  it('额度充足时不显示', async () => {
    stateMock.mockResolvedValue(snapshot(4321));
    render(<RateLimitBanner />);
    await waitFor(() => expect(stateMock).toHaveBeenCalled());
    expect(screen.queryByTestId('rate-limit-banner')).not.toBeInTheDocument();
  });

  it('额度耗尽显示缓存降级提示与重置时间', async () => {
    stateMock.mockResolvedValue(snapshot(0));
    render(<RateLimitBanner />);
    expect(await screen.findByTestId('rate-limit-banner')).toBeInTheDocument();
    expect(screen.getByText('GitHub API 额度已用尽，正在展示缓存数据。')).toBeInTheDocument();
    expect(screen.getByTestId('rate-limit-reset')).toHaveTextContent('恢复');
  });

  it('剩余额度低于阈值走提醒档，只报数字', async () => {
    stateMock.mockResolvedValue(snapshot(30));
    render(<RateLimitBanner />);
    expect(await screen.findByTestId('rate-limit-banner')).toBeInTheDocument();
    expect(screen.getByText('GitHub API 剩余额度较低：30/5000。')).toBeInTheDocument();
    expect(screen.queryByTestId('rate-limit-reset')).not.toBeInTheDocument();
  });

  it('手动刷新调用网络命令并按新快照收起横幅', async () => {
    stateMock.mockResolvedValueOnce(snapshot(0));
    render(<RateLimitBanner />);
    fireEvent.click(await screen.findByTestId('rate-limit-refresh'));

    await waitFor(() => expect(refreshMock).toHaveBeenCalledWith('github.com'));
    // 刷新后额度回来了：stateMock 的默认 mock（null）在下次轮询生效前，
    // 组件直接用了刷新返回的快照 → 横幅消失
    await waitFor(() => expect(screen.queryByTestId('rate-limit-banner')).not.toBeInTheDocument());
  });
});
