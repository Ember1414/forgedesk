import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { QueryClientProvider } from '@tanstack/react-query';
import { MemoryRouter, Route, Routes } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { SnapshotsPage } from '@/features/snapshots/SnapshotsPage';
import {
  snapshotCleanup,
  snapshotCreate,
  snapshotDiff,
  snapshotList,
  snapshotPrune,
  snapshotRestore,
  snapshotRestoreAbandon,
  snapshotRestorePending,
  snapshotUsage,
} from '@/lib/ipc/snapshots';
import type { SnapshotMeta } from '@/lib/ipc/snapshots';
import { createTestQueryClient } from '@/test/queryClient';

/**
 * 快照页（T3.8：内容备份的可见性）。
 *
 * 钉住的都是"说错会伤人"的结论：
 * - 手动打点后必须把"这次没备上什么"挂在页面上（不静默跳过）；
 * - 回滚前必须先取差异摘要，并且摘要要说清未跟踪内容的三分类
 *   （会恢复什么 / 找不回什么 / 不会删什么）；
 * - 回滚后的报告要说清"恢复了几个、校验过没过"，而不是一句"回滚成功"。
 */
vi.mock('@/lib/ipc/snapshots', () => ({
  snapshotList: vi.fn(),
  snapshotDiff: vi.fn(),
  snapshotRestore: vi.fn(),
  snapshotPrune: vi.fn(),
  snapshotCreate: vi.fn(),
  snapshotUsage: vi.fn(),
  snapshotEstimate: vi.fn(),
  snapshotCleanup: vi.fn(),
  snapshotRestorePending: vi.fn(),
  snapshotRestoreAbandon: vi.fn(),
}));

// 页面订阅 repo:changed 做失效：E2E 的事件通道在单元测试里没有意义，直接短路
vi.mock('@/lib/repoChanged', () => ({
  useRepoChangeInvalidation: () => undefined,
}));

const listMock = vi.mocked(snapshotList);
const diffMock = vi.mocked(snapshotDiff);
const restoreMock = vi.mocked(snapshotRestore);
const pruneMock = vi.mocked(snapshotPrune);
const createMock = vi.mocked(snapshotCreate);
const usageMock = vi.mocked(snapshotUsage);
const cleanupMock = vi.mocked(snapshotCleanup);
const pendingMock = vi.mocked(snapshotRestorePending);
const abandonMock = vi.mocked(snapshotRestoreAbandon);

const HEAD = 'a'.repeat(40);

const META: SnapshotMeta = {
  id: 7,
  label: 'pre-commit',
  kind: 'pre-commit',
  headOid: HEAD,
  branch: 'main',
  detached: false,
  createdAtMs: 1_700_000_000_000,
};

function renderPage(): void {
  const queryClient = createTestQueryClient();
  render(
    <QueryClientProvider client={queryClient}>
      <MemoryRouter initialEntries={['/repo/1/snapshots']}>
        <Routes>
          <Route path="/repo/:repoId/snapshots" element={<SnapshotsPage />} />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  listMock.mockResolvedValue([META]);
  usageMock.mockResolvedValue({
    repoId: 1,
    snapshotCount: 1,
    backupBytes: 2048,
    maxSnapshotBytes: 200 * 1024 * 1024,
    maxRepoBytes: 2 * 1024 * 1024 * 1024,
    orphanDirs: [],
  });
  pendingMock.mockResolvedValue(null);
});

describe('快照页', () => {
  it('列出快照并显示内容备份的占用与配额', async () => {
    renderPage();

    expect(await screen.findByText('提交前')).toBeInTheDocument();
    const usage = await screen.findByTestId('snapshot-usage');
    expect(usage).toHaveTextContent('1 个快照');
    expect(usage).toHaveTextContent('2.0 KB');
    expect(usage).toHaveTextContent('上限 2.0 GB');
  });

  it('有孤儿目录时在占用行里点出来', async () => {
    usageMock.mockResolvedValue({
      repoId: 1,
      snapshotCount: 1,
      backupBytes: 0,
      maxSnapshotBytes: 0,
      maxRepoBytes: 0,
      orphanDirs: ['999999', '.tmp-crashed'],
    });

    renderPage();

    const orphans = await screen.findByTestId('snapshot-orphans');
    expect(orphans).toHaveTextContent('2 个孤立目录');
    expect(screen.getByTestId('snapshot-usage')).toHaveTextContent('未设上限');
  });

  it('手动打点后把"未包含什么"挂在页面上，而不是只弹一个 toast', async () => {
    createMock.mockResolvedValue({
      id: 9,
      backupBytes: 0,
      backedUp: 0,
      untrackedTotal: 3,
      skipped: ['big.bin', 'scratch/a.txt', 'scratch/b.txt'],
      warnings: [
        {
          kind: 'untrackedBackupSkipped',
          count: 3,
          bytes: 300 * 1024 * 1024,
          limit: 200 * 1024 * 1024,
          paths: [],
          detail: null,
          removed: [],
          freedBytes: null,
        },
      ],
      pruned: [],
    });

    renderPage();
    await screen.findByText('提交前');
    fireEvent.click(screen.getByTestId('snapshot-create'));

    const warning = await screen.findByTestId('snapshot-warning');
    expect(warning).toHaveTextContent('未包含 3 个未跟踪文件');
    expect(warning).toHaveTextContent('200.0 MB');
    expect(createMock).toHaveBeenCalledWith(1);
  });

  it('清理缓存调用 snapshot_cleanup（孤儿 + 回收一起走）', async () => {
    cleanupMock.mockResolvedValue({
      orphansRemoved: 2,
      reclaimed: [3],
      freedBytes: 1024 * 1024,
      remainingBytes: 0,
    });

    renderPage();
    await screen.findByText('提交前');
    fireEvent.click(screen.getByTestId('snapshot-cleanup'));

    await waitFor(() => {
      expect(cleanupMock).toHaveBeenCalledWith(1);
    });
  });

  it('回滚先取差异摘要，摘要说清未跟踪内容的三分类，确认后展示报告', async () => {
    diffMock.mockResolvedValue({
      headChanged: true,
      indexChanged: false,
      currentHeadOid: 'b'.repeat(40),
      currentIndexTreeOid: null,
      refMissing: false,
      untrackedRestorable: ['scratch.txt'],
      untrackedMissing: ['gone.txt'],
      untrackedExtra: ['new.txt'],
    });
    restoreMock.mockResolvedValue({
      restoredSnapshotId: 7,
      headOid: HEAD,
      indexTreeOid: 'c'.repeat(40),
      preRestoreSnapshotId: 8,
      untrackedPaths: ['scratch.txt'],
      untrackedRestored: 1,
      untrackedFailed: [],
      untrackedExtra: ['new.txt'],
      verified: true,
      outcome: 'completed',
      stages: [
        { stage: 'protection', ok: true, detail: null, durationMs: 1 },
        { stage: 'head', ok: true, detail: null, durationMs: 2 },
        { stage: 'index', ok: true, detail: null, durationMs: 1 },
        { stage: 'untracked', ok: true, detail: null, durationMs: 1 },
        { stage: 'verify', ok: true, detail: null, durationMs: 1 },
      ],
      reportLines: ['head: ok (2 ms)'],
      emergency: null,
    });

    renderPage();
    fireEvent.click(await screen.findByRole('button', { name: '回滚到这里' }));

    // 摘要：会恢复 1 个、找不回 1 个、不会删 1 个
    expect(await screen.findByText(/会写回 1 个未跟踪文件/)).toBeInTheDocument();
    expect(screen.getByText(/找不回来/)).toBeInTheDocument();
    expect(screen.getByText(/不会被删除/)).toBeInTheDocument();
    expect(restoreMock).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole('button', { name: '回滚' }));

    await waitFor(() => {
      expect(restoreMock).toHaveBeenCalledWith(1, 7);
    });
    const report = await screen.findByTestId('snapshot-report');
    expect(report).toHaveTextContent('未跟踪文件已恢复 1 个');
    expect(report).toHaveTextContent('校验通过');
    expect(report).toHaveTextContent('未被删除');
  });

  it('保留策略清理仍然走 snapshot_prune（与清理缓存分开）', async () => {
    pruneMock.mockResolvedValue([3, 4]);

    renderPage();
    await screen.findByText('提交前');
    fireEvent.click(screen.getByRole('button', { name: '清理旧快照' }));

    await waitFor(() => {
      expect(pruneMock).toHaveBeenCalledWith(1);
    });
  });

  it('未完成的回滚会被点出来，"继续"就是再回滚一次（幂等）', async () => {
    pendingMock.mockResolvedValue({ snapshotId: 7, stage: 'index', startedAtMs: 1_700 });
    restoreMock.mockResolvedValue({
      restoredSnapshotId: 7,
      headOid: HEAD,
      indexTreeOid: 'c'.repeat(40),
      preRestoreSnapshotId: 8,
      untrackedPaths: [],
      untrackedRestored: 0,
      untrackedFailed: [],
      untrackedExtra: [],
      verified: true,
      outcome: 'completed',
      stages: [{ stage: 'head', ok: true, detail: null, durationMs: 1 }],
      reportLines: [],
      emergency: null,
    });

    renderPage();

    const banner = await screen.findByTestId('restore-pending');
    expect(banner).toHaveTextContent('检测到上次回滚未完成');
    expect(banner).toHaveTextContent('快照 #7');
    // 说清停在哪一步：用户才知道"继续"会从哪儿接着做
    expect(banner).toHaveTextContent('恢复索引');

    fireEvent.click(screen.getByTestId('restore-pending-continue'));

    await waitFor(() => {
      expect(restoreMock).toHaveBeenCalledWith(1, 7);
    });
  });

  it('"清除提示"走 snapshot_restore_abandon（不回退任何东西）', async () => {
    pendingMock.mockResolvedValue({ snapshotId: 9, stage: null, startedAtMs: null });
    abandonMock.mockResolvedValue(1);

    renderPage();

    const banner = await screen.findByTestId('restore-pending');
    // 认不出阶段时不能装作知道，也不该因此不提示
    expect(banner).toHaveTextContent('未知阶段');

    fireEvent.click(screen.getByTestId('restore-pending-abandon'));

    await waitFor(() => {
      expect(abandonMock).toHaveBeenCalledWith(1);
    });
    expect(restoreMock).not.toHaveBeenCalled();
  });

  it('紧急模式：展示阶段清单与可复制的恢复指引', async () => {
    diffMock.mockResolvedValue({
      headChanged: true,
      indexChanged: false,
      currentHeadOid: 'b'.repeat(40),
      currentIndexTreeOid: null,
      refMissing: false,
      untrackedRestorable: [],
      untrackedMissing: [],
      untrackedExtra: [],
    });
    restoreMock.mockResolvedValue({
      restoredSnapshotId: 7,
      headOid: 'a'.repeat(40),
      indexTreeOid: 'c'.repeat(40),
      preRestoreSnapshotId: 8,
      untrackedPaths: [],
      untrackedRestored: 0,
      untrackedFailed: [],
      untrackedExtra: [],
      verified: false,
      outcome: 'emergency',
      stages: [
        { stage: 'protection', ok: true, detail: null, durationMs: 1 },
        { stage: 'head', ok: false, detail: 'git reset --hard failed: boom', durationMs: 3 },
      ],
      reportLines: ['head: failed - boom'],
      emergency: {
        snapshotId: 7,
        targetSnapshotId: 8,
        backupDir: '/cache/snapshots/1/8',
        commands: ['git reset --hard abc', 'git read-tree def'],
        notes: ['在仓库目录里执行：/tmp/repo'],
      },
    });

    renderPage();
    fireEvent.click(await screen.findByRole('button', { name: '回滚到这里' }));
    fireEvent.click(await screen.findByRole('button', { name: '回滚' }));

    const report = await screen.findByTestId('snapshot-report');
    expect(report).toHaveTextContent('需要手动恢复');

    // 阶段清单：说清"哪一步成了、哪一步没成、为什么"
    const stages = screen.getByTestId('report-stages');
    expect(stages).toHaveTextContent('打回滚前保护点');
    expect(stages).toHaveTextContent('恢复 HEAD 与工作区');
    expect(stages).toHaveTextContent('git reset --hard failed: boom');

    // 指引：命令逐条可复制，且带上下文（说明 + 备份目录）
    const guidance = screen.getByTestId('report-emergency');
    expect(guidance).toHaveTextContent('git reset --hard abc');
    expect(guidance).toHaveTextContent('git read-tree def');
    expect(guidance).toHaveTextContent('/cache/snapshots/1/8');
  });
});
