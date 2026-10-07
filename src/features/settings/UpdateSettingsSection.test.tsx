import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { UpdateSettingsSection } from '@/features/settings/UpdateSettingsSection';
import { settingsAll, settingsSet } from '@/lib/ipc';
import {
  initialSettingsState,
  UPDATE_AUTO_CHECK_KEY,
  UPDATE_SKIPPED_VERSION_KEY,
  useSettingsStore,
} from '@/stores/settingsStore';

/**
 * 「更新」设置区块（T7.1）。
 *
 * 钉住三件事：
 *   跳过版本会被显示出来（否则用户忘了自己跳过过，会把"怎么不提示更新"当故障）；
 *   「不再跳过」把值清空；
 *   自动检查开关写入设置。
 */
vi.mock('@/lib/ipc', () => ({
  settingsSet: vi.fn(async () => undefined),
  settingsAll: vi.fn(async () => ({})),
}));

const setMock = vi.mocked(settingsSet);

beforeEach(() => {
  vi.clearAllMocks();
  setMock.mockResolvedValue(undefined);
  vi.mocked(settingsAll).mockResolvedValue({});
  useSettingsStore.setState({ ...initialSettingsState, loaded: true });
});

afterEach(() => {
  cleanup();
  useSettingsStore.setState(initialSettingsState);
});

describe('更新设置区块', () => {
  it('没有跳过任何版本时给出明确文案，且没有清除按钮', () => {
    render(<UpdateSettingsSection />);

    expect(screen.getByText('没有跳过任何版本')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: '不再跳过' })).not.toBeInTheDocument();
  });

  it('显示已跳过的版本，并可清除', async () => {
    useSettingsStore.setState({
      ...initialSettingsState,
      loaded: true,
      values: { [UPDATE_SKIPPED_VERSION_KEY]: '"1.1.0"' },
    });
    render(<UpdateSettingsSection />);

    expect(screen.getByText('1.1.0')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '不再跳过' }));

    await waitFor(() => {
      expect(setMock).toHaveBeenCalledWith('global', UPDATE_SKIPPED_VERSION_KEY, 'null', undefined);
    });
    await waitFor(() => {
      expect(screen.getByText('没有跳过任何版本')).toBeInTheDocument();
    });
  });

  it('关掉自动检查会写入设置', async () => {
    render(<UpdateSettingsSection />);

    fireEvent.click(screen.getByRole('checkbox', { name: '自动检查更新' }));

    await waitFor(() => {
      expect(setMock).toHaveBeenCalledWith('global', UPDATE_AUTO_CHECK_KEY, 'false', undefined);
    });
  });
});
