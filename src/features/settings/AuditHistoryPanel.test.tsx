import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { QueryClientProvider } from '@tanstack/react-query';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { AuditHistoryPanel } from '@/features/settings/AuditHistoryPanel';
import { auditExport, auditList, auditPrune, pickSavePath, repoRecentList } from '@/lib/ipc';
import type { AuditEntry, AuditPage } from '@/lib/ipc';
import { initialSettingsState, useSettingsStore } from '@/stores/settingsStore';
import { useToastStore } from '@/stores/toastStore';
import { createTestQueryClient } from '@/test/queryClient';

/**
 * 操作历史面板的测试重点（T1.11 验收）：
 *
 *   - 表格要同时说清"什么时候、做了什么、成没成、花了多久、关联哪个快照"；
 *   - **`running` 不能被显示成成功**：它意味着应用崩在了写操作中间，
 *     混进成功记录里等于这个信号不存在；
 *   - 失败的行要能看到 git 的原话（已脱敏）；
 *   - 导出与清理要**真的调用后端**，并把结果（条数、路径、策略）告诉用户。
 */
vi.mock('@/lib/ipc', () => ({
  auditList: vi.fn(),
  auditExport: vi.fn(),
  auditPrune: vi.fn(),
  repoRecentList: vi.fn(),
  // T7.6：导出前先弹保存对话框，路径由它给出
  pickSavePath: vi.fn(),
}));

const auditListMock = vi.mocked(auditList);
const auditExportMock = vi.mocked(auditExport);
const auditPruneMock = vi.mocked(auditPrune);
const repoRecentListMock = vi.mocked(repoRecentList);
const pickSavePathMock = vi.mocked(pickSavePath);

function entry(overrides: Partial<AuditEntry> = {}): AuditEntry {
  return {
    id: 1,
    repoId: 7,
    opType: 'commit',
    argsJson: '{"subject":"add parser","files":2}',
    startedAtMs: 1_700_000_000_000,
    endedAtMs: 1_700_000_000_250,
    durationMs: 250,
    exitCode: 0,
    result: 'ok',
    stderrSummary: null,
    snapshotId: 12,
    reversible: true,
    ...overrides,
  };
}

function page(entries: readonly AuditEntry[], total = entries.length): AuditPage {
  return { total, entries };
}

function renderPanel() {
  const client = createTestQueryClient();
  return render(
    <QueryClientProvider client={client}>
      <AuditHistoryPanel />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  useSettingsStore.setState(initialSettingsState);
  useToastStore.setState({ toasts: [] });
  repoRecentListMock.mockResolvedValue([]);
  // 默认"用户选了保存位置"；取消的用例自己覆盖成 null
  pickSavePathMock.mockResolvedValue('D:\\reports\\audit.csv');
});

afterEach(() => {
  useSettingsStore.setState(initialSettingsState);
});

describe('AuditHistoryPanel', () => {
  it('把每条记录的时间、操作、结果、耗时与快照都列出来', async () => {
    auditListMock.mockResolvedValue(page([entry()]));

    renderPanel();

    expect(await screen.findByText('提交')).toBeInTheDocument();
    expect(screen.getByText('成功')).toBeInTheDocument();
    expect(screen.getByText('250 毫秒')).toBeInTheDocument();
    expect(screen.getByText('#12')).toBeInTheDocument();
    // 参数摘要要与记录一起显示：它是"这次到底做了什么"的唯一线索
    expect(screen.getByText(/"files":2/)).toBeInTheDocument();
  });

  it('没有收尾的记录显示为未收尾（崩溃特征），不会被当成成功', async () => {
    // `result` 由后端派生（`endedAt` 为空 → `running`）：界面如实显示它，
    // 而不是自己再推导一遍——两处推导迟早会出现"界面说成功、库里没结束"的分叉
    auditListMock.mockResolvedValue(
      page([
        entry({
          id: 9,
          result: 'running',
          endedAtMs: null,
          exitCode: null,
          durationMs: null,
          snapshotId: null,
        }),
      ]),
    );

    renderPanel();

    expect(await screen.findByText('未收尾')).toBeInTheDocument();
    expect(screen.queryByText('成功')).not.toBeInTheDocument();
  });

  it('失败的行把 git 的原话一起显示', async () => {
    auditListMock.mockResolvedValue(
      page([
        entry({
          id: 3,
          opType: 'snapshot_restore',
          result: 'failed',
          exitCode: 1,
          stderrSummary: 'error: Your local changes would be overwritten',
          reversible: false,
        }),
      ]),
    );

    renderPanel();

    expect(await screen.findByText('失败')).toBeInTheDocument();
    expect(screen.getByText(/Your local changes would be overwritten/)).toBeInTheDocument();
    expect(screen.getByText('回滚快照')).toBeInTheDocument();
  });

  it('切换操作类型会带着筛选条件重新查询', async () => {
    auditListMock.mockResolvedValue(page([entry()]));

    renderPanel();
    await screen.findByText('提交');

    fireEvent.click(screen.getByRole('combobox', { name: '操作类型' }));
    fireEvent.click(await screen.findByRole('option', { name: '暂存' }));

    await waitFor(() => {
      expect(auditListMock).toHaveBeenLastCalledWith(
        { repoId: null, opType: 'stage' },
        expect.any(Number),
        0,
      );
    });
  });

  it('导出先让用户选位置，再把选定的路径交给后端', async () => {
    auditListMock.mockResolvedValue(page([entry()]));
    pickSavePathMock.mockResolvedValue('D:\\reports\\audit.csv');
    auditExportMock.mockResolvedValue({
      path: 'D:\\reports\\audit.csv',
      rows: 1,
      format: 'csv',
    });

    renderPanel();
    await screen.findByText('提交');
    fireEvent.click(screen.getByRole('button', { name: /导出 CSV/ }));

    await waitFor(() => {
      expect(auditExportMock).toHaveBeenCalledWith(
        { repoId: null, opType: null },
        'csv',
        'D:\\reports\\audit.csv',
      );
    });
    // 建议文件名要带扩展名，且与本次格式一致（后端会拒绝不符的组合）
    expect(pickSavePathMock.mock.calls[0]?.[1]).toMatch(/^forgedesk-audit-\d{8}-\d{6}\.csv$/);
    expect(await screen.findByText(/D:\\reports\\audit\.csv/)).toBeInTheDocument();
  });

  it('保存对话框被取消时不写任何文件', async () => {
    auditListMock.mockResolvedValue(page([entry()]));
    pickSavePathMock.mockResolvedValue(null);

    renderPanel();
    await screen.findByText('提交');
    fireEvent.click(screen.getByRole('button', { name: /导出 CSV/ }));

    await waitFor(() => {
      expect(pickSavePathMock).toHaveBeenCalledTimes(1);
    });
    // 取消后**不能**偷偷回退到临时目录：那会让用户以为文件在自己选的地方
    expect(auditExportMock).not.toHaveBeenCalled();
  });

  it('清理旧记录后报告条数与所用策略，并重新拉取列表', async () => {
    auditListMock.mockResolvedValue(page([entry()]));
    auditPruneMock.mockResolvedValue({ removed: 42, retentionDays: 90, retentionRows: 10_000 });

    renderPanel();
    await screen.findByText('提交');
    const callsBefore = auditListMock.mock.calls.length;

    fireEvent.click(screen.getByRole('button', { name: /清理旧记录/ }));

    await waitFor(() => {
      expect(auditPruneMock).toHaveBeenCalledTimes(1);
    });
    await waitFor(() => {
      expect(auditListMock.mock.calls.length).toBeGreaterThan(callsBefore);
    });
    // 用户要知道"删了多少、按什么策略删的"——否则这个按钮像什么都没做
    const toasts = useToastStore.getState().toasts;
    expect(toasts.some((toast) => toast.title.includes('42'))).toBe(true);
  });

  it('空历史给出说明而不是一张空表', async () => {
    auditListMock.mockResolvedValue(page([], 0));

    renderPanel();

    expect(await screen.findByText(/还没有任何记录/)).toBeInTheDocument();
  });
});
