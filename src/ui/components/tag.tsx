import type { HTMLAttributes } from 'react';

import { X } from 'lucide-react';

import { IconButton } from '@/ui/components/icon-button';
import { cn } from '@/lib/utils';

/**
 * 标签：可筛选、可删除的分类标识（分支标签、Issue 标签、路径过滤条件）。
 *
 * 与 Badge 的区别是**交互性**：Badge 是只读状态，Tag 可以被点选筛选或被删除。
 * 需要"点一下用来筛选"时请传 onClick，此时它会渲染为 button 而不是 span，
 * 键盘可达性由原生元素保证。
 */
export interface TagProps extends Omit<HTMLAttributes<HTMLSpanElement>, 'onClick'> {
  /** 点选筛选（传入后渲染为按钮）。 */
  readonly onClick?: () => void;
  /** 删除回调；传入后显示删除按钮。 */
  readonly onRemove?: () => void;
  /** 删除按钮的无障碍名称（必填，来自上层 i18n，例如"移除标签 后端"）。 */
  readonly removeLabel?: string;
  /** 选中态（用于筛选场景）。 */
  readonly selected?: boolean;
}

export function Tag({
  className,
  onClick,
  onRemove,
  removeLabel,
  selected = false,
  children,
  ...props
}: TagProps) {
  const baseClassName = cn(
    'fd-transition inline-flex items-center gap-1 rounded-sm border px-1.5 py-0.5 text-12',
    selected
      ? 'border-brand bg-brand-subtle font-medium text-brand'
      : 'border-line bg-surface-sunken text-fg-muted',
    onClick !== undefined && 'cursor-pointer',
    className,
  );

  return (
    <span
      className={cn('inline-flex items-center', onRemove !== undefined && 'gap-0.5')}
      {...props}
    >
      {onClick !== undefined ? (
        <button
          type="button"
          onClick={onClick}
          aria-pressed={selected}
          className={cn(baseClassName, 'hover:border-line-strong hover:text-fg')}
        >
          {children}
        </button>
      ) : (
        <span className={baseClassName}>{children}</span>
      )}

      {onRemove !== undefined ? (
        <IconButton label={removeLabel ?? 'remove'} size="sm" className="size-5" onClick={onRemove}>
          <X aria-hidden="true" className="size-3" />
        </IconButton>
      ) : null}
    </span>
  );
}
