import { beforeEach, describe, expect, it, vi } from 'vitest';

import { gitBranchCompare, gitBranchList } from '@/lib/ipc';
import type { Branch } from '@/lib/ipc';
import { loadSyncStatus, remoteOf } from '@/features/sync/syncStatus';

vi.mock('@/lib/ipc', () => ({
  gitBranchList: vi.fn(),
  gitBranchCompare: vi.fn(),
}));

const listMock = vi.mocked(gitBranchList);
const compareMock = vi.mocked(gitBranchCompare);

/** 一个"当前分支"的默认形状；用例只覆写自己关心的字段。 */
function branch(overrides: Partial<Branch> = {}): Branch {
  return {
    name: 'main',
    isRemote: false,
    isHead: true,
    target: 'abc',
    upstream: 'origin/main',
    ahead: null,
    behind: null,
    upstreamGone: false,
    ...overrides,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  compareMock.mockResolvedValue({ ahead: 0, behind: 0, onlyInA: [] });
});

describe('remoteOf', () => {
  it('只切第一个斜杠（分支名本身可以带斜杠）', () => {
    expect(remoteOf('origin/main')).toBe('origin');
    expect(remoteOf('origin/feature/xyz')).toBe('origin');
    expect(remoteOf('upstream/main')).toBe('upstream');
  });

  it('没有上游时返回 null，不拼一个假远端名', () => {
    expect(remoteOf(null)).toBe(null);
  });
});

describe('loadSyncStatus', () => {
  it('列表已给出 ahead/behind 时不再跑一次比较', async () => {
    listMock.mockResolvedValue([branch({ ahead: 2, behind: 1 })]);

    const status = await loadSyncStatus(7);

    expect(status).toEqual({
      branch: 'main',
      upstream: 'origin/main',
      remote: 'origin',
      ahead: 2,
      behind: 1,
    });
    expect(compareMock).not.toHaveBeenCalled();
  });

  it('列表缺 ahead/behind 时用 branch_compare 补齐（读引擎不为此跑 rev-list）', async () => {
    listMock.mockResolvedValue([branch()]);
    compareMock.mockResolvedValue({ ahead: 3, behind: 4, onlyInA: [] });

    const status = await loadSyncStatus(7);

    expect(compareMock).toHaveBeenCalledWith(7, 'main', 'origin/main');
    expect(status.ahead).toBe(3);
    expect(status.behind).toBe(4);
  });

  it('没有当前分支（空仓库 / detached）时不比较', async () => {
    listMock.mockResolvedValue([branch({ isHead: false })]);

    const status = await loadSyncStatus(7);

    expect(status).toEqual({ branch: null, upstream: null, remote: null, ahead: 0, behind: 0 });
    expect(compareMock).not.toHaveBeenCalled();
  });

  it('没有配置上游时不比较', async () => {
    listMock.mockResolvedValue([branch({ upstream: null })]);

    const status = await loadSyncStatus(7);

    expect(status).toEqual({ branch: 'main', upstream: null, remote: null, ahead: 0, behind: 0 });
    expect(compareMock).not.toHaveBeenCalled();
  });

  it('上游已被删除（gone）时按"没有上游"处理，而不是拿一个不存在的引用去比较', async () => {
    listMock.mockResolvedValue([branch({ upstreamGone: true })]);

    const status = await loadSyncStatus(7);

    expect(status.upstream).toBe(null);
    expect(compareMock).not.toHaveBeenCalled();
  });
});
