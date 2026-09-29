import { forwardRef, useId } from 'react';
import type { InputHTMLAttributes, ReactNode } from 'react';

import { cn } from '@/lib/utils';

/**
 * 单行输入框。
 *
 * 约定：
 *   - 错误态由 `aria-invalid` 驱动（而不是额外的 className 传参），
 *     所以视觉与无障碍语义不会各说各话。
 *   - `label` 会生成 id 并用 htmlFor 关联；即使不显示 label（hideLabel），
 *     也仍然输出供屏幕阅读器使用的名称。
 */
export interface InputProps extends Omit<InputHTMLAttributes<HTMLInputElement>, 'aria-label'> {
  /** 可见标签文案。 */
  readonly label?: string;
  /** 仅给屏幕阅读器的名称（与 label 二选一，label 优先）。 */
  readonly srLabel?: string;
  /** 辅助说明（与输入框通过 aria-describedby 关联）。 */
  readonly hint?: ReactNode;
  /** 错误信息；存在时自动置 aria-invalid。 */
  readonly error?: string;
}

export const Input = forwardRef<HTMLInputElement, InputProps>(function Input(
  { className, label, srLabel, hint, error, id, ...props },
  ref,
) {
  const generatedId = useId();
  const inputId = id ?? generatedId;
  const hintId = `${inputId}-hint`;
  const hasHint = hint !== undefined && hint !== null;

  return (
    // min-w-0：在 flex 行里与按钮并排时，收缩压力由输入框承担——
    // 否则 w-full 的 wrapper 会把旁边的按钮挤到文字换行（仪表盘"浏览"按钮事故）
    <div className="flex w-full min-w-0 flex-col gap-1">
      {label !== undefined ? (
        <label htmlFor={inputId} className="text-12 font-medium text-fg-muted">
          {label}
        </label>
      ) : null}

      <input
        ref={ref}
        id={inputId}
        aria-label={label === undefined ? srLabel : undefined}
        aria-invalid={error !== undefined ? true : undefined}
        aria-describedby={hasHint ? hintId : undefined}
        className={cn(
          'fd-transition h-8 w-full rounded-md border border-line bg-surface px-2.5 text-13 text-fg',
          'placeholder:text-fg-subtle hover:border-line-strong',
          // focus 用 border + 1px outline 双保险：高 DPI 缩放下 WebView2 可能把
          // 1px 边框的某一条边渲染到亚像素而弱化（用户报告"紫色线缺左边"），
          // outline 沿边框外沿再画一圈，四边在任何缩放下都可见
          'focus:border-brand focus:outline focus:outline-1 focus:outline-brand/40',
          'disabled:cursor-not-allowed disabled:opacity-50',
          error !== undefined && 'border-danger focus:border-danger',
          className,
        )}
        {...props}
      />

      {hasHint ? (
        <p
          id={hintId}
          className={cn('text-12', error !== undefined ? 'text-danger' : 'text-fg-subtle')}
        >
          {error ?? hint}
        </p>
      ) : null}
    </div>
  );
});
