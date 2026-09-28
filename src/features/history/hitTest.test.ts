import { describe, expect, it } from 'vitest';

import type { GraphRow } from '@/lib/ipc/history';

import { graphMetrics, laneCenterX, rowCenterY } from '@/features/history/graphGeometry';
import {
  buildHitGrid,
  hitTest,
  hitTestNode,
  hitTestRow,
  isRowFullyVisible,
  pointerToContent,
  rowViewportTop,
} from '@/features/history/hitTest';

// ---------------------------------------------------------------- 夹具

function makeRow(oid: string, row: number, lane: number, isMerge = false): GraphRow {
  return { oid, row, lane, colorIndex: lane % 8, isMerge, hidden: false, collapsed: [] };
}

const ROWS: GraphRow[] = [
  makeRow('aaa', 0, 0),
  makeRow('bbb', 1, 1),
  makeRow('ccc', 2, 0),
  makeRow('ddd', 3, 2, true),
];

// ---------------------------------------------------------------- pointerToContent

describe('pointerToContent', () => {
  it('scale=1 无滚动时内容坐标等于客户端偏移', () => {
    const rect = { left: 100, top: 50 };
    const point = pointerToContent(0, rect, 150, 80);
    expect(point.x).toBe(50);
    expect(point.y).toBe(30);
  });

  it('有滚动时 y 加上 scrollY', () => {
    const rect = { left: 0, top: 0 };
    const point = pointerToContent(200, rect, 10, 20);
    expect(point.x).toBe(10);
    expect(point.y).toBe(220);
  });

  it('负偏移（rect.left > clientX）时 x 为负', () => {
    const rect = { left: 300, top: 100 };
    const point = pointerToContent(0, rect, 250, 80);
    expect(point.x).toBe(-50);
    expect(point.y).toBe(-20);
  });

  it('0.5x 缩放下不乘 scale（度量已内含缩放）', () => {
    // pointerToContent 不做缩放换算——graphMetrics 已把 scale 乘进了 rowHeight/laneWidth
    const rect = { left: 10, top: 10 };
    const point = pointerToContent(50, rect, 30, 40);
    expect(point.x).toBe(20);
    expect(point.y).toBe(80); // 40-10+50
  });

  it('3x 缩放下同样不乘 scale', () => {
    const rect = { left: 0, top: 0 };
    const point = pointerToContent(100, rect, 50, 25);
    expect(point.x).toBe(50);
    expect(point.y).toBe(125);
  });
});

// ---------------------------------------------------------------- buildHitGrid

describe('buildHitGrid', () => {
  it('网格的 columns = max(1, ceil(laneCount))', () => {
    const m = graphMetrics(1);
    const grid = buildHitGrid(ROWS, m, 3, 4);
    expect(grid.columns).toBe(3);
  });

  it('laneCount=0 时至少 1 列', () => {
    const m = graphMetrics(1);
    const grid = buildHitGrid([], m, 0, 0);
    expect(grid.columns).toBe(1);
    expect(grid.rows).toBe(1);
  });

  it('cellWidth = laneWidth, cellHeight = rowHeight', () => {
    const m = graphMetrics(2);
    const grid = buildHitGrid(ROWS, m, 3, 4);
    expect(grid.cellWidth).toBe(m.laneWidth);
    expect(grid.cellHeight).toBe(m.rowHeight);
  });

  it('每个节点被登记到至少一个格子', () => {
    const m = graphMetrics(1);
    const grid = buildHitGrid(ROWS, m, 3, 4);
    expect(grid.cells.size).toBeGreaterThan(0);
  });
});

// ---------------------------------------------------------------- hitTestNode

describe('hitTestNode', () => {
  it('精确命中节点中心时返回该节点', () => {
    const m = graphMetrics(1);
    const grid = buildHitGrid(ROWS, m, 3, 4);
    const center = { x: laneCenterX(m, 0), y: rowCenterY(m, 0) };
    const result = hitTestNode(grid, center);
    expect(result).not.toBeNull();
    expect(result!.oid).toBe('aaa');
    expect(result!.kind).toBe('node');
  });

  it('命中第二行第一泳道节点', () => {
    const m = graphMetrics(1);
    const grid = buildHitGrid(ROWS, m, 3, 4);
    const center = { x: laneCenterX(m, 1), y: rowCenterY(m, 1) };
    const result = hitTestNode(grid, center);
    expect(result).not.toBeNull();
    expect(result!.oid).toBe('bbb');
  });

  it('偏离节点时返回 null', () => {
    const m = graphMetrics(1);
    const grid = buildHitGrid(ROWS, m, 3, 4);
    // 行中心但泳道完全偏开
    const miss = { x: laneCenterX(m, 0), y: rowCenterY(m, 1) };
    const result = hitTestNode(grid, miss);
    // row 1 的节点在 lane 1，lane 0 的行 1 没有节点
    expect(result).toBeNull();
  });

  it('超出网格范围时返回 null', () => {
    const m = graphMetrics(1);
    const grid = buildHitGrid(ROWS, m, 3, 4);
    const result = hitTestNode(grid, { x: -100, y: -100 });
    expect(result).toBeNull();
  });

  it('0.5x 缩放下命中正确（度量已内含缩放）', () => {
    const m = graphMetrics(0.5);
    const grid = buildHitGrid(ROWS, m, 3, 4);
    const center = { x: laneCenterX(m, 0), y: rowCenterY(m, 0) };
    const result = hitTestNode(grid, center);
    expect(result).not.toBeNull();
    expect(result!.oid).toBe('aaa');
  });

  it('3x 缩放下命中正确', () => {
    const m = graphMetrics(3);
    const grid = buildHitGrid(ROWS, m, 3, 4);
    // lane 1, row 1 的节点 'bbb'（lane 2 在 3x 下超出网格列数）
    const center = { x: laneCenterX(m, 1), y: rowCenterY(m, 1) };
    const result = hitTestNode(grid, center);
    expect(result).not.toBeNull();
    expect(result!.oid).toBe('bbb');
  });
});

// ---------------------------------------------------------------- hitTestRow

describe('hitTestRow', () => {
  it('行内空白处返回 kind=row', () => {
    const m = graphMetrics(1);
    // 行 0 的 lane 2 没有节点
    const point = { x: laneCenterX(m, 2), y: rowCenterY(m, 0) };
    const result = hitTestRow(m, point, 4);
    expect(result).not.toBeNull();
    expect(result!.kind).toBe('row');
    expect(result!.row).toBe(0);
    expect(result!.oid).toBeNull();
  });

  it('超出 rowCount 时返回 null', () => {
    const m = graphMetrics(1);
    const point = { x: laneCenterX(m, 0), y: rowCenterY(m, 10) };
    expect(hitTestRow(m, point, 4)).toBeNull();
  });

  it('负 y 时返回 null', () => {
    const m = graphMetrics(1);
    expect(hitTestRow(m, { x: 20, y: -5 }, 4)).toBeNull();
  });
});

// ---------------------------------------------------------------- hitTest（完整流程）

describe('hitTest', () => {
  it('客户端坐标命中节点（含滚动与 rect 偏移）', () => {
    const m = graphMetrics(1);
    const grid = buildHitGrid(ROWS, m, 3, 4);
    // 节点 'bbb' 在 row=1, lane=1
    const contentX = laneCenterX(m, 1);
    const contentY = rowCenterY(m, 1);
    const scrollY = 0;
    const rect = { left: 200, top: 100 };
    // 反算客户端坐标: clientX = contentX + rect.left, clientY = contentY + rect.top - scrollY
    const clientX = contentX + rect.left;
    const clientY = contentY + rect.top - scrollY;
    const result = hitTest(grid, m, 4, scrollY, rect, clientX, clientY);
    expect(result).not.toBeNull();
    expect(result!.oid).toBe('bbb');
    expect(result!.kind).toBe('node');
  });

  it('带滚动偏移时正确换算', () => {
    const m = graphMetrics(1);
    const grid = buildHitGrid(ROWS, m, 3, 4);
    const scrollY = 28; // 滚动了一行
    const rect = { left: 0, top: 0 };
    // row=2 的节点 ccc 的内容 y = rowCenterY(m,2) = 2*28+14 = 70
    // 视口 y = 70 - 28 = 42 → clientY = 42
    const clientX = laneCenterX(m, 0);
    const clientY = rowCenterY(m, 2) - scrollY;
    const result = hitTest(grid, m, 4, scrollY, rect, clientX, clientY);
    expect(result).not.toBeNull();
    expect(result!.oid).toBe('ccc');
  });

  it('未命中节点时退化为行', () => {
    const m = graphMetrics(1);
    const grid = buildHitGrid(ROWS, m, 3, 4);
    const rect = { left: 0, top: 0 };
    // 行 0 的 lane 2 位置（无节点）
    const clientX = laneCenterX(m, 2);
    const clientY = rowCenterY(m, 0);
    const result = hitTest(grid, m, 4, 0, rect, clientX, clientY);
    expect(result).not.toBeNull();
    expect(result!.kind).toBe('row');
    expect(result!.row).toBe(0);
  });
});

// ---------------------------------------------------------------- rowViewportTop / isRowFullyVisible

describe('rowViewportTop', () => {
  it('返回行顶边在视口中的 y', () => {
    const m = graphMetrics(1);
    expect(rowViewportTop(m, 0, 0)).toBe(0);
    expect(rowViewportTop(m, 2, 28)).toBe(2 * 28 - 28);
  });
});

describe('isRowFullyVisible', () => {
  const m = graphMetrics(1);

  it('行在视口内时返回 true', () => {
    expect(isRowFullyVisible(m, 0, 0, 280)).toBe(true);
    expect(isRowFullyVisible(m, 5, 0, 280)).toBe(true);
  });

  it('行超出视口底部时返回 false', () => {
    // 行 10: top = 280, bottom = 308 > 280 (viewportHeight)
    expect(isRowFullyVisible(m, 10, 0, 280)).toBe(false);
  });

  it('行在视口上方时返回 false', () => {
    expect(isRowFullyVisible(m, 0, 100, 280)).toBe(false);
  });
});
