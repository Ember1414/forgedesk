import { forwardRef } from 'react';
import type { ComponentPropsWithoutRef, ComponentRef } from 'react';

import * as TabsPrimitive from '@radix-ui/react-tabs';

import { cn } from '@/lib/utils';

/**
 * 标签页。
 *
 * 键盘行为（Radix 内建，符合 WAI-ARIA Tabs 模式）：
 * 方向键在标签间移动、Home/End 跳到首尾，且**只有当前标签**在 Tab 序列里（roving tabindex），
 * 因此用户从页面 Tab 过来不会逐个标签停留。
 *
 * 与侧栏导航的分工：标签切换的是"同一上下文的视角"（仓库内的五页），
 * 侧栏切换的是"不同的工作区域"。层级不同，不要混用。
 */
export const Tabs = TabsPrimitive.Root;

export const TabsList = forwardRef<
  ComponentRef<typeof TabsPrimitive.List>,
  ComponentPropsWithoutRef<typeof TabsPrimitive.List>
>(function TabsList({ className, ...props }, ref) {
  return (
    <TabsPrimitive.List
      ref={ref}
      className={cn('flex flex-wrap items-center gap-1 border-b border-line pb-1', className)}
      {...props}
    />
  );
});

export const TabsTrigger = forwardRef<
  ComponentRef<typeof TabsPrimitive.Trigger>,
  ComponentPropsWithoutRef<typeof TabsPrimitive.Trigger> & { readonly count?: number }
>(function TabsTrigger({ className, count, children, ...props }, ref) {
  return (
    <TabsPrimitive.Trigger
      ref={ref}
      className={cn(
        'fd-transition inline-flex items-center gap-1.5 rounded-md px-2.5 py-1 text-13 text-fg-muted',
        'hover:bg-surface-sunken hover:text-fg',
        'data-[state=active]:bg-brand-subtle data-[state=active]:font-medium data-[state=active]:text-brand',
        'disabled:cursor-not-allowed disabled:opacity-50',
        className,
      )}
      {...props}
    >
      {children}
      {count !== undefined ? (
        <span className="rounded-xs bg-surface-sunken px-1 font-mono text-12 text-fg-subtle">
          {count}
        </span>
      ) : null}
    </TabsPrimitive.Trigger>
  );
});

export const TabsContent = forwardRef<
  ComponentRef<typeof TabsPrimitive.Content>,
  ComponentPropsWithoutRef<typeof TabsPrimitive.Content>
>(function TabsContent({ className, ...props }, ref) {
  return (
    <TabsPrimitive.Content ref={ref} className={cn('mt-3 outline-none', className)} {...props} />
  );
});
