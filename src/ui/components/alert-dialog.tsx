import { forwardRef } from 'react';
import type { ComponentPropsWithoutRef, ComponentRef, ReactNode } from 'react';

import * as AlertDialogPrimitive from '@radix-ui/react-alert-dialog';
import { cva } from 'class-variance-authority';
import type { VariantProps } from 'class-variance-authority';

import { Button } from '@/ui/components/button';
import { overlayClassName } from '@/ui/components/dialog';
import { cn } from '@/lib/utils';

/**
 * 确认对话框（用于破坏性操作）。
 *
 * 本组件是 AGENTS.md 红线 R7「破坏性 Git 操作必须有安全网」在 UI 层的第一道闸门：
 *
 *   1. **impact 必填**：每个 AlertDialog 都必须说明"这次操作会发生什么"，
 *      并且是类型层面强制（不是文档约定）。用户看不到后果的确认框等于没有确认。
 *   2. 默认焦点落在**取消**上，而不是确认；"确认"按钮必须由调用方显式声明为 danger 变体。
 *   3. 关闭方式只有两个明确出口（取消 / 确认），点击遮罩不会误关（Radix 的 AlertDialog 语义）。
 *
 * 注意：这里只负责"确认"，真正的执行仍须走 计划预览 → 快照 → 执行 → 可回滚（M1 起）。
 */
const impactVariants = cva('rounded-md border p-3 text-12 leading-relaxed', {
  variants: {
    tone: {
      danger: 'border-danger bg-surface text-fg',
      warning: 'border-warning bg-surface text-fg',
      info: 'border-line bg-surface-sunken text-fg-muted',
    },
  },
  defaultVariants: { tone: 'danger' },
});

export const AlertDialog = AlertDialogPrimitive.Root;
export const AlertDialogTrigger = AlertDialogPrimitive.Trigger;
export const AlertDialogPortal = AlertDialogPrimitive.Portal;

/**
 * AlertDialog 的遮罩必须由 AlertDialog 自己的原语渲染。
 *
 * 踩过的坑：直接复用 Dialog 的 Overlay 会抛
 * "`DialogOverlay` must be used within `Dialog`" —— Radix 的每个组件族有独立 context，
 * 跨族复用子组件不成立。样式共用（overlayClassName），组件不复用。
 */
export const AlertDialogOverlay = forwardRef<
  ComponentRef<typeof AlertDialogPrimitive.Overlay>,
  ComponentPropsWithoutRef<typeof AlertDialogPrimitive.Overlay>
>(function AlertDialogOverlay({ className, ...props }, ref) {
  return (
    <AlertDialogPrimitive.Overlay
      ref={ref}
      className={cn(overlayClassName, className)}
      {...props}
    />
  );
});

export interface AlertDialogContentProps
  extends
    ComponentPropsWithoutRef<typeof AlertDialogPrimitive.Content>,
    VariantProps<typeof impactVariants> {
  /** 影响说明（必填）：这次操作会改变什么、能不能撤销、会丢什么。 */
  readonly impact: ReactNode;
  /** 影响说明的标题，例如"影响"。若不传则只显示内容。 */
  readonly impactLabel?: string;
}

export const AlertDialogContent = forwardRef<
  ComponentRef<typeof AlertDialogPrimitive.Content>,
  AlertDialogContentProps
>(function AlertDialogContent({ className, children, impact, impactLabel, tone, ...props }, ref) {
  return (
    <AlertDialogPortal>
      <AlertDialogOverlay />
      <AlertDialogPrimitive.Content
        ref={ref}
        className={cn(
          'fixed left-1/2 top-1/2 z-50 w-full max-w-md -translate-x-1/2 -translate-y-1/2',
          'rounded-xl border border-line bg-surface-raised p-5 shadow-lg',
          className,
        )}
        {...props}
      >
        {children}

        {/* 影响说明：role="note" 让屏幕阅读器把它当作补充信息读出来，而不是正文的一部分 */}
        <div role="note" className={cn(impactVariants({ tone }), 'mt-4')}>
          {impactLabel !== undefined ? (
            <p className="mb-1 font-medium text-fg">{impactLabel}</p>
          ) : null}
          {impact}
        </div>
      </AlertDialogPrimitive.Content>
    </AlertDialogPortal>
  );
});

export function AlertDialogHeader({ className, ...props }: ComponentPropsWithoutRef<'div'>) {
  return <div className={cn('flex flex-col gap-1.5', className)} {...props} />;
}

export function AlertDialogFooter({ className, ...props }: ComponentPropsWithoutRef<'div'>) {
  return (
    <div
      className={cn('mt-5 flex flex-wrap items-center justify-end gap-2', className)}
      {...props}
    />
  );
}

export const AlertDialogTitle = forwardRef<
  ComponentRef<typeof AlertDialogPrimitive.Title>,
  ComponentPropsWithoutRef<typeof AlertDialogPrimitive.Title>
>(function AlertDialogTitle({ className, ...props }, ref) {
  return (
    <AlertDialogPrimitive.Title
      ref={ref}
      className={cn('text-16 font-semibold tracking-tight text-fg', className)}
      {...props}
    />
  );
});

export const AlertDialogDescription = forwardRef<
  ComponentRef<typeof AlertDialogPrimitive.Description>,
  ComponentPropsWithoutRef<typeof AlertDialogPrimitive.Description>
>(function AlertDialogDescription({ className, ...props }, ref) {
  return (
    <AlertDialogPrimitive.Description
      ref={ref}
      className={cn('text-13 text-fg-muted', className)}
      {...props}
    />
  );
});

/**
 * 取消按钮。
 *
 * 为什么单独封装：Radix 的 AlertDialog 需要 Actions 区域有明确的两个出口，
 * 且"取消"应当是默认聚焦项（`autoFocus`）——用户的默认动作永远不该是破坏性的。
 */
export const AlertDialogCancel = forwardRef<
  ComponentRef<typeof AlertDialogPrimitive.Cancel>,
  ComponentPropsWithoutRef<typeof AlertDialogPrimitive.Cancel>
>(function AlertDialogCancel({ className, children, ...props }, ref) {
  return (
    <AlertDialogPrimitive.Cancel asChild>
      <Button ref={ref} variant="secondary" className={className} {...props}>
        {children}
      </Button>
    </AlertDialogPrimitive.Cancel>
  );
});

export interface AlertDialogActionProps extends ComponentPropsWithoutRef<
  typeof AlertDialogPrimitive.Action
> {
  /** 破坏性操作请保持 true；只有"无副作用的重试"之类才可改为 false。 */
  readonly destructive?: boolean;
}

export const AlertDialogAction = forwardRef<
  ComponentRef<typeof AlertDialogPrimitive.Action>,
  AlertDialogActionProps
>(function AlertDialogAction({ className, children, destructive = true, ...props }, ref) {
  return (
    <AlertDialogPrimitive.Action asChild>
      <Button
        ref={ref}
        variant={destructive ? 'danger' : 'primary'}
        className={className}
        {...props}
      >
        {children}
      </Button>
    </AlertDialogPrimitive.Action>
  );
});
