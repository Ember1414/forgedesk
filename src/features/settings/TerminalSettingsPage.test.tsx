import { render, screen } from '@testing-library/react';
import { fireEvent } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { TerminalSettingsPage } from '@/features/settings/TerminalSettingsPage';
import {
  initialSettingsState,
  TERMINAL_SAFETY_AUTO_SNAPSHOT_KEY,
  TERMINAL_SAFETY_ENABLED_KEY,
  TERMINAL_SAFETY_LEVEL_KEY,
  useSettingsStore,
} from '@/stores/settingsStore';

/**
 * 终端设置页（T5.3）：断言设置项与存储键的绑定——
 * 页面是纯设置读写，没有额外逻辑值得测；这里锁住"键名与默认值"不漂移。
 */
vi.mock('@/lib/ipc', () => ({
  settingsAll: vi.fn(),
  settingsSet: vi.fn(),
  settingsGet: vi.fn(),
}));

import { settingsSet } from '@/lib/ipc';

const settingsSetMock = vi.mocked(settingsSet);
settingsSetMock.mockResolvedValue(undefined);

beforeEach(() => {
  vi.clearAllMocks();
  useSettingsStore.setState(initialSettingsState);
});

afterEach(() => {
  useSettingsStore.setState(initialSettingsState);
});

describe('TerminalSettingsPage（终端设置）', () => {
  it('默认值：提示级、开关与补偿快照开启', () => {
    render(<TerminalSettingsPage />);

    expect(screen.getByRole('radio', { name: '提示（推荐）' })).toBeChecked();
    expect(screen.getByRole('radio', { name: '执行前确认' })).not.toBeChecked();
    expect(screen.getByRole('checkbox', { name: '启用危险命令提示' })).toBeChecked();
    expect(screen.getByRole('checkbox', { name: '危险命令执行后自动创建补偿快照' })).toBeChecked();
  });

  it('切换到确认级时写入存储键 terminal.safety.level', async () => {
    render(<TerminalSettingsPage />);

    fireEvent.click(screen.getByRole('radio', { name: '执行前确认' }));

    await vi.waitFor(() => {
      expect(settingsSetMock).toHaveBeenCalledWith(
        'global',
        TERMINAL_SAFETY_LEVEL_KEY,
        '"confirm"',
        undefined,
      );
    });
  });

  it('关闭安全提示时写入 terminal.safety.enabled=false', async () => {
    render(<TerminalSettingsPage />);

    fireEvent.click(screen.getByRole('checkbox', { name: '启用危险命令提示' }));

    await vi.waitFor(() => {
      expect(settingsSetMock).toHaveBeenCalledWith(
        'global',
        TERMINAL_SAFETY_ENABLED_KEY,
        'false',
        undefined,
      );
    });
  });

  it('关闭补偿快照时写入 terminal.safety.autoSnapshot=false', async () => {
    render(<TerminalSettingsPage />);

    fireEvent.click(screen.getByRole('checkbox', { name: '危险命令执行后自动创建补偿快照' }));

    await vi.waitFor(() => {
      expect(settingsSetMock).toHaveBeenCalledWith(
        'global',
        TERMINAL_SAFETY_AUTO_SNAPSHOT_KEY,
        'false',
        undefined,
      );
    });
  });

  it('存储里的确认级能正确回显', () => {
    useSettingsStore.setState({
      values: { [TERMINAL_SAFETY_LEVEL_KEY]: '"confirm"' },
      loaded: true,
    });

    render(<TerminalSettingsPage />);

    expect(screen.getByRole('radio', { name: '执行前确认' })).toBeChecked();
  });
});
