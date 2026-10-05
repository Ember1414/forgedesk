import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { settingsAll, settingsSet } from '@/lib/ipc';
import { AppearanceSettingsPage } from '@/features/settings/AppearanceSettingsPage';
import {
  initialCustomThemesState,
  useCustomThemesStore,
} from '@/features/themes/customThemesStore';
import { setActiveCustomTheme } from '@/features/themes/activeCustomTheme';
import { applyCustomThemeColors } from '@/features/themes/themeApply';
import { BUILTIN_THEMES } from '@/features/themes/builtinThemes';
import { initialSettingsState, useSettingsStore } from '@/stores/settingsStore';
import { initialUiState, useUiStore } from '@/stores/uiStore';

/**
 * 外观设置页（T6.6）的测试重点：
 *   - 主题画廊展示内置主题并支持激活；
 *   - 激活暗色主题时把外观切到暗色（外观归属原则在页面上可感知）；
 *   - 自定义主题可删除（带确认对话框），删除真的写后端。
 * 持久化与校验的细节由 customThemesStore / themeModel 的测试覆盖。
 */
vi.mock('@/lib/ipc', () => ({
  settingsAll: vi.fn(async () => ({})),
  settingsSet: vi.fn(async () => undefined),
}));

const settingsAllMock = vi.mocked(settingsAll);
const settingsSetMock = vi.mocked(settingsSet);

beforeEach(() => {
  vi.clearAllMocks();
  useSettingsStore.setState({ ...initialSettingsState });
  useCustomThemesStore.setState({ ...initialCustomThemesState });
  useUiStore.setState({ ...initialUiState });
  document.documentElement.removeAttribute('data-theme');
  setActiveCustomTheme(null);
});

afterEach(() => {
  applyCustomThemeColors(null, 'light');
  setActiveCustomTheme(null);
  window.localStorage.clear();
});

describe('外观设置页 · 主题画廊', () => {
  it('展示全部内置主题卡片', async () => {
    settingsAllMock.mockResolvedValue({});

    render(<AppearanceSettingsPage />);

    await waitFor(() => {
      expect(useCustomThemesStore.getState().loaded).toBe(true);
    });
    expect(screen.getByTestId('theme-card-forgedesk-light')).toBeInTheDocument();
    expect(screen.getByTestId('theme-card-forgedesk-dark')).toBeInTheDocument();
    expect(screen.getByTestId('theme-card-sandstone-dawn')).toBeInTheDocument();
    expect(screen.getByTestId('theme-card-pine-nocturne')).toBeInTheDocument();
  });

  it('从亮色激活暗色内置主题时同步切换外观', async () => {
    settingsAllMock.mockResolvedValue({});

    render(<AppearanceSettingsPage />);
    const card = await screen.findByTestId('theme-card-pine-nocturne');
    expect(document.documentElement.getAttribute('data-theme')).not.toBe('dark');

    fireEvent.click(within(card).getByRole('button', { name: '使用此主题' }));

    await waitFor(() => {
      expect(document.documentElement.getAttribute('data-theme')).toBe('dark');
    });
  });

  it('自定义主题卡片带删除入口，确认后从列表移除并写入后端', async () => {
    // IPC 直接返回已存储的自定义主题（页面的加载会以 IPC 结果为准）；
    // 色值取自内置主题定义，避免在非主题文件里出现颜色字面量（check:colors 会拦）
    settingsAllMock.mockResolvedValue({
      'themes.customList': JSON.stringify([
        {
          id: 'midnight-test',
          name: 'Midnight Test',
          appearance: 'dark',
          version: '1.0.0',
          colors: { canvas: BUILTIN_THEMES.at(3)?.colors['canvas'] },
        },
      ]),
    });
    settingsSetMock.mockResolvedValue(undefined);

    render(<AppearanceSettingsPage />);
    const card = await screen.findByTestId('theme-card-midnight-test');

    fireEvent.click(within(card).getByRole('button', { name: '删除' }));
    // 确认对话框出现
    const confirm = await screen.findByRole('alertdialog');
    fireEvent.click(within(confirm).getByRole('button', { name: '删除' }));

    await waitFor(() => {
      expect(useCustomThemesStore.getState().themes).toEqual([]);
      expect(settingsSetMock).toHaveBeenCalledWith('global', 'themes.customList', '[]', undefined);
    });
  });
});
