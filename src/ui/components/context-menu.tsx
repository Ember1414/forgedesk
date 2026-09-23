import { forwardRef } from 'react';
import type { ComponentPropsWithoutRef, ComponentRef } from 'react';

import * as ContextMenuPrimitive from '@radix-ui/react-context-menu';
import { Check, ChevronRight } from 'lucide-react';

import { cn } from '@/lib/utils';

/**
 * 右键菜单。
 *
 * 为什么值得单独做一个：文件列表、提交列表里的"复制哈希""在编辑器中打开""检出此提交"
 * 这类操作放在右键菜单里最顺手；但右键菜单**不能是唯一入口**——
 * 触摸板长按、键盘用户都可能用不上，因此同一批操作必须在 DropdownMenu
 * 或工具栏里有等价入口（使用约定，见组件展示页的对照示例）。
 */
export const ContextMenu = ContextMenuPrimitive.Root;
export const ContextMenuTrigger = ContextMenuPrimitive.Trigger;
export const ContextMenuGroup = ContextMenuPrimitive.Group;
export const ContextMenuSub = ContextMenuPrimitive.Sub;

export const ContextMenuContent = forwardRef<
  ComponentRef<typeof ContextMenuPrimitive.Content>,
  ComponentPropsWithoutRef<typeof ContextMenuPrimitive.Content>
>(function ContextMenuContent({ className, ...props }, ref) {
  return (
    <ContextMenuPrimitive.Portal>
      <ContextMenuPrimitive.Content
        ref={ref}
        className={cn(
          'z-50 min-w-52 rounded-lg border border-line bg-surface-raised p-1 shadow-lg',
          className,
        )}
        {...props}
      />
    </ContextMenuPrimitive.Portal>
  );
});

const itemClassName = cn(
  'fd-transition relative flex cursor-default select-none items-center gap-2 rounded-sm px-2 py-1.5 text-13 outline-none',
  'data-[highlighted]:bg-surface-sunken data-[highlighted]:text-fg',
  'data-[disabled]:pointer-events-none data-[disabled]:opacity-50',
);

export const ContextMenuItem = forwardRef<
  ComponentRef<typeof ContextMenuPrimitive.Item>,
  ComponentPropsWithoutRef<typeof ContextMenuPrimitive.Item> & {
    readonly destructive?: boolean;
  }
>(function ContextMenuItem({ className, destructive = false, ...props }, ref) {
  return (
    <ContextMenuPrimitive.Item
      ref={ref}
      className={cn(itemClassName, destructive && 'text-danger', className)}
      {...props}
    />
  );
});

export const ContextMenuCheckboxItem = forwardRef<
  ComponentRef<typeof ContextMenuPrimitive.CheckboxItem>,
  ComponentPropsWithoutRef<typeof ContextMenuPrimitive.CheckboxItem>
>(function ContextMenuCheckboxItem({ className, children, ...props }, ref) {
  return (
    <ContextMenuPrimitive.CheckboxItem
      ref={ref}
      className={cn(itemClassName, className)}
      {...props}
    >
      <ContextMenuPrimitive.ItemIndicator className="flex items-center">
        <Check aria-hidden="true" className="size-3.5 text-brand" />
      </ContextMenuPrimitive.ItemIndicator>
      {children}
    </ContextMenuPrimitive.CheckboxItem>
  );
});

export const ContextMenuRadioGroup = ContextMenuPrimitive.RadioGroup;

export const ContextMenuRadioItem = forwardRef<
  ComponentRef<typeof ContextMenuPrimitive.RadioItem>,
  ComponentPropsWithoutRef<typeof ContextMenuPrimitive.RadioItem>
>(function ContextMenuRadioItem({ className, children, ...props }, ref) {
  return (
    <ContextMenuPrimitive.RadioItem ref={ref} className={cn(itemClassName, className)} {...props}>
      <ContextMenuPrimitive.ItemIndicator className="flex items-center">
        <span aria-hidden="true" className="size-2 rounded-full bg-brand" />
      </ContextMenuPrimitive.ItemIndicator>
      {children}
    </ContextMenuPrimitive.RadioItem>
  );
});

export const ContextMenuLabel = forwardRef<
  ComponentRef<typeof ContextMenuPrimitive.Label>,
  ComponentPropsWithoutRef<typeof ContextMenuPrimitive.Label>
>(function ContextMenuLabel({ className, ...props }, ref) {
  return (
    <ContextMenuPrimitive.Label
      ref={ref}
      className={cn('px-2 py-1 text-12 font-medium text-fg-subtle', className)}
      {...props}
    />
  );
});

export const ContextMenuSeparator = forwardRef<
  ComponentRef<typeof ContextMenuPrimitive.Separator>,
  ComponentPropsWithoutRef<typeof ContextMenuPrimitive.Separator>
>(function ContextMenuSeparator({ className, ...props }, ref) {
  return (
    <ContextMenuPrimitive.Separator
      ref={ref}
      className={cn('my-1 h-px bg-line', className)}
      {...props}
    />
  );
});

export const ContextMenuSubTrigger = forwardRef<
  ComponentRef<typeof ContextMenuPrimitive.SubTrigger>,
  ComponentPropsWithoutRef<typeof ContextMenuPrimitive.SubTrigger>
>(function ContextMenuSubTrigger({ className, children, ...props }, ref) {
  return (
    <ContextMenuPrimitive.SubTrigger
      ref={ref}
      className={cn(itemClassName, 'data-[state=open]:bg-surface-sunken', className)}
      {...props}
    >
      {children}
      <ChevronRight aria-hidden="true" className="ml-auto size-3.5 shrink-0 text-fg-subtle" />
    </ContextMenuPrimitive.SubTrigger>
  );
});

export const ContextMenuSubContent = forwardRef<
  ComponentRef<typeof ContextMenuPrimitive.SubContent>,
  ComponentPropsWithoutRef<typeof ContextMenuPrimitive.SubContent>
>(function ContextMenuSubContent({ className, ...props }, ref) {
  return (
    <ContextMenuPrimitive.Portal>
      <ContextMenuPrimitive.SubContent
        ref={ref}
        className={cn(
          'z-50 min-w-48 rounded-lg border border-line bg-surface-raised p-1 shadow-lg',
          className,
        )}
        {...props}
      />
    </ContextMenuPrimitive.Portal>
  );
});
