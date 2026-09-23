import { forwardRef } from 'react';
import type { ComponentPropsWithoutRef, ComponentRef, HTMLAttributes } from 'react';

import * as DialogPrimitive from '@radix-ui/react-dialog';
import { X } from 'lucide-react';

import { IconButton } from '@/ui/components/icon-button';
import { cn } from '@/lib/utils';

/**
 * 模态对话框。
 *
 * 无障碍要点（Radix 内建，这里只负责样式与结构）：
 *   - 打开后焦点移入对话框，关闭后回到触发元素；
 *   - Tab 被限制在对话框内（焦点陷阱），Esc 关闭；
 *   - 标题/描述通过 aria-labelledby、aria-describedby 关联。
 *
 * 文案约定：组件**不产生**任何用户可见文案。关闭按钮的无障碍名称由 `closeLabel`
 * 从上层的 i18n 传入（类型上是必填），这样不会出现"组件里藏了一句没翻译的英文"。
 *
 * 结构要求：`DialogContent` 内必须有 `DialogTitle`，
 * 否则屏幕阅读器只会读出"对话框"而没有任何上下文。
 */
export const Dialog = DialogPrimitive.Root;
export const DialogTrigger = DialogPrimitive.Trigger;
export const DialogClose = DialogPrimitive.Close;
export const DialogPortal = DialogPrimitive.Portal;

/**
 * 遮罩层的样式（Dialog 与 AlertDialog 共用）。
 *
 * 共用是为了避免两处遮罩渐渐不一致（例如一边加了 blur 一边没加）——
 * 它们叠在同一层级上，样式漂移会表现为"有的弹窗背景暗、有的亮"。
 */
export const overlayClassName = 'fixed inset-0 z-50 bg-scrim';

export const DialogOverlay = forwardRef<
  ComponentRef<typeof DialogPrimitive.Overlay>,
  ComponentPropsWithoutRef<typeof DialogPrimitive.Overlay>
>(function DialogOverlay({ className, ...props }, ref) {
  return (
    <DialogPrimitive.Overlay ref={ref} className={cn(overlayClassName, className)} {...props} />
  );
});

export interface DialogContentProps extends ComponentPropsWithoutRef<
  typeof DialogPrimitive.Content
> {
  /** 关闭按钮的无障碍名称（必填，来自上层 i18n）。 */
  readonly closeLabel: string;
  /** 是否显示右上角关闭按钮（默认显示）。 */
  readonly showClose?: boolean;
}

export const DialogContent = forwardRef<
  ComponentRef<typeof DialogPrimitive.Content>,
  DialogContentProps
>(function DialogContent({ className, children, closeLabel, showClose = true, ...props }, ref) {
  return (
    <DialogPortal>
      <DialogOverlay />
      <DialogPrimitive.Content
        ref={ref}
        className={cn(
          'fixed left-1/2 top-1/2 z-50 w-full max-w-lg -translate-x-1/2 -translate-y-1/2',
          'rounded-xl border border-line bg-surface-raised p-5 shadow-lg',
          className,
        )}
        {...props}
      >
        {children}
        {showClose ? (
          <DialogPrimitive.Close asChild>
            <IconButton label={closeLabel} size="sm" className="absolute right-3 top-3">
              <X aria-hidden="true" className="size-3.5" />
            </IconButton>
          </DialogPrimitive.Close>
        ) : null}
      </DialogPrimitive.Content>
    </DialogPortal>
  );
});

export function DialogHeader({ className, ...props }: HTMLAttributes<HTMLDivElement>) {
  return <div className={cn('flex flex-col gap-1.5 pr-6', className)} {...props} />;
}

export function DialogFooter({ className, ...props }: HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      className={cn('mt-5 flex flex-wrap items-center justify-end gap-2', className)}
      {...props}
    />
  );
}

export const DialogTitle = forwardRef<
  ComponentRef<typeof DialogPrimitive.Title>,
  ComponentPropsWithoutRef<typeof DialogPrimitive.Title>
>(function DialogTitle({ className, ...props }, ref) {
  return (
    <DialogPrimitive.Title
      ref={ref}
      className={cn('text-16 font-semibold tracking-tight text-fg', className)}
      {...props}
    />
  );
});

export const DialogDescription = forwardRef<
  ComponentRef<typeof DialogPrimitive.Description>,
  ComponentPropsWithoutRef<typeof DialogPrimitive.Description>
>(function DialogDescription({ className, ...props }, ref) {
  return (
    <DialogPrimitive.Description
      ref={ref}
      className={cn('text-13 text-fg-muted', className)}
      {...props}
    />
  );
});
