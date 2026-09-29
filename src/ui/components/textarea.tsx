import { forwardRef, useId } from 'react';
import type { ReactNode, TextareaHTMLAttributes } from 'react';

import { cn } from '@/lib/utils';

/**
 * 多行输入框。
 *
 * 与 Input 保持一致的约定（label/hint/error + aria 关联），
 * 因为提交信息、PR 描述等场景会让用户在两者之间来回切换，
 * 行为一致才能避免"这个框的错误提示怎么没有"这类困惑。
 */
export interface TextareaProps extends Omit<
  TextareaHTMLAttributes<HTMLTextAreaElement>,
  'aria-label'
> {
  readonly label?: string;
  readonly srLabel?: string;
  readonly hint?: ReactNode;
  readonly error?: string;
  /** 显示已输入字符数（用于有长度限制的场景，如提交标题）。 */
  readonly showCount?: boolean;
}

export const Textarea = forwardRef<HTMLTextAreaElement, TextareaProps>(function Textarea(
  { className, label, srLabel, hint, error, showCount = false, id, value, maxLength, ...props },
  ref,
) {
  const generatedId = useId();
  const textareaId = id ?? generatedId;
  const hintId = `${textareaId}-hint`;
  const hasHint = hint !== undefined && hint !== null;
  const length = typeof value === 'string' ? value.length : 0;

  return (
    <div className="flex w-full flex-col gap-1">
      {label !== undefined ? (
        <label htmlFor={textareaId} className="text-12 font-medium text-fg-muted">
          {label}
        </label>
      ) : null}

      <textarea
        ref={ref}
        id={textareaId}
        value={value}
        maxLength={maxLength}
        aria-label={label === undefined ? srLabel : undefined}
        aria-invalid={error !== undefined ? true : undefined}
        aria-describedby={hasHint ? hintId : undefined}
        className={cn(
          'fd-transition min-h-20 w-full resize-y rounded-md border border-line bg-surface px-2.5 py-1.5 text-13 text-fg',
          'placeholder:text-fg-subtle hover:border-line-strong',
          // 与 Input 同一策略：focus 时 border + outline 双保险（高 DPI 下
          // 1px 边框可能单边被亚像素渲染弱化）
          'focus:border-brand focus:outline focus:outline-1 focus:outline-brand/40',
          'disabled:cursor-not-allowed disabled:opacity-50',
          error !== undefined && 'border-danger focus:border-danger',
          className,
        )}
        {...props}
      />

      {hasHint || showCount ? (
        <div className="flex items-start justify-between gap-2">
          <p
            id={hasHint ? hintId : undefined}
            className={cn('text-12', error !== undefined ? 'text-danger' : 'text-fg-subtle')}
          >
            {error ?? hint}
          </p>
          {showCount ? (
            <span className="shrink-0 font-mono text-12 text-fg-subtle">
              {length}
              {maxLength === undefined ? '' : ` / ${String(maxLength)}`}
            </span>
          ) : null}
        </div>
      ) : null}
    </div>
  );
});
