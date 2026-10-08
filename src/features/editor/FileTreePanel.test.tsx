import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { QueryClientProvider } from '@tanstack/react-query';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { FileTreePanel } from '@/features/editor/FileTreePanel';
import { fsCreate, fsTree } from '@/lib/ipc/fs';
import { workspaceStatus } from '@/lib/ipc/workspace';
import { createTestQueryClient } from '@/test/queryClient';
import { initialToastState, useToastStore } from '@/stores/toastStore';

/**
 * 文件树的三条契约（2026-10-08 修掉的真实缺陷，逐条钉住）：
 *
 *   1. **"显示隐藏文件"要作用于每一层**——子目录此前固定传 `showHidden: false`，
 *      开关只在根层级生效；
 *   2. **手动刷新要重取子目录**——刷新版本号此前不进子目录的查询键，
 *      在子目录里新建/删除后界面保持旧结果；
 *   3. **创建失败必须说出来**——只有 try/finally，失败时对话框照常关闭、界面无反应。
 */
vi.mock('@/lib/ipc/fs', () => ({
  fsTree: vi.fn(),
  fsCreate: vi.fn(),
  fsRename: vi.fn(),
  fsDelete: vi.fn(),
}));
vi.mock('@/lib/ipc/workspace', () => ({
  workspaceStatus: vi.fn(),
}));

const treeMock = vi.mocked(fsTree);
const createMock = vi.mocked(fsCreate);
const statusMock = vi.mocked(workspaceStatus);

beforeEach(() => {
  vi.clearAllMocks();
  useToastStore.setState(initialToastState);
  statusMock.mockResolvedValue({
    staged: [],
    unstaged: [],
    untracked: [],
    conflicted: [],
    branch: null,
    operation: null,
  } as unknown as Awaited<ReturnType<typeof workspaceStatus>>);
  treeMock.mockImplementation((_repoId, path) =>
    Promise.resolve(
      path === ''
        ? [{ name: 'src', relPath: 'src', kind: 'dir', size: 0 }]
        : [{ name: 'main.ts', relPath: 'src/main.ts', kind: 'file', size: 10 }],
    ),
  );
});

afterEach(() => {
  vi.clearAllMocks();
  useToastStore.setState(initialToastState);
});

function renderPanel(): void {
  render(
    <QueryClientProvider client={createTestQueryClient()}>
      <FileTreePanel repoId={1} onOpenFile={() => undefined} />
    </QueryClientProvider>,
  );
}

describe('FileTreePanel', () => {
  it('子目录沿用根层级的"显示隐藏文件"开关', async () => {
    renderPanel();

    // 展开 src（子目录的查询才会发出）
    fireEvent.click(await screen.findByRole('button', { name: /src/ }));
    await waitFor(() => {
      expect(treeMock).toHaveBeenCalledWith(1, 'src', { showHidden: false });
    });

    treeMock.mockClear();
    fireEvent.click(screen.getByRole('checkbox', { name: '显示隐藏' }));

    await waitFor(() => {
      expect(treeMock).toHaveBeenCalledWith(1, '', { showHidden: true });
      // 关键断言：子目录也跟着变（此前恒为 false）
      expect(treeMock).toHaveBeenCalledWith(1, 'src', { showHidden: true });
    });
  });

  it('手动刷新会重取子目录（不再只刷新根层级）', async () => {
    renderPanel();
    fireEvent.click(await screen.findByRole('button', { name: /src/ }));
    await waitFor(() => {
      expect(treeMock).toHaveBeenCalledWith(1, 'src', { showHidden: false });
    });

    treeMock.mockClear();
    fireEvent.click(screen.getByRole('button', { name: '刷新' }));

    await waitFor(() => {
      expect(treeMock).toHaveBeenCalledWith(1, 'src', { showHidden: false });
    });
  });

  it('新建文件失败时给出提示，而不是静默关闭对话框', async () => {
    createMock.mockRejectedValue({
      code: 'VALIDATION',
      message: 'already exists',
      actions: [],
      retryable: false,
    });
    renderPanel();

    fireEvent.click(await screen.findByRole('button', { name: '新建文件' }));
    fireEvent.change(screen.getByLabelText('名称'), { target: { value: 'dup.ts' } });
    fireEvent.click(screen.getByRole('button', { name: '确认' }));

    await waitFor(() => {
      expect(createMock).toHaveBeenCalledWith(1, 'dup.ts', false);
      expect(useToastStore.getState().toasts).toHaveLength(1);
    });
  });
});
