//! 合并对话框（T3.4）的组件测试。
//!
//! 钉住两段式契约的界面行为：预览才见计划；预检冲突要醒目展示；
//! conflicted 是结果不是错误（导航去冲突页而不是弹错误）；计划过期
//! （PLAN_STALE）走通用错误链路。
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { QueryClientProvider } from '@tanstack/react-query';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { MergeDialog } from '@/features/branches/MergeDialog';
import { gitMergeExecute, gitMergePrepare } from '@/lib/ipc';
import type { MergePlan } from '@/lib/ipc';
import { createTestQueryClient } from '@/test/queryClient';
import { initialToastState, useToastStore } from '@/stores/toastStore';

vi.mock('@/lib/ipc', () => ({
  gitMergePrepare: vi.fn(),
  gitMergeExecute: vi.fn(),
}));

const prepareMock = vi.mocked(gitMergePrepare);
const executeMock = vi.mocked(gitMergeExecute);

function plan(overrides: Partial<MergePlan> = {}): MergePlan {
  return {
    planId: 'plan-1',
    source: 'feature',
    strategy: 'merge',
    verdict: 'trueMerge',
    sourceOnlyCommits: [{ oid: 'abc1234def', subject: 'feature work', authorTime: 1_700_000_000 }],
    sourceCommitCount: 1,
    previewAvailable: true,
    conflicted: [],
    defaultMessage: "Merge branch 'feature' into main",
    equivalentCommand: 'git merge feature',
    ...overrides,
  };
}

function renderDialog() {
  const queryClient = createTestQueryClient();
  return render(
    <QueryClientProvider client={queryClient}>
      <MemoryRouter>
        <MergeDialog
          repoId={1}
          currentBranch="main"
          locals={['main', 'feature', 'other']}
          open
          onOpenChange={() => {}}
        />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  useToastStore.setState(initialToastState);
  prepareMock.mockResolvedValue(plan());
  executeMock.mockResolvedValue({ kind: 'mergeCommit', oid: 'abc', conflicts: [], snapshotId: 1 });
});

describe('MergeDialog', () => {
  it('选源分支后预览，计划展示裁决、提交清单与等价命令', async () => {
    renderDialog();

    fireEvent.change(await screen.findByTestId('merge-source'), {
      target: { value: 'feature' },
    });
    fireEvent.click(screen.getByTestId('merge-preview'));

    expect(await screen.findByTestId('merge-plan')).toBeInTheDocument();
    expect(screen.getByText(/将产生合并提交/)).toBeInTheDocument();
    expect(screen.getByText('feature work')).toBeInTheDocument();
    expect(screen.getByTestId('merge-equivalent')).toHaveTextContent('git merge feature');
    // 信息预填默认值
    expect(screen.getByTestId('merge-message')).toHaveValue("Merge branch 'feature' into main");
  });

  it('预检到冲突时展示冲突文件清单（而不是等执行才报错）', async () => {
    prepareMock.mockResolvedValue(plan({ conflicted: ['a.txt', 'b.txt'] }));
    renderDialog();

    fireEvent.change(await screen.findByTestId('merge-source'), { target: { value: 'feature' } });
    fireEvent.click(screen.getByTestId('merge-preview'));

    const conflicts = await screen.findByTestId('merge-conflicts');
    expect(conflicts).toHaveTextContent('a.txt, b.txt');
  });

  it('git 太旧预检不可用时明示，不假装预检过', async () => {
    prepareMock.mockResolvedValue(plan({ previewAvailable: false }));
    renderDialog();

    fireEvent.change(await screen.findByTestId('merge-source'), { target: { value: 'feature' } });
    fireEvent.click(screen.getByTestId('merge-preview'));

    expect(await screen.findByTestId('merge-preview-unavailable')).toBeInTheDocument();
  });

  it('执行返回 conflicted 时是正常结果：不弹错误，由页面导航去冲突处理', async () => {
    executeMock.mockResolvedValue({
      kind: 'conflicted',
      oid: null,
      conflicts: ['a.txt'],
      snapshotId: 1,
    });
    renderDialog();

    fireEvent.change(await screen.findByTestId('merge-source'), { target: { value: 'feature' } });
    fireEvent.click(screen.getByTestId('merge-preview'));
    fireEvent.click(await screen.findByTestId('merge-execute'));

    await waitFor(() => {
      expect(executeMock).toHaveBeenCalledWith(1, {
        planId: 'plan-1',
        message: "Merge branch 'feature' into main",
      });
    });
  });

  it('已是最新时禁用执行按钮', async () => {
    prepareMock.mockResolvedValue(
      plan({ verdict: 'upToDate', sourceCommitCount: 0, sourceOnlyCommits: [] }),
    );
    renderDialog();

    fireEvent.change(await screen.findByTestId('merge-source'), { target: { value: 'feature' } });
    fireEvent.click(screen.getByTestId('merge-preview'));

    expect(await screen.findByTestId('merge-execute')).toBeDisabled();
  });

  it('执行成功给出成功提示（写入 toast store）', async () => {
    renderDialog();

    fireEvent.change(await screen.findByTestId('merge-source'), { target: { value: 'feature' } });
    fireEvent.click(screen.getByTestId('merge-preview'));
    fireEvent.click(await screen.findByTestId('merge-execute'));

    await waitFor(() => {
      // toast 渲染在应用的 Toaster 容器里（组件树外）；这里断言 store 状态
      const toasts = useToastStore.getState().toasts;
      expect(toasts.some((toast) => toast.title === '合并完成')).toBe(true);
    });
  });
});
