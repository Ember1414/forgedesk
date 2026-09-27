import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { createRepoChangeInvalidator, queryKeysForChange } from '@/lib/repoChanged';
import { createTestQueryClient } from '@/test/queryClient';

/**
 * `repo:changed` 处理的测试重点：
 *
 * 1. **类别 → 查询的映射**：只改了文件不该去刷历史（大仓库上那是可见的白刷）；
 * 2. **1 秒节流**：`git checkout` 会连发几十个事件，逐个失效会让界面抖成一团；
 *   窗口内的**最后一次**必须被处理（否则会丢掉"最终状态"）；
 * 3. **按路径收敛 diff**：打开着的查看器不该因为别的文件变了而重取。
 */

const THROTTLE_MS = 1_000;

function keysOf(repoId: number, kind: 'workspace' | 'refs' | 'large'): string[] {
  return queryKeysForChange(repoId, kind).map((key) => JSON.stringify(key));
}

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

describe('queryKeysForChange', () => {
  it('工作区变化只失效状态（不动历史与分支）', () => {
    const keys = keysOf(1, 'workspace');

    expect(keys).toContain(JSON.stringify(['status', 1]));
    expect(keys).not.toContain(JSON.stringify(['log', 1]));
    expect(keys).not.toContain(JSON.stringify(['branches', 1]));
  });

  it('引用变化同时失效状态、历史与分支', () => {
    const keys = keysOf(1, 'refs');

    // 状态也必须刷：checkout 之后"已暂存"是相对新的 HEAD 而言的
    expect(keys).toContain(JSON.stringify(['status', 1]));
    expect(keys).toContain(JSON.stringify(['log', 1]));
    expect(keys).toContain(JSON.stringify(['branches', 1]));
    expect(keys).toContain(JSON.stringify(['snapshots', 1]));
  });

  it('大量变化把 diff 也一起失效（此时没有路径可用）', () => {
    expect(keysOf(7, 'large')).toContain(JSON.stringify(['diff', 7]));
    expect(keysOf(7, 'workspace')).not.toContain(JSON.stringify(['diff', 7]));
  });

  it('只影响本仓库', () => {
    expect(keysOf(2, 'refs')).not.toContain(JSON.stringify(['status', 1]));
  });
});

describe('createRepoChangeInvalidator', () => {
  it('窗口外的变化立刻失效', () => {
    const queryClient = createTestQueryClient();
    const invalidate = vi.spyOn(queryClient, 'invalidateQueries');
    const invalidator = createRepoChangeInvalidator(queryClient);

    invalidator.invalidate(1, 'workspace', []);

    expect(invalidate).toHaveBeenCalledWith({ queryKey: ['status', 1] });
  });

  it('1 秒内的连续变化合并成一次，并保留最后一次的类别', () => {
    const queryClient = createTestQueryClient();
    const invalidate = vi.spyOn(queryClient, 'invalidateQueries');
    const invalidator = createRepoChangeInvalidator(queryClient);

    invalidator.invalidate(1, 'workspace', []);
    invalidate.mockClear();

    // 窗口内再来两个事件：都不该立刻触发
    invalidator.invalidate(1, 'workspace', []);
    invalidator.invalidate(1, 'refs', []);
    expect(invalidate).not.toHaveBeenCalled();

    // 窗口结束：只跑一次，且用最后一次的类别（refs）
    vi.advanceTimersByTime(THROTTLE_MS);
    const calledWith = invalidate.mock.calls.map((call) => JSON.stringify(call[0]?.queryKey));
    expect(calledWith).toContain(JSON.stringify(['log', 1]));
    expect(calledWith.filter((key) => key === JSON.stringify(['status', 1]))).toHaveLength(1);
  });

  it('不同仓库各自计时，互不吞掉对方的事件', () => {
    const queryClient = createTestQueryClient();
    const invalidate = vi.spyOn(queryClient, 'invalidateQueries');
    const invalidator = createRepoChangeInvalidator(queryClient);

    invalidator.invalidate(1, 'workspace', []);
    invalidator.invalidate(2, 'workspace', []);

    // 两个仓库都该各自失效一次
    const repoIds = invalidate.mock.calls.map((call) => call[0]?.queryKey?.[1]);
    expect(new Set(repoIds)).toEqual(new Set([1, 2]));
  });

  it('带路径时只失效受影响的 diff，不碰别的文件', () => {
    const queryClient = createTestQueryClient();
    const invalidate = vi.spyOn(queryClient, 'invalidateQueries');
    const invalidator = createRepoChangeInvalidator(queryClient);

    invalidator.invalidate(1, 'workspace', ['src/a.ts']);

    // 前两类调用是键前缀失效；diff 那次带 predicate
    const diffCall = invalidate.mock.calls.find((call) => call[0]?.queryKey?.[0] === 'diff');
    expect(diffCall).toBeDefined();

    const predicate = diffCall?.[0]?.predicate;
    expect(predicate).toBeTypeOf('function');
    // 该文件自己的 diff 查询 → 命中
    expect(predicate?.({ queryKey: ['diff', 1, 'unstaged', 'src/a.ts', 3, false] } as never)).toBe(
      true,
    );
    // 别的文件 → 不命中（打开着的查看器不该白刷）
    expect(predicate?.({ queryKey: ['diff', 1, 'unstaged', 'src/b.ts', 3, false] } as never)).toBe(
      false,
    );
  });

  it('卸载时清掉挂起的尾随调用（不在已卸载的组件上触发请求）', () => {
    const queryClient = createTestQueryClient();
    const invalidate = vi.spyOn(queryClient, 'invalidateQueries');
    const invalidator = createRepoChangeInvalidator(queryClient);

    invalidator.invalidate(1, 'workspace', []);
    invalidate.mockClear();
    invalidator.invalidate(1, 'workspace', []);
    invalidator.dispose();

    vi.advanceTimersByTime(THROTTLE_MS * 2);
    expect(invalidate).not.toHaveBeenCalled();
  });
});
