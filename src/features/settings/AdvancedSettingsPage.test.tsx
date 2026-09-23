import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { QueryClientProvider } from '@tanstack/react-query';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { AdvancedSettingsPage } from '@/features/settings/AdvancedSettingsPage';
import { logsOpen, logsTail } from '@/lib/ipc';
import { createTestQueryClient } from '@/test/queryClient';
import { initialToastState, useToastStore } from '@/stores/toastStore';

/**
 * 高级设置页的测试重点（T0.8 验收："设置页提供查看日志目录按钮"）：
 *   - 日志策略说明可见（用户有权知道日志留多久、会不会上传）；
 *   - 打开目录按钮真的调用命令；
 *   - 打开失败时给出错误提示，而不是静默无反应。
 */
vi.mock('@/lib/ipc', () => ({
  logsOpen: vi.fn(),
  logsTail: vi.fn(),
}));

const logsOpenMock = vi.mocked(logsOpen);
const logsTailMock = vi.mocked(logsTail);

beforeEach(() => {
  vi.clearAllMocks();
  useToastStore.setState(initialToastState);
  logsTailMock.mockResolvedValue([
    {
      timestamp: 1_787_000_000_000,
      level: 'INFO',
      target: 'forgedesk',
      message: '应用已启动',
      raw: '{"fields":{"message":"应用已启动"}}',
    },
  ]);
});

afterEach(() => {
  vi.restoreAllMocks();
  useToastStore.setState(initialToastState);
});

function renderPage() {
  return render(
    <QueryClientProvider client={createTestQueryClient()}>
      <AdvancedSettingsPage />
    </QueryClientProvider>,
  );
}

describe('AdvancedSettingsPage', () => {
  it('展示日志保留策略（本地、轮转、脱敏、不上传）', () => {
    renderPage();

    const policy = screen.getByText(/保留最近 7 天/);
    expect(policy).toBeInTheDocument();
    expect(policy.textContent).toContain('不会上传');
  });

  it('点击"打开日志目录"调用 logs_open', async () => {
    logsOpenMock.mockResolvedValue(undefined);
    renderPage();

    fireEvent.click(screen.getByRole('button', { name: '打开日志目录' }));

    await waitFor(() => {
      expect(logsOpenMock).toHaveBeenCalledTimes(1);
    });
  });

  it('就地在页面里展示最近日志', async () => {
    renderPage();

    expect(await screen.findByText('应用已启动')).toBeInTheDocument();
  });

  it('打开目录失败时弹出错误提示（不静默）', async () => {
    logsOpenMock.mockRejectedValue({
      code: 'INTERNAL',
      message: 'could not open the folder',
      detail: 'xdg-open: No such file or directory',
    });
    renderPage();

    fireEvent.click(screen.getByRole('button', { name: '打开日志目录' }));

    await waitFor(() => {
      expect(useToastStore.getState().toasts).toHaveLength(1);
    });
    const [toast] = useToastStore.getState().toasts;
    expect(toast?.tone).toBe('danger');
    // 详情（含可手动复制的路径）必须可展开
    expect(toast?.detail).toContain('xdg-open');
  });
});
