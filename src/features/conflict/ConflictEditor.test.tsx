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
  gitConflictContinue,
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
  gitConflictContinue: vi.fn(),
}));

const detailMock = vi.mocked(gitConflictFileDetail);
const applyMock = vi.mocked(gitConflictApplyResolution);
const markOnlyMock = vi.mocked(gitConflictMarkResolved);
const takeSideMock = vi.mocked(gitConflictTakeSide);
const removeMock = vi.mocked(gitConflictRemoveFile);
const continueMock = vi.mocked(gitConflictContinue);

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
  continueMock.mockResolvedValue({ oid: 'abc', conflicts: [] });
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

  // ---------------------------------------------------------------- 键盘流（T3.3）

  it('键盘 j 选中第一处冲突，o 采用本方并自动前进', async () => {
    renderEditor();

    await screen.findByTestId('conflict-adopt-ours-0');
    fireEvent.keyDown(window, { key: 'j' });
    expect(screen.getByTestId('conflict-result-0').closest('article')).toHaveAttribute(
      'data-selected',
      'true',
    );

    fireEvent.keyDown(window, { key: 'o' });
    // 单文件块全部解决：采用后选中点循环前进（只有一块则停在原处）
    expect(screen.getByTestId('conflict-result-0').closest('article')).toHaveAttribute(
      'data-resolved',
      'true',
    );
    // 纯前端操作：不产生任何 IPC 调用
    expect(applyMock).not.toHaveBeenCalled();
  });

  it('输入框聚焦时单键不触发块操作（不破坏正常输入）', async () => {
    renderEditor();

    await screen.findByTestId('conflict-adopt-ours-0');
    fireEvent.keyDown(window, { key: 'j' });
    const textarea = screen.getByTestId('conflict-result-0');
    fireEvent.keyDown(textarea, { key: 'o' });

    expect(textarea.closest('article')).toHaveAttribute('data-resolved', 'false');
  });

  it('Ctrl+S 保存，Ctrl+Enter 在全部解决后保存并继续', async () => {
    renderEditor();

    await screen.findByTestId('conflict-adopt-ours-0');
    // j 选中第一处冲突，o 采用本方
    fireEvent.keyDown(window, { key: 'j' });
    fireEvent.keyDown(window, { key: 'o' });
    // 等采用的重渲落地：连续真实按键之间必然隔着重渲，测试里显式等待
    await waitFor(() => {
      expect(screen.getByTestId('conflict-result-0').closest('article')).toHaveAttribute(
        'data-resolved',
        'true',
      );
    });

    fireEvent.keyDown(window, { key: 's', ctrlKey: true });
    await waitFor(() => {
      expect(applyMock).toHaveBeenCalledWith(1, 'a.txt', {
        content: 'head\nours',
        eol: 'lf',
        bom: false,
        trailingNewline: true,
      });
    });

    // Ctrl+Enter：文件已全部解决 → 保存后自动 continue
    fireEvent.keyDown(window, { key: 'Enter', ctrlKey: true });
    await waitFor(() => {
      expect(continueMock).toHaveBeenCalledWith(1);
    });
  });

  it('批量采用必须确认，取消时不产生任何修改', async () => {
    const many = textDetail({
      blocks: [
        { type: 'context', lines: ['head'] },
        { type: 'conflict', base: ['b1'], ours: ['o1'], theirs: ['t1', 't2'] },
        { type: 'conflict', base: ['b2'], ours: ['o2', 'o3'], theirs: ['t3'] },
      ],
    });
    detailMock.mockResolvedValue(many);
    renderEditor();

    await screen.findByTestId('conflict-adopt-ours-0');
    fireEvent.click(screen.getByTestId('editor-batch-ours'));
    // 确认框出现且尚未修改
    expect(await screen.findByTestId('editor-batch-confirm')).toBeInTheDocument();
    const cards = screen.getAllByTestId(/conflict-result-/);
    expect(cards).toHaveLength(2);
    expect(cards[0]?.closest('article')).toHaveAttribute('data-resolved', 'false');
    expect(cards[1]?.closest('article')).toHaveAttribute('data-resolved', 'false');

    fireEvent.click(screen.getByTestId('editor-batch-cancel'));
    expect(screen.getByTestId('conflict-result-0').closest('article')).toHaveAttribute(
      'data-resolved',
      'false',
    );
    expect(applyMock).not.toHaveBeenCalled();

    // 确认后全部采用
    fireEvent.click(screen.getByTestId('editor-batch-ours'));
    fireEvent.click(screen.getByTestId('editor-batch-confirm'));
    expect(screen.getByTestId('conflict-result-0').closest('article')).toHaveAttribute(
      'data-resolved',
      'true',
    );
    expect(screen.getByTestId('conflict-result-1').closest('article')).toHaveAttribute(
      'data-resolved',
      'true',
    );
  });

  it('整个文件采用一方必须确认，确认后调用 take_side', async () => {
    renderEditor();

    fireEvent.click(await screen.findByTestId('editor-file-ours'));
    expect(await screen.findByTestId('editor-file-confirm')).toBeInTheDocument();
    expect(takeSideMock).not.toHaveBeenCalled();

    fireEvent.click(screen.getByTestId('editor-file-confirm'));
    await waitFor(() => {
      expect(takeSideMock).toHaveBeenCalledWith(1, 'a.txt', 'ours');
    });
  });
});
