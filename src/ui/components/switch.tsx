import { forwardRef, useId } from 'react';

import * as SwitchPrimitive from '@radix-ui/react-switch';

import { cn } from '@/lib/utils';

/**
 * 开关。
 *
 * 只用于**立即生效**的布尔设置（如"自动获取远程更新"）。
 * 需要"改完再保存"的场景应该用 Checkbox + 保存按钮，
 * 否则用户无法判断这个开关到底生效了没有。
 */
export interface SwitchProps extends SwitchPrimitive.SwitchProps {
  readonly label?: string;
}

export const Switch = forwardRef<HTMLButtonElement, SwitchProps>(function Switch(
  { className, label, id, ...props },
  ref,
) {
  const generatedId = useId();
  const switchId = id ?? generatedId;

  return (
    <div className="flex items-center gap-2">
      <SwitchPrimitive.Root
        ref={ref}
        id={switchId}
        className={cn(
          'fd-transition inline-flex h-5 w-9 shrink-0 items-center rounded-full border border-line-strong bg-surface-sunken',
          'data-[state=checked]:border-brand data-[state=checked]:bg-brand',
          'disabled:cursor-not-allowed disabled:opacity-50',
          className,
        )}
        {...props}
      >
        <SwitchPrimitive.Thumb
          className={cn(
            'fd-transition block size-4 rounded-full bg-surface shadow-sm',
            'translate-x-0.5 data-[state=checked]:translate-x-4',
            'data-[state=checked]:bg-brand-fg',
          )}
        />
      </SwitchPrimitive.Root>

      {label !== undefined ? (
        <label htmlFor={switchId} className="text-13 text-fg">
          {label}
        </label>
      ) : null}
    </div>
  );
});
