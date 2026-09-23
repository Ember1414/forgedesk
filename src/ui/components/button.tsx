import { forwardRef } from 'react';
import type { ButtonHTMLAttributes } from 'react';

import { Slot } from '@radix-ui/react-slot';
import { cva } from 'class-variance-authority';
import type { VariantProps } from 'class-variance-authority';
import { LoaderCircle } from 'lucide-react';

import { cn } from '@/lib/utils';

/**
 * 按钮。
 *
 * 设计约束（T0.5）：
 *   - 颜色一律走语义 token（bg-brand / border-danger / text-fg…），组件内禁止出现十六进制色值；
 *     这样对比度由 scripts/design/check-contrast.mjs 统一守护，而不是每个组件各管一段。
 *   - 危险操作刻意用「描边 + 危险色文字」而不是实心红底：实心红底在暗色主题下需要另一套前景色，
 *     多一套配色就多一处对比度风险；描边形态在明暗两套主题下都天然可读。
 *   - loading 时既禁用交互，也置 aria-busy，屏幕阅读器才不会把"点不动"误解为"坏了"。
 */
const buttonVariants = cva(
  cn(
    'fd-transition inline-flex items-center justify-center gap-2 rounded-md font-medium',
    'disabled:pointer-events-none disabled:opacity-50',
  ),
  {
    variants: {
      variant: {
        primary: 'bg-brand text-brand-fg hover:bg-brand-hover',
        secondary:
          'border border-line bg-surface text-fg hover:border-line-strong hover:bg-surface-sunken',
        ghost: 'text-fg-muted hover:bg-surface-sunken hover:text-fg',
        danger: 'border border-danger bg-surface text-danger hover:bg-danger/10',
        link: 'text-brand underline-offset-4 hover:underline',
      },
      size: {
        sm: 'h-7 px-2.5 text-12',
        md: 'h-8 px-3 text-13',
        lg: 'h-10 px-4 text-14',
      },
    },
    defaultVariants: { variant: 'primary', size: 'md' },
  },
);

export interface ButtonProps
  extends ButtonHTMLAttributes<HTMLButtonElement>, VariantProps<typeof buttonVariants> {
  /** 加载态：禁用交互并显示进度指示（用于触发了后端长任务的按钮）。 */
  readonly loading?: boolean;
  /** 用子元素作为渲染宿主（如 `<Button asChild><a/></Button>`），样式透传而不嵌套按钮。 */
  readonly asChild?: boolean;
}

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(function Button(
  { className, variant, size, loading = false, asChild = false, disabled, children, ...props },
  ref,
) {
  const classes = cn(buttonVariants({ variant, size }), className);

  /**
   * asChild 单独分支，不做"统一处理"。
   *
   * 两个必须分开的原因（都踩过）：
   *   1. Slot 要求**恰好一个** React 元素子节点，而 loading 会额外插入一个 spinner，
   *      于是渲染期直接抛 "Slot failed to slot onto its children"；
   *   2. `disabled` 落到子元素（如 `<a>`）上是非法属性。
   * 因此 asChild 形态下忽略 loading（链接无法真正被禁用，硬渲染 spinner 只会误导用户）。
   */
  if (asChild) {
    return (
      <Slot ref={ref} className={classes} {...props}>
        {children}
      </Slot>
    );
  }

  return (
    <button
      ref={ref}
      disabled={disabled === true || loading}
      aria-busy={loading ? true : undefined}
      data-loading={loading ? '' : undefined}
      className={classes}
      {...props}
    >
      {loading ? <LoaderCircle aria-hidden="true" className="size-3.5 animate-spin" /> : null}
      {children}
    </button>
  );
});
