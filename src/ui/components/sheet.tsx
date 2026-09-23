import { forwardRef } from 'react';
import type { ComponentPropsWithoutRef, ComponentRef } from 'react';

import * as DialogPrimitive from '@radix-ui/react-dialog';
import { cva } from 'class-variance-authority';
import type { VariantProps } from 'class-variance-authority';
import { X } from 'lucide-react';

import { DialogOverlay, DialogPortal } from '@/ui/components/dialog';
import { IconButton } from '@/ui/components/icon-button';
import { cn } from '@/lib/utils';

/**
 * 抽屉（从屏幕边缘滑出的面板）。
 *
 * 与 Dialog 的区别是"语义定位"而不是尺寸：Dialog 打断流程（必须做决定），
 * 抽屉不打断（看详情、挑分支、临时配置），因此抽屉允许内容滚动并常驻一个标题栏。
 * 底层复用 Radix Dialog 原语以继承焦点陷阱与 Esc 行为。
 */
const sheetVariants = cva(
  'fixed z-50 flex flex-col gap-4 border-line bg-surface-raised p-5 shadow-lg',
  {
    variants: {
      side: {
        right: 'inset-y-0 right-0 h-full w-full max-w-sm border-l',
        left: 'inset-y-0 left-0 h-full w-full max-w-sm border-r',
        top: 'inset-x-0 top-0 w-full border-b',
        bottom: 'inset-x-0 bottom-0 w-full border-t',
      },
    },
    defaultVariants: { side: 'right' },
  },
);

export interface SheetContentProps
  extends
    ComponentPropsWithoutRef<typeof DialogPrimitive.Content>,
    VariantProps<typeof sheetVariants> {
  /** 关闭按钮的无障碍名称（必填，来自上层 i18n）。 */
  readonly closeLabel: string;
}

export const SheetContent = forwardRef<
  ComponentRef<typeof DialogPrimitive.Content>,
  SheetContentProps
>(function SheetContent({ className, children, closeLabel, side, ...props }, ref) {
  return (
    <DialogPortal>
      <DialogOverlay />
      <DialogPrimitive.Content
        ref={ref}
        className={cn(sheetVariants({ side }), className)}
        {...props}
      >
        {children}
        <DialogPrimitive.Close asChild>
          <IconButton label={closeLabel} size="sm" className="absolute right-3 top-3">
            <X aria-hidden="true" className="size-3.5" />
          </IconButton>
        </DialogPrimitive.Close>
      </DialogPrimitive.Content>
    </DialogPortal>
  );
});

export const Sheet = DialogPrimitive.Root;
export const SheetTrigger = DialogPrimitive.Trigger;
export const SheetClose = DialogPrimitive.Close;
export const SheetTitle = DialogPrimitive.Title;
export const SheetDescription = DialogPrimitive.Description;

/** 抽屉内可滚动的正文区（固定标题栏 + 独立滚动是抽屉的常见形态）。 */
export function SheetBody({ className, ...props }: ComponentPropsWithoutRef<'div'>) {
  return <div className={cn('min-h-0 flex-1 overflow-y-auto pr-1', className)} {...props} />;
}
