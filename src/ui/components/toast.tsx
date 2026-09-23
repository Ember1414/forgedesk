import * as ToastPrimitive from '@radix-ui/react-toast';
import { CircleCheck, CircleX, Info, TriangleAlert, X } from 'lucide-react';

import { IconButton } from '@/ui/components/icon-button';
import { useToastStore } from '@/stores/toastStore';
import type { ToastTone } from '@/stores/toastStore';
import { cn } from '@/lib/utils';

/**
 * 轻提示出口。
 *
 * 设计约定：
 *   - **错误提示不会被自动关掉**（tone=danger 时调用方传 duration: 0）：
 *     一闪而过的错误等于没提示。Radix 的 `duration={0}` 正对应"保持到用户处理"。
 *   - 提示区固定在状态栏上方，而不是屏幕正中：它不该挡住用户正在看的内容。
 *   - 关闭按钮一定有 aria-label（来自上层 i18n），Action 一定带 altText——
 *     这是 Radix Toast 的无障碍要求，缺了读屏软件读不出动作按钮。
 */
const TONE_ICONS: Record<ToastTone, typeof Info> = {
  info: Info,
  success: CircleCheck,
  warning: TriangleAlert,
  danger: CircleX,
};

const TONE_ICON_CLASS: Record<ToastTone, string> = {
  info: 'text-info',
  success: 'text-success',
  warning: 'text-warning',
  danger: 'text-danger',
};

export interface ToasterProps {
  /** 关闭按钮的无障碍名称（必填，来自上层 i18n）。 */
  readonly closeLabel: string;
}

export function Toaster({ closeLabel }: ToasterProps) {
  const toasts = useToastStore((state) => state.toasts);
  const dismissToast = useToastStore((state) => state.dismissToast);

  return (
    <ToastPrimitive.Provider swipeDirection="right">
      {toasts.map((toast) => {
        const Icon = TONE_ICONS[toast.tone];
        return (
          <ToastPrimitive.Root
            key={toast.id}
            open
            duration={toast.duration}
            onOpenChange={(open) => {
              if (!open) {
                dismissToast(toast.id);
              }
            }}
            className={cn(
              'fd-transition flex items-start gap-2 rounded-lg border border-line bg-surface-raised p-3 shadow-lg',
              'data-[state=closed]:opacity-0',
            )}
          >
            <Icon
              aria-hidden="true"
              className={cn('mt-0.5 size-4 shrink-0', TONE_ICON_CLASS[toast.tone])}
            />

            <div className="flex min-w-0 flex-1 flex-col gap-0.5">
              <ToastPrimitive.Title className="text-13 font-medium text-fg">
                {toast.title}
              </ToastPrimitive.Title>
              {toast.description !== undefined ? (
                <ToastPrimitive.Description className="text-12 text-fg-muted">
                  {toast.description}
                </ToastPrimitive.Description>
              ) : null}
              {toast.actionLabel !== undefined && toast.onAction !== undefined ? (
                <ToastPrimitive.Action
                  altText={toast.actionLabel}
                  onClick={() => {
                    toast.onAction?.();
                  }}
                  className="fd-transition mt-1 self-start rounded-sm border border-line px-2 py-0.5 text-12 text-fg hover:bg-surface-sunken"
                >
                  {toast.actionLabel}
                </ToastPrimitive.Action>
              ) : null}
            </div>

            <ToastPrimitive.Close asChild>
              <IconButton label={closeLabel} size="sm">
                <X aria-hidden="true" className="size-3" />
              </IconButton>
            </ToastPrimitive.Close>
          </ToastPrimitive.Root>
        );
      })}

      <ToastPrimitive.Viewport className="fixed bottom-9 right-3 z-50 flex w-80 max-w-[calc(100vw-1.5rem)] flex-col gap-2" />
    </ToastPrimitive.Provider>
  );
}
