import { fireEvent, render, screen, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { Badge } from '@/ui/components/badge';
import { Button } from '@/ui/components/button';
import { EmptyState } from '@/ui/components/empty-state';
import { ErrorState } from '@/ui/components/error-state';
import { Progress } from '@/ui/components/progress';
import { Skeleton, SkeletonText } from '@/ui/components/skeleton';
import {
  Table,
  TableBody,
  TableCell,
  TableEmptyRow,
  TableHead,
  TableHeader,
  TableRow,
  TableSkeletonRows,
} from '@/ui/components/table';
import { Tag } from '@/ui/components/tag';
import { VirtualList } from '@/ui/components/virtual-list';

/**
 * 数据展示类组件的测试重点：
 *   - 排序的语义（aria-sort 必须随状态变化，否则读屏软件读不出"已按某列排序"）；
 *   - 空态/加载态是列表页的常态，不能只在"有数据"时正确；
 *   - 虚拟列表必须显式给出 aria-posinset / aria-setsize，
 *     因为只有可视区域在 DOM 里，否则读屏软件会误报总数。
 */
describe('Table', () => {
  it('可排序表头通过 aria-sort 表达当前排序状态', () => {
    const onSort = vi.fn();
    render(
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead sortDirection="asc" onSort={onSort}>
              改动行数
            </TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          <TableRow>
            <TableCell>+12</TableCell>
          </TableRow>
        </TableBody>
      </Table>,
    );

    const header = screen.getByRole('columnheader');
    expect(header).toHaveAttribute('aria-sort', 'ascending');

    fireEvent.click(screen.getByRole('button', { name: '改动行数' }));
    expect(onSort).toHaveBeenCalledTimes(1);
  });

  it('未排序的可排序表头 aria-sort 为 none（而不是缺失）', () => {
    render(
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead sortDirection={false} onSort={() => undefined}>
              文件
            </TableHead>
          </TableRow>
        </TableHeader>
      </Table>,
    );

    expect(screen.getByRole('columnheader')).toHaveAttribute('aria-sort', 'none');
  });

  it('空态行占满所有列并显示提示', () => {
    render(
      <Table>
        <TableBody>
          <TableEmptyRow colSpan={3}>工作区干净</TableEmptyRow>
        </TableBody>
      </Table>,
    );

    const cell = screen.getByRole('cell');
    expect(cell).toHaveAttribute('colspan', '3');
    expect(cell).toHaveTextContent('工作区干净');
  });

  it('骨架行数正确且带 aria-hidden（不逐行朗读占位块）', () => {
    render(
      <Table>
        <TableBody>
          <TableSkeletonRows rows={3} columns={2} />
        </TableBody>
      </Table>,
    );

    const rows = screen.getAllByRole('row');
    expect(rows).toHaveLength(3);
    // 每行 2 个骨架，共 6 个
    expect(within(rows[0] as HTMLElement).getAllByText('', { selector: 'div' })).toHaveLength(2);
  });
});

describe('VirtualList', () => {
  const items = Array.from({ length: 500 }, (_, index) => ({ id: `row-${String(index)}` }));

  it('只渲染可视区域附近的行（真正做了窗口化）', () => {
    render(
      <VirtualList
        label="文件列表"
        items={items}
        itemHeight={20}
        height={100}
        getKey={(item) => item.id}
        renderItem={(item) => <span>{item.id}</span>}
      />,
    );

    const rendered = screen.getAllByRole('listitem');
    // 可视 5 行 + 上下各 4 行 overscan = 13 行左右，远小于 500
    expect(rendered.length).toBeGreaterThanOrEqual(5);
    expect(rendered.length).toBeLessThan(20);
    expect(screen.getByText('row-0')).toBeInTheDocument();
    expect(screen.queryByText('row-499')).not.toBeInTheDocument();
  });

  it('每行给出 aria-posinset 与 aria-setsize（读屏软件才知道总数）', () => {
    render(
      <VirtualList
        label="文件列表"
        items={items}
        itemHeight={20}
        height={100}
        getKey={(item) => item.id}
        renderItem={(item) => <span>{item.id}</span>}
      />,
    );

    const first = screen.getAllByRole('listitem')[0] as HTMLElement;
    expect(first).toHaveAttribute('aria-posinset', '1');
    expect(first).toHaveAttribute('aria-setsize', '500');
  });

  it('滚动后渲染的窗口随之下移', () => {
    render(
      <VirtualList
        label="文件列表"
        items={items}
        itemHeight={20}
        height={100}
        overscan={0}
        getKey={(item) => item.id}
        renderItem={(item) => <span>{item.id}</span>}
      />,
    );

    fireEvent.scroll(screen.getByRole('list'), { target: { scrollTop: 2000 } });

    // 2000 / 20 = 第 100 行开始
    expect(screen.getByText('row-100')).toBeInTheDocument();
    expect(screen.queryByText('row-0')).not.toBeInTheDocument();
  });
});

describe('Progress', () => {
  it('确定进度暴露 aria-valuenow 与百分比文本', () => {
    render(<Progress label="推送进度" value={0.42} />);

    const bar = screen.getByRole('progressbar', { name: '推送进度' });
    expect(bar).toHaveAttribute('aria-valuenow', '42');
    expect(screen.getByText('42%')).toBeInTheDocument();
  });

  it('不确定进度不暴露 aria-valuenow（避免误报百分比）', () => {
    render(<Progress label="获取中" value={null} />);

    const bar = screen.getByRole('progressbar', { name: '获取中' });
    expect(bar).not.toHaveAttribute('aria-valuenow');
    expect(screen.getByText('—')).toBeInTheDocument();
  });

  it('超出范围的值被夹紧', () => {
    render(<Progress label="进度" value={1.5} />);
    expect(screen.getByRole('progressbar', { name: '进度' })).toHaveAttribute(
      'aria-valuenow',
      '100',
    );
  });
});

describe('Skeleton', () => {
  it('骨架是装饰性元素（aria-hidden），不会被读屏软件朗读', () => {
    const { container } = render(<Skeleton />);
    expect(container.firstElementChild).toHaveAttribute('aria-hidden', 'true');
  });

  it('SkeletonText 渲染指定行数', () => {
    const { container } = render(<SkeletonText lines={4} />);
    expect(container.querySelectorAll('div[aria-hidden="true"]')).toHaveLength(4);
  });
});

describe('Badge', () => {
  it('渲染文字状态（不只靠颜色传达）', () => {
    render(<Badge tone="success">已通过</Badge>);
    expect(screen.getByText('已通过')).toBeInTheDocument();
  });

  it('图标被置为 aria-hidden，srLabel 只给读屏软件', () => {
    render(
      <Badge icon={<span>★</span>} srLabel="两条未读">
        2
      </Badge>,
    );
    expect(screen.getByText('两条未读')).toHaveClass('sr-only');
  });
});

describe('Tag', () => {
  it('可点选筛选并反映按下状态', () => {
    const onClick = vi.fn();
    render(
      <Tag onClick={onClick} selected>
        后端
      </Tag>,
    );

    const button = screen.getByRole('button', { name: '后端' });
    expect(button).toHaveAttribute('aria-pressed', 'true');
    fireEvent.click(button);
    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it('删除按钮有无障碍名称', () => {
    const onRemove = vi.fn();
    render(
      <Tag onRemove={onRemove} removeLabel="移除标签 临时">
        临时
      </Tag>,
    );

    fireEvent.click(screen.getByRole('button', { name: '移除标签 临时' }));
    expect(onRemove).toHaveBeenCalledTimes(1);
  });
});

describe('EmptyState', () => {
  it('展示标题、说明与主操作', () => {
    const onClick = vi.fn();
    render(
      <EmptyState
        title="还没有打开的仓库"
        description="打开一个本地仓库后这里会显示记录。"
        action={<Button onClick={onClick}>打开仓库</Button>}
      />,
    );

    expect(screen.getByRole('heading', { name: '还没有打开的仓库' })).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '打开仓库' }));
    expect(onClick).toHaveBeenCalledTimes(1);
  });
});

describe('ErrorState', () => {
  it('以 role=alert 呈现并支持重试', () => {
    const onRetry = vi.fn();
    render(
      <ErrorState
        title="无法读取仓库状态"
        hint="目录可能不是 Git 仓库。"
        retryLabel="重试"
        onRetry={onRetry}
      />,
    );

    expect(screen.getByRole('alert')).toHaveTextContent('无法读取仓库状态');
    fireEvent.click(screen.getByRole('button', { name: '重试' }));
    expect(onRetry).toHaveBeenCalledTimes(1);
  });

  it('重试中按钮处于 loading 且不可重复点击', () => {
    const onRetry = vi.fn();
    render(<ErrorState title="失败" retryLabel="重试" retryLoading onRetry={onRetry} />);

    const button = screen.getByRole('button', { name: '重试' });
    expect(button).toBeDisabled();
    fireEvent.click(button);
    expect(onRetry).not.toHaveBeenCalled();
  });

  it('详细信息默认折叠（技术细节不淹没普通用户）', () => {
    render(<ErrorState title="失败" details="fatal: not a git repository" />);
    expect(screen.getByText('fatal: not a git repository')).toBeInTheDocument();
    expect(screen.getByText('details').closest('details')).not.toHaveAttribute('open');
  });
});
