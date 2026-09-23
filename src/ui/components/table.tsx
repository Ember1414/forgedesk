import { forwardRef } from 'react';
import type { HTMLAttributes, ReactNode, ThHTMLAttributes } from 'react';

import { ArrowDown, ArrowUp } from 'lucide-react';

import { Skeleton } from '@/ui/components/skeleton';
import { cn } from '@/lib/utils';

/**
 * 表格（用于变更列表、分支列表、提交列表）。
 *
 * 排序的实现要点：可排序的表头渲染成 `<th aria-sort>` 里的按钮，而不是可点击的 `<th>`。
 * 原因：`<th>` 不可聚焦、不响应 Enter，键盘用户点不到；`aria-sort` 放在 th 上，
 * 读屏软件才会在进入该列时播报"已按升序排列"。
 *
 * 空态与加载态是表格的必备部分（列表页最常见的两种状态），因此一起提供，
 * 避免每个页面各写一套"暂无症状"的样式。
 */
export type SortDirection = 'asc' | 'desc' | false;

export const Table = forwardRef<HTMLTableElement, HTMLAttributes<HTMLTableElement>>(function Table(
  { className, ...props },
  ref,
) {
  return (
    <table
      ref={ref}
      className={cn('w-full border-collapse text-left text-13', className)}
      {...props}
    />
  );
});

export const TableHeader = forwardRef<
  HTMLTableSectionElement,
  HTMLAttributes<HTMLTableSectionElement>
>(function TableHeader({ className, ...props }, ref) {
  return <thead ref={ref} className={cn('text-12 text-fg-subtle', className)} {...props} />;
});

export const TableBody = forwardRef<
  HTMLTableSectionElement,
  HTMLAttributes<HTMLTableSectionElement>
>(function TableBody({ className, ...props }, ref) {
  return <tbody ref={ref} className={cn('text-fg', className)} {...props} />;
});

export const TableRow = forwardRef<HTMLTableRowElement, HTMLAttributes<HTMLTableRowElement>>(
  function TableRow({ className, ...props }, ref) {
    return (
      <tr
        ref={ref}
        className={cn(
          'fd-transition border-b border-line last:border-b-0 hover:bg-surface-sunken',
          'data-[selected]:bg-brand-subtle',
          className,
        )}
        {...props}
      />
    );
  },
);

export interface TableHeadProps extends ThHTMLAttributes<HTMLTableCellElement> {
  /** 排序方向；false 表示当前未按该列排序。 */
  readonly sortDirection?: SortDirection;
  /** 点击排序回调；提供后该表头变为可排序按钮。 */
  readonly onSort?: () => void;
  /** 列宽（如 'w-32'）。 */
  readonly widthClassName?: string;
}

export const TableHead = forwardRef<HTMLTableCellElement, TableHeadProps>(function TableHead(
  { className, children, sortDirection = false, onSort, widthClassName, ...props },
  ref,
) {
  return (
    <th
      ref={ref}
      scope="col"
      aria-sort={
        sortDirection === 'asc'
          ? 'ascending'
          : sortDirection === 'desc'
            ? 'descending'
            : onSort !== undefined
              ? 'none'
              : undefined
      }
      className={cn('border-b border-line px-2 py-1.5 font-medium', widthClassName, className)}
      {...props}
    >
      {onSort === undefined ? (
        children
      ) : (
        <button
          type="button"
          onClick={onSort}
          className="fd-transition inline-flex items-center gap-1 rounded-xs hover:text-fg"
        >
          {children}
          {sortDirection === 'asc' ? (
            <ArrowUp aria-hidden="true" className="size-3" />
          ) : sortDirection === 'desc' ? (
            <ArrowDown aria-hidden="true" className="size-3" />
          ) : null}
        </button>
      )}
    </th>
  );
});

export const TableCell = forwardRef<HTMLTableCellElement, ThHTMLAttributes<HTMLTableCellElement>>(
  function TableCell({ className, ...props }, ref) {
    return <td ref={ref} className={cn('px-2 py-1.5 align-middle', className)} {...props} />;
  },
);

/** 空态行：整表只有一行提示，避免出现"表头孤零零地悬着"。 */
export function TableEmptyRow({
  colSpan,
  children,
}: {
  readonly colSpan: number;
  readonly children: ReactNode;
}) {
  return (
    <tr>
      <td colSpan={colSpan} className="px-2 py-8 text-center text-13 text-fg-muted">
        {children}
      </td>
    </tr>
  );
}

/** 加载态行：行高与真实行接近，避免加载完成后整页跳动。 */
export function TableSkeletonRows({
  rows = 5,
  columns,
}: {
  readonly rows?: number;
  readonly columns: number;
}) {
  return (
    <>
      {Array.from({ length: rows }, (_, rowIndex) => (
        <tr key={rowIndex} className="border-b border-line last:border-b-0">
          {Array.from({ length: columns }, (_, columnIndex) => (
            <td key={columnIndex} className="px-2 py-2">
              <Skeleton className={columnIndex === 0 ? 'w-3/4' : 'w-1/2'} />
            </td>
          ))}
        </tr>
      ))}
    </>
  );
}
