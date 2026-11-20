import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { FirstRunTrustHint } from '@/features/system/FirstRunTrustHint';
import { settingsSet, systemOpenUrl } from '@/lib/ipc';
import {
  initialSettingsState,
  TRUST_HINT_DISMISSED_KEY,
  useSettingsStore,
} from '@/stores/settingsStore';

/**
 * 首次启动的安装可信性提示（M8 / T8.2）。
 *
 * 钉住五条用户可见行为：
 *   首次启动 → 显示说明与两个动作；
 *   「我知道了」→ 写入设置并立刻消失（下次启动不再出现）；
 *   已关闭 / 设置未加载 / 非 Tauri 宿主 → 什么都不显示；
 *   「查看校验指引」→ 打开官网下载页，且卡片保留（不强制关闭）。
 */

let inTauri = true;

vi.mock('@/lib/ipc', () => ({
  systemOpenUrl: vi.fn(async () => undefined),
  settingsSet: vi.fn(async () => undefined),
  settingsAll: vi.fn(async () => ({})),
  isTauriRuntime: () => inTauri,
}));

const openMock = vi.mocked(systemOpenUrl);
const setMock = vi.mocked(settingsSet);

beforeEach(() => {
  vi.clearAllMocks();
  inTauri = true;
  setMock.mockResolvedValue(undefined);
  // 设置必须先"加载完成"：否则按未就绪处理，避免"闪一下又消失"
  useSettingsStore.setState({ ...initialSettingsState, loaded: true });
});

afterEach(() => {
  cleanup();
  useSettingsStore.setState(initialSettingsState);
});

describe('安装可信性提示', () => {
  it('首次启动显示说明与两个动作', () => {
    render(<FirstRunTrustHint />);

    expect(screen.getByText('关于安装安全提示')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '查看校验指引' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '我知道了' })).toBeInTheDocument();
  });

  it('「我知道了」写入设置并立刻消失', async () => {
    render(<FirstRunTrustHint />);

    fireEvent.click(screen.getByRole('button', { name: '我知道了' }));

    await waitFor(() => {
      expect(setMock).toHaveBeenCalledWith('global', TRUST_HINT_DISMISSED_KEY, 'true', undefined);
    });
    await waitFor(() => {
      expect(screen.queryByText('关于安装安全提示')).not.toBeInTheDocument();
    });
  });

  it('已经关闭过就不再显示', () => {
    useSettingsStore.setState({
      ...initialSettingsState,
      loaded: true,
      values: { [TRUST_HINT_DISMISSED_KEY]: 'true' },
    });

    render(<FirstRunTrustHint />);

    expect(screen.queryByText('关于安装安全提示')).not.toBeInTheDocument();
  });

  it('设置尚未加载完成时不显示（避免闪一下又消失）', () => {
    useSettingsStore.setState({ ...initialSettingsState, loaded: false });

    render(<FirstRunTrustHint />);

    expect(screen.queryByText('关于安装安全提示')).not.toBeInTheDocument();
  });

  it('非 Tauri 宿主（浏览器预览）不显示', () => {
    inTauri = false;

    render(<FirstRunTrustHint />);

    expect(screen.queryByText('关于安装安全提示')).not.toBeInTheDocument();
  });

  it('「查看校验指引」打开官网下载页，且卡片保留', async () => {
    render(<FirstRunTrustHint />);

    fireEvent.click(screen.getByRole('button', { name: '查看校验指引' }));

    await waitFor(() => {
      expect(openMock).toHaveBeenCalledWith('https://forgedesk.pages.dev/download');
    });
    // 打开指引不关闭卡片：用户看完后还要回来点「我知道了」
    expect(screen.getByText('关于安装安全提示')).toBeInTheDocument();
  });
});
