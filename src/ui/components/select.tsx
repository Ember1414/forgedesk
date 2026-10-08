import { forwardRef, useId } from 'react';

import * as SelectPrimitive from '@radix-ui/react-select';
import { Check, ChevronDown } from 'lucide-react';

import { cn } from '@/lib/utils';

/**
 * 下拉选择。
 *
 * 为什么用 Radix Select 而不是原生 `<select>`：
 *   原生控件无法承载"带描述的两行选项""分组标题""带图标的当前值"，
 *   而分支选择、远端选择等场景都需要这些；同时 Radix 保留了原生级别的键盘
 *   （输入字母跳转、Home/End、方向键）与 aria 语义。
 *
 * 导出的是组合式 API（Trigger/Content/Item…），并额外提供 `SelectField`
 * 覆盖"标签 + 选项列表"这一最常用形态。
 */
export const Select = SelectPrimitive.Root;
export const SelectGroup = SelectPrimitive.Group;
export const SelectValue = SelectPrimitive.Value;

export const SelectTrigger = forwardRef<
  React.ComponentRef<typeof SelectPrimitive.Trigger>,
  React.ComponentPropsWithoutRef<typeof SelectPrimitive.Trigger>
>(function SelectTrigger({ className, children, ...props }, ref) {
  return (
    <SelectPrimitive.Trigger
      ref={ref}
      className={cn(
        'fd-transition flex h-8 w-full items-center justify-between gap-2 rounded-md border border-line bg-surface px-2.5 text-13 text-fg',
        'hover:border-line-strong focus:border-brand',
        'disabled:cursor-not-allowed disabled:opacity-50',
        'data-[placeholder]:text-fg-subtle',
        className,
      )}
      {...props}
    >
      {/* 选中值一律单行截断而不是折行：中文四五个字的选项在窄触发器里
          折成"三个字在上一个字在下"非常难看（2026-10-08 用户反馈） */}
      <span className="min-w-0 flex-1 truncate text-start">{children}</span>
      <SelectPrimitive.Icon asChild>
        <ChevronDown aria-hidden="true" className="size-3.5 shrink-0 text-fg-subtle" />
      </SelectPrimitive.Icon>
    </SelectPrimitive.Trigger>
  );
});

export const SelectContent = forwardRef<
  React.ComponentRef<typeof SelectPrimitive.Content>,
  React.ComponentPropsWithoutRef<typeof SelectPrimitive.Content>
>(function SelectContent({ className, children, position = 'popper', ...props }, ref) {
  return (
    <SelectPrimitive.Portal>
      <SelectPrimitive.Content
        ref={ref}
        position={position}
        sideOffset={4}
        className={cn(
          'z-50 max-h-72 min-w-[var(--radix-select-trigger-width)] overflow-y-auto',
          'rounded-lg border border-line bg-surface-raised p-1 shadow-lg',
          className,
        )}
        {...props}
      >
        <SelectPrimitive.Viewport>{children}</SelectPrimitive.Viewport>
      </SelectPrimitive.Content>
    </SelectPrimitive.Portal>
  );
});

export const SelectLabel = forwardRef<
  React.ComponentRef<typeof SelectPrimitive.Label>,
  React.ComponentPropsWithoutRef<typeof SelectPrimitive.Label>
>(function SelectLabel({ className, ...props }, ref) {
  return (
    <SelectPrimitive.Label
      ref={ref}
      className={cn('px-2 py-1 text-12 font-medium text-fg-subtle', className)}
      {...props}
    />
  );
});

export const SelectItem = forwardRef<
  React.ComponentRef<typeof SelectPrimitive.Item>,
  React.ComponentPropsWithoutRef<typeof SelectPrimitive.Item>
>(function SelectItem({ className, children, ...props }, ref) {
  return (
    <SelectPrimitive.Item
      ref={ref}
      className={cn(
        'fd-transition relative flex cursor-default select-none items-center gap-2 rounded-sm py-1.5 pl-2 pr-7 text-13 outline-none',
        'data-[highlighted]:bg-surface-sunken data-[highlighted]:text-fg',
        'data-[disabled]:pointer-events-none data-[disabled]:opacity-50',
        className,
      )}
      {...props}
    >
      <SelectPrimitive.ItemText>{children}</SelectPrimitive.ItemText>
      <SelectPrimitive.ItemIndicator className="absolute right-2 flex items-center">
        <Check aria-hidden="true" className="size-3.5 text-brand" />
      </SelectPrimitive.ItemIndicator>
    </SelectPrimitive.Item>
  );
});

export interface SelectOption {
  readonly value: string;
  /** 显示文案；未提供时直接显示 value（便于先用枚举值占位）。 */
  readonly label?: string;
}

export interface SelectFieldProps {
  readonly label: string;
  readonly value: string | undefined;
  readonly options: readonly SelectOption[];
  readonly placeholder?: string;
  readonly disabled?: boolean;
  readonly onValueChange?: (value: string) => void;
  readonly className?: string;
}

/**
 * 带标签的下拉选择（最常用形态）。
 * 组件内的全部文案都来自 props，组件本身不产生用户可见文案（便于上层走 i18n）。
 */
export function SelectField({
  label,
  value,
  options,
  placeholder,
  disabled = false,
  onValueChange,
  className,
}: SelectFieldProps) {
  const fieldId = useId();

  return (
    <div className={cn('flex w-full flex-col gap-1', className)}>
      <label htmlFor={fieldId} className="text-12 font-medium text-fg-muted">
        {label}
      </label>
      {/* exactOptionalPropertyTypes 下不能显式传 undefined，因此按存在性展开 */}
      <Select
        disabled={disabled}
        {...(value === undefined ? {} : { value })}
        {...(onValueChange === undefined ? {} : { onValueChange })}
      >
        <SelectTrigger id={fieldId} aria-label={label}>
          <SelectValue placeholder={placeholder} />
        </SelectTrigger>
        <SelectContent>
          {options.map((option) => (
            <SelectItem key={option.value} value={option.value}>
              {option.label ?? option.value}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </div>
  );
}
