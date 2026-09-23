import { forwardRef } from 'react';
import type { ComponentRef } from 'react';

import * as ToggleGroupPrimitive from '@radix-ui/react-toggle-group';

import { cn } from '@/lib/utils';

/**
 * 分段切换（多选一）。
 *
 * 本组件替代了 T0.4 临时手写的 `SegmentedControl`：当时为了不引入依赖先写了一份，
 * T0.5 落地组件库后统一到这里，避免两份实现长期并存、行为渐渐分叉。
 *
 * 无障碍：Radix ToggleGroup 提供 roving tabindex 与方向键移动，
 * 选中态用 `data-state=on` 表达；这里刻意**不**用 tablist 语义——
 * 该控件切换的是设置值（主题、语言、面板位置），不是内容面板。
 *
 * 只支持单选：本项目所有实际用法都是"多选一"，多选形态留到真有需求时再加，
 * 否则要额外处理"空选"这个语义空洞（多选组全不选时该显示什么）。
 */
export interface ToggleGroupOption {
  readonly value: string;
  /** 已本地化的显示文案（组件自身不产生用户可见文案）。 */
  readonly label: string;
  readonly disabled?: boolean;
}

export interface ToggleGroupProps {
  /** 控件用途（用于 aria-label）。 */
  readonly label: string;
  readonly value: string;
  readonly options: readonly ToggleGroupOption[];
  onValueChange(value: string): void;
  readonly className?: string;
}

export const ToggleGroup = forwardRef<
  ComponentRef<typeof ToggleGroupPrimitive.Root>,
  ToggleGroupProps
>(function ToggleGroup({ label, value, options, onValueChange, className }, ref) {
  return (
    <ToggleGroupPrimitive.Root
      ref={ref}
      type="single"
      aria-label={label}
      value={value}
      // Radix 在"再次点击已选项"时会传出空字符串；多选一场景下应保持原值，否则会出现"全不选"
      onValueChange={(next) => {
        if (next !== '') {
          onValueChange(next);
        }
      }}
      className={cn(
        'inline-flex items-center gap-0.5 rounded-md border border-line bg-surface p-0.5',
        className,
      )}
    >
      {options.map((option) => (
        <ToggleGroupPrimitive.Item
          key={option.value}
          value={option.value}
          disabled={option.disabled ?? false}
          className={cn(
            'fd-transition rounded-sm px-2.5 py-1 text-12 text-fg-muted',
            'hover:bg-surface-sunken hover:text-fg',
            'data-[state=on]:bg-brand data-[state=on]:font-medium data-[state=on]:text-brand-fg',
            'disabled:cursor-not-allowed disabled:opacity-50',
          )}
        >
          {option.label}
        </ToggleGroupPrimitive.Item>
      ))}
    </ToggleGroupPrimitive.Root>
  );
});
