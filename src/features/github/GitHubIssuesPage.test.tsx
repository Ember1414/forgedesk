import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { GitHubIssuesPage } from '@/features/github/GitHubIssuesPage';
import {
  repoIssueAssignees,
  repoIssueAssigneesSet,
  repoIssueBody,
  repoIssueCommentCreate,
  repoIssueCommentsList,
  repoIssueCreate,
  repoIssueEdit,
  repoIssueGet,
  repoIssueList,
  repoIssueStateSet,
} from '@/lib/ipc';
import type { IssueDetail, IssueSummary } from '@/lib/ipc';
import { initialToastState, useToastStore } from '@/stores/toastStore';

/**
 * Issue 页（T4.8 UI）。
 *
 * 钉住：列表渲染（标签/指派/评论数）与详情打开、创建/关闭/编辑/指派
 * 各命令的参数形状、以及未登录时的引导。
 */
vi.mock('@/lib/ipc', () => ({
  repoIssueList: vi.fn(),
  repoIssueGet: vi.fn(),
  repoIssueBody: vi.fn(),
  repoIssueCommentsList: vi.fn(),
  repoIssueCommentCreate: vi.fn(),
  repoIssueCreate: vi.fn(),
  repoIssueEdit: vi.fn(),
  repoIssueStateSet: vi.fn(),
  repoIssueAssignees: vi.fn(),
  repoIssueAssigneesSet: vi.fn(),
}));

const listMock = vi.mocked(repoIssueList);
const getMock = vi.mocked(repoIssueGet);
const bodyMock = vi.mocked(repoIssueBody);
const commentsListMock = vi.mocked(repoIssueCommentsList);
const commentCreateMock = vi.mocked(repoIssueCommentCreate);
const createMock = vi.mocked(repoIssueCreate);
const editMock = vi.mocked(repoIssueEdit);
const stateSetMock = vi.mocked(repoIssueStateSet);
const assigneesMock = vi.mocked(repoIssueAssignees);
const assigneesSetMock = vi.mocked(repoIssueAssigneesSet);

function summary(number: number, title: string): IssueSummary {
  return {
    number,
    title,
    state: 'open',
    author: 'octocat',
    labels: ['bug'],
    assignees: ['hubot'],
    comments: 2,
    createdAt: '2026-10-01T00:00:00Z',
  };
}

function detail(number: number): IssueDetail {
  return {
    number,
    title: `Issue ${number}`,
    state: 'open',
    author: 'octocat',
    labels: ['bug'],
    assignees: [],
    comments: 0,
    bodyHtml: '<p>描述</p>',
  };
}

function renderPage(): void {
  render(<GitHubIssuesPage />);
}

async function openDetail(): Promise<void> {
  renderPage();
  const input = screen.getByLabelText('仓库（owner/repo，如 octocat/Hello-World）');
  fireEvent.change(input, { target: { value: 'octocat/x' } });
  fireEvent.submit(screen.getByTestId('issues-repo-go'));
  await waitFor(() => expect(listMock).toHaveBeenCalled());
  fireEvent.click(screen.getByTestId('issues-item-1'));
  await waitFor(() => expect(getMock).toHaveBeenCalled());
}

beforeEach(() => {
  vi.clearAllMocks();
  useToastStore.setState(initialToastState);
  listMock.mockResolvedValue({ items: [summary(1, 'Crash on open')], nextPage: null });
  getMock.mockResolvedValue(detail(1));
  bodyMock.mockResolvedValue('描述正文');
  commentsListMock.mockResolvedValue([]);
  commentCreateMock.mockResolvedValue({ id: 9, author: 'me', body: 'pong' });
  createMock.mockResolvedValue(detail(2));
  editMock.mockResolvedValue(detail(1));
  stateSetMock.mockResolvedValue({ ...detail(1), state: 'closed' });
  assigneesMock.mockResolvedValue([{ login: 'hubot' }, { login: 'octocat' }]);
  assigneesSetMock.mockResolvedValue({ ...detail(1), assignees: ['hubot'] });
});

afterEach(() => {
  useToastStore.setState(initialToastState);
});

describe('GitHubIssuesPage — 列表', () => {
  it('输入仓库后加载列表并展示标签、指派与评论数', async () => {
    renderPage();
    const input = screen.getByLabelText('仓库（owner/repo，如 octocat/Hello-World）');
    fireEvent.change(input, { target: { value: 'octocat/x' } });
    fireEvent.submit(screen.getByTestId('issues-repo-go'));

    await waitFor(() => expect(screen.getByTestId('issues-item-1')).toBeInTheDocument());
    expect(screen.getByText('Crash on open')).toBeInTheDocument();
    expect(screen.getByText('bug')).toBeInTheDocument();
    expect(screen.getByText(/hubot/)).toBeInTheDocument();
    expect(listMock).toHaveBeenCalledWith({
      host: 'github.com',
      owner: 'octocat',
      repo: 'x',
      stateFilter: 'open',
      page: 1,
    });
  });

  it('未登录时给出登录引导', async () => {
    listMock.mockRejectedValue({ code: 'AUTH_REQUIRED' });
    renderPage();
    const input = screen.getByLabelText('仓库（owner/repo，如 octocat/Hello-World）');
    fireEvent.change(input, { target: { value: 'octocat/x' } });
    fireEvent.submit(screen.getByTestId('issues-repo-go'));

    await waitFor(() => expect(screen.getByText('还没有登录账号')).toBeInTheDocument());
  });

  it('切换到已关闭标签会带新状态过滤重新加载', async () => {
    renderPage();
    const input = screen.getByLabelText('仓库（owner/repo，如 octocat/Hello-World）');
    fireEvent.change(input, { target: { value: 'octocat/x' } });
    fireEvent.submit(screen.getByTestId('issues-repo-go'));
    await waitFor(() => expect(screen.getByTestId('issues-tab-closed')).toBeInTheDocument());

    fireEvent.click(screen.getByTestId('issues-tab-closed'));
    await waitFor(() =>
      expect(listMock).toHaveBeenCalledWith(expect.objectContaining({ stateFilter: 'closed' })),
    );
  });
});

describe('GitHubIssuesPage — 详情与动作', () => {
  it('发表评论走评论命令', async () => {
    await openDetail();
    fireEvent.change(screen.getByTestId('issue-comment-input'), { target: { value: 'pong' } });
    fireEvent.click(screen.getByTestId('issue-comment-post'));

    await waitFor(() =>
      expect(commentCreateMock).toHaveBeenCalledWith('github.com', 'octocat', 'x', 1, 'pong'),
    );
    expect(await screen.findByText('pong')).toBeInTheDocument();
  });

  it('关闭 Issue 带 open:false，并通知列表刷新', async () => {
    await openDetail();
    fireEvent.click(screen.getByTestId('issue-state-toggle'));

    await waitFor(() =>
      expect(stateSetMock).toHaveBeenCalledWith({
        host: 'github.com',
        owner: 'octocat',
        repo: 'x',
        number: 1,
        open: false,
      }),
    );
    await waitFor(() => expect(listMock.mock.calls.length).toBeGreaterThanOrEqual(2));
  });

  it('编辑标题保存走编辑命令', async () => {
    await openDetail();
    fireEvent.click(screen.getByTestId('issue-edit-open'));
    fireEvent.change(screen.getByTestId('issue-edit-title'), { target: { value: '改名' } });
    fireEvent.click(screen.getByTestId('issue-edit-save'));

    await waitFor(() =>
      expect(editMock).toHaveBeenCalledWith({
        host: 'github.com',
        owner: 'octocat',
        repo: 'x',
        number: 1,
        title: '改名',
      }),
    );
  });

  it('勾选指派人后保存走指派命令', async () => {
    await openDetail();
    fireEvent.click(screen.getByTestId('issue-assign-open'));
    await waitFor(() => expect(assigneesMock).toHaveBeenCalled());
    fireEvent.click(screen.getByTestId('issue-assign-hubot'));
    fireEvent.click(screen.getByTestId('issue-assign-save'));

    await waitFor(() =>
      expect(assigneesSetMock).toHaveBeenCalledWith({
        host: 'github.com',
        owner: 'octocat',
        repo: 'x',
        number: 1,
        assignees: ['hubot'],
      }),
    );
  });

  it('创建 Issue 后刷新列表', async () => {
    renderPage();
    const input = screen.getByLabelText('仓库（owner/repo，如 octocat/Hello-World）');
    fireEvent.change(input, { target: { value: 'octocat/x' } });
    fireEvent.submit(screen.getByTestId('issues-repo-go'));
    await waitFor(() => expect(screen.getByTestId('issues-create-open')).toBeInTheDocument());

    fireEvent.click(screen.getByTestId('issues-create-open'));
    fireEvent.change(screen.getByTestId('issue-create-title'), { target: { value: '新问题' } });
    fireEvent.click(screen.getByTestId('issue-create-submit'));

    await waitFor(() =>
      expect(createMock).toHaveBeenCalledWith({
        host: 'github.com',
        owner: 'octocat',
        repo: 'x',
        title: '新问题',
      }),
    );
    await waitFor(() => expect(listMock.mock.calls.length).toBeGreaterThanOrEqual(2));
  });
});
