import { forwardRef } from 'react';

import * as RadioGroupPrimitive from '@radix-ui/react-radio-group';

import { cn } from '@/lib/utils';

/**
 * 单选组。
 *
 * 键盘行为由 Radix 保证：Tab 进入组内，方向键在选项间移动（roving tabindex），
 * 这是单选组的标准交互——用一组按钮假装单选会让键盘用户需要按 N 次 Tab。
 */
export interface RadioOption {
  readonly value: string;
  readonly label: string;
  readonly description?: string;
  readonly disabled?: boolean;
}

export interface RadioGroupProps extends RadioGroupPrimitive.RadioGroupProps {
  readonly options: readonly RadioOption[];
  /** 组标题（用于 role="group" 的无障碍名称）。 */
  readonly label: string;
}

export const RadioGroup = forwardRef<HTMLDivElement, RadioGroupProps>(function RadioGroup(
  { className, options, label, ...props },
  ref,
) {
  return (
    <RadioGroupPrimitive.Root
      ref={ref}
      aria-label={label}
      className={cn('flex flex-col gap-2', className)}
      {...props}
    >
      {options.map((option) => (
        <div key={option.value} className="flex items-start gap-2">
          <RadioGroupPrimitive.Item
            id={`${label}-${option.value}`}
            value={option.value}
            disabled={option.disabled ?? false}
            className={cn(
              'fd-transition mt-0.5 size-4 shrink-0 rounded-full border border-line-strong bg-surface',
              'data-[state=checked]:border-brand',
              'disabled:cursor-not-allowed disabled:opacity-50',
            )}
          >
            <RadioGroupPrimitive.Indicator className="flex h-full w-full items-center justify-center">
              <span aria-hidden="true" className="size-2 rounded-full bg-brand" />
            </RadioGroupPrimitive.Indicator>
          </RadioGroupPrimitive.Item>

          <div className="flex flex-col">
            <label htmlFor={`${label}-${option.value}`} className="text-13 text-fg">
              {option.label}
            </label>
            {option.description !== undefined ? (
              <span className="text-12 text-fg-subtle">{option.description}</span>
            ) : null}
          </div>
        </div>
      ))}
    </RadioGroupPrimitive.Root>
  );
});
