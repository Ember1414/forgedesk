import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import {
  NO_MODIFIERS,
  initialGraphSelectionState,
  oidRange,
  orderByRow,
  useGraphSelectionStore,
} from '@/features/history/graphSelectionStore';

/**
 * graphSelectionStore 测试。
 *
 * store 是模块级单例，用例之间必须复位。
 */
beforeEach(() => {
  useGraphSelectionStore.setState(initialGraphSelectionState);
});

afterEach(() => {
  useGraphSelectionStore.setState(initialGraphSelectionState);
});

const ORDER = ['oid-a', 'oid-b', 'oid-c', 'oid-d', 'oid-e'];

// ---------------------------------------------------------------- orderByRow

describe('orderByRow', () => {
  it('按行序归一化选中集', () => {
    expect(orderByRow(['oid-c', 'oid-a', 'oid-e'], ORDER)).toEqual(['oid-a', 'oid-c', 'oid-e']);
  });

  it('order 为空时原样返回', () => {
    expect(orderByRow(['x', 'y'], [])).toEqual(['x', 'y']);
  });

  it('不在 order 里的 oid 排在最后', () => {
    const result = orderByRow(['oid-z', 'oid-a'], ORDER);
    expect(result[0]).toBe('oid-a');
    expect(result[1]).toBe('oid-z');
  });
});

// ---------------------------------------------------------------- oidRange

describe('oidRange', () => {
  it('正序区间含两端', () => {
    expect(oidRange('oid-b', 'oid-d', ORDER)).toEqual(['oid-b', 'oid-c', 'oid-d']);
  });

  it('逆序区间同样含两端', () => {
    expect(oidRange('oid-d', 'oid-b', ORDER)).toEqual(['oid-b', 'oid-c', 'oid-d']);
  });

  it('单 oid 区间返回自身', () => {
    expect(oidRange('oid-c', 'oid-c', ORDER)).toEqual(['oid-c']);
  });

  it('锚点不在 order 里时返回 null', () => {
    expect(oidRange('missing', 'oid-b', ORDER)).toBeNull();
  });

  it('目标不在 order 里时返回 null', () => {
    expect(oidRange('oid-a', 'missing', ORDER)).toBeNull();
  });
});

// ---------------------------------------------------------------- select（单选）

describe('select — 单选', () => {
  it('无修饰键时选中单个 oid', () => {
    useGraphSelectionStore.getState().select('oid-b', NO_MODIFIERS, ORDER);
    const state = useGraphSelectionStore.getState();
    expect(state.selectedOids).toEqual(['oid-b']);
    expect(state.anchorOid).toBe('oid-b');
    expect(state.detailOid).toBe('oid-b');
  });

  it('连续单选会替换前一个', () => {
    const store = useGraphSelectionStore.getState();
    store.select('oid-a', NO_MODIFIERS, ORDER);
    store.select('oid-c', NO_MODIFIERS, ORDER);
    expect(useGraphSelectionStore.getState().selectedOids).toEqual(['oid-c']);
  });
});

// ---------------------------------------------------------------- select（Ctrl 多选）

describe('select — Ctrl 多选', () => {
  const additive = { additive: true, range: false };

  it('Ctrl 点击追加到选中集', () => {
    const store = useGraphSelectionStore.getState();
    store.select('oid-a', NO_MODIFIERS, ORDER);
    store.select('oid-c', additive, ORDER);
    const state = useGraphSelectionStore.getState();
    expect(state.selectedOids).toEqual(['oid-a', 'oid-c']);
    expect(state.anchorOid).toBe('oid-c');
  });

  it('Ctrl 点击已选中的 oid 则取消选择', () => {
    const store = useGraphSelectionStore.getState();
    store.select('oid-a', NO_MODIFIERS, ORDER);
    store.select('oid-b', additive, ORDER);
    store.select('oid-a', additive, ORDER);
    expect(useGraphSelectionStore.getState().selectedOids).toEqual(['oid-b']);
  });

  it('多选结果按行序归一化', () => {
    const store = useGraphSelectionStore.getState();
    store.select('oid-d', NO_MODIFIERS, ORDER);
    store.select('oid-a', additive, ORDER);
    expect(useGraphSelectionStore.getState().selectedOids).toEqual(['oid-a', 'oid-d']);
  });
});

// ---------------------------------------------------------------- select（Shift 区间选）

describe('select — Shift 区间选', () => {
  const range = { additive: false, range: true };

  it('Shift 从锚点到目标选中整个区间', () => {
    const store = useGraphSelectionStore.getState();
    store.select('oid-b', NO_MODIFIERS, ORDER); // 设锚点
    store.select('oid-d', range, ORDER);
    expect(useGraphSelectionStore.getState().selectedOids).toEqual(['oid-b', 'oid-c', 'oid-d']);
  });

  it('区间选替换（不追加）', () => {
    const store = useGraphSelectionStore.getState();
    store.select('oid-a', NO_MODIFIERS, ORDER);
    store.select('oid-c', range, ORDER);
    // 原来的 oid-a 被替换
    expect(useGraphSelectionStore.getState().selectedOids).toEqual(['oid-a', 'oid-b', 'oid-c']);
  });

  it('无锚点时 Shift 退化为单选', () => {
    // anchorOid 初始为 null
    useGraphSelectionStore.getState().select('oid-c', range, ORDER);
    const state = useGraphSelectionStore.getState();
    expect(state.selectedOids).toEqual(['oid-c']);
    expect(state.anchorOid).toBe('oid-c');
  });

  it('锚点不在 order 里时退化为单选', () => {
    useGraphSelectionStore.setState({ anchorOid: 'missing-oid' });
    useGraphSelectionStore.getState().select('oid-b', range, ORDER);
    expect(useGraphSelectionStore.getState().selectedOids).toEqual(['oid-b']);
  });

  it('detailOid 更新为区间末端', () => {
    const store = useGraphSelectionStore.getState();
    store.select('oid-a', NO_MODIFIERS, ORDER);
    store.select('oid-c', range, ORDER);
    expect(useGraphSelectionStore.getState().detailOid).toBe('oid-c');
  });
});

// ---------------------------------------------------------------- selectMany

describe('selectMany', () => {
  it('全选按行序排列', () => {
    useGraphSelectionStore.getState().selectMany(['oid-e', 'oid-c', 'oid-a'], ORDER);
    expect(useGraphSelectionStore.getState().selectedOids).toEqual(['oid-a', 'oid-c', 'oid-e']);
  });

  it('不改变 anchorOid', () => {
    useGraphSelectionStore.getState().selectMany(ORDER, ORDER);
    expect(useGraphSelectionStore.getState().anchorOid).toBeNull();
  });
});

// ---------------------------------------------------------------- clearSelection

describe('clearSelection', () => {
  it('清空选中集与锚点', () => {
    useGraphSelectionStore.getState().select('oid-a', NO_MODIFIERS, ORDER);
    useGraphSelectionStore.getState().clearSelection();
    const state = useGraphSelectionStore.getState();
    expect(state.selectedOids).toEqual([]);
    expect(state.anchorOid).toBeNull();
  });
});

// ---------------------------------------------------------------- compareBase

describe('比较基准', () => {
  it('setCompareBase 设置与清除', () => {
    useGraphSelectionStore.getState().setCompareBase('oid-b');
    expect(useGraphSelectionStore.getState().compareBaseOid).toBe('oid-b');
    useGraphSelectionStore.getState().setCompareBase(null);
    expect(useGraphSelectionStore.getState().compareBaseOid).toBeNull();
  });

  it('比较基准不影响选中集', () => {
    useGraphSelectionStore.getState().select('oid-a', NO_MODIFIERS, ORDER);
    useGraphSelectionStore.getState().setCompareBase('oid-c');
    expect(useGraphSelectionStore.getState().selectedOids).toEqual(['oid-a']);
  });
});

// ---------------------------------------------------------------- 视图状态

describe('视图状态', () => {
  it('setScale 夹到 [0.5, 3]', () => {
    useGraphSelectionStore.getState().setScale(5);
    expect(useGraphSelectionStore.getState().scale).toBe(3);
    useGraphSelectionStore.getState().setScale(0.1);
    expect(useGraphSelectionStore.getState().scale).toBe(0.5);
  });

  it('zoomBy 累乘并夹取', () => {
    useGraphSelectionStore.getState().zoomBy(2);
    expect(useGraphSelectionStore.getState().scale).toBe(2);
    useGraphSelectionStore.getState().zoomBy(2);
    expect(useGraphSelectionStore.getState().scale).toBe(3); // 夹到 MAX_SCALE
  });

  it('toggleMinimap 切换开关', () => {
    expect(useGraphSelectionStore.getState().minimapOpen).toBe(false);
    useGraphSelectionStore.getState().toggleMinimap();
    expect(useGraphSelectionStore.getState().minimapOpen).toBe(true);
  });

  it('resetView 复位缩放、模式、迷你地图（不动选中集）', () => {
    const store = useGraphSelectionStore.getState();
    store.setScale(2.5);
    store.setViewMode('list');
    store.toggleMinimap();
    store.select('oid-a', NO_MODIFIERS, ORDER);
    store.resetView();
    const state = useGraphSelectionStore.getState();
    expect(state.scale).toBe(1);
    expect(state.viewMode).toBe('graph');
    expect(state.minimapOpen).toBe(false);
    expect(state.selectedOids).toEqual(['oid-a']); // 选中集不动
  });

  it('setHoverOid 相同值不触发新状态', () => {
    useGraphSelectionStore.getState().setHoverOid('oid-a');
    const before = useGraphSelectionStore.getState();
    useGraphSelectionStore.getState().setHoverOid('oid-a');
    const after = useGraphSelectionStore.getState();
    // Zustand 的 set 在值相同时不会创建新引用（通过 Object.is 比较）
    expect(after.hoverOid).toBe(before.hoverOid);
  });
});
