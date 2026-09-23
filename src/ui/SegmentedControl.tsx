import { cn } from '@/lib/utils';

/**
 * 分段控件（M0 临时实现）。
 *
 * 说明：T0.5 会基于 Radix 原语提供正式组件库（ToggleGroup / Tabs），
 * 本组件只覆盖 M0 已出现的三处用法：主题切换、语言切换、详情面板位置。
 * 到 T0.5 时请用它替换并删除本文件，避免两份实现长期并存。
 *
 * 无障碍：用 role="group" + aria-pressed 表达"多选一"的开关态。
 * 之所以不用 role="tablist"：这些控件切换的是设置值而不是内容面板，
 * 用 tab 语义会让键盘用户期待不存在的面板切换行为。
 */
export interface SegmentedOption<TValue extends string> {
  readonly value: TValue;
  /** 已本地化的显示文案。 */
  readonly label: string;
}

export interface SegmentedControlProps<TValue extends string> {
  /** 控件整体用途（用于 aria-label）。 */
  readonly label: string;
  readonly value: TValue;
  readonly options: readonly SegmentedOption<TValue>[];
  onChange(value: TValue): void;
  readonly className?: string;
}

export function SegmentedControl<TValue extends string>({
  label,
  value,
  options,
  onChange,
  className,
}: SegmentedControlProps<TValue>) {
  return (
    <div
      role="group"
      aria-label={label}
      className={cn(
        'inline-flex items-center gap-0.5 rounded-md border border-line bg-surface p-0.5',
        className,
      )}
    >
      {options.map((option) => {
        const selected = option.value === value;
        return (
          <button
            key={option.value}
            type="button"
            aria-pressed={selected}
            onClick={() => {
              onChange(option.value);
            }}
            className={cn(
              'fd-transition rounded-sm px-2.5 py-1 text-12',
              selected
                ? 'bg-brand font-medium text-brand-fg'
                : 'text-fg-muted hover:bg-surface-sunken hover:text-fg',
            )}
          >
            {option.label}
          </button>
        );
      })}
    </div>
  );
}
