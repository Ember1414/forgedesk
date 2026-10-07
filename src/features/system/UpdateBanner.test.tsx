import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { QueryClientProvider } from '@tanstack/react-query';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { UpdateBanner } from '@/features/system/UpdateBanner';
import { listenUpdateProgress, settingsSet, updateCheck, updateInstall } from '@/lib/ipc';
import type { UpdateCheck, UpdateProgressPayload } from '@/lib/ipc';
import {
  initialSettingsState,
  UPDATE_AUTO_CHECK_KEY,
  UPDATE_SKIPPED_VERSION_KEY,
  useSettingsStore,
} from '@/stores/settingsStore';
import { createTestQueryClient } from '@/test/queryClient';

/**
 * 新版本提示横幅（T7.1）。
 *
 * 钉住五条用户可见行为：
 *   有更新 → 显示版本与三个动作；
 *   「立即更新」把**当前显示的版本号**传给后端（后端据此拒绝过期请求）；
 *   「跳过此版本」写进设置并立刻不再显示；
 *   关闭"自动检查"后**根本不发请求**（不是"发了但不显示"）；
 *   未配置更新源 / 无更新 → 什么都不显示。
 *   进度事件 → 显示下载百分比。
 */
vi.mock('@/lib/ipc', () => ({
  updateCheck: vi.fn(),
  updateInstall: vi.fn(async () => undefined),
  listenUpdateProgress: vi.fn(async () => () => undefined),
  settingsSet: vi.fn(async () => undefined),
  settingsAll: vi.fn(async () => ({})),
  isTauriRuntime: () => true,
}));

const checkMock = vi.mocked(updateCheck);
const installMock = vi.mocked(updateInstall);
const listenMock = vi.mocked(listenUpdateProgress);
const setMock = vi.mocked(settingsSet);

function available(version = '1.1.0'): UpdateCheck {
  return {
    configured: true,
    update: { version, currentVersion: '0.7.0', notes: null, date: null },
  };
}

function renderBanner() {
  return render(
    <QueryClientProvider client={createTestQueryClient()}>
      <UpdateBanner />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  installMock.mockResolvedValue(undefined);
  setMock.mockResolvedValue(undefined);
  listenMock.mockImplementation(async () => () => undefined);
  // 设置必须先"加载完成"，否则横幅按未就绪处理（避免用户关掉开关仍发请求）
  useSettingsStore.setState({ ...initialSettingsState, loaded: true });
});

afterEach(() => {
  cleanup();
  useSettingsStore.setState(initialSettingsState);
});

describe('更新横幅', () => {
  it('有可用更新时显示版本与三个动作', async () => {
    checkMock.mockResolvedValue(available());
    renderBanner();

    expect(await screen.findByText('有新版本可用：1.1.0')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '立即更新' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '跳过此版本' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '稍后提醒' })).toBeInTheDocument();
  });

  it('「立即更新」把当前显示的版本号交给后端', async () => {
    checkMock.mockResolvedValue(available('2.0.0'));
    renderBanner();

    fireEvent.click(await screen.findByRole('button', { name: '立即更新' }));

    await waitFor(() => {
      expect(installMock).toHaveBeenCalledWith('2.0.0');
    });
  });

  it('「稍后提醒」后横幅消失', async () => {
    checkMock.mockResolvedValue(available());
    renderBanner();

    fireEvent.click(await screen.findByRole('button', { name: '稍后提醒' }));

    expect(screen.queryByText('有新版本可用：1.1.0')).not.toBeInTheDocument();
  });

  it('「跳过此版本」写入设置并立刻不再显示', async () => {
    checkMock.mockResolvedValue(available('1.1.0'));
    renderBanner();

    fireEvent.click(await screen.findByRole('button', { name: '跳过此版本' }));

    await waitFor(() => {
      expect(setMock).toHaveBeenCalledWith(
        'global',
        UPDATE_SKIPPED_VERSION_KEY,
        '"1.1.0"',
        undefined,
      );
    });
    await waitFor(() => {
      expect(screen.queryByText('有新版本可用：1.1.0')).not.toBeInTheDocument();
    });
  });

  it('关闭「自动检查」后不发起检查', () => {
    useSettingsStore.setState({
      ...initialSettingsState,
      loaded: true,
      values: { [UPDATE_AUTO_CHECK_KEY]: 'false' },
    });
    checkMock.mockResolvedValue(available());

    renderBanner();

    expect(checkMock).not.toHaveBeenCalled();
    expect(screen.queryByRole('button', { name: '立即更新' })).not.toBeInTheDocument();
  });

  it('未配置更新源或无更新时什么都不显示', async () => {
    checkMock.mockResolvedValue({ configured: false, update: null });
    renderBanner();

    await waitFor(() => {
      expect(checkMock).toHaveBeenCalledTimes(1);
    });
    expect(screen.queryByRole('button', { name: '立即更新' })).not.toBeInTheDocument();

    cleanup();
    checkMock.mockResolvedValue({ configured: true, update: null });
    renderBanner();
    await waitFor(() => {
      expect(checkMock).toHaveBeenCalledTimes(2);
    });
    expect(screen.queryByRole('button', { name: '立即更新' })).not.toBeInTheDocument();
  });

  it('下载进度事件会显示为百分比', async () => {
    checkMock.mockResolvedValue(available());
    let handler: ((payload: UpdateProgressPayload) => void) | null = null;
    listenMock.mockImplementation(async (next) => {
      handler = next;
      return () => undefined;
    });
    // 让安装一直处于进行中：进度文案只在"正在更新"时显示
    installMock.mockImplementation(() => new Promise<void>(() => undefined));

    renderBanner();
    fireEvent.click(await screen.findByRole('button', { name: '立即更新' }));

    await waitFor(() => {
      expect(handler).not.toBeNull();
    });
    act(() => {
      handler?.({ phase: 'downloading', received: 42, total: 100 });
    });

    expect(await screen.findByText('下载中 42%')).toBeInTheDocument();
  });
});
