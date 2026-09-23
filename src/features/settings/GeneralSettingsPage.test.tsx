import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { settingsAll, settingsSet } from '@/lib/ipc';
import { GeneralSettingsPage } from '@/features/settings/GeneralSettingsPage';
import { DENSITY_KEY, initialSettingsState, useSettingsStore } from '@/stores/settingsStore';

/**
 * 通用设置页的测试重点（T0.7 验收）：
 *   - 打开页面时读取设置，并展示当前值；
 *   - 修改后**真的写到后端**（而不是只改本地 state）；
 *   - 读取失败时给出可重试的错误态，而不是空白页。
 */
vi.mock('@/lib/ipc', () => ({
  settingsAll: vi.fn(),
  settingsSet: vi.fn(),
}));

const settingsAllMock = vi.mocked(settingsAll);
const settingsSetMock = vi.mocked(settingsSet);

beforeEach(() => {
  vi.clearAllMocks();
  useSettingsStore.setState(initialSettingsState);
  document.documentElement.removeAttribute('data-density');
});

afterEach(() => {
  useSettingsStore.setState(initialSettingsState);
});

describe('GeneralSettingsPage', () => {
  it('加载并展示当前密度', async () => {
    settingsAllMock.mockResolvedValue({ [DENSITY_KEY]: '"compact"' });

    render(<GeneralSettingsPage />);

    const compact = await screen.findByRole('radio', { name: '紧凑' });
    await waitFor(() => {
      expect(compact).toHaveAttribute('data-state', 'on');
    });
  });

  it('切换密度后写入后端并应用到 <html>', async () => {
    settingsAllMock.mockResolvedValue({ [DENSITY_KEY]: '"comfortable"' });
    settingsSetMock.mockResolvedValue(undefined);

    render(<GeneralSettingsPage />);
    await screen.findByRole('radio', { name: '紧凑' });

    fireEvent.click(screen.getByRole('radio', { name: '紧凑' }));

    await waitFor(() => {
      expect(settingsSetMock).toHaveBeenCalledWith('global', DENSITY_KEY, '"compact"', undefined);
    });
    expect(document.documentElement.getAttribute('data-density')).toBe('compact');
  });

  it('说明本地数据的存储位置（用户要知道设置会保留）', async () => {
    settingsAllMock.mockResolvedValue({});
    render(<GeneralSettingsPage />);

    expect(await screen.findByText('本地数据')).toBeInTheDocument();
  });

  it('读取失败时显示错误态并可重试', async () => {
    settingsAllMock.mockRejectedValueOnce(new Error('database is locked'));
    settingsAllMock.mockResolvedValueOnce({});

    render(<GeneralSettingsPage />);

    expect(await screen.findByText('读取设置失败')).toBeInTheDocument();
    expect(screen.getByText(/database is locked/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: '重试' }));

    await waitFor(() => {
      expect(settingsAllMock).toHaveBeenCalledTimes(2);
    });
  });
});
