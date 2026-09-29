import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { QueryClientProvider } from '@tanstack/react-query';
import { MemoryRouter, Route, Routes } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { HistoryOpsPanel } from '@/features/history/HistoryOpsPanel';
import { gitResetExecute, gitResetPrepare, gitReflog } from '@/lib/ipc';
import type { ResetPlan } from '@/lib/ipc';
import { useGraphSelectionStore } from '@/features/history/graphSelectionStore';
import { createTestQueryClient } from '@/test/queryClient';

/**
 * 历史操作面板（T2.8）。
 *
 * 重点钉住**重置的闸门**：计划必须先看、`--hard` 必须输入后端给的确认词、
 * 确认词不对时按钮保持禁用。这三件事错了，破坏性操作就没有安全网了。
 */
vi.mock('@/lib/ipc', () => ({
  gitCherryPick: vi.fn(),
  gitRevert: vi.fn(),
  gitResetPrepare: vi.fn(),
  gitResetExecute: vi.fn(),
  gitReflog: vi.fn(),
}));

const prepareMock = vi.mocked(gitResetPrepare);
const executeMock = vi.mocked(gitResetExecute);
const reflogMock = vi.mocked(gitReflog);

function plan(overrides: Partial<ResetPlan> = {}): ResetPlan {
  return {
    planId: 'reset-1-1',
    mode: 'hard',
    targetOid: 'bbb222333444555666777888999aaabbbcccddde',
    targetSubject: 'older commit',
    headBefore: 'aaa111222333444555666777888999aaabbbccc0',
    discarded: [{ oid: 'ccc0', subject: 'doomed', authorTime: 1 }],
    discardedTruncated: false,
    discardedCount: 1,
    lostStaged: [],
    lostWorktree: [],
    untrackedToRemove: [],
    remote: { upstream: 'origin/main', notOnRemote: 1 },
    requiresConfirmation: true,
    confirmationWord: 'reset',
    snapshotRequired: true,
    ...overrides,
  };
}

function renderPanel() {
  const queryClient = createTestQueryClient();
  return render(
    <QueryClientProvider client={queryClient}>
      <MemoryRouter initialEntries={['/repo/1/history']}>
        <Routes>
          <Route path="/repo/:repoId/history" element={<HistoryOpsPanel />} />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  reflogMock.mockResolvedValue([]);
  useGraphSelectionStore.setState({ detailOid: 'aaa111222333444555666777888999aaabbbccc0' });
  prepareMock.mockResolvedValue(plan());
  executeMock.mockResolvedValue({
    mode: 'hard',
    headBefore: 'aaa111222333444555666777888999aaabbbccc0',
    headAfter: 'bbb222333444555666777888999aaabbbcccddde',
    discardedCount: 1,
    snapshotId: 7,
  });
});

describe('历史操作面板', () => {
  it('没有选中提交时动作全部禁用', async () => {
    useGraphSelectionStore.setState({ detailOid: null });

    renderPanel();

    expect(await screen.findByTestId('history-ops')).toHaveTextContent('点选一个提交');
    expect(screen.getByTestId('history-ops-reset')).toBeDisabled();
    expect(screen.getByTestId('history-ops-cherry-pick')).toBeDisabled();
  });

  it('重置先出计划：--hard 必须输入后端给的确认词才能执行', async () => {
    renderPanel();
    await waitFor(() => {
      expect(reflogMock).toHaveBeenCalled();
    });

    fireEvent.click(screen.getByTestId('history-ops-reset'));

    const dialog = await screen.findByTestId('reset-plan-dialog');
    await waitFor(() => {
      expect(dialog).toHaveTextContent('有 1 个远端也没有');
    });
    expect(dialog).toHaveTextContent('doomed');

    fireEvent.change(screen.getByTestId('reset-confirm-input'), { target: { value: 'yes' } });
    expect(screen.getByTestId('reset-confirm')).toBeDisabled();
    expect(executeMock).not.toHaveBeenCalled();

    fireEvent.change(screen.getByTestId('reset-confirm-input'), { target: { value: ' reset ' } });
    fireEvent.click(screen.getByTestId('reset-confirm'));

    await waitFor(() => {
      expect(executeMock).toHaveBeenCalledTimes(1);
    });
    // 逐个断言：确认词必须原样传给后端（它是执行闸门的输入）
    expect(executeMock.mock.calls[0]?.[0]).toBe(1);
    expect(executeMock.mock.calls[0]?.[1]).toBe('reset-1-1');
    expect(executeMock.mock.calls[0]?.[2]).toBe(' reset ');
  });
});
