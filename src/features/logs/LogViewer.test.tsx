import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { HIGHLIGHT_WINDOW_MS, LogViewer, isNearTimestamp } from '@/features/logs/LogViewer';
import { logsTail } from '@/lib/ipc';
import type { LogLine } from '@/lib/ipc';
import { createTestQueryClient } from '@/test/queryClient';
import { QueryClientProvider } from '@tanstack/react-query';

/**
 * 日志查看器的测试重点：
 *   - 时间线顺序与结构化字段的呈现（时间/级别/消息）；
 *   - **高亮窗口**：用户从错误提示点进来时，只有"错误前后"的行被强调，
 *     否则整屏都标黄等于没标；
 *   - 空态与失败态可自助（没有日志、日志读不到都不是死路）。
 */
vi.mock('@/lib/ipc', () => ({
  logsTail: vi.fn(),
}));

const logsTailMock = vi.mocked(logsTail);

const NOW = 1_787_000_000_000;

function line(offsetMs: number, level: string, message: string): LogLine {
  return {
    timestamp: NOW + offsetMs,
    level,
    target: 'forgedesk_commands::settings',
    message,
    raw: `{"level":"${level}","fields":{"message":"${message}"}}`,
  };
}

function renderViewer(props: { nearTimestamp?: number | null } = {}) {
  const queryClient = createTestQueryClient();
  return render(
    <QueryClientProvider client={queryClient}>
      <LogViewer {...props} />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe('isNearTimestamp', () => {
  it('窗口内为真，窗口外为假', () => {
    expect(isNearTimestamp(NOW, NOW)).toBe(true);
    expect(isNearTimestamp(NOW + HIGHLIGHT_WINDOW_MS, NOW)).toBe(true);
    expect(isNearTimestamp(NOW + HIGHLIGHT_WINDOW_MS + 1, NOW)).toBe(false);
  });

  it('缺少任一时间时不做任何高亮', () => {
    expect(isNearTimestamp(null, NOW)).toBe(false);
    expect(isNearTimestamp(NOW, null)).toBe(false);
    expect(isNearTimestamp(NOW, undefined)).toBe(false);
  });
});

describe('LogViewer', () => {
  it('按时间线顺序渲染日志的级别与消息', async () => {
    logsTailMock.mockResolvedValue([
      line(-1000, 'INFO', '应用已启动'),
      line(0, 'ERROR', '保存设置失败'),
    ]);

    renderViewer();

    expect(await screen.findByText('应用已启动')).toBeInTheDocument();
    expect(screen.getByText('保存设置失败')).toBeInTheDocument();
    expect(screen.getByText('INFO')).toBeInTheDocument();
    expect(screen.getByText('ERROR')).toBeInTheDocument();
    // 请求的是末尾若干行（缺省由组件决定），至少要向 IPC 要过数据
    expect(logsTailMock).toHaveBeenCalledTimes(1);
  });

  it('只请求固定行数（避免一次拉走整个日志文件）', async () => {
    logsTailMock.mockResolvedValue([line(0, 'INFO', 'x')]);

    renderViewer();
    await screen.findByText('x');

    const requested = logsTailMock.mock.calls[0]?.[0];
    expect(typeof requested).toBe('number');
    expect(requested).toBeGreaterThan(0);
    expect(requested).toBeLessThanOrEqual(2000);
  });

  it('高亮错误时间附近的日志，并提示高亮了多少行', async () => {
    logsTailMock.mockResolvedValue([
      line(-10 * 60 * 1000, 'INFO', '很久以前'),
      line(-5 * 1000, 'WARN', '刚刚的警告'),
      line(0, 'ERROR', '刚刚的错误'),
    ]);

    renderViewer({ nearTimestamp: NOW });
    await screen.findByText('刚刚的错误');

    const highlighted = screen.getAllByText(/刚刚的/, { selector: 'span' });
    expect(highlighted.length).toBeGreaterThanOrEqual(2);
    // 远处的行不应被标记
    const far = screen.getByText('很久以前').closest('li');
    expect(far?.className).not.toContain('border-brand');

    expect(screen.getByText(/已高亮 2 行/)).toBeInTheDocument();
  });

  it('未指定锚点时不高亮任何行', async () => {
    logsTailMock.mockResolvedValue([line(0, 'ERROR', '错误')]);

    renderViewer();
    await screen.findByText('错误');

    expect(screen.queryByText(/已高亮/)).not.toBeInTheDocument();
  });

  it('没有日志时显示空态而不是空白列表', async () => {
    logsTailMock.mockResolvedValue([]);

    renderViewer();

    expect(await screen.findByText('还没有日志')).toBeInTheDocument();
  });

  it('读取失败时给出可重试的错误态', async () => {
    logsTailMock.mockRejectedValueOnce(new Error('permission denied'));
    logsTailMock.mockResolvedValueOnce([line(0, 'INFO', '恢复后的日志')]);

    renderViewer();

    expect(await screen.findByText('读取日志失败')).toBeInTheDocument();
    expect(screen.getByText(/permission denied/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: '重试' }));

    await waitFor(() => {
      expect(screen.getByText('恢复后的日志')).toBeInTheDocument();
    });
  });
});
