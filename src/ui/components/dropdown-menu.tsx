import { forwardRef } from 'react';
import type { ComponentPropsWithoutRef, ComponentRef } from 'react';

import * as DropdownMenuPrimitive from '@radix-ui/react-dropdown-menu';
import { Check, ChevronRight } from 'lucide-react';

import { cn } from '@/lib/utils';

/**
 * 下拉菜单（点击触发器打开）。
 *
 * 键盘行为由 Radix 保证：触发键 Enter/Space/↓ 打开，↑↓ 移动、Home/End 跳转、
 * 输入字母做前缀匹配、Esc 关闭并把焦点还给触发器。
 * 这些不是"加分项"——桌面 Git 工具的高频操作几乎都在键盘上完成。
 */
export const DropdownMenu = DropdownMenuPrimitive.Root;
export const DropdownMenuTrigger = DropdownMenuPrimitive.Trigger;
export const DropdownMenuGroup = DropdownMenuPrimitive.Group;
export const DropdownMenuSub = DropdownMenuPrimitive.Sub;

export const DropdownMenuContent = forwardRef<
  ComponentRef<typeof DropdownMenuPrimitive.Content>,
  ComponentPropsWithoutRef<typeof DropdownMenuPrimitive.Content>
>(function DropdownMenuContent({ className, sideOffset = 6, ...props }, ref) {
  return (
    <DropdownMenuPrimitive.Portal>
      <DropdownMenuPrimitive.Content
        ref={ref}
        sideOffset={sideOffset}
        className={cn(
          'z-50 min-w-56 rounded-lg border border-line bg-surface-raised p-1 shadow-lg',
          className,
        )}
        {...props}
      />
    </DropdownMenuPrimitive.Portal>
  );
});

const itemClassName = cn(
  'fd-transition relative flex cursor-default select-none items-center gap-2 rounded-sm px-2 py-1.5 text-13 outline-none',
  'data-[highlighted]:bg-surface-sunken data-[highlighted]:text-fg',
  'data-[disabled]:pointer-events-none data-[disabled]:opacity-50',
);

export const DropdownMenuItem = forwardRef<
  ComponentRef<typeof DropdownMenuPrimitive.Item>,
  ComponentPropsWithoutRef<typeof DropdownMenuPrimitive.Item> & {
    /** 危险项（如删除分支）：用危险色文字，避免与普通项混淆。 */
    readonly destructive?: boolean;
    /** 右侧的快捷键提示（如 ⌘C）；纯展示，不注册快捷键。 */
    readonly shortcut?: string;
  }
>(function DropdownMenuItem({ className, destructive = false, shortcut, children, ...props }, ref) {
  return (
    <DropdownMenuPrimitive.Item
      ref={ref}
      className={cn(itemClassName, destructive && 'text-danger', className)}
      {...props}
    >
      {children}
      {shortcut !== undefined ? (
        <span className="ml-auto shrink-0 font-mono text-12 text-fg-subtle">{shortcut}</span>
      ) : null}
    </DropdownMenuPrimitive.Item>
  );
});

export const DropdownMenuCheckboxItem = forwardRef<
  ComponentRef<typeof DropdownMenuPrimitive.CheckboxItem>,
  ComponentPropsWithoutRef<typeof DropdownMenuPrimitive.CheckboxItem>
>(function DropdownMenuCheckboxItem({ className, children, ...props }, ref) {
  return (
    <DropdownMenuPrimitive.CheckboxItem
      ref={ref}
      className={cn(itemClassName, className)}
      {...props}
    >
      <DropdownMenuPrimitive.ItemIndicator className="flex items-center">
        <Check aria-hidden="true" className="size-3.5 text-brand" />
      </DropdownMenuPrimitive.ItemIndicator>
      {children}
    </DropdownMenuPrimitive.CheckboxItem>
  );
});

export const DropdownMenuRadioGroup = DropdownMenuPrimitive.RadioGroup;

export const DropdownMenuRadioItem = forwardRef<
  ComponentRef<typeof DropdownMenuPrimitive.RadioItem>,
  ComponentPropsWithoutRef<typeof DropdownMenuPrimitive.RadioItem>
>(function DropdownMenuRadioItem({ className, children, ...props }, ref) {
  return (
    <DropdownMenuPrimitive.RadioItem ref={ref} className={cn(itemClassName, className)} {...props}>
      <DropdownMenuPrimitive.ItemIndicator className="flex items-center">
        <span aria-hidden="true" className="size-2 rounded-full bg-brand" />
      </DropdownMenuPrimitive.ItemIndicator>
      {children}
    </DropdownMenuPrimitive.RadioItem>
  );
});

export const DropdownMenuLabel = forwardRef<
  ComponentRef<typeof DropdownMenuPrimitive.Label>,
  ComponentPropsWithoutRef<typeof DropdownMenuPrimitive.Label>
>(function DropdownMenuLabel({ className, ...props }, ref) {
  return (
    <DropdownMenuPrimitive.Label
      ref={ref}
      className={cn('px-2 py-1 text-12 font-medium text-fg-subtle', className)}
      {...props}
    />
  );
});

export const DropdownMenuSeparator = forwardRef<
  ComponentRef<typeof DropdownMenuPrimitive.Separator>,
  ComponentPropsWithoutRef<typeof DropdownMenuPrimitive.Separator>
>(function DropdownMenuSeparator({ className, ...props }, ref) {
  return (
    <DropdownMenuPrimitive.Separator
      ref={ref}
      className={cn('my-1 h-px bg-line', className)}
      {...props}
    />
  );
});

export const DropdownMenuSubTrigger = forwardRef<
  ComponentRef<typeof DropdownMenuPrimitive.SubTrigger>,
  ComponentPropsWithoutRef<typeof DropdownMenuPrimitive.SubTrigger>
>(function DropdownMenuSubTrigger({ className, children, ...props }, ref) {
  return (
    <DropdownMenuPrimitive.SubTrigger
      ref={ref}
      className={cn(itemClassName, 'data-[state=open]:bg-surface-sunken', className)}
      {...props}
    >
      {children}
      <ChevronRight aria-hidden="true" className="ml-auto size-3.5 shrink-0 text-fg-subtle" />
    </DropdownMenuPrimitive.SubTrigger>
  );
});

export const DropdownMenuSubContent = forwardRef<
  ComponentRef<typeof DropdownMenuPrimitive.SubContent>,
  ComponentPropsWithoutRef<typeof DropdownMenuPrimitive.SubContent>
>(function DropdownMenuSubContent({ className, ...props }, ref) {
  return (
    <DropdownMenuPrimitive.Portal>
      <DropdownMenuPrimitive.SubContent
        ref={ref}
        className={cn(
          'z-50 min-w-48 rounded-lg border border-line bg-surface-raised p-1 shadow-lg',
          className,
        )}
        {...props}
      />
    </DropdownMenuPrimitive.Portal>
  );
});
