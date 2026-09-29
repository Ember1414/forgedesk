/**
 * 性能模式（T2.9）的判定逻辑单测。
 *
 * 纯函数（parse / entryCount / enabled）直接断言；`usePerformanceMode`
 * 的接线（订阅、随缓存更新）依赖组件环境，由 e2e 覆盖。
 */
import { describe, expect, it } from 'vitest';

import type { WorkspaceFileChange, WorkspaceStatus } from '@/lib/ipc/workspace';
import {
  DEFAULT_PERFORMANCE_MODE,
  PERFORMANCE_MODE_ENTRY_THRESHOLD,
  parsePerformanceMode,
  performanceModeEnabled,
  statusEntryCount,
} from '@/lib/performanceMode';

function makeStatus(counts: {
  staged?: number;
  unstaged?: number;
  untracked?: number;
  conflicted?: number;
}): WorkspaceStatus {
  const files = (n: number, prefix: string): WorkspaceFileChange[] =>
    Array.from({ length: n }, (_, index) => ({
      path: `${prefix}-${index}.txt`,
      kind: 'ordinary',
      indexStatus: '.',
      worktreeStatus: 'M',
      isBinary: false,
      isLfs: false,
      isSubmodule: false,
    }));
  return {
    branch: { detached: false, ahead: 0, behind: 0 },
    operation: 'none',
    staged: files(counts.staged ?? 0, 'staged'),
    unstaged: files(counts.unstaged ?? 0, 'unstaged'),
    untracked: files(counts.untracked ?? 0, 'untracked'),
    conflicted: files(counts.conflicted ?? 0, 'conflicted'),
    ignored: [],
    ignoredCount: 0,
  };
}

describe('parsePerformanceMode', () => {
  it('未设置时回落到 auto', () => {
    expect(parsePerformanceMode(undefined)).toBe(DEFAULT_PERFORMANCE_MODE);
    expect(parsePerformanceMode(null)).toBe('auto');
  });

  it('JSON 字符串解析为对应档位', () => {
    expect(parsePerformanceMode(JSON.stringify('on'))).toBe('on');
    expect(parsePerformanceMode(JSON.stringify('off'))).toBe('off');
    expect(parsePerformanceMode(JSON.stringify('auto'))).toBe('auto');
  });

  it('坏值与未知取值回落到 auto（存储里可能有手工改坏的数据）', () => {
    expect(parsePerformanceMode('not-json')).toBe('auto');
    expect(parsePerformanceMode(JSON.stringify('turbo'))).toBe('auto');
    expect(parsePerformanceMode(JSON.stringify(42))).toBe('auto');
  });
});

describe('statusEntryCount', () => {
  it('四类条目求和', () => {
    expect(
      statusEntryCount(makeStatus({ staged: 2, unstaged: 3, untracked: 4, conflicted: 1 })),
    ).toBe(10);
  });

  it('无数据时为 0（缓存还没到，不误判成大仓库）', () => {
    expect(statusEntryCount(undefined)).toBe(0);
    expect(statusEntryCount(null)).toBe(0);
  });
});

describe('performanceModeEnabled', () => {
  it('on 恒开、off 恒关', () => {
    expect(performanceModeEnabled('on', 0)).toBe(true);
    expect(performanceModeEnabled('off', 1_000_000)).toBe(false);
  });

  it('auto 按阈值判定：达到生效、不足不生效', () => {
    expect(performanceModeEnabled('auto', PERFORMANCE_MODE_ENTRY_THRESHOLD - 1)).toBe(false);
    expect(performanceModeEnabled('auto', PERFORMANCE_MODE_ENTRY_THRESHOLD)).toBe(true);
  });
});
