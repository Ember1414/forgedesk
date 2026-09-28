import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import type { RowText } from '@/features/history/commitMeta';
import { GraphListMode } from '@/features/history/GraphListMode';
import {
  initialGraphSelectionState,
  useGraphSelectionStore,
} from '@/features/history/graphSelectionStore';

// ---------------------------------------------------------------- 夹具

function makeText(oid: string, index: number, overrides: Partial<RowText> = {}): RowText {
  return {
    oid,
    row: index,
    lane: index % 3,
    colorIndex: index % 8,
    isMerge: false,
    hidden: false,
    subject: `提交消息 ${index}`,
    author: `作者${index}`,
    time: `${index}天前`,
    shortOid: oid.slice(0, 7),
    refs: [],
    collapsed: 0,
    ...overrides,
  };
}

const TEXTS: RowText[] = [
  makeText('aaaaaaa1111', 0),
  makeText('bbbbbbb2222', 1),
  makeText('ccccccc3333', 2),
  makeText('ddddddd4444', 3),
  makeText('eeeeeee5555', 4),
];

// ---------------------------------------------------------------- 设置

beforeEach(() => {
  useGraphSelectionStore.setState(initialGraphSelectionState);
});

afterEach(() => {
  cleanup();
  useGraphSelectionStore.setState(initialGraphSelectionState);
});

// ---------------------------------------------------------------- aria 属性

describe('GraphListMode — aria 属性', () => {
  it('容器有 role=grid 与 aria-multiselectable', () => {
    render(<GraphListMode texts={TEXTS} />);
    const grid = screen.getByTestId('graph-list');
    expect(grid).toHaveAttribute('role', 'grid');
    expect(grid).toHaveAttribute('aria-multiselectable', 'true');
    expect(grid).toHaveAttribute('aria-colcount', '6');
  });

  it('aria-activedescendant 指向当前活动行', () => {
    render(<GraphListMode texts={TEXTS} />);
    const grid = screen.getByTestId('graph-list');
    expect(grid).toHaveAttribute('aria-activedescendant', 'fd-history-list-row-0');
  });

  it('每行有 role=row、aria-posinset、aria-setsize', () => {
    render(<GraphListMode texts={TEXTS} />);
    const rows = screen.getAllByRole('row');
    // 第一行是列头，后面 5 行是数据
    const dataRows = rows.filter((r) => r.getAttribute('aria-posinset') !== null);
    expect(dataRows).toHaveLength(5);
    expect(dataRows[0]).toHaveAttribute('aria-posinset', '1');
    expect(dataRows[0]).toHaveAttribute('aria-setsize', '5');
    expect(dataRows[4]).toHaveAttribute('aria-posinset', '5');
  });

  it('未选中行的 aria-selected 为 false', () => {
    render(<GraphListMode texts={TEXTS} />);
    const rows = screen.getAllByRole('row');
    const dataRows = rows.filter((r) => r.getAttribute('aria-posinset') !== null);
    for (const row of dataRows) {
      expect(row).toHaveAttribute('aria-selected', 'false');
    }
  });

  it('列头行有 role=row 且含 columnheader', () => {
    render(<GraphListMode texts={TEXTS} />);
    const headers = screen.getAllByRole('columnheader');
    expect(headers.length).toBe(6);
  });

  it('空数据时显示空态文案', () => {
    render(<GraphListMode texts={[]} />);
    // 组件应渲染空态
    const grid = screen.getByTestId('graph-list');
    expect(grid).toBeInTheDocument();
  });
});

// ---------------------------------------------------------------- 键盘导航

describe('GraphListMode — 键盘 ↑/↓', () => {
  it('ArrowDown 将活动行下移', () => {
    render(<GraphListMode texts={TEXTS} />);
    const grid = screen.getByTestId('graph-list');
    fireEvent.keyDown(grid, { key: 'ArrowDown' });
    expect(grid).toHaveAttribute('aria-activedescendant', 'fd-history-list-row-1');
  });

  it('ArrowUp 将活动行上移', () => {
    render(<GraphListMode texts={TEXTS} />);
    const grid = screen.getByTestId('graph-list');
    fireEvent.keyDown(grid, { key: 'ArrowDown' });
    fireEvent.keyDown(grid, { key: 'ArrowDown' });
    fireEvent.keyDown(grid, { key: 'ArrowUp' });
    expect(grid).toHaveAttribute('aria-activedescendant', 'fd-history-list-row-1');
  });

  it('ArrowUp 在第一行时不越界', () => {
    render(<GraphListMode texts={TEXTS} />);
    const grid = screen.getByTestId('graph-list');
    fireEvent.keyDown(grid, { key: 'ArrowUp' });
    expect(grid).toHaveAttribute('aria-activedescendant', 'fd-history-list-row-0');
  });

  it('ArrowDown 在最后一行时不越界', () => {
    render(<GraphListMode texts={TEXTS} />);
    const grid = screen.getByTestId('graph-list');
    for (let i = 0; i < 10; i++) {
      fireEvent.keyDown(grid, { key: 'ArrowDown' });
    }
    expect(grid).toHaveAttribute('aria-activedescendant', 'fd-history-list-row-4');
  });

  it('Home 跳到第一行', () => {
    render(<GraphListMode texts={TEXTS} />);
    const grid = screen.getByTestId('graph-list');
    fireEvent.keyDown(grid, { key: 'ArrowDown' });
    fireEvent.keyDown(grid, { key: 'ArrowDown' });
    fireEvent.keyDown(grid, { key: 'Home' });
    expect(grid).toHaveAttribute('aria-activedescendant', 'fd-history-list-row-0');
  });

  it('End 跳到最后一行', () => {
    render(<GraphListMode texts={TEXTS} />);
    const grid = screen.getByTestId('graph-list');
    fireEvent.keyDown(grid, { key: 'End' });
    expect(grid).toHaveAttribute('aria-activedescendant', 'fd-history-list-row-4');
  });
});

// ---------------------------------------------------------------- Enter 选中并打开详情

describe('GraphListMode — Enter 选中', () => {
  it('Enter 选中当前活动行并设置 detailOid', () => {
    render(<GraphListMode texts={TEXTS} />);
    const grid = screen.getByTestId('graph-list');
    fireEvent.keyDown(grid, { key: 'ArrowDown' }); // → 行 1
    fireEvent.keyDown(grid, { key: 'Enter' });
    const state = useGraphSelectionStore.getState();
    expect(state.selectedOids).toEqual(['bbbbbbb2222']);
    expect(state.detailOid).toBe('bbbbbbb2222');
  });

  it('Space 与 Enter 行为一致', () => {
    render(<GraphListMode texts={TEXTS} />);
    const grid = screen.getByTestId('graph-list');
    fireEvent.keyDown(grid, { key: ' ' });
    expect(useGraphSelectionStore.getState().selectedOids).toEqual(['aaaaaaa1111']);
  });

  it('选中行的 aria-selected 变为 true', () => {
    render(<GraphListMode texts={TEXTS} />);
    const grid = screen.getByTestId('graph-list');
    fireEvent.keyDown(grid, { key: 'Enter' });
    const row = screen.getByRole('row', { selected: true });
    expect(row).toHaveAttribute('data-oid', 'aaaaaaa1111');
  });
});

// ---------------------------------------------------------------- Ctrl+A 全选

describe('GraphListMode — Ctrl+A 全选', () => {
  it('Ctrl+A 选中全部行', () => {
    render(<GraphListMode texts={TEXTS} />);
    const grid = screen.getByTestId('graph-list');
    fireEvent.keyDown(grid, { key: 'a', ctrlKey: true });
    const state = useGraphSelectionStore.getState();
    expect(state.selectedOids).toHaveLength(5);
    expect(state.selectedOids).toEqual(TEXTS.map((t) => t.oid));
  });

  it('全选后所有行的 aria-selected 为 true', () => {
    render(<GraphListMode texts={TEXTS} />);
    const grid = screen.getByTestId('graph-list');
    fireEvent.keyDown(grid, { key: 'a', ctrlKey: true });
    const selectedRows = screen.getAllByRole('row', { selected: true });
    expect(selectedRows).toHaveLength(5);
  });

  it('Cmd+A（metaKey）同样触发全选', () => {
    render(<GraphListMode texts={TEXTS} />);
    const grid = screen.getByTestId('graph-list');
    fireEvent.keyDown(grid, { key: 'a', metaKey: true });
    expect(useGraphSelectionStore.getState().selectedOids).toHaveLength(5);
  });
});

// ---------------------------------------------------------------- 鼠标点击

describe('GraphListMode — 鼠标点击', () => {
  it('单击行选中该提交', () => {
    render(<GraphListMode texts={TEXTS} />);
    const row = screen
      .getAllByRole('row')
      .find((r) => r.getAttribute('data-oid') === 'ccccccc3333');
    expect(row).toBeDefined();
    fireEvent.click(row!);
    expect(useGraphSelectionStore.getState().selectedOids).toEqual(['ccccccc3333']);
  });

  it('Ctrl+单击追加选中', () => {
    render(<GraphListMode texts={TEXTS} />);
    const rows = screen.getAllByRole('row');
    const rowA = rows.find((r) => r.getAttribute('data-oid') === 'aaaaaaa1111')!;
    const rowC = rows.find((r) => r.getAttribute('data-oid') === 'ccccccc3333')!;
    fireEvent.click(rowA);
    fireEvent.click(rowC, { ctrlKey: true });
    expect(useGraphSelectionStore.getState().selectedOids).toEqual(['aaaaaaa1111', 'ccccccc3333']);
  });
});
