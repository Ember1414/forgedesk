import { forwardRef } from 'react';
import type { ComponentPropsWithoutRef, ComponentRef } from 'react';

import * as PopoverPrimitive from '@radix-ui/react-popover';

import { cn } from '@/lib/utils';

/**
 * 浮层面板（非模态）。
 *
 * 与 Dialog 的区别：Popover 不阻断流程，因此**不锁焦点**——
 * 用户可以点别处继续操作。适合"边看边操作"的场景（挑远端、选标签、快速筛选）。
 * 需要用户必须回应时用 Dialog，别用 Popover 硬凑。
 */
export const Popover = PopoverPrimitive.Root;
export const PopoverTrigger = PopoverPrimitive.Trigger;
export const PopoverAnchor = PopoverPrimitive.Anchor;
export const PopoverClose = PopoverPrimitive.Close;

export const PopoverContent = forwardRef<
  ComponentRef<typeof PopoverPrimitive.Content>,
  ComponentPropsWithoutRef<typeof PopoverPrimitive.Content>
>(function PopoverContent({ className, align = 'start', sideOffset = 6, ...props }, ref) {
  return (
    <PopoverPrimitive.Portal>
      <PopoverPrimitive.Content
        ref={ref}
        align={align}
        sideOffset={sideOffset}
        className={cn(
          'z-50 w-72 rounded-lg border border-line bg-surface-raised p-3 shadow-lg',
          className,
        )}
        {...props}
      />
    </PopoverPrimitive.Portal>
  );
});
