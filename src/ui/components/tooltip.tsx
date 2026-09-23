import { forwardRef } from 'react';
import type { ComponentPropsWithoutRef, ComponentRef, ReactElement, ReactNode } from 'react';

import * as TooltipPrimitive from '@radix-ui/react-tooltip';

import { cn } from '@/lib/utils';

/**
 * 提示气泡。
 *
 * 使用约定（重要）：Tooltip **只用于补充说明**，绝不能是唯一的标签来源。
 * 气泡在触摸设备与屏幕阅读器里都可能拿不到，所以触发器本身必须已经有可读文本
 * 或 aria-label（例如 IconButton 的 label）。
 *
 * `TooltipProvider` 需要挂在应用根部（Radix 用它统一管理延迟与全局快捷键），
 * 已在 `src/app/App.tsx` 中挂载。
 */
export const TooltipProvider = TooltipPrimitive.Provider;
export const Tooltip = TooltipPrimitive.Root;
export const TooltipTrigger = TooltipPrimitive.Trigger;

export const TooltipContent = forwardRef<
  ComponentRef<typeof TooltipPrimitive.Content>,
  ComponentPropsWithoutRef<typeof TooltipPrimitive.Content>
>(function TooltipContent({ className, sideOffset = 6, ...props }, ref) {
  return (
    <TooltipPrimitive.Portal>
      <TooltipPrimitive.Content
        ref={ref}
        sideOffset={sideOffset}
        className={cn(
          'z-50 max-w-64 rounded-md border border-line bg-surface-raised px-2 py-1 text-12 text-fg shadow-md',
          className,
        )}
        {...props}
      />
    </TooltipPrimitive.Portal>
  );
});

export interface TipProps extends Omit<ComponentPropsWithoutRef<typeof TooltipContent>, 'content'> {
  /** 触发器元素（通常是 IconButton 或按钮）。 */
  readonly children: ReactElement;
  /** 提示内容。 */
  readonly content: ReactNode;
}

/**
 * 便捷形态：把触发器与内容一次性包好，避免每个使用点都写三行 Provider/Trigger/Content。
 */
export function Tip({ children, content, ...contentProps }: TipProps) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>{children}</TooltipTrigger>
      <TooltipContent {...contentProps}>{content}</TooltipContent>
    </Tooltip>
  );
}
