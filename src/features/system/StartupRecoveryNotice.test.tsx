import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { QueryClientProvider } from '@tanstack/react-query';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { StartupRecoveryNotice } from '@/features/system/StartupRecoveryNotice';
import { appRestart, appStartupReport } from '@/lib/ipc';
import type { StartupReport } from '@/lib/ipc';
import { createTestQueryClient } from '@/test/queryClient';

/**
 * 启动恢复提示（T7.5）。
 *
 * 钉住三条用户可见行为：
 *   异常退出 → 弹一次恢复提示，可选「以安全模式重启」；
 *   安全模式 → 常驻横幅，可「退出安全模式」；
 *   正常启动 → 什么都不显示（不能平白打扰用户）。
 * 后端"安全模式到底禁用了什么"由 Rust 侧测试钉住。
 */
vi.mock('@/lib/ipc', () => ({
  appStartupReport: vi.fn(),
  appRestart: vi.fn(async () => undefined),
  isTauriRuntime: () => true,
}));

const reportMock = vi.mocked(appStartupReport);
const restartMock = vi.mocked(appRestart);

function report(overrides: Partial<StartupReport> = {}): StartupReport {
  return {
    abnormalExit: false,
    lastExit: null,
    safeMode: false,
    logDir: 'C:\\logs',
    ...overrides,
  };
}

function renderNotice() {
  return render(
    <QueryClientProvider client={createTestQueryClient()}>
      <StartupRecoveryNotice />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  restartMock.mockResolvedValue(undefined);
});

afterEach(() => {
  cleanup();
});

describe('启动恢复提示', () => {
  it('上次异常退出时弹出提示，并展示上次版本', async () => {
    reportMock.mockResolvedValue(
      report({
        abnormalExit: true,
        lastExit: {
          pid: 4242,
          version: '0.7.0',
          startedAtMs: 1_700_000_000_000,
          detectedAtMs: 1_700_000_005_000,
        },
      }),
    );

    renderNotice();

    expect(await screen.findByText('上次异常退出')).toBeInTheDocument();
    expect(screen.getByText('上次运行版本：0.7.0')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '以安全模式重启' })).toBeInTheDocument();
  });

  it('点「以安全模式重启」会以安全模式重启应用', async () => {
    reportMock.mockResolvedValue(report({ abnormalExit: true }));
    renderNotice();

    const button = await screen.findByRole('button', { name: '以安全模式重启' });
    fireEvent.click(button);

    await waitFor(() => {
      expect(restartMock).toHaveBeenCalledWith(true);
    });
  });

  it('点「继续使用」后提示消失，且不会重启', async () => {
    reportMock.mockResolvedValue(report({ abnormalExit: true }));
    renderNotice();

    fireEvent.click(await screen.findByRole('button', { name: '继续使用' }));

    expect(screen.queryByText('上次异常退出')).not.toBeInTheDocument();
    expect(restartMock).not.toHaveBeenCalled();
  });

  it('安全模式启动时显示常驻横幅，可退出安全模式', async () => {
    reportMock.mockResolvedValue(report({ safeMode: true }));
    renderNotice();

    expect(await screen.findByText('安全模式：插件与内嵌终端已禁用')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: '退出安全模式' }));
    await waitFor(() => {
      expect(restartMock).toHaveBeenCalledWith(false);
    });
  });

  it('正常启动时不显示任何提示', async () => {
    reportMock.mockResolvedValue(report());
    renderNotice();

    // 等查询落地后再断言"没有东西"（否则可能只是还没取到数据）
    await waitFor(() => {
      expect(reportMock).toHaveBeenCalledTimes(1);
    });
    expect(screen.queryByText('上次异常退出')).not.toBeInTheDocument();
    expect(screen.queryByText('安全模式：插件与内嵌终端已禁用')).not.toBeInTheDocument();
  });
});
