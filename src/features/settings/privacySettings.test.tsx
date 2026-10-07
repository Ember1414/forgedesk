import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { PrivacySettingsPage } from '@/features/settings/PrivacySettingsPage';
import { systemOpenUrl } from '@/lib/ipc';

/**
 * 隐私说明页（T7.6）。
 *
 * 钉住两件事：关键承诺真的渲染出来了（不是空壳页），
 * 以及"查看完整政策"确实把 URL 交给系统浏览器（而不是静默什么都不做）。
 */
vi.mock('@/lib/ipc', () => ({
  systemOpenUrl: vi.fn(async () => undefined),
}));

const openMock = vi.mocked(systemOpenUrl);

beforeEach(() => {
  vi.clearAllMocks();
});

afterEach(() => {
  cleanup();
});

describe('隐私说明页', () => {
  it('渲染标题与关键承诺条目', () => {
    render(<PrivacySettingsPage />);

    expect(screen.getByRole('heading', { name: '隐私' })).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: '无遥测' })).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: '无 AI 功能' })).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: '凭据' })).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: '本地数据与删除' })).toBeInTheDocument();
  });

  it('「查看完整隐私政策」把文档地址交给系统浏览器', async () => {
    render(<PrivacySettingsPage />);

    fireEvent.click(screen.getByRole('button', { name: '查看完整隐私政策' }));

    await waitFor(() => {
      expect(openMock).toHaveBeenCalledTimes(1);
    });
    expect(openMock.mock.calls[0]?.[0]).toContain('docs/PRIVACY.md');
  });
});
