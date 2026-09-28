import { renderHook, waitFor } from '@testing-library/react';
import { QueryClientProvider } from '@tanstack/react-query';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ReactNode } from 'react';

import type { GraphRow, HistoryPage } from '@/lib/ipc/history';
import { logKey } from '@/lib/queryKeys';
import { createTestQueryClient } from '@/test/queryClient';

import {
  DEFAULT_PAGE_SIZE,
  EMPTY_FILTERS,
  MAX_PAGE_SIZE,
  buildGraphModel,
  clampPageSize,
  filtersSignature,
  useGraphQuery,
} from '@/features/history/useGraphQuery';
import { useGraphPagingStore } from '@/features/history/graphPagingStore';

// Mock IPC 层（useGraphQuery 内部调 gitLogPage）
vi.mock('@/lib/ipc/history', () => ({
  gitLogPage: vi.fn(),
}));
vi.mock('@/lib/ipc/client', () => ({
  isTauriRuntime: () => false,
  invokeCommand: vi.fn(),
}));

import { gitLogPage } from '@/lib/ipc/history';
const gitLogPageMock = vi.mocked(gitLogPage);

// ---------------------------------------------------------------- 夹具

function makeRow(oid: string, row: number, lane: number, colorIndex = 0): GraphRow {
  return { oid, row, lane, colorIndex, isMerge: false, hidden: false, collapsed: [] };
}

function makePage(rows: GraphRow[], nextCursor: number | null): HistoryPage {
  return {
    commits: rows.map((r) => ({
      oid: r.oid,
      parents: [],
      author: { name: 'A', email: 'a@b.c', time: 1000 },
      committer: { name: 'A', email: 'a@b.c', time: 1000 },
      refs: [],
      signature: 'unsigned' as const,
      subject: `msg ${r.oid}`,
      body: null,
    })),
    layout: { rows, edges: [], laneCount: Math.max(1, ...rows.map((r) => r.lane + 1)) },
    nextCursor,
  };
}

// ---------------------------------------------------------------- clampPageSize

describe('clampPageSize', () => {
  it('正常值原样返回（向下取整）', () => {
    expect(clampPageSize(100)).toBe(100);
    expect(clampPageSize(99.7)).toBe(99);
  });

  it('超过上限时夹到 MAX_PAGE_SIZE', () => {
    expect(clampPageSize(1000)).toBe(MAX_PAGE_SIZE);
  });

  it('低于 1 时夹到 1', () => {
    expect(clampPageSize(0)).toBe(1);
    expect(clampPageSize(-5)).toBe(1);
  });

  it('非有限值回退到默认', () => {
    expect(clampPageSize(NaN)).toBe(DEFAULT_PAGE_SIZE);
    expect(clampPageSize(Infinity)).toBe(DEFAULT_PAGE_SIZE);
  });
});

// ---------------------------------------------------------------- filtersSignature

describe('filtersSignature', () => {
  it('空筛选产生稳定签名', () => {
    const sig = filtersSignature(EMPTY_FILTERS, 200);
    expect(sig).toContain('pageSize=200');
    expect(sig).toContain('paths=');
  });

  it('不同 pageSize 产生不同签名', () => {
    const a = filtersSignature(EMPTY_FILTERS, 100);
    const b = filtersSignature(EMPTY_FILTERS, 200);
    expect(a).not.toBe(b);
  });

  it('paths 排序后签名一致', () => {
    const a = filtersSignature({ paths: ['b.ts', 'a.ts'] }, 200);
    const b = filtersSignature({ paths: ['a.ts', 'b.ts'] }, 200);
    expect(a).toBe(b);
  });

  it('包含有值的筛选字段', () => {
    const sig = filtersSignature({ revision: 'main', author: 'alice' }, 200);
    expect(sig).toContain('revision="main"');
    expect(sig).toContain('author="alice"');
  });

  it('未设置的字段不出现在签名里', () => {
    const sig = filtersSignature({}, 200);
    expect(sig).not.toContain('revision');
    expect(sig).not.toContain('author');
  });
});

// ---------------------------------------------------------------- buildGraphModel

describe('buildGraphModel', () => {
  it('空输入返回 EMPTY_GRAPH_MODEL', () => {
    const model = buildGraphModel([]);
    expect(model.rowCount).toBe(0);
    expect(model.loadedPages).toBe(0);
    expect(model.rows).toEqual([]);
  });

  it('null/undefined 页被跳过', () => {
    const model = buildGraphModel([null, undefined]);
    expect(model.loadedPages).toBe(0);
  });

  it('单页数据正确合并', () => {
    const page = makePage([makeRow('a', 0, 0), makeRow('b', 1, 1)], 200);
    const model = buildGraphModel([page]);
    expect(model.loadedPages).toBe(1);
    expect(model.rowCount).toBe(2);
    expect(model.lastRow).toBe(1);
    expect(model.nextCursor).toBe(200);
    expect(model.rows).toHaveLength(2);
    expect(model.commits).toHaveLength(2);
  });

  it('多页合并且行按行号升序', () => {
    const page1 = makePage([makeRow('a', 0, 0), makeRow('b', 1, 0)], 2);
    const page2 = makePage([makeRow('c', 2, 1), makeRow('d', 3, 0)], null);
    const model = buildGraphModel([page1, page2]);
    expect(model.loadedPages).toBe(2);
    expect(model.rowCount).toBe(4);
    expect(model.nextCursor).toBeNull();
    const rowNumbers = model.rows.map((r) => r.row);
    expect(rowNumbers).toEqual([0, 1, 2, 3]);
  });

  it('重叠 oid 去重（以先到页为准）', () => {
    const page1 = makePage([makeRow('a', 0, 0, 1)], 1);
    const page2 = makePage([makeRow('a', 0, 0, 5), makeRow('b', 1, 0, 2)], null);
    const model = buildGraphModel([page1, page2]);
    // 'a' 应该保留 page1 的 colorIndex=1
    expect(model.index.get('a')!.colorIndex).toBe(1);
    expect(model.rows).toHaveLength(2);
  });

  it('laneCount 取各页最大值', () => {
    const page1 = makePage([makeRow('a', 0, 0)], 1);
    const page2: HistoryPage = {
      commits: [],
      layout: { rows: [makeRow('b', 1, 2)], edges: [], laneCount: 5 },
      nextCursor: null,
    };
    const model = buildGraphModel([page1, page2]);
    expect(model.laneCount).toBe(5);
  });

  it('commitByOid 映射正确', () => {
    const page = makePage([makeRow('x', 0, 0)], null);
    const model = buildGraphModel([page]);
    expect(model.commitByOid.get('x')?.subject).toBe('msg x');
  });
});

// ---------------------------------------------------------------- queryKey 形状

describe('useGraphQuery — queryKey 形状', () => {
  let queryClient: ReturnType<typeof createTestQueryClient>;

  beforeEach(() => {
    queryClient = createTestQueryClient();
    useGraphPagingStore.getState().resetAll();
    gitLogPageMock.mockReset();
  });

  afterEach(() => {
    queryClient.clear();
  });

  function wrapper({ children }: { children: ReactNode }) {
    return <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>;
  }

  it('首页 queryKey = logKey(repoId, 0, signature)', async () => {
    const page = makePage([makeRow('a', 0, 0)], null);
    gitLogPageMock.mockResolvedValue(page);

    const { result } = renderHook(() => useGraphQuery(7), { wrapper });
    await waitFor(() => {
      expect(result.current.isPending).toBe(false);
    });

    const expectedSig = filtersSignature(EMPTY_FILTERS, DEFAULT_PAGE_SIZE);
    const expectedKey = logKey(7, 0, expectedSig);
    const cached = queryClient.getQueryData(expectedKey);
    expect(cached).toBeDefined();
  });

  it('repoId 非有限数时查询被禁用', () => {
    gitLogPageMock.mockResolvedValue(makePage([], null));
    const { result } = renderHook(() => useGraphQuery(NaN), { wrapper });
    // TanStack Query v5: 禁用查询的 status 仍为 'pending'，但 fetchStatus 为 'idle'
    expect(result.current.isFetching).toBe(false);
    expect(gitLogPageMock).not.toHaveBeenCalled();
  });
});

// ---------------------------------------------------------------- 续页游标传递

describe('useGraphQuery — 续页', () => {
  let queryClient: ReturnType<typeof createTestQueryClient>;

  beforeEach(() => {
    queryClient = createTestQueryClient();
    useGraphPagingStore.getState().resetAll();
    gitLogPageMock.mockReset();
  });

  afterEach(() => {
    queryClient.clear();
    useGraphPagingStore.getState().resetAll();
  });

  function wrapper({ children }: { children: ReactNode }) {
    return <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>;
  }

  it('loadMore 追加游标并触发第二页请求', async () => {
    const page1 = makePage([makeRow('a', 0, 0)], 200);
    const page2 = makePage([makeRow('b', 1, 0)], null);
    gitLogPageMock.mockResolvedValueOnce(page1).mockResolvedValueOnce(page2);

    const { result } = renderHook(() => useGraphQuery(1), { wrapper });
    await waitFor(() => {
      expect(result.current.isPending).toBe(false);
    });

    expect(result.current.hasNextPage).toBe(true);
    const appended = result.current.loadMore();
    expect(appended).toBe(true);

    await waitFor(() => {
      expect(result.current.model.loadedPages).toBe(2);
    });
    expect(gitLogPageMock).toHaveBeenCalledTimes(2);
    // 第二次调用应带 cursor=200
    expect(gitLogPageMock.mock.calls[1]![1]).toMatchObject({ cursor: 200 });
  });

  it('已到末页时 loadMore 返回 false', async () => {
    const page = makePage([makeRow('a', 0, 0)], null);
    gitLogPageMock.mockResolvedValue(page);

    const { result } = renderHook(() => useGraphQuery(1), { wrapper });
    await waitFor(() => {
      expect(result.current.isPending).toBe(false);
    });

    expect(result.current.hasNextPage).toBe(false);
    expect(result.current.loadMore()).toBe(false);
  });
});
