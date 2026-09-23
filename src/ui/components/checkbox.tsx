import { forwardRef, useId } from 'react';

import * as CheckboxPrimitive from '@radix-ui/react-checkbox';
import { Check, Minus } from 'lucide-react';

import { cn } from '@/lib/utils';

/**
 * 复选框。
 *
 * 支持三态（`checked="indeterminate"`）——暂存区里"部分文件被勾选"的场景必需，
 * 三态用横线表示，并且**始终**有额外的文字说明（见 label 的使用约定），
 * 不让"半选"只靠一个符号传达。
 */
export interface CheckboxProps extends CheckboxPrimitive.CheckboxProps {
  readonly label?: string;
}

export const Checkbox = forwardRef<HTMLButtonElement, CheckboxProps>(function Checkbox(
  { className, label, id, ...props },
  ref,
) {
  const generatedId = useId();
  const checkboxId = id ?? generatedId;

  return (
    <div className="flex items-center gap-2">
      <CheckboxPrimitive.Root
        ref={ref}
        id={checkboxId}
        className={cn(
          'fd-transition flex size-4 shrink-0 items-center justify-center rounded-xs border border-line-strong bg-surface',
          'data-[state=checked]:border-brand data-[state=checked]:bg-brand data-[state=checked]:text-brand-fg',
          'data-[state=indeterminate]:border-brand data-[state=indeterminate]:bg-brand-subtle data-[state=indeterminate]:text-brand',
          'disabled:cursor-not-allowed disabled:opacity-50',
          'aria-[invalid=true]:border-danger',
          className,
        )}
        {...props}
      >
        <CheckboxPrimitive.Indicator className="flex items-center justify-center">
          {props.checked === 'indeterminate' ? (
            <Minus aria-hidden="true" className="size-3" />
          ) : (
            <Check aria-hidden="true" className="size-3" />
          )}
        </CheckboxPrimitive.Indicator>
      </CheckboxPrimitive.Root>

      {label !== undefined ? (
        <label htmlFor={checkboxId} className="text-13 text-fg">
          {label}
        </label>
      ) : null}
    </div>
  );
});
