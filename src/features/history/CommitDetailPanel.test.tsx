import { act, fireEvent, render, screen } from '@testing-library/react';
import { QueryClientProvider } from '@tanstack/react-query';
import { beforeEach, afterEach, describe, expect, it } from 'vitest';
import type { ReactNode } from 'react';

import type { GraphRow, HistoryPage } from '@/lib/ipc/history';
import { logKey } from '@/lib/queryKeys';
import { createTestQueryClient } from '@/test/queryClient';

import { CommitDetailPanel, findEntryInPages } from '@/features/history/CommitDetailPanel';
import {
  initialGraphSelectionState,
  useGraphSelectionStore,
} from '@/features/history/graphSelectionStore';

// ---------------------------------------------------------------- 夹具

function makeRow(oid: string, row: number, lane = 0): GraphRow {
  return { oid, row, lane, colorIndex: lane, isMerge: false, hidden: false, collapsed: [] };
}

function makePage(oids: readonly string[], nextCursor: number | null): HistoryPage {
  const rows = oids.map((oid, index) => makeRow(oid, index));
  return {
    commits: oids.map((oid) => ({
      oid,
      parents: [],
      author: { name: 'Ada', email: 'ada@example.com', time: 1_700_000_000 },
      committer: { name: 'Ada', email: 'ada@example.com', time: 1_700_000_000 },
      refs: [],
      signature: 'unsigned' as const,
      subject: `msg ${oid}`,
      body: null,
    })),
    layout: { rows, edges: [], laneCount: 1 },
    nextCursor,
  };
}

const QUERY_KEY = logKey(1, 0, 'sig');

// ---------------------------------------------------------------- findEntryInPages（纯函数）

describe('findEntryInPages', () => {
  it('按 oid 找到提交并返回它所在的行', () => {
    const page = makePage(['a', 'b'], null);
    const entry = findEntryInPages([page], 'b');
    expect(entry).not.toBeNull();
    expect(entry?.commit.oid).toBe('b');
    expect(entry?.row?.row).toBe(1);
  });

  it('找不到 oid 时返回 null（而不是抛错）', () => {
    const page = makePage(['a'], null);
    expect(findEntryInPages([page], 'zzz')).toBeNull();
    expect(findEntryInPages([], 'a')).toBeNull();
    expect(findEntryInPages([null, undefined], 'a')).toBeNull();
  });

  it('提交存在但布局缺行时 row 为 null（分页边界的容错）', () => {
    const page = makePage(['a'], null);
    const rowsOnly = {
      ...page,
      layout: { rows: [] as GraphRow[], edges: [], laneCount: 1 },
    };
    const entry = findEntryInPages([rowsOnly], 'a');
    expect(entry?.commit.oid).toBe('a');
    expect(entry?.row).toBeNull();
  });
});

// ---------------------------------------------------------------- 渲染（快照稳定性回归）

describe('CommitDetailPanel 渲染', () => {
  let queryClient: ReturnType<typeof createTestQueryClient>;

  beforeEach(() => {
    queryClient = createTestQueryClient();
    useGraphSelectionStore.setState({ ...initialGraphSelectionState });
  });

  afterEach(() => {
    queryClient.clear();
    useGraphSelectionStore.setState({ ...initialGraphSelectionState });
  });

  function wrapper({ children }: { children: ReactNode }) {
    return <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>;
  }

  function seedHistory(page: HistoryPage): void {
    queryClient.setQueryData(QUERY_KEY, page);
  }

  /**
   * 死循环回归（T2.2 E2E 抓到的 "Maximum update depth exceeded"）：
   * getSnapshot 曾经每次都返回新建的 `{ commit, row }` 包装对象，
   * useSyncExternalStore 会因此无限强制重渲染。只要"缓存里有提交 + detailOid
   * 指向它"时面板能稳定渲染出内容，就说明快照引用稳定。
   */
  it('选中提交后渲染详情，且不会陷入无限重渲染', () => {
    seedHistory(makePage(['a', 'b'], null));
    useGraphSelectionStore.setState({ detailOid: 'b' });

    render(<CommitDetailPanel repoId={1} fallback={<p>empty-fallback</p>} />, { wrapper });

    expect(screen.getByText('msg b')).toBeInTheDocument();
    expect(screen.queryByText('empty-fallback')).not.toBeInTheDocument();
  });

  it('缓存数据更新后（翻页 / refetch 替换对象）面板跟随新内容', () => {
    seedHistory(makePage(['a'], null));
    useGraphSelectionStore.setState({ detailOid: 'a' });

    const { rerender } = render(<CommitDetailPanel repoId={1} fallback={<p>empty-fallback</p>} />, {
      wrapper,
    });
    expect(screen.getByText('msg a')).toBeInTheDocument();

    // setQueryData 换上一份新的 Page 对象（模拟 refetch / 追加翻页后重算）；
    // 缓存事件会同步唤醒面板的 useSyncExternalStore，包进 act 避免告警
    act(() => {
      seedHistory(makePage(['a', 'b'], null));
    });
    rerender(<CommitDetailPanel repoId={1} fallback={<p>empty-fallback</p>} />);
    expect(screen.getByText('msg a')).toBeInTheDocument();
  });

  it('detailOid 清空后退回 fallback', () => {
    seedHistory(makePage(['a'], null));
    useGraphSelectionStore.setState({ detailOid: 'a' });

    const { rerender } = render(<CommitDetailPanel repoId={1} fallback={<p>empty-fallback</p>} />, {
      wrapper,
    });
    expect(screen.getByText('msg a')).toBeInTheDocument();

    useGraphSelectionStore.setState({ detailOid: null });
    rerender(<CommitDetailPanel repoId={1} fallback={<p>empty-fallback</p>} />);
    expect(screen.getByText('empty-fallback')).toBeInTheDocument();
  });

  it('提交不在缓存里时显示 fallback（不抛错）', () => {
    seedHistory(makePage(['a'], null));
    useGraphSelectionStore.setState({ detailOid: 'missing-oid' });

    render(<CommitDetailPanel repoId={1} fallback={<p>empty-fallback</p>} />, { wrapper });
    expect(screen.getByText('empty-fallback')).toBeInTheDocument();
  });

  it('"设为比较基准"把 oid 写进选中 store', () => {
    seedHistory(makePage(['a'], null));
    useGraphSelectionStore.setState({ detailOid: 'a' });

    render(<CommitDetailPanel repoId={1} fallback={<p>empty-fallback</p>} />, { wrapper });

    fireEvent.click(screen.getByRole('button', { name: '设为比较基准' }));
    expect(useGraphSelectionStore.getState().compareBaseOid).toBe('a');
  });
});
