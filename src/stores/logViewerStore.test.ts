import { afterEach, describe, expect, it } from 'vitest';

import { initialLogViewerState, openLogViewer, useLogViewerStore } from '@/stores/logViewerStore';

/**
 * 日志查看器 store 的测试重点：**打开时携带的高亮锚点**。
 * 它是"从错误提示直达相关日志"这条链路的中间环节——
 * 锚点丢了，用户就只能自己在几百行里找。
 */
afterEach(() => {
  useLogViewerStore.setState(initialLogViewerState);
});

describe('logViewerStore', () => {
  it('默认关闭', () => {
    expect(useLogViewerStore.getState().open).toBe(false);
    expect(useLogViewerStore.getState().nearTimestamp).toBeNull();
  });

  it('打开时记录高亮锚点', () => {
    openLogViewer({ nearTimestamp: 1_787_000_000_000 });

    expect(useLogViewerStore.getState().open).toBe(true);
    expect(useLogViewerStore.getState().nearTimestamp).toBe(1_787_000_000_000);
  });

  it('不带锚点打开时不残留上一次的锚点', () => {
    openLogViewer({ nearTimestamp: 1_787_000_000_000 });
    openLogViewer();

    // 残留会让"这次错误"高亮到"上次错误"的时间附近，误导用户
    expect(useLogViewerStore.getState().nearTimestamp).toBeNull();
  });

  it('关闭只改开关，不清空锚点（关闭动画期间列表仍需保持高亮）', () => {
    openLogViewer({ nearTimestamp: 1_787_000_000_000 });
    useLogViewerStore.getState().closeViewer();

    expect(useLogViewerStore.getState().open).toBe(false);
    expect(useLogViewerStore.getState().nearTimestamp).toBe(1_787_000_000_000);
  });
});
