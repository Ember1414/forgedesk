import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { DiagnosticsCard } from '@/features/diagnostics/DiagnosticsCard';
import { initialSettingsState, useSettingsStore } from '@/stores/settingsStore';
import { initialTerminalState, useTerminalStore } from '@/stores/terminalStore';
import { initialToastState, useToastStore } from '@/stores/toastStore';
import { initialUiState, useUiStore } from '@/stores/uiStore';

/**
 * 诊断卡片（T5.6）的组件测试：i18n 渲染、置信度、修复动作的分派
 * （command 直执行 / dangerous 转交确认对话框 / guide 跳转）。
 */
const push = {
  id: 'push-non-fast-forward',
  confidence: 0.95,
  titleKey: 'diag.push-non-fast-forward.title',
  explanationKey: 'diag.push-non-fast-forward.explanation',
  causes: ['diag.push-non-fast-forward.cause1'],
  fixes: [
    {
      id: 'fetch-then-retry',
      labelKey: 'diag.push-non-fast-forward.fix_fetch_then_retry',
      action: { kind: 'command' as const, command: 'git_fetch', args: {} },
    },
    {
      id: 'use-force-with-lease',
      labelKey: 'diag.push-non-fast-forward.fix_use_force_with_lease',
      action: {
        kind: 'dangerous' as const,
        command: 'git_push',
        args: { force_with_lease: true },
      },
    },
  ],
};

const REPORT = {
  primary: push,
  alternatives: [],
  rawSummary: '! [rejected] main -> main (non-fast-forward)',
};

import type * as IpcModule from '@/lib/ipc';

vi.mock('@/lib/ipc', async (importOriginal) => ({
  ...(await importOriginal<typeof IpcModule>()),
  gitFetch: vi.fn().mockResolvedValue({ jobId: 'job-1' }),
  gitPush: vi.fn().mockResolvedValue({ jobId: 'job-2' }),
  gitStashSave: vi.fn().mockResolvedValue({}),
  systemOpenUrl: vi.fn().mockResolvedValue(undefined),
}));

import { gitFetch, gitPush } from '@/lib/ipc';

function renderCard(): void {
  render(
    <DiagnosticsCard
      report={REPORT}
      context={{
        repoId: 7,
        invalidate: () => {},
        onDangerous: () => {},
        onNavigate: () => {},
        onDone: () => {},
      }}
    />,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  useSettingsStore.setState(initialSettingsState);
  useTerminalStore.setState(initialTerminalState);
  useToastStore.setState(initialToastState);
  useUiStore.setState(initialUiState);
});

afterEach(() => {
  useSettingsStore.setState(initialSettingsState);
  useTerminalStore.setState(initialTerminalState);
  useToastStore.setState(initialToastState);
  useUiStore.setState(initialUiState);
});

describe('DiagnosticsCard（诊断卡片）', () => {
  it('渲染标题、解释、原因与置信度', () => {
    renderCard();

    expect(screen.getByText('推送被拒绝：远端有新提交')).toBeInTheDocument();
    expect(screen.getByText('高置信度')).toBeInTheDocument();
    expect(screen.getByText('远端分支上有你本地还没有的提交（别人先推了）。')).toBeInTheDocument();
  });

  it('command 修复动作直接执行并刷新', async () => {
    const invalidate = vi.fn();
    render(
      <DiagnosticsCard
        report={REPORT}
        context={{
          repoId: 7,
          invalidate,
          onDangerous: () => {},
          onNavigate: () => {},
          onDone: () => {},
        }}
      />,
    );

    fireEvent.click(screen.getByRole('button', { name: '先抓取再重试（推荐）' }));

    await waitFor(() => {
      expect(gitFetch).toHaveBeenCalledWith(7, {});
      expect(invalidate).toHaveBeenCalled();
    });
  });

  it('dangerous 修复动作转交确认对话框而不直接执行', async () => {
    const onDangerous = vi.fn();
    render(
      <DiagnosticsCard
        report={REPORT}
        context={{
          repoId: 7,
          invalidate: () => {},
          onDangerous,
          onNavigate: () => {},
          onDone: () => {},
        }}
      />,
    );

    fireEvent.click(screen.getByRole('button', { name: '用 force-with-lease 强推' }));

    await waitFor(() => {
      expect(onDangerous).toHaveBeenCalledWith(push.fixes[1]);
    });
    expect(gitPush).not.toHaveBeenCalled();
  });

  it('白名单外的 command 动作被拒绝执行', async () => {
    const report = {
      ...REPORT,
      primary: {
        ...push,
        fixes: [
          {
            id: 'sneaky',
            labelKey: 'diag.x.fix',
            action: { kind: 'command' as const, command: 'git_reset_execute', args: {} },
          },
        ],
      },
    };
    render(
      <DiagnosticsCard
        report={report}
        context={{
          repoId: 7,
          invalidate: () => {},
          onDangerous: () => {},
          onNavigate: () => {},
          onDone: () => {},
        }}
      />,
    );

    fireEvent.click(screen.getByRole('button', { name: 'diag.x.fix' }));

    // 拒绝执行：没有任何命令被调用，错误进入 toast
    await waitFor(() => {
      expect(useToastStore.getState().toasts.length).toBe(1);
    });
    expect(gitFetch).not.toHaveBeenCalled();
  });
});
