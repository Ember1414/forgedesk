//! 冲突编辑器（T3.2）的组件测试。
//!
//! 钉住"说错会伤人"的结论：保存的内容就是预览的内容；残留标记警告不阻断；
//! 二进制 / 删除类冲突有各自的解决路径且都真的发出对应命令。
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { QueryClientProvider } from '@tanstack/react-query';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { ConflictEditor } from '@/features/conflict/ConflictEditor';
import {
  gitConflictApplyResolution,
  gitConflictFileDetail,
  gitConflictMarkResolved,
  gitConflictRemoveFile,
  gitConflictTakeSide,
} from '@/lib/ipc';
import type { ConflictFileDetail } from '@/lib/ipc';
import { createTestQueryClient } from '@/test/queryClient';

vi.mock('@/lib/ipc', () => ({
  gitConflictFileDetail: vi.fn(),
  gitConflictApplyResolution: vi.fn(),
  gitConflictMarkResolved: vi.fn(),
  gitConflictTakeSide: vi.fn(),
  gitConflictRemoveFile: vi.fn(),
}));

const detailMock = vi.mocked(gitConflictFileDetail);
const applyMock = vi.mocked(gitConflictApplyResolution);
const markOnlyMock = vi.mocked(gitConflictMarkResolved);
const takeSideMock = vi.mocked(gitConflictTakeSide);
const removeMock = vi.mocked(gitConflictRemoveFile);

function blob(content: string): ConflictFileDetail['ours'] {
  return { size: content.length, isBinary: false, encodingHint: 'utf-8', content };
}

function textDetail(overrides: Partial<ConflictFileDetail> = {}): ConflictFileDetail {
  return {
    path: 'a.txt',
    kind: 'text',
    base: blob('base\n'),
    ours: blob('ours\n'),
    theirs: blob('theirs\n'),
    worktreeExists: true,
    eol: 'lf',
    bom: false,
    trailingNewline: true,
    blocks: [
      { type: 'context', lines: ['head'] },
      {
        type: 'conflict',
        base: ['base'],
        ours: ['ours'],
        theirs: ['theirs'],
      },
    ],
    ...overrides,
  };
}

function renderEditor(detail: ConflictFileDetail = textDetail()) {
  const queryClient = createTestQueryClient();
  const onResolved = vi.fn();
  render(
    <QueryClientProvider client={queryClient}>
      <ConflictEditor repoId={1} path={detail.path} onResolved={onResolved} />
    </QueryClientProvider>,
  );
  return { onResolved };
}

beforeEach(() => {
  vi.clearAllMocks();
  detailMock.mockResolvedValue(textDetail());
  applyMock.mockResolvedValue(undefined);
  markOnlyMock.mockResolvedValue(undefined);
  takeSideMock.mockResolvedValue(undefined);
  removeMock.mockResolvedValue(undefined);
});

describe('ConflictEditor', () => {
  it('文本冲突渲染冲突卡片与两侧对照', async () => {
    renderEditor();

    expect(await screen.findByText('第 1 / 1 处冲突')).toBeInTheDocument();
    expect(screen.getByRole('region', { name: '本方（ours）' })).toBeInTheDocument();
    expect(screen.getByRole('region', { name: '对方（theirs）' })).toBeInTheDocument();
    expect(screen.getByTestId('conflict-adopt-ours-0')).toBeInTheDocument();
  });

  it('采用本方后保存，后端收到按块拼装的结果与原文件形状', async () => {
    renderEditor();

    fireEvent.click(await screen.findByTestId('conflict-adopt-ours-0'));
    fireEvent.click(screen.getByTestId('editor-save-resolve'));

    await waitFor(() => {
      expect(applyMock).toHaveBeenCalledWith(1, 'a.txt', {
        content: 'head\nours',
        eol: 'lf',
        bom: false,
        trailingNewline: true,
      });
    });
  });

  it('未解决直接保存会先警告，确认后仍然保存（标记不阻断）', async () => {
    renderEditor();

    fireEvent.click(await screen.findByTestId('editor-save-resolve'));
    expect(await screen.findByTestId('editor-markers-keep')).toBeInTheDocument();
    expect(applyMock).not.toHaveBeenCalled();

    fireEvent.click(screen.getByTestId('editor-markers-keep'));
    await waitFor(() => {
      expect(applyMock).toHaveBeenCalled();
    });
    // 内容里保留未解决块的标记原文
    const request = applyMock.mock.calls[0]?.[2];
    expect(request?.content).toContain('<<<<<<< ours');
  });

  it('手动编辑的内容成为自定义结果', async () => {
    renderEditor();

    fireEvent.click(await screen.findByTestId('conflict-manual-0'));
    fireEvent.change(screen.getByTestId('conflict-result-0'), {
      target: { value: 'hand edited' },
    });
    fireEvent.click(screen.getByTestId('editor-save-resolve'));

    await waitFor(() => {
      expect(applyMock).toHaveBeenCalledWith(1, 'a.txt', {
        content: 'head\nhand edited',
        eol: 'lf',
        bom: false,
        trailingNewline: true,
      });
    });
  });

  it('撤销把块状态退回未解决', async () => {
    renderEditor();

    fireEvent.click(await screen.findByTestId('conflict-adopt-ours-0'));
    expect(screen.getByTestId('editor-undo')).toBeEnabled();
    fireEvent.click(screen.getByTestId('editor-undo'));

    // 撤销后未解决计数回到 1，直接保存会触发残留警告
    fireEvent.click(screen.getByTestId('editor-save-resolve'));
    expect(await screen.findByTestId('editor-markers-keep')).toBeInTheDocument();
  });

  it('标记已解决（不保存）发出 mark_resolved', async () => {
    renderEditor();

    fireEvent.click(await screen.findByTestId('conflict-adopt-theirs-0'));
    fireEvent.click(screen.getByTestId('editor-mark-only'));

    await waitFor(() => {
      expect(markOnlyMock).toHaveBeenCalledWith(1, ['a.txt']);
    });
  });

  it('二进制冲突走采用一方路径，不渲染文本卡片', async () => {
    detailMock.mockResolvedValue(
      textDetail({
        kind: 'binary',
        base: { size: 3, isBinary: true, encodingHint: null, content: null },
        ours: { size: 3, isBinary: true, encodingHint: null, content: null },
        theirs: { size: 3, isBinary: true, encodingHint: null, content: null },
        blocks: [],
      }),
    );
    renderEditor();

    expect(await screen.findByTestId('conflict-binary-panel')).toBeInTheDocument();
    fireEvent.click(screen.getByTestId('binary-take-ours'));

    await waitFor(() => {
      expect(takeSideMock).toHaveBeenCalledWith(1, 'a.txt', 'ours');
    });
  });

  it('删除类冲突提供保留与删除两条路，删除需要确认', async () => {
    detailMock.mockResolvedValue(textDetail({ worktreeExists: false, kind: 'deletedByThem' }));
    renderEditor();

    expect(await screen.findByTestId('conflict-deleted-panel')).toBeInTheDocument();
    // 对方删了、我们改了 → 保留本方的修改
    fireEvent.click(screen.getByTestId('deleted-keep-ours'));
    await waitFor(() => {
      expect(takeSideMock).toHaveBeenCalledWith(1, 'a.txt', 'ours');
    });
  });

  it('删除文件的按钮必须经过确认框', async () => {
    detailMock.mockResolvedValue(textDetail({ worktreeExists: false, kind: 'deletedByUs' }));
    renderEditor();

    fireEvent.click(await screen.findByTestId('deleted-remove'));
    expect(await screen.findByTestId('conflict-delete-confirm')).toBeInTheDocument();
    expect(removeMock).not.toHaveBeenCalled();

    fireEvent.click(screen.getByTestId('conflict-delete-confirm'));
    await waitFor(() => {
      expect(removeMock).toHaveBeenCalledWith(1, 'a.txt');
    });
  });
});
