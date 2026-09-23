import { useState } from 'react';
import type { CSSProperties, ReactNode } from 'react';

import { cn } from '@/lib/utils';

/**
 * 定高虚拟列表。
 *
 * 为什么自己实现而不引入虚拟化库：本项目的长列表（提交历史、文件树、日志）
 * 都是**等高行**，定高窗口化本身不到 30 行代码，而通用库要处理动态高度、
 * 测量缓存、滚动锚定等大量边界，体积与心智成本都不划算。
 * 一旦出现"可变行高"的真实需求（如 diff 视图），再评估引入专用库。
 *
 * 无障碍：容器是 role="list"，每行 role="listitem" 并给出 aria-posinset / aria-setsize。
 * 由于只有可视区域在 DOM 里，必须显式提供这两个属性，读屏软件才知道
 * "第 3 行，共 1284 行"，否则会以为总共就那几行。
 */
export interface VirtualListProps<TItem> {
  readonly items: readonly TItem[];
  /** 每行固定高度（px）。 */
  readonly itemHeight: number;
  /** 可视区高度（px）。 */
  readonly height: number;
  readonly renderItem: (item: TItem, index: number) => ReactNode;
  readonly getKey: (item: TItem, index: number) => string;
  /** 列表的无障碍名称。 */
  readonly label: string;
  /** 可视区外额外渲染的行数（滚动时减少白屏闪烁）。 */
  readonly overscan?: number;
  readonly className?: string;
}

export function VirtualList<TItem>({
  items,
  itemHeight,
  height,
  renderItem,
  getKey,
  label,
  overscan = 4,
  className,
}: VirtualListProps<TItem>) {
  const [scrollTop, setScrollTop] = useState(0);

  const firstVisible = Math.floor(scrollTop / itemHeight);
  const start = Math.max(0, firstVisible - overscan);
  const visibleCount = Math.ceil(height / itemHeight) + overscan * 2;
  const end = Math.min(items.length, start + visibleCount);
  const visible = items.slice(start, end);

  return (
    <div
      role="list"
      aria-label={label}
      className={cn('overflow-y-auto rounded-md border border-line bg-surface', className)}
      style={{ height }}
      onScroll={(event) => {
        setScrollTop(event.currentTarget.scrollTop);
      }}
    >
      {/* 占位层撑出真实滚动高度，真正的行绝对定位在可视区内 */}
      <div className="relative" style={{ height: items.length * itemHeight }}>
        {visible.map((item, offset) => {
          const index = start + offset;
          const style: CSSProperties = {
            position: 'absolute',
            top: index * itemHeight,
            left: 0,
            right: 0,
            height: itemHeight,
          };
          return (
            <div
              key={getKey(item, index)}
              role="listitem"
              aria-posinset={index + 1}
              aria-setsize={items.length}
              style={style}
            >
              {renderItem(item, index)}
            </div>
          );
        })}
      </div>
    </div>
  );
}
