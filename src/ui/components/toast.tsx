import * as ToastPrimitive from '@radix-ui/react-toast';
import { CircleCheck, CircleX, Info, TriangleAlert, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { ErrorToastContent } from '@/ui/components/error-toast';
import { IconButton } from '@/ui/components/icon-button';
import { useAppError } from '@/lib/errors';
import { useToastStore } from '@/stores/toastStore';
import type { ToastAction, ToastTone } from '@/stores/toastStore';
import { cn } from '@/lib/utils';

/**
 * 全局提示出口（Toast 队列的渲染端）。
 *
 * 设计约定：
 *   - **错误提示不会被自动关掉**（`duration: 0`，由 `useAppError()` 设置）：
 *     一闪而过的错误等于没提示。
 *   - 提示区固定在状态栏上方，而不是屏幕正中：它不该挡住用户正在看的内容。
 *   - 内容（标题/建议/详情/动作）由 [`ErrorToastContent`] 渲染，
 *     与"就地展示错误"的场景共用同一份排版。
 *   - 动作执行有反馈：`command` 型动作失败时会弹出新的错误提示，
 *     不会出现"点了没反应"（静默失败是用户最无法排查的形态）。
 *
 * 无障碍：Root 自带 `role="status"` + aria-live，内部可见文本会被读出；
 * 关闭按钮一定有 aria-label（来自上层 i18n，组件不产生用户可见文案）。
 *
 * 这里刻意**没有**使用 Radix 的 `Toast.Title` / `Toast.Description`：
 * 它们只是给内容加语义标签，若再用 sr-only 包一层标题，读屏软件会把同一句话念两遍。
 * 可见内容已经足够表达"发生了什么"，因此保持单一文本源。
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
  const { t } = useTranslation('errors');
  const toasts = useToastStore((state) => state.toasts);
  const dismissToast = useToastStore((state) => state.dismissToast);
  const { runAction } = useAppError();

  async function handleAction(action: ToastAction): Promise<void> {
    if (action.onClick !== undefined) {
      action.onClick();
      return;
    }
    if (action.command !== undefined) {
      await runAction({
        label: action.label,
        command: action.command,
        ...(action.args === undefined ? {} : { args: action.args }),
      });
    }
  }

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
            <ErrorToastContent
              title={toast.title}
              {...(toast.description === undefined ? {} : { hint: toast.description })}
              {...(toast.detail === undefined ? {} : { detail: toast.detail })}
              detailsLabel={t('details')}
              leading={
                <Icon
                  aria-hidden="true"
                  className={cn('mt-0.5 size-4 shrink-0', TONE_ICON_CLASS[toast.tone])}
                />
              }
              {...(toast.actions === undefined
                ? {}
                : {
                    actions: toast.actions.map((action) => ({
                      id: action.id,
                      label: action.label,
                    })),
                  })}
              onAction={(actionId) => {
                const action = toast.actions?.find((candidate) => candidate.id === actionId);
                if (action === undefined) {
                  return;
                }
                void handleAction(action);
              }}
            />

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
