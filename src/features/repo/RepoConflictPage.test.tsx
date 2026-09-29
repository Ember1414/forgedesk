//! 冲突页（T3.1 状态机视图）的组件测试。
//!
//! 钉住的都是"说错会伤人"的结论：无操作时是空态不是报错；continue 在有
//! 未解决文件时禁用；"又停在冲突上"由 query 失效表达而不是错误弹窗；
//! 中止必须经过确认框。
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { QueryClientProvider } from '@tanstack/react-query';
import { MemoryRouter, Route, Routes } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { RepoConflictPage } from '@/features/repo/RepoConflictPage';
import {
  gitConflictAbort,
  gitConflictContinue,
  gitConflictMarkResolved,
  gitConflictSkip,
  gitConflictState,
} from '@/lib/ipc';
import type { ConflictFile, ConflictState } from '@/lib/ipc';
import { createTestQueryClient } from '@/test/queryClient';

vi.mock('@/lib/ipc', () => ({
  gitConflictState: vi.fn(),
  gitConflictMarkResolved: vi.fn(),
  gitConflictContinue: vi.fn(),
  gitConflictAbort: vi.fn(),
  gitConflictSkip: vi.fn(),
}));

const stateMock = vi.mocked(gitConflictState);
const resolveMock = vi.mocked(gitConflictMarkResolved);
const continueMock = vi.mocked(gitConflictContinue);
const abortMock = vi.mocked(gitConflictAbort);
const skipMock = vi.mocked(gitConflictSkip);

function blob(content: string): ConflictState['files'][number]['ours'] {
  return {
    size: content.length,
    isBinary: false,
    encodingHint: 'utf-8',
    content,
  };
}

function file(overrides: Partial<ConflictFile> = {}): ConflictFile {
  return {
    path: 'src/a.ts',
    kind: 'text',
    base: blob('base\n'),
    ours: blob('ours\n'),
    theirs: blob('theirs\n'),
    worktreeExists: true,
    ...overrides,
  };
}

function state(overrides: Partial<ConflictState> = {}): ConflictState {
  return {
    opKind: null,
    opInProgress: false,
    currentStep: null,
    totalSteps: null,
    headName: null,
    intoBranch: null,
    files: [],
    canContinue: false,
    canAbort: false,
    canSkip: false,
    ...overrides,
  };
}

function mergeConflict(overrides: Partial<ConflictState> = {}): ConflictState {
  return state({
    opKind: 'merge',
    opInProgress: true,
    intoBranch: 'main',
    files: [file()],
    canAbort: true,
    ...overrides,
  });
}

function renderPage() {
  const queryClient = createTestQueryClient();
  return render(
    <QueryClientProvider client={queryClient}>
      <MemoryRouter initialEntries={['/repo/1/conflict']}>
        <Routes>
          <Route path="/repo/:repoId/conflict" element={<RepoConflictPage />} />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe('RepoConflictPage', () => {
  it('没有进行中的操作时显示空态而不是报错', async () => {
    stateMock.mockResolvedValue(state());

    renderPage();

    expect(await screen.findByTestId('conflict-page-empty')).toBeInTheDocument();
  });

  it('合并冲突显示操作类型与未解决文件清单', async () => {
    stateMock.mockResolvedValue(mergeConflict());

    renderPage();

    expect(await screen.findByTestId('conflict-op-kind')).toHaveTextContent('合并');
    expect(screen.getByTestId('conflict-file-row')).toBeInTheDocument();
    expect(screen.getByText('src/a.ts')).toBeInTheDocument();
    // 有未解决文件：不能继续
    expect(screen.getByTestId('conflict-continue')).toBeDisabled();
  });

  it('文本冲突展示类别与三方版本可用性', async () => {
    stateMock.mockResolvedValue(mergeConflict());

    renderPage();

    expect(await screen.findByText('文本冲突')).toBeInTheDocument();
    expect(screen.getByTestId('conflict-file-versions')).toBeInTheDocument();
  });

  it('标记已解决会调用后端命令并刷新状态', async () => {
    stateMock.mockResolvedValueOnce(mergeConflict());
    stateMock.mockResolvedValue(mergeConflict({ files: [], canContinue: true }));
    resolveMock.mockResolvedValue(undefined);

    renderPage();

    fireEvent.click(await screen.findByRole('button', { name: '标记已解决' }));

    await waitFor(() => {
      expect(resolveMock).toHaveBeenCalledWith(1, ['src/a.ts']);
    });
  });

  it('全部解决后可以继续', async () => {
    stateMock.mockResolvedValue(mergeConflict({ files: [], canContinue: true }));
    continueMock.mockResolvedValue({ oid: 'abc', conflicts: [] });

    renderPage();

    const button = await screen.findByTestId('conflict-continue');
    expect(button).toBeEnabled();
    fireEvent.click(button);

    await waitFor(() => {
      expect(continueMock).toHaveBeenCalled();
    });
  });

  it('中止必须经过确认框才会执行', async () => {
    stateMock.mockResolvedValue(mergeConflict());
    abortMock.mockResolvedValue({ headOid: 'abc', headRef: 'main', snapshotId: 1 });

    renderPage();

    fireEvent.click(await screen.findByTestId('conflict-abort'));
    // 确认框出现且尚未执行
    expect(await screen.findByTestId('conflict-abort-confirm')).toBeInTheDocument();
    expect(abortMock).not.toHaveBeenCalled();

    fireEvent.click(screen.getByTestId('conflict-abort-confirm'));
    await waitFor(() => {
      expect(abortMock).toHaveBeenCalled();
    });
  });

  it('变基冲突显示进度与跳过按钮', async () => {
    stateMock.mockResolvedValue(
      state({
        opKind: 'rebase',
        opInProgress: true,
        currentStep: 1,
        totalSteps: 2,
        headName: 'refs/heads/feature',
        files: [file()],
        canAbort: true,
        canSkip: true,
      }),
    );
    skipMock.mockResolvedValue({ oid: null, conflicts: ['src/a.ts'] });

    renderPage();

    expect(await screen.findByTestId('conflict-progress')).toHaveTextContent('1 / 2');
    expect(screen.getByTestId('conflict-skip')).toBeInTheDocument();
  });
});
