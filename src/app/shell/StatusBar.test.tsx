import { render, screen, waitFor } from '@testing-library/react';
import { QueryClientProvider } from '@tanstack/react-query';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { StatusBar } from '@/app/shell/StatusBar';
import { operationHistory } from '@/lib/ipc/audit';
import type { OperationHistoryEntry } from '@/lib/ipc/audit';
import { createTestQueryClient } from '@/test/queryClient';

/**
 * 状态栏的"最近可回滚点"指示器（T3.10）。
 *
 * 为什么在这里钉：它是本产品最要紧的一句"你现在还回得去"，而它的规则只有两条，
 * 两条都必须准确——有可回滚点时**出现**，其余情况**完全不出现**。
 * 一个常年亮着的警示牌等于没有警示，所以"不出现"的用例与"出现"的用例一样重要。
 *
 * 放在组件测试而不是 E2E：状态栏的"当前仓库"来自用户在外壳里选过的仓库，
 * 深链进入时它为空——在 E2E 里断言"不出现"会是因为没数据而通过的假阳性，
 * 而真阳性（有数据但不该显示）恰恰是最需要覆盖的那一类。
 */
vi.mock('@/lib/ipc/audit', () => ({ operationHistory: vi.fn() }));
vi.mock('@/features/repo/recentRepos', () => ({
  useCurrentRepo: () => ({ id: 1, name: 'fixture', defaultBranch: 'main' }),
}));
vi.mock('@/stores/jobStore', () => ({
  useJobStore: (selector: (state: { jobs: readonly unknown[] }) => unknown) =>
    selector({ jobs: [] }),
  countActiveJobs: () => 0,
}));

const historyMock = vi.mocked(operationHistory);

/** 一条带快照的记录（字段与后端 DTO 对齐）。 */
function entry(canRollback: boolean): OperationHistoryEntry {
  return {
    id: 7,
    repoId: 1,
    opType: 'reset',
    argsJson: '{"mode":"hard"}',
    startedAtMs: 1_700_000_000_000,
    endedAtMs: 1_700_000_000_005,
    durationMs: 5,
    exitCode: 0,
    result: 'ok',
    stderrSummary: null,
    snapshotId: 3,
    reversible: true,
    canRollback,
  };
}

function renderBar(): void {
  render(
    <QueryClientProvider client={createTestQueryClient()}>
      <MemoryRouter>
        <StatusBar />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe('状态栏的可回滚指示器', () => {
  it('最近一次破坏性操作仍可回滚时显示指示器', async () => {
    historyMock.mockResolvedValue({ total: 1, entries: [entry(true)] });

    renderBar();

    expect(await screen.findByTestId('status-rollback-point')).toBeInTheDocument();
    expect(historyMock).toHaveBeenCalledWith(1, { onlyReversible: true }, 1, 0);
  });

  it('没有任何记录时不显示', async () => {
    historyMock.mockResolvedValue({ total: 0, entries: [] });

    renderBar();

    await waitFor(() => {
      expect(historyMock).toHaveBeenCalled();
    });
    expect(screen.queryByTestId('status-rollback-point')).not.toBeInTheDocument();
  });

  it('记录还在但回滚点已失效时不显示（那是作废的点，点下去只会失败）', async () => {
    historyMock.mockResolvedValue({ total: 1, entries: [entry(false)] });

    renderBar();

    await waitFor(() => {
      expect(historyMock).toHaveBeenCalled();
    });
    expect(screen.queryByTestId('status-rollback-point')).not.toBeInTheDocument();
  });
});
