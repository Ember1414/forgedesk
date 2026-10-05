import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { GitHubDashboardPage } from '@/features/github/GitHubDashboardPage';
import { repoDashboard } from '@/lib/ipc';
import type { RepoDashboard } from '@/lib/ipc';

/**
 * 概览页（T4.11 Dashboard 聚合）。
 *
 * 钉住：列表解析（逗号分隔、去重、截断到 10）、聚合卡片渲染
 * （PR 数/待审高亮/CI 徽标）、单仓库降级的"获取失败"文案、
 * 非法输入不发请求。
 */
vi.mock('@/lib/ipc', () => ({
  repoDashboard: vi.fn(),
}));

const dashboardMock = vi.mocked(repoDashboard);

function report(repo: string, overrides: Partial<RepoDashboard> = {}): RepoDashboard {
  return {
    owner: 'octocat',
    repo,
    pulls: { openTotal: 3, openTruncated: false, awaitingReview: 1 },
    runs: { name: 'CI', status: 'completed', conclusion: 'failure' },
    errors: [],
    ...overrides,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  dashboardMock.mockResolvedValue({ repos: [report('a'), report('b')] });
});

function renderPage(): void {
  render(<GitHubDashboardPage />);
}

describe('GitHubDashboardPage — 聚合概览', () => {
  it('输入仓库列表后渲染每仓库卡片与 CI 徽标', async () => {
    renderPage();
    fireEvent.change(screen.getByTestId('dashboard-repo-input'), {
      target: { value: 'octocat/a, octocat/b' },
    });
    fireEvent.click(screen.getByTestId('dashboard-go'));

    await waitFor(() =>
      expect(dashboardMock).toHaveBeenCalledWith({
        host: 'github.com',
        targets: [
          { owner: 'octocat', repo: 'a' },
          { owner: 'octocat', repo: 'b' },
        ],
      }),
    );
    expect(await screen.findByTestId('dashboard-item-octocat-a')).toBeInTheDocument();
    expect(screen.getByTestId('dashboard-run-octocat-a')).toHaveTextContent('失败');
    expect(screen.getByTestId('dashboard-pulls-octocat-a')).toHaveTextContent('进行中 PR: 3');
    expect(screen.getByTestId('dashboard-awaiting-octocat-a')).toHaveTextContent('待我审查: 1');
  });

  it('重复与非法条目被丢弃，超过 10 个截断', async () => {
    renderPage();
    const many = Array.from({ length: 12 }, (_, i) => `o/r${i + 1}`).join(', ');
    fireEvent.change(screen.getByTestId('dashboard-repo-input'), {
      target: { value: `octocat/a, OCTOCAT/A, bad-input, ${many}` },
    });
    fireEvent.click(screen.getByTestId('dashboard-go'));

    await waitFor(() => expect(dashboardMock).toHaveBeenCalled());
    const sent = dashboardMock.mock.calls[0]?.[0].targets ?? [];
    expect(sent[0]).toEqual({ owner: 'octocat', repo: 'a' });
    expect(sent).toHaveLength(10);
  });

  it('后端降级的仓库显示获取失败而不影响其他卡片', async () => {
    dashboardMock.mockResolvedValue({
      repos: [report('a', { pulls: null, runs: null, errors: ['boom'] }), report('b')],
    });
    renderPage();
    fireEvent.change(screen.getByTestId('dashboard-repo-input'), {
      target: { value: 'octocat/a, octocat/b' },
    });
    fireEvent.click(screen.getByTestId('dashboard-go'));

    const degraded = await screen.findByTestId('dashboard-item-octocat-a');
    expect(degraded).toHaveTextContent('获取失败');
    expect(screen.getByTestId('dashboard-run-octocat-a')).toHaveTextContent('获取失败');
    expect(screen.getByTestId('dashboard-item-octocat-b')).toBeInTheDocument();
  });

  it('没有 run 记录时显示未运行文案', async () => {
    dashboardMock.mockResolvedValue({ repos: [report('a', { runs: null, errors: [] })] });
    renderPage();
    fireEvent.change(screen.getByTestId('dashboard-repo-input'), {
      target: { value: 'octocat/a' },
    });
    fireEvent.click(screen.getByTestId('dashboard-go'));

    expect(await screen.findByTestId('dashboard-run-octocat-a')).toHaveTextContent(
      '还没有运行记录',
    );
  });

  it('非法输入（没有任何有效仓库）不发请求', () => {
    renderPage();
    fireEvent.change(screen.getByTestId('dashboard-repo-input'), {
      target: { value: 'not-a-repo' },
    });
    fireEvent.click(screen.getByTestId('dashboard-go'));
    expect(dashboardMock).not.toHaveBeenCalled();
    expect(screen.getByTestId('dashboard-empty')).toBeInTheDocument();
  });
});
