import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { QueryClientProvider } from '@tanstack/react-query';
import { MemoryRouter, Route, Routes } from 'react-router-dom';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { CommitPage } from '@/features/commit/CommitPage';
import type { WorkspaceStatus } from '@/features/workspace/statusModel';
import {
  commitAmendContext,
  commitExecute,
  commitHooksList,
  commitMessageHint,
  commitPrepare,
} from '@/lib/ipc/commit';
import type { CommitPlan } from '@/lib/ipc/commit';
import { onRepoChanged, workspaceStatus } from '@/lib/ipc/workspace';
import { createTestQueryClient } from '@/test/queryClient';

/**
 * 提交面板的测试重点：
 *   - 禁用与**原因**（只禁用不解释是"界面看起来坏了"的经典来源）；
 *   - 预览里展示的确实是后端给的那份计划（文件、等价命令、钩子）；
 *   - 失败留在对话框里（钩子拒绝的原始输出必须能读到），且不静默清空用户输入。
 */
vi.mock('@/lib/ipc/commit', () => ({
  commitPrepare: vi.fn(),
  commitExecute: vi.fn(),
  commitMessageHint: vi.fn(),
  commitAmendContext: vi.fn(),
  commitHooksList: vi.fn(),
}));
vi.mock('@/lib/ipc/workspace', () => ({
  workspaceStatus: vi.fn(),
  onRepoChanged: vi.fn(),
}));
// 测试环境没有 Tauri：事件订阅整条链路跳过（它由 e2e 覆盖）
vi.mock('@/lib/ipc/client', () => ({
  isTauriRuntime: () => false,
  invokeCommand: vi.fn(),
}));

const commitPrepareMock = vi.mocked(commitPrepare);
const commitExecuteMock = vi.mocked(commitExecute);
const commitMessageHintMock = vi.mocked(commitMessageHint);
const commitAmendContextMock = vi.mocked(commitAmendContext);
const commitHooksListMock = vi.mocked(commitHooksList);
const workspaceStatusMock = vi.mocked(workspaceStatus);
const onRepoChangedMock = vi.mocked(onRepoChanged);

function statusWith(stagedPaths: readonly string[]): WorkspaceStatus {
  return {
    branch: {
      oid: 'a'.repeat(40),
      head: 'main',
      detached: false,
      upstream: null,
      ahead: null,
      behind: null,
    },
    operation: 'none',
    staged: stagedPaths.map((path) => ({
      path,
      oldPath: null,
      kind: 'ordinary' as const,
      indexStatus: 'M',
      worktreeStatus: '.',
      isBinary: false,
      isLfs: false,
      isSubmodule: false,
      sizeBytes: 12,
    })),
    unstaged: [],
    untracked: [],
    conflicted: [],
    ignored: [],
    ignoredCount: null,
  };
}

function plan(overrides: Partial<CommitPlan> = {}): CommitPlan {
  return {
    planId: 'plan-1',
    repoId: 1,
    files: [{ path: 'src/app.ts', indexStatus: 'M' }],
    message: 'feat: thing\n',
    description: null,
    author: null,
    sign: 'auto',
    signOff: false,
    noVerify: false,
    amend: false,
    amendMode: 'includeStaged',
    headPushed: false,
    hooks: ['pre-commit'],
    equivalentCommand: 'git commit -m "feat: thing"',
    headOid: 'b'.repeat(40),
    indexFingerprint: 'c'.repeat(40),
    createdAtMs: 1_700_000_000_000,
    expiresAtMs: 1_700_000_300_000,
    subject: 'feat: thing',
    subjectChars: 11,
    warnings: [],
    ...overrides,
  };
}

function renderCommit() {
  const queryClient = createTestQueryClient();
  return render(
    <QueryClientProvider client={queryClient}>
      <MemoryRouter initialEntries={['/repo/1/commit']}>
        <Routes>
          <Route path="/repo/:repoId/commit" element={<CommitPage />} />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

/** 填入提交信息（首行）。 */
function typeSubject(value: string) {
  fireEvent.change(screen.getByLabelText('提交信息'), { target: { value } });
}

beforeEach(() => {
  vi.clearAllMocks();
  onRepoChangedMock.mockResolvedValue(() => undefined);
  commitMessageHintMock.mockResolvedValue({
    recentMessages: [],
    template: null,
    branchStyle: null,
  });
  commitAmendContextMock.mockResolvedValue({
    subject: 'base',
    body: null,
    headOid: 'b'.repeat(40),
    pushed: false,
    pushedRefs: [],
  });
  commitHooksListMock.mockResolvedValue([
    { name: 'pre-commit', executable: true, commitHook: true },
  ]);
  workspaceStatusMock.mockResolvedValue(statusWith([]));
});

afterEach(() => {
  // 先卸载再复位：不 cleanup 会让上一个用例的 DOM 留在 document 里，
  // 于是同一个文案被匹配到两次（"Found multiple elements"），
  // 排查起来像是实现有问题，实际是测试自己没清干净
  cleanup();
  vi.restoreAllMocks();
});

describe('提交面板', () => {
  it('没有暂存内容时按钮禁用并说明原因', async () => {
    renderCommit();

    expect(await screen.findByText('还没有暂存任何改动')).toBeInTheDocument();
    expect(screen.getByTestId('commit-submit')).toBeDisabled();
  });

  it('有暂存内容时展示数量，填入信息后可预览计划', async () => {
    workspaceStatusMock.mockResolvedValue(statusWith(['src/a.ts', 'src/b.ts']));
    commitPrepareMock.mockResolvedValue(plan());

    renderCommit();

    expect(await screen.findByText('已暂存 2 个文件')).toBeInTheDocument();
    typeSubject('feat: thing');
    const submit = screen.getByTestId('commit-submit');
    expect(submit).toBeEnabled();

    fireEvent.click(submit);

    // 预览里展示的必须是后端给的那份计划。
    // 断言限定在对话框内：页面右侧栏也有一份"仓库里的钩子"清单，同名文本会撞车
    const dialog = await screen.findByRole('dialog');
    expect(within(dialog).getByText('src/app.ts')).toBeInTheDocument();
    expect(within(dialog).getByText('git commit -m "feat: thing"')).toBeInTheDocument();
    expect(within(dialog).getByText('pre-commit')).toBeInTheDocument();
    expect(commitPrepareMock).toHaveBeenCalledWith(
      1,
      expect.objectContaining({ message: 'feat: thing', amend: false, sign: 'auto' }),
    );
  });

  it('确认后执行计划，并在成功后清空表单', async () => {
    workspaceStatusMock.mockResolvedValue(statusWith(['src/a.ts']));
    commitPrepareMock.mockResolvedValue(plan());
    commitExecuteMock.mockResolvedValue({
      oid: 'd'.repeat(40),
      subject: 'feat: thing',
      snapshotId: null,
      paths: ['src/app.ts'],
    });

    renderCommit();
    // 必须等到状态查询落地：在那之前"有没有暂存内容"还不知道，按钮是禁用的
    expect(await screen.findByText('已暂存 1 个文件')).toBeInTheDocument();
    typeSubject('feat: thing');
    fireEvent.click(screen.getByTestId('commit-submit'));

    const dialog = await screen.findByRole('dialog');
    fireEvent.click(within(dialog).getByRole('button', { name: '提交' }));

    await waitFor(() => {
      expect(commitExecuteMock).toHaveBeenCalledWith('plan-1');
    });
    await waitFor(() => {
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    });
    expect(screen.getByLabelText('提交信息')).toHaveValue('');
  });

  it('钩子拒绝时把原始输出留在对话框里，输入不被清空', async () => {
    workspaceStatusMock.mockResolvedValue(statusWith(['src/a.ts']));
    commitPrepareMock.mockResolvedValue(plan());
    commitExecuteMock.mockRejectedValue({
      code: 'HOOK_REJECTED',
      message: 'a commit hook rejected the commit',
      detail: 'pre-commit: nope',
      hint: 'pre-commit',
      actions: [],
      retryable: false,
    });

    renderCommit();
    expect(await screen.findByText('已暂存 1 个文件')).toBeInTheDocument();
    typeSubject('feat: thing');
    fireEvent.click(screen.getByTestId('commit-submit'));

    const dialog = await screen.findByRole('dialog');
    fireEvent.click(within(dialog).getByRole('button', { name: '提交' }));

    // T1.8 起钩子拒绝走结构化面板：先给"看起来是哪一类"（并标明是推断），
    // 再把原文一字不改地摆出来
    expect(await within(dialog).findByText(/无法从输出判断是哪个钩子失败/)).toBeInTheDocument();
    expect(within(dialog).getByText('pre-commit: nope')).toBeInTheDocument();
    expect(screen.getByLabelText('提交信息')).toHaveValue('feat: thing');
  });

  it('amend 模式下没有暂存内容也能提交（只改信息）', async () => {
    commitPrepareMock.mockResolvedValue(plan({ amend: true, files: [] }));

    renderCommit();
    expect(await screen.findByText('还没有暂存任何改动')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('checkbox', { name: 'Amend 上一次提交' }));
    typeSubject('fix: message only');

    expect(screen.getByTestId('commit-submit')).toBeEnabled();
    fireEvent.click(screen.getByTestId('commit-submit'));

    expect(await screen.findByText('确认这次提交')).toBeInTheDocument();
    expect(commitPrepareMock).toHaveBeenCalledWith(1, expect.objectContaining({ amend: true }));
  });

  it('Ctrl+Enter 触发预览', async () => {
    workspaceStatusMock.mockResolvedValue(statusWith(['src/a.ts']));
    commitPrepareMock.mockResolvedValue(plan());

    renderCommit();
    expect(await screen.findByText('已暂存 1 个文件')).toBeInTheDocument();
    typeSubject('feat: keyboard');

    fireEvent.keyDown(screen.getByTestId('commit-panel'), { key: 'Enter', ctrlKey: true });

    await waitFor(() => {
      expect(commitPrepareMock).toHaveBeenCalledTimes(1);
    });
  });

  it('最近提交下拉可以把历史信息填回首行', async () => {
    commitMessageHintMock.mockResolvedValue({
      recentMessages: ['feat: previous'],
      template: 'feat: ',
      branchStyle: 'feat',
    });
    workspaceStatusMock.mockResolvedValue(statusWith(['src/a.ts']));

    renderCommit();

    // 模板按钮与下拉都来自后端提示（纯本地规则）
    expect(await screen.findByRole('button', { name: '用「feat: 」开头' })).toBeInTheDocument();
    const hint = await screen.findByRole('combobox', { name: '最近提交' });
    expect(hint).toBeInTheDocument();
  });

  it('勾选 amend 会填入上一次信息，并在可能已推送时给出警示', async () => {
    commitAmendContextMock.mockResolvedValue({
      subject: 'fix: previous',
      body: 'old body',
      headOid: 'b'.repeat(40),
      pushed: true,
      pushedRefs: ['origin/main'],
    });

    renderCommit();
    fireEvent.click(await screen.findByRole('checkbox', { name: 'Amend 上一次提交' }));

    await waitFor(() => {
      expect(screen.getByLabelText('提交信息')).toHaveValue('fix: previous');
    });
    expect(screen.getByLabelText('正文')).toHaveValue('old body');
    // 改写已推送的历史必须说清后果，而不是只给一个开关
    expect(await screen.findByText(/origin\/main/)).toBeInTheDocument();
    expect(screen.getByText(/force-with-lease/)).toBeInTheDocument();
  });

  it('amend 时可以选择只改信息，并把该选择原样传给后端', async () => {
    commitPrepareMock.mockResolvedValue(plan({ amend: true, amendMode: 'messageOnly' }));

    renderCommit();
    fireEvent.click(await screen.findByRole('checkbox', { name: 'Amend 上一次提交' }));
    // 等 amend 语境到达：模式选择是它之后才出现的元素
    // （不用 /base/ 这类文本匹配——插值会把一句话拆成多个文本节点）
    await screen.findByRole('radio', { name: '只改提交信息' });
    fireEvent.click(screen.getByRole('radio', { name: '只改提交信息' }));
    typeSubject('fix: message only');
    fireEvent.click(screen.getByTestId('commit-submit'));

    await waitFor(() => {
      expect(commitPrepareMock).toHaveBeenCalledWith(
        1,
        expect.objectContaining({ amend: true, amendMode: 'messageOnly' }),
      );
    });
  });

  it('钩子拒绝时展示结构化输出，并能跳过钩子重试', async () => {
    workspaceStatusMock.mockResolvedValue(statusWith(['src/a.ts']));
    commitPrepareMock.mockResolvedValue(plan());
    commitExecuteMock.mockRejectedValue({
      code: 'HOOK_REJECTED',
      message: 'a commit hook rejected the commit',
      detail: '  3:5  error  Unexpected console statement  no-console',
      hint: 'pre-commit',
      actions: [],
      retryable: false,
    });

    renderCommit();
    expect(await screen.findByText('已暂存 1 个文件')).toBeInTheDocument();
    typeSubject('feat: thing');
    fireEvent.click(screen.getByTestId('commit-submit'));

    const dialog = await screen.findByRole('dialog');
    fireEvent.click(within(dialog).getByRole('button', { name: '提交' }));

    // 推断出的阶段 + 错误计数 + 一字不改的原文
    expect(await within(dialog).findByText(/看起来是代码检查/)).toBeInTheDocument();
    expect(within(dialog).getByText(/1 行错误/)).toBeInTheDocument();
    expect(within(dialog).getByText(/Unexpected console statement/)).toBeInTheDocument();

    fireEvent.click(within(dialog).getByRole('button', { name: '跳过钩子重试' }));
    await waitFor(() => {
      expect(commitPrepareMock).toHaveBeenCalledWith(
        1,
        expect.objectContaining({ noVerify: true }),
      );
    });
  });

  it('列出仓库里的钩子，并对不会执行的如实标注', async () => {
    commitHooksListMock.mockResolvedValue([
      { name: 'pre-commit', executable: true, commitHook: true },
      { name: 'pre-push', executable: false, commitHook: false },
    ]);

    renderCommit();

    expect(await screen.findByText('pre-commit')).toBeInTheDocument();
    expect(screen.getByText('pre-push')).toBeInTheDocument();
    expect(screen.getByText('缺少执行位，git 会忽略它')).toBeInTheDocument();
  });
});
