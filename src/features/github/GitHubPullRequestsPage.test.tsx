import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { GitHubPullRequestsPage } from '@/features/github/GitHubPullRequestsPage';
import {
  repoPullCommentCreate,
  repoPullCommentsList,
  repoPullFiles,
  repoPullGet,
  repoPullList,
  repoPullMerge,
  repoPullReviewCommentsList,
  repoPullReviewSubmit,
  repoPullReviews,
} from '@/lib/ipc';
import type { PullDetail, PullReview, PullSummary } from '@/lib/ipc';
import { initialToastState, useToastStore } from '@/stores/toastStore';

/**
 * PR 列表页（T4.7 UI）。
 *
 * 钉住：合并请求必须带 expectedHeadSha（远端版 PLAN_STALE 的判定输入）、
 * 确认对话框在真正 merge 调用之前、状态过滤切换重新加载、
 * 以及非法 owner/repo 不发请求。
 */
vi.mock('@/lib/ipc', () => ({
  repoPullList: vi.fn(),
  repoPullGet: vi.fn(),
  repoPullReviews: vi.fn(),
  repoPullMerge: vi.fn(),
  repoPullCommentsList: vi.fn(),
  repoPullCommentCreate: vi.fn(),
  repoPullReviewSubmit: vi.fn(),
  repoPullFiles: vi.fn(),
  repoPullReviewCommentsList: vi.fn(),
  repoPullReviewCommentCreate: vi.fn(),
  repoPullReviewCommentReply: vi.fn(),
}));

const listMock = vi.mocked(repoPullList);
const getMock = vi.mocked(repoPullGet);
const reviewsMock = vi.mocked(repoPullReviews);
const mergeMock = vi.mocked(repoPullMerge);
const commentsListMock = vi.mocked(repoPullCommentsList);
const commentCreateMock = vi.mocked(repoPullCommentCreate);
const reviewSubmitMock = vi.mocked(repoPullReviewSubmit);
const filesMock = vi.mocked(repoPullFiles);
const reviewCommentsListMock = vi.mocked(repoPullReviewCommentsList);

function summary(number: number, title: string): PullSummary {
  return {
    number,
    title,
    state: 'open',
    draft: false,
    merged: false,
    author: 'octocat',
    headLabel: 'octocat:feature',
    baseLabel: 'github:main',
    htmlUrl: `https://github.com/octocat/x/pull/${number}`,
  };
}

function detail(number: number): PullDetail {
  return {
    number,
    title: `PR ${number}`,
    state: 'open',
    draft: false,
    merged: false,
    author: 'octocat',
    headLabel: 'octocat:feature',
    baseLabel: 'github:main',
    headSha: 'abc123',
    htmlUrl: `https://github.com/octocat/x/pull/${number}`,
    bodyHtml: '<p>描述</p>',
    changedFiles: 2,
    additions: 10,
    deletions: 4,
    mergeable: true,
    mergeableState: 'clean',
  };
}

const reviews: PullReview[] = [{ id: 1, author: 'hubot', state: 'APPROVED', body: 'lgtm' }];

function renderPage(): void {
  render(<GitHubPullRequestsPage />);
}

beforeEach(() => {
  vi.clearAllMocks();
  useToastStore.setState(initialToastState);
  listMock.mockResolvedValue({ items: [summary(1, 'Add feature')], nextPage: null });
  getMock.mockResolvedValue(detail(1));
  reviewsMock.mockResolvedValue(reviews);
  commentsListMock.mockResolvedValue([
    { id: 9, author: 'hubot', body: 'ping', createdAt: '2026-10-01T00:00:00Z' },
  ]);
  commentCreateMock.mockResolvedValue({
    id: 10,
    author: 'me',
    body: 'pong',
    createdAt: '2026-10-01T01:00:00Z',
  });
  reviewSubmitMock.mockResolvedValue(undefined);
  filesMock.mockResolvedValue({ items: [], nextPage: null });
  reviewCommentsListMock.mockResolvedValue([]);
  mergeMock.mockResolvedValue({
    merged: true,
    sha: 'deadbeef',
    message: 'Pull Request successfully merged',
    branchDeleted: true,
  });
});

afterEach(() => {
  useToastStore.setState(initialToastState);
});

describe('GitHubPullRequestsPage — 列表', () => {
  it('提交 owner/repo 后加载列表并渲染条目', async () => {
    renderPage();

    fireEvent.change(screen.getByTestId('prs-repo-input'), {
      target: { value: 'octocat/x' },
    });
    fireEvent.click(screen.getByTestId('prs-repo-go'));

    await waitFor(() => {
      expect(listMock).toHaveBeenCalledWith({
        host: 'github.com',
        owner: 'octocat',
        repo: 'x',
        stateFilter: 'open',
        page: 1,
      });
    });
    await waitFor(() => {
      expect(screen.getByTestId('prs-item-1')).toBeVisible();
    });
    expect(screen.getByText('Add feature')).toBeVisible();
  });

  it('非法 owner/repo 不发起请求', async () => {
    renderPage();

    fireEvent.change(screen.getByTestId('prs-repo-input'), { target: { value: 'not-a-repo' } });
    fireEvent.click(screen.getByTestId('prs-repo-go'));

    expect(listMock).not.toHaveBeenCalled();
  });

  it('切换状态过滤重新加载', async () => {
    renderPage();

    fireEvent.change(screen.getByTestId('prs-repo-input'), { target: { value: 'octocat/x' } });
    fireEvent.click(screen.getByTestId('prs-repo-go'));
    await waitFor(() => {
      expect(screen.getByTestId('prs-tab-closed')).toBeVisible();
    });
    fireEvent.click(screen.getByTestId('prs-tab-closed'));

    await waitFor(() => {
      expect(listMock).toHaveBeenCalledWith({
        host: 'github.com',
        owner: 'octocat',
        repo: 'x',
        stateFilter: 'closed',
        page: 1,
      });
    });
  });

  it('AUTH_REQUIRED 显示登录引导错误态', async () => {
    listMock.mockRejectedValue({ code: 'AUTH_REQUIRED', message: 'no account' });
    renderPage();

    fireEvent.change(screen.getByTestId('prs-repo-input'), { target: { value: 'octocat/x' } });
    fireEvent.click(screen.getByTestId('prs-repo-go'));

    await waitFor(() => {
      expect(screen.getByText('还没有登录账号')).toBeVisible();
    });
  });
});

describe('GitHubPullRequestsPage — 评论与 review', () => {
  it('详情加载评论列表，发表后追加并清空输入', async () => {
    renderPage();

    fireEvent.change(screen.getByTestId('prs-repo-input'), { target: { value: 'octocat/x' } });
    fireEvent.click(screen.getByTestId('prs-repo-go'));
    await waitFor(() => {
      expect(screen.getByTestId('prs-item-1')).toBeVisible();
    });
    fireEvent.click(screen.getByTestId('prs-item-1'));
    await waitFor(() => {
      expect(screen.getByTestId('pull-comment-list')).toBeVisible();
    });
    expect(screen.getByText('ping')).toBeVisible();

    fireEvent.change(screen.getByTestId('pull-comment-input'), { target: { value: 'pong' } });
    fireEvent.click(screen.getByTestId('pull-comment-post'));
    await waitFor(() => {
      expect(commentCreateMock).toHaveBeenCalledWith('github.com', 'octocat', 'x', 1, 'pong');
    });
    await waitFor(() => {
      expect(screen.getByText('pong')).toBeVisible();
    });
    expect(screen.getByTestId('pull-comment-input')).toHaveValue('');
  });

  it('空评论不能发表', async () => {
    renderPage();

    fireEvent.change(screen.getByTestId('prs-repo-input'), { target: { value: 'octocat/x' } });
    fireEvent.click(screen.getByTestId('prs-repo-go'));
    await waitFor(() => {
      expect(screen.getByTestId('prs-item-1')).toBeVisible();
    });
    fireEvent.click(screen.getByTestId('prs-item-1'));
    await waitFor(() => {
      expect(screen.getByTestId('pull-comment-post')).toBeVisible();
    });
    expect(screen.getByTestId('pull-comment-post')).toBeDisabled();
    expect(commentCreateMock).not.toHaveBeenCalled();
  });

  it('review 提交携带事件与正文，成功后刷新 reviews', async () => {
    renderPage();

    fireEvent.change(screen.getByTestId('prs-repo-input'), { target: { value: 'octocat/x' } });
    fireEvent.click(screen.getByTestId('prs-repo-go'));
    await waitFor(() => {
      expect(screen.getByTestId('prs-item-1')).toBeVisible();
    });
    fireEvent.click(screen.getByTestId('prs-item-1'));
    await waitFor(() => {
      expect(screen.getByTestId('pull-review-controls')).toBeVisible();
    });
    fireEvent.click(screen.getByTestId('pull-review-APPROVE'));
    fireEvent.change(screen.getByTestId('pull-review-body'), { target: { value: 'ship it' } });
    fireEvent.click(screen.getByTestId('pull-review-submit'));

    await waitFor(() => {
      expect(reviewSubmitMock).toHaveBeenCalledWith(
        'github.com',
        'octocat',
        'x',
        1,
        'APPROVE',
        'ship it',
      );
    });
    await waitFor(() => {
      expect(reviewsMock).toHaveBeenCalledTimes(2);
    });
  });
});

describe('GitHubPullRequestsPage — 详情与合并', () => {
  it('点击条目打开详情：加载详情与 reviews 并展示合并条件', async () => {
    renderPage();

    fireEvent.change(screen.getByTestId('prs-repo-input'), { target: { value: 'octocat/x' } });
    fireEvent.click(screen.getByTestId('prs-repo-go'));
    await waitFor(() => {
      expect(screen.getByTestId('prs-item-1')).toBeVisible();
    });
    fireEvent.click(screen.getByTestId('prs-item-1'));

    await waitFor(() => {
      expect(getMock).toHaveBeenCalledWith('github.com', 'octocat', 'x', 1);
      expect(reviewsMock).toHaveBeenCalledWith('github.com', 'octocat', 'x', 1);
    });
    await waitFor(() => {
      expect(screen.getByTestId('pull-detail')).toBeVisible();
    });
    expect(screen.getByTestId('pull-mergeable')).toHaveTextContent('可以合并');
    expect(screen.getByText('hubot：已批准')).toBeVisible();
  });

  it('合并走确认对话框，请求带策略、head 预检与删分支', async () => {
    renderPage();

    fireEvent.change(screen.getByTestId('prs-repo-input'), { target: { value: 'octocat/x' } });
    fireEvent.click(screen.getByTestId('prs-repo-go'));
    await waitFor(() => {
      expect(screen.getByTestId('prs-item-1')).toBeVisible();
    });
    fireEvent.click(screen.getByTestId('prs-item-1'));
    await waitFor(() => {
      expect(screen.getByTestId('pull-merge-controls')).toBeVisible();
    });

    // 选 rebase + 勾选删分支
    fireEvent.click(screen.getByTestId('pull-strategy-rebase'));
    fireEvent.click(screen.getByTestId('pull-delete-branch'));
    fireEvent.click(screen.getByTestId('pull-merge-open'));

    // 确认框出现时还没有真正合并
    expect(screen.getByTestId('pull-merge-confirm')).toBeVisible();
    expect(mergeMock).not.toHaveBeenCalled();

    fireEvent.click(screen.getByTestId('pull-merge-confirm'));
    await waitFor(() => {
      expect(mergeMock).toHaveBeenCalledWith({
        host: 'github.com',
        owner: 'octocat',
        repo: 'x',
        number: 1,
        strategy: 'rebase',
        expectedHeadSha: 'abc123',
        deleteBranch: true,
        headBranch: 'feature',
      });
    });
    await waitFor(() => {
      expect(useToastStore.getState().toasts.length).toBeGreaterThan(0);
    });
  });
});
