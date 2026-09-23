import { forwardRef } from 'react';
import type { ButtonHTMLAttributes } from 'react';

import { cva } from 'class-variance-authority';
import type { VariantProps } from 'class-variance-authority';

import { cn } from '@/lib/utils';

/**
 * 图标按钮。
 *
 * 关键约定：`label` 是**必填**的，并被写成 aria-label。
 * 原因：只含图标的按钮如果没有无障碍名称，屏幕阅读器读出来是"按钮"，
 * 键盘用户也无法知道它做什么。把它做成类型层面的必填，比写在文档里靠人记更可靠。
 */
const iconButtonVariants = cva(
  cn(
    'fd-transition inline-flex shrink-0 items-center justify-center rounded-md',
    'disabled:pointer-events-none disabled:opacity-50',
  ),
  {
    variants: {
      variant: {
        ghost: 'text-fg-muted hover:bg-surface-sunken hover:text-fg',
        secondary:
          'border border-line bg-surface text-fg hover:border-line-strong hover:bg-surface-sunken',
        danger: 'text-danger hover:bg-danger/10',
      },
      size: {
        sm: 'size-6',
        md: 'size-8',
        lg: 'size-10',
      },
    },
    defaultVariants: { variant: 'ghost', size: 'md' },
  },
);

export interface IconButtonProps
  extends
    Omit<ButtonHTMLAttributes<HTMLButtonElement>, 'aria-label'>,
    VariantProps<typeof iconButtonVariants> {
  /** 无障碍名称（必填）。 */
  readonly label: string;
  /** 鼠标悬停提示文案；缺省时使用 label。 */
  readonly tooltip?: string;
}

export const IconButton = forwardRef<HTMLButtonElement, IconButtonProps>(function IconButton(
  { className, variant, size, label, tooltip, type = 'button', children, ...props },
  ref,
) {
  return (
    <button
      ref={ref}
      type={type}
      aria-label={label}
      title={tooltip ?? label}
      className={cn(iconButtonVariants({ variant, size }), className)}
      {...props}
    >
      {children}
    </button>
  );
});
