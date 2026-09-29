import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { QueryClientProvider } from '@tanstack/react-query';
import { MemoryRouter, Route, Routes } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { StashPanel } from '@/features/stash/StashPanel';
import { gitStashApply, gitStashDrop, gitStashList, gitStashSave, gitStashShow } from '@/lib/ipc';
import type { StashEntry, StashShowOutcome } from '@/lib/ipc';
import { createTestQueryClient } from '@/test/queryClient';

/**
 * 储藏面板（T2.8）。
 *
 * 钉住的都是"说错会伤人"的结论：`stashed=false` 是正常结果；冲突是横幅（结果）
 * 不是错误弹窗；丢弃必须确认且确认框里能看到将丢掉哪条。
 */
vi.mock('@/lib/ipc', () => ({
  gitStashList: vi.fn(),
  gitStashShow: vi.fn(),
  gitStashSave: vi.fn(),
  gitStashApply: vi.fn(),
  gitStashPop: vi.fn(),
  gitStashDrop: vi.fn(),
  gitStashClear: vi.fn(),
  gitStashBranch: vi.fn(),
}));

const listMock = vi.mocked(gitStashList);
const showMock = vi.mocked(gitStashShow);
const saveMock = vi.mocked(gitStashSave);
const applyMock = vi.mocked(gitStashApply);
const dropMock = vi.mocked(gitStashDrop);

function entry(overrides: Partial<StashEntry> = {}): StashEntry {
  return {
    index: 0,
    oid: 'aaa111222333444555666777888999aaabbbccc0',
    baseOid: 'bbb000111222333444555666777888999aaabbbcc',
    message: 'WIP on main: stashed change',
    createdAt: 1_700_000_000,
    includesUntracked: false,
    ...overrides,
  };
}

function renderPanel() {
  const queryClient = createTestQueryClient();
  // 面板从路由参数里拿 repoId：必须真的走在带 `:repoId` 的路由上
  return render(
    <QueryClientProvider client={queryClient}>
      <MemoryRouter initialEntries={['/repo/1/status']}>
        <Routes>
          <Route path="/repo/:repoId/status" element={<StashPanel />} />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  listMock.mockResolvedValue([entry()]);
  showMock.mockResolvedValue({
    entry: entry(),
    diff: { files: [{ path: 'a.txt' }] },
    untracked: undefined,
  } as unknown as StashShowOutcome);
  saveMock.mockResolvedValue({ stashed: true, entry: entry() });
  applyMock.mockResolvedValue({ conflicts: [] });
  dropMock.mockResolvedValue({ dropped: [entry()] });
});

describe('储藏面板', () => {
  it('列出储藏、标记未跟踪，并允许展开文件清单', async () => {
    listMock.mockResolvedValue([entry({ includesUntracked: true })]);

    renderPanel();

    const list = await screen.findByTestId('stash-list');
    expect(list).toHaveTextContent('WIP on main');
    expect(list).toHaveTextContent('含未跟踪文件');

    fireEvent.click(screen.getByTestId('stash-toggle-0'));
    expect(await screen.findByTestId('stash-files')).toHaveTextContent('a.txt');
  });

  it('储藏按"含未跟踪文件"发出请求；stashed=false 不是错误路径', async () => {
    saveMock.mockResolvedValue({ stashed: false });

    renderPanel();
    await screen.findByTestId('stash-list');

    fireEvent.click(screen.getByTestId('stash-save'));

    await waitFor(() => {
      expect(saveMock).toHaveBeenCalledTimes(1);
    });
    expect(saveMock.mock.calls[0]?.[1]).toEqual({
      includeUntracked: true,
      message: '储藏（ForgeDesk）',
    });
  });

  it('应用冲突时展示横幅并列出冲突文件，而不是弹错误', async () => {
    applyMock.mockResolvedValue({ conflicts: ['a.txt'] });

    renderPanel();
    await screen.findByTestId('stash-list');

    fireEvent.click(screen.getByTestId('stash-apply-0'));

    const banner = await screen.findByTestId('stash-conflict-banner');
    expect(banner).toHaveTextContent('仓库已进入冲突状态');
    expect(banner).toHaveTextContent('a.txt');
  });

  it('丢弃需要先确认，确认后才按 index 调用', async () => {
    renderPanel();
    await screen.findByTestId('stash-list');

    fireEvent.click(screen.getByTestId('stash-drop-0'));

    const dialog = await screen.findByTestId('stash-drop-dialog');
    // 确认框里能看到将丢掉哪一条（信息 + oid），否则用户只能靠记忆
    expect(dialog).toHaveTextContent('WIP on main');
    expect(dialog).toHaveTextContent('aaa1112');
    expect(dropMock).not.toHaveBeenCalled();

    fireEvent.click(screen.getByTestId('stash-drop-confirm'));

    await waitFor(() => {
      expect(dropMock).toHaveBeenCalledWith(1, 0);
    });
    // 确认后对话框必须关掉
    await waitFor(() => {
      expect(screen.queryByTestId('stash-drop-dialog')).not.toBeInTheDocument();
    });
  });
});
