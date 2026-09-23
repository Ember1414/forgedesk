import { useState } from 'react';
import type { ReactNode } from 'react';

import { Resizable } from '@/ui/components/resizable';
import { cn } from '@/lib/utils';

/**
 * 可拖拽分栏。
 *
 * 由 `Resizable`（主区）+ 剩余空间（副区）组成，而不是"两个都可调"：
 * 两个把手争抢同一段空间时，拖动一个会让另一个也跟着变，
 * 用户失去"我调的是哪一个"的直觉。Git 客户端里真正需要自由的场景
 * （详情面板宽度）用一个把手就够了。
 *
 * 受控/非受控都支持：传 `primarySize` 即受控（便于把尺寸持久化到设置里，
 * 后续 T5.10 的"布局持久化"会用到）。
 */
export interface SplitPaneProps {
  readonly orientation?: 'horizontal' | 'vertical';
  readonly primary: ReactNode;
  readonly secondary: ReactNode;
  /** 受控尺寸（px）。 */
  readonly primarySize?: number;
  /** 非受控初始尺寸（px）。 */
  readonly defaultPrimarySize?: number;
  readonly minPrimarySize?: number;
  readonly maxPrimarySize?: number;
  onPrimarySizeChange?(size: number): void;
  /** 分隔条的无障碍名称（必填，来自上层 i18n）。 */
  readonly separatorLabel: string;
  readonly className?: string;
}

export function SplitPane({
  orientation = 'horizontal',
  primary,
  secondary,
  primarySize,
  defaultPrimarySize = 280,
  minPrimarySize = 160,
  maxPrimarySize = 720,
  onPrimarySizeChange,
  separatorLabel,
  className,
}: SplitPaneProps) {
  const [internalSize, setInternalSize] = useState(defaultPrimarySize);
  const size = primarySize ?? internalSize;

  function handleSizeChange(next: number): void {
    if (primarySize === undefined) {
      setInternalSize(next);
    }
    onPrimarySizeChange?.(next);
  }

  return (
    <div
      className={cn(
        'flex min-h-0 min-w-0 flex-1',
        orientation === 'horizontal' ? 'flex-row' : 'flex-col',
        className,
      )}
    >
      <Resizable
        edge="end"
        orientation={orientation}
        size={size}
        minSize={minPrimarySize}
        maxSize={maxPrimarySize}
        onSizeChange={handleSizeChange}
        handleLabel={separatorLabel}
      >
        {primary}
      </Resizable>

      <div className="min-h-0 min-w-0 flex-1 overflow-hidden">{secondary}</div>
    </div>
  );
}
