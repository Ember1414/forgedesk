import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import {
  clampDetailSize,
  DEFAULT_LAYOUT,
  initialLayoutState,
  parseLayout,
  useLayoutStore,
} from '@/stores/layoutStore';

/**
 * layoutStore 测试。
 *
 * 注意：Zustand 的 store 是**模块级单例**，用例之间必须复位，
 * 否则会出现"单独跑通过、一起跑失败"的顺序依赖（典型的假绿来源）。
 */
beforeEach(() => {
  useLayoutStore.setState(initialLayoutState);
});

afterEach(() => {
  useLayoutStore.setState(initialLayoutState);
});

describe('布局解析（parseLayout）', () => {
  it('空值与 undefined 回到默认布局', () => {
    expect(parseLayout(undefined)).toEqual({ layout: { ...DEFAULT_LAYOUT }, error: null });
    expect(parseLayout('')).toEqual({ layout: { ...DEFAULT_LAYOUT }, error: null });
  });

  it('旧版布局 JSON 没有 detailSize 时不算损坏，尺寸跟随默认', () => {
    const legacy = JSON.stringify({ preset: 'review', treeWidth: 320, detailPanel: 'bottom' });
    const { layout, error } = parseLayout(legacy);
    expect(error).toBeNull();
    expect(layout.preset).toBe('review');
    expect(layout.treeWidth).toBe(320);
    expect(layout.detailPanel).toBe('bottom');
    expect(layout.detailSize).toBeNull();
  });

  it('detailSize 在持久化值里被钳制到合法区间', () => {
    const { layout } = parseLayout(JSON.stringify({ detailSize: 9999 }));
    expect(layout.detailSize).toBe(640);
  });

  it('坏 JSON 回退默认并报错', () => {
    const { layout, error } = parseLayout('{oops');
    expect(layout).toEqual({ ...DEFAULT_LAYOUT });
    expect(error).not.toBeNull();
  });
});

describe('详情面板尺寸（clampDetailSize）', () => {
  it('横向（右侧宽度）钳制到 240..640', () => {
    expect(clampDetailSize(100, 'horizontal')).toBe(240);
    expect(clampDetailSize(400, 'horizontal')).toBe(400);
    expect(clampDetailSize(9999, 'horizontal')).toBe(640);
  });

  it('纵向（底部高度）钳制到 96..480', () => {
    expect(clampDetailSize(20, 'vertical')).toBe(96);
    expect(clampDetailSize(300, 'vertical')).toBe(300);
    expect(clampDetailSize(9999, 'vertical')).toBe(480);
  });

  it('null 与非有限数回 null（跟随默认尺寸）', () => {
    expect(clampDetailSize(null, 'horizontal')).toBeNull();
    expect(clampDetailSize(Number.NaN, 'horizontal')).toBeNull();
  });
});

describe('详情面板尺寸动作（setDetailSize）', () => {
  it('写入 store 前按方向钳制', () => {
    useLayoutStore.getState().setDetailSize(9999, 'horizontal');
    expect(useLayoutStore.getState().layout.detailSize).toBe(640);

    useLayoutStore.getState().setDetailSize(20, 'vertical');
    expect(useLayoutStore.getState().layout.detailSize).toBe(96);
  });

  it('恢复默认布局后尺寸回到跟随默认（null）', () => {
    useLayoutStore.getState().setDetailSize(360, 'horizontal');
    expect(useLayoutStore.getState().layout.detailSize).toBe(360);

    useLayoutStore.getState().resetToDefault();
    expect(useLayoutStore.getState().layout.detailSize).toBeNull();
  });
});
