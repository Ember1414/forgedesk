import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { PullFilesPanel } from '@/features/github/PullFilesPanel';
import {
  repoPullFiles,
  repoPullReviewCommentCreate,
  repoPullReviewCommentReply,
  repoPullReviewCommentsList,
} from '@/lib/ipc';
import type { PullFile, PullReviewComment } from '@/lib/ipc';
import { initialToastState, useToastStore } from '@/stores/toastStore';

/**
 * PR 变更文件面板（T4.7 收尾）。
 *
 * 钉住：diff 行来自后端解析的 hunk（前端不再解析 patch 文本）、
 * 行内评论的锚点（path/side/line）原样传给后端、
 * 已有评论按行号回贴（回复嵌套），以及二进制文件不给评论入口。
 */
vi.mock('@/lib/ipc', () => ({
  repoPullFiles: vi.fn(),
  repoPullReviewCommentsList: vi.fn(),
  repoPullReviewCommentCreate: vi.fn(),
  repoPullReviewCommentReply: vi.fn(),
}));

const filesMock = vi.mocked(repoPullFiles);
const reviewCommentsListMock = vi.mocked(repoPullReviewCommentsList);
const createMock = vi.mocked(repoPullReviewCommentCreate);
const replyMock = vi.mocked(repoPullReviewCommentReply);

const hunks: PullFile['hunks'] = [
  {
    oldStart: 1,
    oldLines: 2,
    newStart: 1,
    newLines: 3,
    header: 'fn main()',
    lines: [
      { kind: 'context', content: 'context line', oldNo: 1, newNo: 1 },
      { kind: 'removed', content: 'old line', oldNo: 2, newNo: null },
      { kind: 'added', content: 'new line', oldNo: null, newNo: 2 },
      { kind: 'added', content: 'another new', oldNo: null, newNo: 3 },
    ],
  },
];

const files: PullFile[] = [
  {
    filename: 'src/a.rs',
    status: 'modified',
    additions: 2,
    deletions: 1,
    hunks,
    patch: '@@ -1,2 +1,3 @@',
  },
  {
    filename: 'assets/logo.png',
    status: 'added',
    additions: 0,
    deletions: 0,
    hunks: [],
  },
];

const inlineComments: PullReviewComment[] = [
  {
    id: 30,
    inReplyTo: null,
    author: 'hubot',
    body: '这里有问题',
    path: 'src/a.rs',
    side: 'RIGHT',
    line: 2,
  },
  { id: 31, inReplyTo: 30, author: 'octocat', body: '已经修了' },
];

function renderPanel(): void {
  render(<PullFilesPanel owner="octocat" repo="x" number={1} />);
}

function expandFile(name: string): void {
  fireEvent.click(screen.getByRole('button', { name: new RegExp(name) }));
}

beforeEach(() => {
  vi.clearAllMocks();
  useToastStore.setState(initialToastState);
  filesMock.mockResolvedValue({ items: files, nextPage: null });
  reviewCommentsListMock.mockResolvedValue(inlineComments);
  createMock.mockResolvedValue({
    id: 77,
    inReplyTo: null,
    author: 'me',
    body: '行内意见',
    path: 'src/a.rs',
    side: 'RIGHT',
    line: 2,
  });
  replyMock.mockResolvedValue({
    id: 32,
    inReplyTo: 30,
    author: 'me',
    body: '新回复',
  });
});

afterEach(() => {
  useToastStore.setState(initialToastState);
});

describe('PullFilesPanel — 变更文件', () => {
  it('展开文件后渲染 diff 行与 hunk 头', async () => {
    renderPanel();
    await waitFor(() => expect(filesMock).toHaveBeenCalled());
    expandFile('src/a.rs');

    expect(screen.getByText('context line')).toBeInTheDocument();
    expect(screen.getByText('old line')).toBeInTheDocument();
    expect(screen.getByText('new line')).toBeInTheDocument();
    expect(screen.getByText(/@@ -1,2 \+1,3 @@/)).toBeInTheDocument();
  });

  it('二进制文件没有 diff，也不给行评论入口', async () => {
    renderPanel();
    await waitFor(() => expect(filesMock).toHaveBeenCalled());
    expandFile('assets/logo.png');

    expect(screen.getByText('diff 不可用：二进制文件或差异过大。')).toBeInTheDocument();
    expect(screen.queryByTestId('pull-line')).not.toBeInTheDocument();
  });

  it('没有变更文件时给出空态', async () => {
    filesMock.mockResolvedValue({ items: [], nextPage: null });
    renderPanel();
    await waitFor(() => expect(filesMock).toHaveBeenCalled());

    expect(screen.getByText('这个 PR 没有变更文件。')).toBeInTheDocument();
  });

  it('加载失败显示错误态', async () => {
    filesMock.mockRejectedValue(new Error('boom'));
    renderPanel();
    await waitFor(() => expect(screen.getByText('变更文件加载失败。')).toBeInTheDocument());
  });
});

describe('PullFilesPanel — 行内评论', () => {
  it('点击新增行发表评论，锚点原样传给后端', async () => {
    renderPanel();
    await waitFor(() => expect(filesMock).toHaveBeenCalled());
    expandFile('src/a.rs');

    // added 行的新文件行号是 2：锚 RIGHT
    fireEvent.click(screen.getByRole('button', { name: '在 src/a.rs 的新文件第 2 行添加评论' }));
    expect(screen.getByText(/src\/a\.rs · 新文件 · 第 2 行/)).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText('写一条行内评论…'), {
      target: { value: '行内意见' },
    });
    fireEvent.click(screen.getByRole('button', { name: '发表' }));

    await waitFor(() =>
      expect(createMock).toHaveBeenCalledWith(
        expect.objectContaining({
          host: 'github.com',
          owner: 'octocat',
          repo: 'x',
          number: 1,
          path: 'src/a.rs',
          side: 'RIGHT',
          line: 2,
          body: '行内意见',
        }),
      ),
    );
    expect(await screen.findByText('行内意见')).toBeInTheDocument();
  });

  it('已有评论按行号回贴，回复嵌套在原评论下', async () => {
    renderPanel();
    await waitFor(() => expect(filesMock).toHaveBeenCalled());
    expandFile('src/a.rs');

    expect(screen.getByText('这里有问题')).toBeInTheDocument();
    fireEvent.click(screen.getByTestId('pull-reply-open-30'));
    fireEvent.change(screen.getByLabelText('回复…'), { target: { value: '新回复' } });
    fireEvent.click(screen.getByRole('button', { name: '发表' }));

    await waitFor(() =>
      expect(replyMock).toHaveBeenCalledWith({
        host: 'github.com',
        owner: 'octocat',
        repo: 'x',
        number: 1,
        commentId: 30,
        body: '新回复',
      }),
    );
    expect(await screen.findByText('新回复')).toBeInTheDocument();
  });

  it('不在此 diff 上的评论单独列出而不丢失', async () => {
    reviewCommentsListMock.mockResolvedValue([
      {
        id: 40,
        inReplyTo: null,
        author: 'hubot',
        body: '旧行的评论',
        path: 'src/a.rs',
        side: 'RIGHT',
        line: 999,
      },
    ]);
    renderPanel();
    await waitFor(() => expect(filesMock).toHaveBeenCalled());
    expandFile('src/a.rs');

    expect(screen.getByText('不在此 diff 上的评论')).toBeInTheDocument();
    expect(screen.getByText('旧行的评论')).toBeInTheDocument();
    expect(screen.queryByTestId('pull-inline-thread')).not.toBeInTheDocument();
  });

  it('取消时清空输入框且不发请求', async () => {
    renderPanel();
    await waitFor(() => expect(filesMock).toHaveBeenCalled());
    expandFile('src/a.rs');

    fireEvent.click(screen.getByRole('button', { name: '在 src/a.rs 的新文件第 1 行添加评论' }));
    fireEvent.change(screen.getByLabelText('写一条行内评论…'), {
      target: { value: '草稿' },
    });
    fireEvent.click(screen.getByRole('button', { name: '取消' }));

    expect(createMock).not.toHaveBeenCalled();
    expect(screen.queryByTestId('pull-inline-composer')).not.toBeInTheDocument();
  });
});
