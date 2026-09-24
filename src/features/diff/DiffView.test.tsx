import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { DiffView } from '@/features/diff/DiffView';
import { workspaceDiff, workspaceDiffPatch } from '@/lib/ipc/workspace';
import type { DiffHunk, DiffReport } from '@/lib/ipc/workspace';
import { initialSettingsState, useSettingsStore } from '@/stores/settingsStore';
import { createTestQueryClient } from '@/test/queryClient';
import { QueryClientProvider } from '@tanstack/react-query';

/**
 * Diff 查看器的测试重点：
 *   - 内联/并排两种模式的真实渲染（这是验收项"切换模式"的自动化部分）；
 *   - 截断横幅与"加载完整 diff"（大文件保护不能是死路）；
 *   - 二进制文件与失败态的自助出口。
 */
vi.mock('@/lib/ipc/workspace', () => ({
  workspaceDiff: vi.fn(),
  workspaceDiffPatch: vi.fn(),
}));
vi.mock('@/lib/ipc', () => ({
  settingsAll: vi.fn().mockResolvedValue({ values: {} }),
  settingsSet: vi.fn().mockResolvedValue(undefined),
}));

const workspaceDiffMock = vi.mocked(workspaceDiff);
const workspaceDiffPatchMock = vi.mocked(workspaceDiffPatch);

/** jsdom 没有 ResizeObserver：桩直接报告一次 600px 高度，让 VirtualList 渲染出来。 */
class ResizeObserverStub {
  private readonly callback: ResizeObserverCallback;
  constructor(callback: ResizeObserverCallback) {
    this.callback = callback;
  }
  observe = (): void => {
    this.callback(
      [{ contentRect: { height: 600 } } as ResizeObserverEntry],
      this as unknown as ResizeObserver,
    );
  };
  unobserve(): void {}
  disconnect(): void {}
}

function hunk(): DiffHunk {
  return {
    oldStart: 1,
    oldLines: 3,
    newStart: 1,
    newLines: 3,
    header: 'fn main() {',
    lines: [
      { kind: 'context', content: 'context line', oldNo: 1, newNo: 1 },
      { kind: 'removed', content: 'old value', oldNo: 2, newNo: null },
      { kind: 'added', content: 'new value', oldNo: null, newNo: 2 },
      { kind: 'context', content: 'tail', oldNo: 3, newNo: 3 },
    ],
  };
}

function report(overrides: Partial<DiffReport['files'][number]> = {}): DiffReport {
  return {
    files: [
      {
        path: 'src/app.ts',
        oldPath: null,
        change: 'modified',
        binary: false,
        additions: 1,
        deletions: 1,
        truncated: false,
        hunks: [hunk()],
        ...overrides,
      },
    ],
    truncatedFiles: 0,
  };
}

function renderDiff() {
  const queryClient = createTestQueryClient();
  return render(
    <QueryClientProvider client={queryClient}>
      <DiffView repoId={1} path="src/app.ts" target="unstaged" />
    </QueryClientProvider>,
  );
}

/** setup.ts 已经补过一个"什么都不做"的 ResizeObserver；这里换成会报告高度的版本。 */
const originalResizeObserver = window.ResizeObserver;

beforeEach(() => {
  vi.clearAllMocks();
  window.ResizeObserver = ResizeObserverStub as unknown as typeof ResizeObserver;
  useSettingsStore.setState(initialSettingsState);
});

afterEach(() => {
  window.ResizeObserver = originalResizeObserver;
  vi.restoreAllMocks();
});

describe('DiffView', () => {
  it('内联模式渲染行号、符号与内容（修改对被拆成词级片段）', async () => {
    workspaceDiffMock.mockResolvedValue(report());

    renderDiff();

    // 上下文行没有修改对，是完整的文本节点
    expect(await screen.findByText('context line')).toBeInTheDocument();
    // "修改对"的行被字符级高亮拆成词片段（片段含词尾空白，用正则匹配）
    expect(screen.getAllByText(/old/).length).toBeGreaterThanOrEqual(1);
    expect(screen.getAllByText(/new/).length).toBeGreaterThanOrEqual(1);
    expect(screen.getAllByText('value').length).toBeGreaterThanOrEqual(1);
    // hunk 头显示范围
    expect(screen.getAllByText('@@ -1,3 +1,3 @@').length).toBeGreaterThanOrEqual(1);
  });

  it('切换到并排模式后同一行对左右并排出现', async () => {
    workspaceDiffMock.mockResolvedValue(report());

    renderDiff();
    fireEvent.click(await screen.findByRole('radio', { name: '并排' }));

    // 并排模式下左右内容都渲染；字符级高亮把"修改对"拆成词片段
    await waitFor(() => {
      expect(screen.getAllByText('old').length).toBeGreaterThanOrEqual(1);
    });
    expect(screen.getAllByText('value').length).toBeGreaterThanOrEqual(2);
    // 模式偏好被持久化
    await waitFor(() => {
      expect(useSettingsStore.getState().getJson('ui.diffViewMode', 'unified')).toBe(
        'side-by-side',
      );
    });
  });

  it('截断的文件给出横幅，点击后以 forceFull 重新请求', async () => {
    workspaceDiffMock.mockResolvedValue(report({ truncated: true, hunks: [hunk()] }));

    renderDiff();

    expect(await screen.findByText(/文件过大，已截断/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '加载完整 diff' }));

    await waitFor(() => {
      const lastCall = workspaceDiffMock.mock.calls.at(-1);
      expect(lastCall?.[1]?.forceFull).toBe(true);
    });
  });

  it('点击 hunk 头折叠正文，再点击展开', async () => {
    workspaceDiffMock.mockResolvedValue(report());

    renderDiff();
    const header = await screen.findByRole('button', { name: /@@ -1,3 \+1,3 @@/ });
    // 修改对的行被字符级高亮拆成片段，用未修改的上下文行做探针
    await screen.findByText('context line');

    expect(header).toHaveAttribute('aria-expanded', 'true');
    fireEvent.click(header);
    expect(header).toHaveAttribute('aria-expanded', 'false');
    expect(screen.queryByText('context line')).not.toBeInTheDocument();

    fireEvent.click(header);
    expect(screen.getByText('context line')).toBeInTheDocument();
  });

  it('二进制文件显示说明而不是行内容', async () => {
    workspaceDiffMock.mockResolvedValue(report({ binary: true, hunks: [] }));

    renderDiff();

    expect(await screen.findByText('二进制文件')).toBeInTheDocument();
    expect(screen.queryByText('old value')).not.toBeInTheDocument();
  });

  it('失败时给出重试入口', async () => {
    workspaceDiffMock.mockRejectedValue({
      code: 'INTERNAL',
      message: 'git diff failed',
      actions: [],
      retryable: true,
    });

    renderDiff();

    expect(await screen.findByText('读取 diff 失败')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '重试' })).toBeInTheDocument();
  });

  it('复制文件补丁走后端原始字节并写入剪贴板', async () => {
    workspaceDiffMock.mockResolvedValue(report());
    workspaceDiffPatchMock.mockResolvedValue(Array.from(new TextEncoder().encode('diff --git x')));
    // jsdom 没有 navigator.clipboard
    const writeText = vi.fn<() => Promise<void>>().mockResolvedValue(undefined);
    Object.assign(navigator, { clipboard: { writeText } });

    renderDiff();
    fireEvent.click(await screen.findByRole('button', { name: '复制文件补丁' }));

    await waitFor(() => {
      expect(workspaceDiffPatchMock).toHaveBeenCalledWith(
        1,
        expect.objectContaining({ target: 'unstaged', forceFull: true }),
      );
      expect(writeText).toHaveBeenCalledWith('diff --git x');
    });
  });
});
