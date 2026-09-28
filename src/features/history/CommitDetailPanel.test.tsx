import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { QueryClientProvider } from '@tanstack/react-query';
import { beforeEach, afterEach, describe, expect, it, vi } from 'vitest';
import type { ReactNode } from 'react';

import type { CommitDetail } from '@/lib/ipc/commitDetail';
import type { HistoryPage } from '@/lib/ipc/history';
import { commitDetailKey, logKey } from '@/lib/queryKeys';
import { createTestQueryClient } from '@/test/queryClient';

import {
  buildNeighbourIndex,
  commitWebUrl,
  CommitDetailPanel,
} from '@/features/history/CommitDetailPanel';
import {
  initialGraphSelectionState,
  useGraphSelectionStore,
} from '@/features/history/graphSelectionStore';

// 详情与文件行级内容都走 IPC：这里 mock 掉（渲染层行为不依赖真实后端）
vi.mock('@/lib/ipc/commitDetail', () => ({
  gitCommitDetail: vi.fn(),
}));
vi.mock('@/lib/ipc/workspace', () => ({
  workspaceDiff: vi.fn(),
  workspaceDiffPatch: vi.fn(),
}));

import { gitCommitDetail } from '@/lib/ipc/commitDetail';
import { workspaceDiff } from '@/lib/ipc/workspace';
const gitCommitDetailMock = vi.mocked(gitCommitDetail);
const workspaceDiffMock = vi.mocked(workspaceDiff);

// ---------------------------------------------------------------- 夹具

function makeDetail(overrides: Partial<CommitDetail> = {}): CommitDetail {
  return {
    meta: {
      oid: 'a'.repeat(40),
      shortOid: 'aaaaaaa',
      parents: ['b'.repeat(40)],
      author: { name: 'Ada', email: 'ada@example.com', time: 1_700_000_000 },
      committer: { name: 'Ada', email: 'ada@example.com', time: 1_700_000_000 },
      subject: 'feat: detail subject',
      body: 'Detailed body text.',
      signature: 'unsigned',
    },
    refs: ['HEAD -> main'],
    stats: { filesChanged: 2, insertions: 10, deletions: 3 },
    files: [
      {
        path: 'src/lib.rs',
        oldPath: null,
        kind: 'modified',
        binary: false,
        additions: 7,
        deletions: 2,
        truncated: false,
      },
      {
        path: 'assets/logo.bin',
        oldPath: null,
        kind: 'added',
        binary: true,
        additions: 0,
        deletions: 0,
        truncated: false,
      },
    ],
    isMerge: false,
    isHead: true,
    isPushed: false,
    webUrl: null,
    parentIndex: 0,
    ...overrides,
  };
}

const QUERY_KEY = commitDetailKey(1, 'a'.repeat(40), 0);

// ---------------------------------------------------------------- 纯函数

describe('buildNeighbourIndex', () => {
  it('按全局行号给出行序（跨页去重取最小行号）', () => {
    const page = (rows: ReadonlyArray<readonly [string, number]>): HistoryPage => ({
      commits: [],
      layout: {
        rows: rows.map(([oid, row]) => ({
          oid,
          row,
          lane: 0,
          colorIndex: 0,
          isMerge: false,
          hidden: false,
          collapsed: [],
        })),
        edges: [],
        laneCount: 1,
      },
      nextCursor: null,
    });

    const index = buildNeighbourIndex([
      page([
        ['b', 1],
        ['a', 0],
      ]),
      page([
        ['a', 9],
        ['c', 2],
      ]),
    ]);
    expect(index.order).toEqual(['a', 'b', 'c']);
  });

  it('子索引来自各提交的 parents（缓存里谁的父里有它）', () => {
    const page: HistoryPage = {
      commits: [
        {
          oid: 'child',
          parents: ['parent', 'merged'],
          author: { name: 'A', email: 'a@b.c', time: 1 },
          committer: { name: 'A', email: 'a@b.c', time: 1 },
          refs: [],
          signature: 'unsigned',
          subject: 's',
          body: null,
        },
      ],
      layout: {
        rows: [
          {
            oid: 'child',
            row: 0,
            lane: 0,
            colorIndex: 0,
            isMerge: true,
            hidden: false,
            collapsed: [],
          },
          {
            oid: 'parent',
            row: 1,
            lane: 0,
            colorIndex: 0,
            isMerge: false,
            hidden: false,
            collapsed: [],
          },
          {
            oid: 'merged',
            row: 2,
            lane: 1,
            colorIndex: 1,
            isMerge: false,
            hidden: false,
            collapsed: [],
          },
        ],
        edges: [],
        laneCount: 2,
      },
      nextCursor: null,
    };

    const index = buildNeighbourIndex([page]);
    expect(index.childrenOf.get('parent')).toEqual(['child']);
    expect(index.childrenOf.get('merged')).toEqual(['child']);
    expect(index.childrenOf.get('child')).toBeUndefined();
  });

  it('空输入返回空索引', () => {
    const index = buildNeighbourIndex([null, undefined]);
    expect(index.order).toEqual([]);
    expect(index.childrenOf.size).toBe(0);
  });
});

describe('commitWebUrl', () => {
  it('按托管商拼出提交页路径', () => {
    const oid = 'a'.repeat(40);
    expect(commitWebUrl('https://github.com/owner/repo', oid)).toBe(
      `https://github.com/owner/repo/commit/${oid}`,
    );
    expect(commitWebUrl('https://gitlab.com/g/s/repo/', oid)).toBe(
      `https://gitlab.com/g/s/repo/-/commit/${oid}`,
    );
    expect(commitWebUrl('https://bitbucket.org/owner/repo', oid)).toBe(
      `https://bitbucket.org/owner/repo/commits/${oid}`,
    );
  });
});

// ---------------------------------------------------------------- 渲染

describe('CommitDetailPanel 渲染', () => {
  let queryClient: ReturnType<typeof createTestQueryClient>;

  beforeEach(() => {
    queryClient = createTestQueryClient();
    useGraphSelectionStore.setState({ ...initialGraphSelectionState });
    gitCommitDetailMock.mockReset();
    gitCommitDetailMock.mockResolvedValue(makeDetail());
    workspaceDiffMock.mockReset();
    workspaceDiffMock.mockResolvedValue({ files: [], truncatedFiles: 0 });
  });

  afterEach(() => {
    queryClient.clear();
    useGraphSelectionStore.setState({ ...initialGraphSelectionState });
  });

  function wrapper({ children }: { children: ReactNode }) {
    return <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>;
  }

  function renderPanel(fallback = <p>empty-fallback</p>) {
    return render(<CommitDetailPanel repoId={1} fallback={fallback} />, { wrapper });
  }

  it('没有选中提交时显示 fallback', () => {
    renderPanel();
    expect(screen.getByText('empty-fallback')).toBeInTheDocument();
    expect(gitCommitDetailMock).not.toHaveBeenCalled();
  });

  it('选中提交后加载并展示详情（元数据 / 统计 / 文件清单 / 标记）', async () => {
    useGraphSelectionStore.setState({ detailOid: 'a'.repeat(40) });
    renderPanel();

    await waitFor(() => {
      expect(screen.getByText('feat: detail subject')).toBeInTheDocument();
    });
    expect(screen.getByText('Detailed body text.')).toBeInTheDocument();
    // 统计行与文件清单
    expect(screen.getByText('src/lib.rs')).toBeInTheDocument();
    expect(screen.getByText('assets/logo.bin')).toBeInTheDocument();
    // HEAD 徽标 + 未推送标记（文字 + 色彩，不只靠颜色）
    expect(screen.getByText('HEAD')).toBeInTheDocument();
    expect(screen.getByText('未推送')).toBeInTheDocument();
    // refs 胶囊（parseRefs 把 "HEAD -> main" 解析成分支名 main）
    expect(screen.getByText('main')).toBeInTheDocument();
    // 详情查询带上了 oid 与缺省父下标
    expect(gitCommitDetailMock).toHaveBeenCalledWith(1, 'a'.repeat(40), 0);
  });

  it('点击文件行展开行级 diff（between 源），再点收起', async () => {
    useGraphSelectionStore.setState({ detailOid: 'a'.repeat(40) });
    renderPanel();

    const row = await screen.findByText('src/lib.rs');
    const toggle = row.closest('button');
    expect(toggle).not.toBeNull();
    fireEvent.click(toggle!);
    await waitFor(() => {
      expect(workspaceDiffMock).toHaveBeenCalled();
    });
    expect(workspaceDiffMock).toHaveBeenCalledWith(1, {
      target: 'between',
      from: 'b'.repeat(40),
      to: 'a'.repeat(40),
      paths: ['src/lib.rs'],
      contextLines: 3,
      forceFull: false,
    });
    expect(toggle!.getAttribute('aria-expanded')).toBe('true');

    fireEvent.click(toggle!);
    expect(toggle!.getAttribute('aria-expanded')).toBe('false');
  });

  it('合并提交出现双父切换，选择第二父重新查询', async () => {
    const mergeDetail = makeDetail({
      isMerge: true,
      parentIndex: 0,
      meta: {
        ...makeDetail().meta,
        parents: ['b'.repeat(40), 'c'.repeat(40)],
      },
    });
    gitCommitDetailMock.mockResolvedValue(mergeDetail);
    useGraphSelectionStore.setState({ detailOid: 'a'.repeat(40) });
    renderPanel();

    const secondParentToggle = await screen.findByRole('radio', { name: '第二父 (ccccccc)' });
    fireEvent.click(secondParentToggle);

    await waitFor(() => {
      expect(gitCommitDetailMock).toHaveBeenCalledWith(1, 'a'.repeat(40), 1);
    });
  });

  it('钉住按钮把 detailPinned 写进选中 store', async () => {
    useGraphSelectionStore.setState({ detailOid: 'a'.repeat(40) });
    renderPanel();

    const pin = await screen.findByRole('button', { name: '钉住详情' });
    fireEvent.click(pin);
    expect(useGraphSelectionStore.getState().detailPinned).toBe(true);
  });

  it('父提交在已加载历史里时可以定位（解除钉住并选中）', async () => {
    // 预置一页历史缓存：包含父提交 bbb…（详情的定位按钮读同一份缓存）
    const parentOid = 'b'.repeat(40);
    const page: HistoryPage = {
      commits: [
        {
          oid: 'a'.repeat(40),
          parents: [parentOid],
          author: { name: 'A', email: 'a@b.c', time: 1 },
          committer: { name: 'A', email: 'a@b.c', time: 1 },
          refs: [],
          signature: 'unsigned',
          subject: 's',
          body: null,
        },
        {
          oid: parentOid,
          parents: [],
          author: { name: 'A', email: 'a@b.c', time: 1 },
          committer: { name: 'A', email: 'a@b.c', time: 1 },
          refs: [],
          signature: 'unsigned',
          subject: 'parent subject',
          body: null,
        },
      ],
      layout: {
        rows: [
          {
            oid: 'a'.repeat(40),
            row: 0,
            lane: 0,
            colorIndex: 0,
            isMerge: false,
            hidden: false,
            collapsed: [],
          },
          {
            oid: parentOid,
            row: 1,
            lane: 0,
            colorIndex: 0,
            isMerge: false,
            hidden: false,
            collapsed: [],
          },
        ],
        edges: [],
        laneCount: 1,
      },
      nextCursor: null,
    };
    queryClient.setQueryData(logKey(1, 0, 'sig'), page);

    useGraphSelectionStore.setState({ detailOid: 'a'.repeat(40), detailPinned: true });
    renderPanel();

    const parentChip = await screen.findByRole('button', { name: '父1 bbbbbbb' });
    fireEvent.click(parentChip);

    // 定位是主动导航：解除钉住 + 选中父提交 + 详情切到父提交
    expect(useGraphSelectionStore.getState().detailPinned).toBe(false);
    expect(useGraphSelectionStore.getState().selectedOids).toEqual([parentOid]);
    expect(useGraphSelectionStore.getState().detailOid).toBe(parentOid);
  });

  it('详情查询失败时显示错误与详情信息', async () => {
    gitCommitDetailMock.mockRejectedValue(new Error('the commit does not exist'));
    useGraphSelectionStore.setState({ detailOid: 'a'.repeat(40) });
    renderPanel();

    await waitFor(() => {
      expect(screen.getByText('读取提交详情失败')).toBeInTheDocument();
    });
  });

  it('QUERY_KEY 形状：详情按 (repoId, oid, parentIndex) 分条缓存', () => {
    expect(QUERY_KEY).toEqual(['commitDetail', 1, 'a'.repeat(40), 0]);
  });
});
