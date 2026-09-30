import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { QueryClientProvider } from '@tanstack/react-query';
import { MemoryRouter, Route, Routes } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { RebasePanel, type RebasePanelProps } from '@/features/rebase/RebasePanel';
import {
  gitConflictAbort,
  gitConflictState,
  gitRebaseContinueEdit,
  gitRebaseExecute,
  gitRebasePreviewOnly,
  gitRebaseRange,
} from '@/lib/ipc';
import type { RebasePreview, RebaseRangeCommit } from '@/lib/ipc';
import { createTestQueryClient } from '@/test/queryClient';

/**
 * Rebase 面板（T3.6）。
 *
 * 钉住的都是"说错会伤人"的路径：
 * - 非法计划即时拦下并给出人话原因（且不白白请求后端预览）；
 * - 执行必须经确认框，且发给后端的 steps 与界面此刻显示的一致
 *   （错位的 steps 会重写出用户没看过的历史）；
 * - 键盘（Alt+↑/↓）与拖拽都真的改变顺序；
 * - 暂停（冲突 / edit）不是错误，出口分别是"去冲突页"与"改完继续"，
 *   中止走 conflict_abort。
 */
vi.mock('@/lib/ipc', () => ({
  gitRebaseRange: vi.fn(),
  gitRebasePreviewOnly: vi.fn(),
  gitRebaseExecute: vi.fn(),
  gitRebaseContinueEdit: vi.fn(),
  gitConflictState: vi.fn(),
  gitConflictAbort: vi.fn(),
}));

const rangeMock = vi.mocked(gitRebaseRange);
const previewMock = vi.mocked(gitRebasePreviewOnly);
const executeMock = vi.mocked(gitRebaseExecute);
const continueMock = vi.mocked(gitRebaseContinueEdit);
const conflictStateMock = vi.mocked(gitConflictState);
const abortMock = vi.mocked(gitConflictAbort);

const C1 = 'c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1';
const C2 = 'c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2';
const C3 = 'c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3';

const RANGE: readonly RebaseRangeCommit[] = [
  { oid: C1, subject: 'one', parents: ['base000'], author: 'Ada', authorTime: 1_700_000_000 },
  { oid: C2, subject: 'two', parents: [C1], author: 'Ada', authorTime: 1_700_000_100 },
  { oid: C3, subject: 'three', parents: [C2], author: 'Ada', authorTime: 1_700_000_200 },
];

const PREVIEW: RebasePreview = {
  surviving: [
    { oid: C1, subject: 'one' },
    { oid: C2, subject: 'two' },
    { oid: C3, subject: 'three' },
  ],
  dropped: [],
  reworded: [],
  squashed: [],
  affectedCount: 3,
  touchesPushed: false,
  todoText: 'pick c1c1c1c one\npick c2c2c2c two\npick c3c3c3c three\n',
};

function renderPanel(props: Partial<RebasePanelProps> = {}) {
  const queryClient = createTestQueryClient();
  return render(
    <QueryClientProvider client={queryClient}>
      <MemoryRouter initialEntries={['/repo/1/status']}>
        <Routes>
          <Route
            path="/repo/:repoId/status"
            element={
              <RebasePanel
                repoId={1}
                base="base000"
                head={C3}
                open
                onOpenChange={() => undefined}
                {...props}
              />
            }
          />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

/** 面板内步骤清单的行（与预览区的 li 隔离）。 */
async function stepRows(): Promise<HTMLElement[]> {
  const list = await screen.findByTestId('rebase-step-list');
  return within(list).getAllByRole('listitem');
}

beforeEach(() => {
  vi.clearAllMocks();
  rangeMock.mockResolvedValue([...RANGE]);
  previewMock.mockResolvedValue(PREVIEW);
  conflictStateMock.mockResolvedValue({
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
  });
});

describe('Rebase 面板', () => {
  it('装载区间并展示步骤与预览', async () => {
    renderPanel();

    const rows = await stepRows();
    expect(rows).toHaveLength(3);
    expect(rows[0]).toHaveTextContent('one');
    expect(rows[0]).toHaveTextContent('Ada');

    const preview = await screen.findByTestId('rebase-preview');
    expect(preview).toHaveTextContent('将重写 3 个提交');
    expect(rangeMock).toHaveBeenCalledWith(1, 'base000', C3);
  });

  it('第一条设为 squash（preset）时即时给出人话原因，且不请求预览', async () => {
    renderPanel({ preset: { oid: C1, action: 'squash' } });

    const issue = await screen.findByTestId(`rebase-issue-${C1}`);
    expect(issue).toHaveTextContent('它前面没有可并入的提交');

    // 计划非法时不该把请求打给后端（preview 只在计划合法时发）
    await new Promise((resolve) => setTimeout(resolve, 350));
    expect(previewMock).not.toHaveBeenCalled();
    expect(screen.getByTestId('rebase-preview-empty')).toHaveTextContent('修正后显示预览');
  });

  it('Alt+↓ 把当前行下移（键盘替代拖拽）', async () => {
    renderPanel();
    const rows = await stepRows();

    fireEvent.keyDown(rows[0]!, { key: 'ArrowDown', altKey: true });

    const after = await stepRows();
    expect(after[0]).toHaveTextContent('two');
    expect(after[1]).toHaveTextContent('one');
  });

  it('拖拽到某行下半区时移动到它后面', async () => {
    renderPanel();
    const rows = await stepRows();

    // jsdom 没有 DragEvent：`fireEvent.dragOver` 会退化成无 clientY 的普通事件，
    // 组件据此判不出"上半/下半"。这里显式构造事件并附上 clientY 与 dataTransfer。
    const makeDragEvent = (type: string, init: Record<string, unknown>): Event => {
      const event = new Event(type, { bubbles: true, cancelable: true });
      Object.assign(event, { dataTransfer: {}, ...init });
      return event;
    };

    fireEvent(rows[0]!, makeDragEvent('dragstart', { effectAllowed: 'move' }));
    fireEvent(rows[1]!, makeDragEvent('dragover', { clientY: 10 }));
    fireEvent(rows[1]!, makeDragEvent('drop', {}));

    const after = await stepRows();
    expect(after[0]).toHaveTextContent('two');
    expect(after[1]).toHaveTextContent('one');
  });

  it('执行必须经确认框，发给后端的 steps 与界面一致，并展示快照与完成态', async () => {
    executeMock.mockResolvedValue({ kind: 'completed', oid: 'new0000', snapshotId: 42 });
    renderPanel();
    await stepRows();
    await screen.findByTestId('rebase-preview');

    fireEvent.click(screen.getByTestId('rebase-execute'));
    expect(executeMock).not.toHaveBeenCalled();

    fireEvent.click(await screen.findByTestId('rebase-confirm-execute'));

    await waitFor(() => {
      expect(executeMock).toHaveBeenCalledTimes(1);
    });
    expect(executeMock.mock.calls[0]?.[1]).toMatchObject({
      base: 'base000',
      head: C3,
      steps: [
        { oid: C1, action: 'pick' },
        { oid: C2, action: 'pick' },
        { oid: C3, action: 'pick' },
      ],
    });
    expect(await screen.findByTestId('rebase-snapshot-hint')).toHaveTextContent('42');
  });

  it('冲突暂停是结果：展示"去解决冲突"与中止（走 conflict_abort）', async () => {
    executeMock.mockResolvedValue({ kind: 'pausedConflict', conflicts: ['a.txt'], snapshotId: 3 });
    abortMock.mockResolvedValue({ headOid: null, headRef: null, snapshotId: 3 });
    renderPanel();
    await stepRows();
    await screen.findByTestId('rebase-preview');

    fireEvent.click(screen.getByTestId('rebase-execute'));
    fireEvent.click(await screen.findByTestId('rebase-confirm-execute'));

    expect(await screen.findByTestId('rebase-goto-conflict')).toBeInTheDocument();
    expect(screen.getByTestId('rebase-outcome')).toHaveTextContent('a.txt');

    fireEvent.click(screen.getByTestId('rebase-abort'));
    await waitFor(() => {
      expect(abortMock).toHaveBeenCalledWith(1);
    });
  });

  it('edit 暂停后"修改完成，继续"调用 continue_edit', async () => {
    executeMock.mockResolvedValue({ kind: 'pausedEdit', oid: C2, snapshotId: 7 });
    continueMock.mockResolvedValue({ kind: 'completed', oid: 'new0000', snapshotId: null });
    renderPanel();
    await stepRows();
    await screen.findByTestId('rebase-preview');

    fireEvent.click(screen.getByTestId('rebase-execute'));
    fireEvent.click(await screen.findByTestId('rebase-confirm-execute'));

    const continueButton = await screen.findByTestId('rebase-continue-edit');
    fireEvent.click(continueButton);

    await waitFor(() => {
      expect(continueMock).toHaveBeenCalledWith(1);
    });
    // 完成后进入完成态（新的 outcome 覆盖 edit 暂停）
    expect(await screen.findByTestId('rebase-close')).toBeInTheDocument();
  });
});
