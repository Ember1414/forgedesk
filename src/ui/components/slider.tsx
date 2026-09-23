import { forwardRef } from 'react';

import * as SliderPrimitive from '@radix-ui/react-slider';

import { cn } from '@/lib/utils';

/**
 * 滑块。
 *
 * 用于"没有精确值的连续量"（缩放比例、并发数上限）。
 * 需要精确数值时请用 Input：滑块的键盘步进容易让用户反复按键却到不了目标值，
 * 因此这里始终渲染当前值文本，并在有 label 时建立无障碍关联。
 */
export interface SliderProps extends SliderPrimitive.SliderProps {
  readonly label?: string;
  /** 是否显示当前值（默认显示）。 */
  readonly showValue?: boolean;
  /** 自定义值格式化（如加百分号或单位）。 */
  readonly formatValue?: (value: number) => string;
}

export const Slider = forwardRef<HTMLSpanElement, SliderProps>(function Slider(
  { className, label, showValue = true, formatValue, ...props },
  ref,
) {
  const values = props.value ?? props.defaultValue ?? [0];

  return (
    <div className="flex w-full flex-col gap-1.5">
      {label !== undefined ? (
        <span className="text-12 font-medium text-fg-muted">{label}</span>
      ) : null}

      <div className="flex items-center gap-3">
        <SliderPrimitive.Root
          ref={ref}
          className={cn(
            'relative flex h-5 w-full touch-none select-none items-center data-[disabled]:opacity-50',
            className,
          )}
          {...props}
        >
          <SliderPrimitive.Track className="relative h-1 w-full grow rounded-full bg-surface-sunken">
            <SliderPrimitive.Range className="absolute h-full rounded-full bg-brand" />
          </SliderPrimitive.Track>
          {values.map((_, index) => (
            <SliderPrimitive.Thumb
              // 滑块的把手顺序稳定（数量由 value 决定），用索引作为 key 是可接受且最简的
              key={index}
              /**
               * aria-label 必须挂在**把手**上，而不是 Root。
               * role="slider" 在把手元素上，挂在 Root 会让读屏软件读出一个没有名称的滑块
               * （Root 上的 aria-label 对可聚焦元素不起作用）。
               */
              aria-label={label}
              className={cn(
                'fd-transition block size-3.5 rounded-full border border-brand bg-surface',
                'hover:border-brand-hover',
              )}
            />
          ))}
        </SliderPrimitive.Root>

        {showValue ? (
          <span className="w-12 shrink-0 text-right font-mono text-12 text-fg-muted">
            {formatValue === undefined ? String(values[0] ?? '') : formatValue(values[0] ?? 0)}
          </span>
        ) : null}
      </div>
    </div>
  );
});
