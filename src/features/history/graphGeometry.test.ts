import { describe, expect, it } from 'vitest';

import type { GraphEdge, GraphRow, HistoryPage } from '@/lib/ipc/history';

import {
  BASE_LANE_WIDTH,
  BASE_MERGE_NODE_WIDTH,
  BASE_NODE_HEIGHT,
  BASE_NODE_WIDTH,
  BASE_PAD_LEFT,
  BASE_ROW_HEIGHT,
  MAX_SCALE,
  MIN_SCALE,
  clampScale,
  edgeGeometry,
  estimateTextWidth,
  graphColumnWidth,
  graphMetrics,
  indexRows,
  laneCenterX,
  layoutCollapsedBadge,
  layoutRefCapsules,
  lastRowOf,
  nodeRadius,
  nodeRect,
  pointInRoundedRect,
  rowAtY,
  rowCenterY,
  rowTop,
  selectionRingRect,
  traceRoundedPath,
  visibleRowRange,
  zoomAround,
  laneAtX,
  contentHeight,
  minimapBuckets,
  overscanRows,
} from '@/features/history/graphGeometry';
import type { RefLabel } from '@/features/history/graphGeometry';

import { buildGraphModel } from '@/features/history/useGraphQuery';

// ---------------------------------------------------------------- 夹具

function makeRow(
  overrides: Partial<GraphRow> & { oid: string; row: number; lane: number },
): GraphRow {
  return {
    colorIndex: overrides.lane % 8,
    isMerge: false,
    hidden: false,
    collapsed: [],
    ...overrides,
  };
}

function makeEdge(
  from: string,
  to: string,
  fromLane: number,
  toLane: number,
  kind: GraphEdge['kind'] = 'straight',
): GraphEdge {
  return { fromOid: from, toOid: to, fromLane, toLane, kind };
}

// ---------------------------------------------------------------- clampScale

describe('clampScale', () => {
  it('正常值原样返回', () => {
    expect(clampScale(1)).toBe(1);
    expect(clampScale(1.5)).toBe(1.5);
  });

  it('低于下限时夹到 MIN_SCALE', () => {
    expect(clampScale(0.1)).toBe(MIN_SCALE);
    expect(clampScale(-2)).toBe(MIN_SCALE);
  });

  it('超过上限时夹到 MAX_SCALE', () => {
    expect(clampScale(5)).toBe(MAX_SCALE);
    expect(clampScale(100)).toBe(MAX_SCALE);
  });

  it('非有限值回退到 1', () => {
    expect(clampScale(NaN)).toBe(1);
    expect(clampScale(Infinity)).toBe(1);
    expect(clampScale(-Infinity)).toBe(1);
  });
});

// ---------------------------------------------------------------- graphMetrics

describe('graphMetrics', () => {
  it('scale=1 时度量等于基准值', () => {
    const m = graphMetrics(1);
    expect(m.rowHeight).toBe(BASE_ROW_HEIGHT);
    expect(m.laneWidth).toBe(BASE_LANE_WIDTH);
    expect(m.padLeft).toBe(BASE_PAD_LEFT);
    expect(m.nodeHeight).toBe(BASE_NODE_HEIGHT);
    expect(m.nodeWidth).toBe(BASE_NODE_WIDTH);
    expect(m.mergeNodeWidth).toBe(BASE_MERGE_NODE_WIDTH);
  });

  it('scale=2 时几何量等比放大', () => {
    const m = graphMetrics(2);
    expect(m.rowHeight).toBe(BASE_ROW_HEIGHT * 2);
    expect(m.laneWidth).toBe(BASE_LANE_WIDTH * 2);
    expect(m.nodeWidth).toBe(BASE_NODE_WIDTH * 2);
  });

  it('线宽不完全等比（strokeScale 被夹到 1.5）', () => {
    const m = graphMetrics(3);
    // strokeScale = min(3, 1.5) = 1.5 → lineWidth = max(1, 1.75*1.5) = 2.625
    expect(m.lineWidth).toBeCloseTo(2.625);
    expect(m.railWidth).toBeCloseTo(1.65);
  });

  it('lineWidth 不小于 1', () => {
    const m = graphMetrics(MIN_SCALE);
    expect(m.lineWidth).toBeGreaterThanOrEqual(1);
  });
});

// ---------------------------------------------------------------- laneCenterX / rowCenterY

describe('坐标换算', () => {
  const m = graphMetrics(1);

  it('lane 0 的中心 = padLeft + laneWidth/2', () => {
    expect(laneCenterX(m, 0)).toBe(BASE_PAD_LEFT + BASE_LANE_WIDTH / 2);
  });

  it('lane 2 的中心 = padLeft + 2*laneWidth + laneWidth/2', () => {
    expect(laneCenterX(m, 2)).toBe(BASE_PAD_LEFT + 2 * BASE_LANE_WIDTH + BASE_LANE_WIDTH / 2);
  });

  it('row 0 顶边为 0，中心为 rowHeight/2', () => {
    expect(rowTop(m, 0)).toBe(0);
    expect(rowCenterY(m, 0)).toBe(BASE_ROW_HEIGHT / 2);
  });

  it('row 3 顶边 = 3*rowHeight', () => {
    expect(rowTop(m, 3)).toBe(3 * BASE_ROW_HEIGHT);
  });
});

// ---------------------------------------------------------------- nodeRect / nodeRadius

describe('节点胶囊几何', () => {
  const m = graphMetrics(1);

  it('普通提交的宽度为 BASE_NODE_WIDTH', () => {
    const row = makeRow({ oid: 'a', row: 0, lane: 0, isMerge: false });
    const rect = nodeRect(m, row);
    expect(rect.w).toBe(BASE_NODE_WIDTH);
    expect(rect.h).toBe(BASE_NODE_HEIGHT);
  });

  it('merge 提交的宽度为 BASE_MERGE_NODE_WIDTH', () => {
    const row = makeRow({ oid: 'b', row: 1, lane: 1, isMerge: true });
    const rect = nodeRect(m, row);
    expect(rect.w).toBe(BASE_MERGE_NODE_WIDTH);
  });

  it('胶囊居中于泳道中心', () => {
    const row = makeRow({ oid: 'c', row: 2, lane: 1 });
    const rect = nodeRect(m, row);
    const cx = rect.x + rect.w / 2;
    expect(cx).toBeCloseTo(laneCenterX(m, 1));
  });

  it('nodeRadius = 高度的一半（完整半圆）', () => {
    const row = makeRow({ oid: 'd', row: 0, lane: 0 });
    const rect = nodeRect(m, row);
    expect(nodeRadius(rect)).toBe(rect.h / 2);
  });

  it('缩放 2x 时胶囊等比放大', () => {
    const m2 = graphMetrics(2);
    const row = makeRow({ oid: 'e', row: 0, lane: 0 });
    const rect = nodeRect(m2, row);
    expect(rect.w).toBe(BASE_NODE_WIDTH * 2);
    expect(rect.h).toBe(BASE_NODE_HEIGHT * 2);
  });
});

// ---------------------------------------------------------------- selectionRingRect

describe('选中环矩形', () => {
  it('比节点外扩一圈', () => {
    const m = graphMetrics(1);
    const row = makeRow({ oid: 'x', row: 0, lane: 0 });
    const rect = nodeRect(m, row);
    const ring = selectionRingRect(m, rect);
    const gap = Math.max(1.5, 2 * m.scale);
    expect(ring.x).toBeCloseTo(rect.x - gap);
    expect(ring.y).toBeCloseTo(rect.y - gap);
    expect(ring.w).toBeCloseTo(rect.w + gap * 2);
    expect(ring.h).toBeCloseTo(rect.h + gap * 2);
  });
});

// ---------------------------------------------------------------- ref 胶囊宽度

describe('layoutRefCapsules', () => {
  const m = graphMetrics(1);
  const measure = (text: string, fontPx: number) => estimateTextWidth(text, fontPx);

  it('单个 ref 的宽度 = 文字宽度 + 2*refPaddingX', () => {
    const refs: RefLabel[] = [{ label: 'main', kind: 'local' }];
    const capsules = layoutRefCapsules(m, refs, measure, 50, 14, 200);
    expect(capsules).toHaveLength(1);
    const textW = estimateTextWidth('main', m.refFont);
    expect(capsules[0]!.rect.w).toBeCloseTo(textW + m.refPaddingX * 2);
  });

  it('多个 ref 依次排列且有间距', () => {
    const refs: RefLabel[] = [
      { label: 'main', kind: 'local' },
      { label: 'v1.0', kind: 'tag' },
    ];
    const capsules = layoutRefCapsules(m, refs, measure, 50, 14, 300);
    expect(capsules).toHaveLength(2);
    const first = capsules[0]!;
    const second = capsules[1]!;
    expect(second.rect.x).toBeCloseTo(first.rect.x + first.rect.w + m.refGap);
  });

  it('超出 maxWidth 时截断', () => {
    const refs: RefLabel[] = [
      { label: 'very-long-branch-name-here', kind: 'local' },
      { label: 'another', kind: 'remote' },
    ];
    const capsules = layoutRefCapsules(m, refs, measure, 0, 14, 20);
    expect(capsules.length).toBeLessThan(2);
  });

  it('radius 不超过 refHeight/2 且为 4*scale（取较小值）', () => {
    const refs: RefLabel[] = [{ label: 'x', kind: 'tag' }];
    const capsules = layoutRefCapsules(m, refs, measure, 0, 14, 200);
    expect(capsules[0]!.radius).toBe(Math.min(m.refHeight / 2, 4 * m.scale));
  });
});

// ---------------------------------------------------------------- 折叠徽标

describe('layoutCollapsedBadge', () => {
  const m = graphMetrics(1);
  const measure = (text: string, fontPx: number) => estimateTextWidth(text, fontPx);
  const format = (n: number) => `+${n}`;

  it('count=0 时返回 null', () => {
    expect(layoutCollapsedBadge(m, 0, measure, 0, 14, 100, format)).toBeNull();
  });

  it('count>0 时返回徽标且 radius = badgeHeight/2', () => {
    const badge = layoutCollapsedBadge(m, 3, measure, 50, 14, 100, format);
    expect(badge).not.toBeNull();
    expect(badge!.label).toBe('+3');
    expect(badge!.radius).toBe(m.badgeHeight / 2);
  });

  it('超出 maxWidth 时返回 null', () => {
    const badge = layoutCollapsedBadge(m, 999, measure, 0, 14, 2, format);
    expect(badge).toBeNull();
  });
});

// ---------------------------------------------------------------- edgeGeometry

describe('edgeGeometry', () => {
  const m = graphMetrics(1);

  it('同泳道 straight 边为单轨竖线', () => {
    const rows = [makeRow({ oid: 'a', row: 0, lane: 0 }), makeRow({ oid: 'b', row: 1, lane: 0 })];
    const index = indexRows(rows);
    const edge = makeEdge('a', 'b', 0, 0, 'straight');
    const geo = edgeGeometry(m, edge, index, 1);
    expect(geo).not.toBeNull();
    expect(geo!.rails).toHaveLength(1);
    expect(geo!.dash).toEqual([]);
    expect(geo!.dangling).toBe(false);
  });

  it('跨泳道 branch 边为虚线', () => {
    const rows = [makeRow({ oid: 'a', row: 0, lane: 0 }), makeRow({ oid: 'b', row: 1, lane: 1 })];
    const index = indexRows(rows);
    const edge = makeEdge('a', 'b', 0, 1, 'branch');
    const geo = edgeGeometry(m, edge, index, 1);
    expect(geo).not.toBeNull();
    expect(geo!.dash).toEqual([...m.branchDash]);
    expect(geo!.rails).toHaveLength(1);
  });

  it('跨泳道 merge 边为双轨', () => {
    const rows = [makeRow({ oid: 'a', row: 0, lane: 0 }), makeRow({ oid: 'b', row: 1, lane: 1 })];
    const index = indexRows(rows);
    const edge = makeEdge('a', 'b', 0, 1, 'merge');
    const geo = edgeGeometry(m, edge, index, 1);
    expect(geo).not.toBeNull();
    expect(geo!.rails).toHaveLength(2);
    expect(geo!.lineWidth).toBeCloseTo(m.railWidth);
  });

  it('同泳道 merge 边退化为单轨', () => {
    const rows = [makeRow({ oid: 'a', row: 0, lane: 0 }), makeRow({ oid: 'b', row: 1, lane: 0 })];
    const index = indexRows(rows);
    const edge = makeEdge('a', 'b', 0, 0, 'merge');
    const geo = edgeGeometry(m, edge, index, 1);
    expect(geo!.rails).toHaveLength(1);
  });

  it('父不在窗口内时 dangling=true', () => {
    const rows = [makeRow({ oid: 'a', row: 0, lane: 0 })];
    const index = indexRows(rows);
    const edge = makeEdge('a', 'missing', 0, 0, 'straight');
    const geo = edgeGeometry(m, edge, index, 0);
    expect(geo!.dangling).toBe(true);
  });

  it('孩子不在窗口内时返回 null', () => {
    const rows = [makeRow({ oid: 'b', row: 1, lane: 0 })];
    const index = indexRows(rows);
    const edge = makeEdge('missing', 'b', 0, 0, 'straight');
    const geo = edgeGeometry(m, edge, index, 1);
    expect(geo).toBeNull();
  });
});

// ---------------------------------------------------------------- traceRoundedPath

describe('traceRoundedPath', () => {
  it('空数组返回空', () => {
    expect(traceRoundedPath([], 5)).toEqual([]);
  });

  it('单点只有 move', () => {
    const cmds = traceRoundedPath([{ x: 1, y: 2 }], 5);
    expect(cmds).toHaveLength(1);
    expect(cmds[0]).toEqual({ type: 'move', x: 1, y: 2 });
  });

  it('两点为 move + line（无拐角）', () => {
    const cmds = traceRoundedPath(
      [
        { x: 0, y: 0 },
        { x: 0, y: 10 },
      ],
      5,
    );
    expect(cmds).toHaveLength(2);
    expect(cmds[0]!.type).toBe('move');
    expect(cmds[1]!.type).toBe('line');
  });

  it('三点带拐角时产生 curve 指令', () => {
    const cmds = traceRoundedPath(
      [
        { x: 0, y: 0 },
        { x: 0, y: 10 },
        { x: 10, y: 10 },
      ],
      3,
    );
    const types = cmds.map((c) => c.type);
    expect(types).toContain('curve');
  });
});

// ---------------------------------------------------------------- pointInRoundedRect

describe('pointInRoundedRect', () => {
  it('矩形内部点命中', () => {
    expect(pointInRoundedRect({ x: 5, y: 5 }, { x: 0, y: 0, w: 10, h: 10 }, 2)).toBe(true);
  });

  it('矩形外部点不命中', () => {
    expect(pointInRoundedRect({ x: 15, y: 5 }, { x: 0, y: 0, w: 10, h: 10 }, 2)).toBe(false);
  });

  it('圆角外区域不命中', () => {
    // 左上角 (0,0) 在 radius=5 时应该不命中（因为距角圆心 (5,5) 超过 5）
    expect(pointInRoundedRect({ x: 0.1, y: 0.1 }, { x: 0, y: 0, w: 20, h: 20 }, 5)).toBe(false);
  });
});

// ---------------------------------------------------------------- zoomAround

describe('zoomAround', () => {
  it('指针处内容坐标不变', () => {
    const scrollY = 100;
    const pointerY = 50;
    const newScroll = zoomAround(scrollY, pointerY, 1, 2);
    // contentY = scrollY + pointerY = 150
    // 缩放后 contentY' = 150 * 2 = 300 → scrollY' = 300 - 50 = 250
    expect(newScroll).toBeCloseTo(250);
  });

  it('同比例缩放不变', () => {
    expect(zoomAround(200, 30, 1.5, 1.5)).toBeCloseTo(200);
  });
});

// ---------------------------------------------------------------- visibleRowRange / overscanRows

describe('visibleRowRange', () => {
  const m = graphMetrics(1);

  it('rowCount=0 时返回空区间', () => {
    const r = visibleRowRange(m, 0, 500, 0);
    expect(r.first).toBe(0);
    expect(r.last).toBe(-1);
  });

  it('正常情况含 overscan', () => {
    const r = visibleRowRange(m, 0, 280, 100);
    expect(r.first).toBe(0);
    expect(r.last).toBeGreaterThan(9); // 280/28=10 行可见 + overscan
  });
});

describe('overscanRows', () => {
  it('至少返回 1', () => {
    const m = graphMetrics(MAX_SCALE); // rowHeight=84, 200/84≈2.38→ceil=3
    expect(overscanRows(m)).toBeGreaterThanOrEqual(1);
  });
});

// ---------------------------------------------------------------- rowAtY / laneAtX

describe('rowAtY / laneAtX', () => {
  const m = graphMetrics(1);

  it('rowAtY 正确换算', () => {
    expect(rowAtY(m, 0)).toBe(0);
    expect(rowAtY(m, 27)).toBe(0);
    expect(rowAtY(m, 28)).toBe(1);
    expect(rowAtY(m, 56)).toBe(2);
  });

  it('laneAtX 在 padLeft 左边返回 -1', () => {
    expect(laneAtX(m, 0)).toBe(-1);
    expect(laneAtX(m, BASE_PAD_LEFT - 1)).toBe(-1);
  });

  it('laneAtX 在 padLeft 处返回 lane 0', () => {
    expect(laneAtX(m, BASE_PAD_LEFT)).toBe(0);
  });

  it('laneAtX 正确换算后续泳道', () => {
    expect(laneAtX(m, BASE_PAD_LEFT + BASE_LANE_WIDTH)).toBe(1);
    expect(laneAtX(m, BASE_PAD_LEFT + 2 * BASE_LANE_WIDTH + 5)).toBe(2);
  });
});

// ---------------------------------------------------------------- 辅助函数

describe('graphColumnWidth / contentHeight / lastRowOf', () => {
  const m = graphMetrics(1);

  it('graphColumnWidth = padLeft*2 + laneCount*laneWidth', () => {
    expect(graphColumnWidth(m, 3)).toBe(BASE_PAD_LEFT * 2 + 3 * BASE_LANE_WIDTH);
  });

  it('graphColumnWidth 至少 1 泳道', () => {
    expect(graphColumnWidth(m, 0)).toBe(BASE_PAD_LEFT * 2 + BASE_LANE_WIDTH);
  });

  it('contentHeight = rowCount * rowHeight', () => {
    expect(contentHeight(m, 10)).toBe(10 * BASE_ROW_HEIGHT);
    expect(contentHeight(m, 0)).toBe(0);
    expect(contentHeight(m, -5)).toBe(0);
  });

  it('lastRowOf 返回最大行号', () => {
    const rows = [
      makeRow({ oid: 'a', row: 0, lane: 0 }),
      makeRow({ oid: 'b', row: 5, lane: 1 }),
      makeRow({ oid: 'c', row: 3, lane: 0 }),
    ];
    expect(lastRowOf(rows)).toBe(5);
  });

  it('lastRowOf 空数组返回 -1', () => {
    expect(lastRowOf([])).toBe(-1);
  });
});

// ---------------------------------------------------------------- estimateTextWidth

describe('estimateTextWidth', () => {
  it('ASCII 字符按 0.58em', () => {
    expect(estimateTextWidth('ab', 10)).toBeCloseTo(2 * 0.58 * 10);
  });

  it('CJK 字符按 1em', () => {
    expect(estimateTextWidth('中', 10)).toBeCloseTo(10);
  });

  it('空串返回 0', () => {
    expect(estimateTextWidth('', 10)).toBe(0);
  });
});

// ---------------------------------------------------------------- 颜色稳定性

describe('颜色稳定性（M2 验收）', () => {
  function fixturePages(): HistoryPage[] {
    const rows: GraphRow[] = [
      makeRow({ oid: 'c1', row: 0, lane: 0, colorIndex: 0 }),
      makeRow({ oid: 'c2', row: 1, lane: 1, colorIndex: 3 }),
      makeRow({ oid: 'c3', row: 2, lane: 0, colorIndex: 0 }),
      makeRow({ oid: 'c4', row: 3, lane: 2, colorIndex: 5 }),
    ];
    return [
      {
        commits: rows.map((r) => ({
          oid: r.oid,
          parents: [],
          author: { name: 'A', email: 'a@b.c', time: 1000 },
          committer: { name: 'A', email: 'a@b.c', time: 1000 },
          refs: [],
          signature: 'unsigned' as const,
          subject: `msg ${r.oid}`,
          body: null,
        })),
        layout: { rows, edges: [], laneCount: 3 },
        nextCursor: null,
      },
    ];
  }

  it('同一夹具两次 buildGraphModel 的 colorIndex 完全一致', () => {
    const model1 = buildGraphModel(fixturePages());
    const model2 = buildGraphModel(fixturePages());
    for (let i = 0; i < model1.rows.length; i++) {
      expect(model1.rows[i]!.colorIndex).toBe(model2.rows[i]!.colorIndex);
    }
  });

  it('行的 oid → colorIndex 映射一致', () => {
    const model1 = buildGraphModel(fixturePages());
    const model2 = buildGraphModel(fixturePages());
    for (const [oid, row1] of model1.index) {
      const row2 = model2.index.get(oid);
      expect(row2).toBeDefined();
      expect(row2!.colorIndex).toBe(row1.colorIndex);
    }
  });

  it('minimapBuckets 众数取最小 colorIndex（不抖动）', () => {
    // 构造两个颜色出现次数相同的桶
    const rows: GraphRow[] = [
      makeRow({ oid: 'a', row: 0, lane: 0, colorIndex: 5 }),
      makeRow({ oid: 'b', row: 1, lane: 0, colorIndex: 2 }),
    ];
    // 两个 colorIndex 各出现 1 次→应取 2（较小的）
    const buckets = minimapBuckets(rows, 2, 2);
    expect(buckets).toHaveLength(1);
    expect(buckets[0]!.colorIndex).toBe(2);
  });
});
