import type { HTMLAttributes, ReactNode } from 'react';

import { cva } from 'class-variance-authority';
import type { VariantProps } from 'class-variance-authority';

import { cn } from '@/lib/utils';

/**
 * 徽标：用于状态、计数、分支名等短标识。
 *
 * 无障碍约定：状态类徽标必须带**文字**（如"已通过"），颜色只是强化。
 * 只靠颜色区分状态对色觉障碍用户不可用，也会在打印/截图里丢失信息。
 * 组件因此不提供"纯色圆点"形态——那类需求请显式传 `srLabel` 自行兜底。
 */
const badgeVariants = cva(
  'inline-flex items-center gap-1 rounded-sm border px-1.5 py-0.5 text-12 font-medium',
  {
    variants: {
      tone: {
        neutral: 'border-line bg-surface-sunken text-fg-muted',
        brand: 'border-brand bg-brand-subtle text-brand',
        success: 'border-success bg-surface text-success',
        warning: 'border-warning bg-surface text-warning',
        danger: 'border-danger bg-surface text-danger',
        info: 'border-info bg-surface text-info',
      },
    },
    defaultVariants: { tone: 'neutral' },
  },
);

export interface BadgeProps
  extends HTMLAttributes<HTMLSpanElement>, VariantProps<typeof badgeVariants> {
  /** 状态图标（装饰性，会被置 aria-hidden）。 */
  readonly icon?: ReactNode;
  /** 仅给屏幕阅读器的补充说明（如抽屉里的"未读"）。 */
  readonly srLabel?: string;
}

export function Badge({ className, tone, icon, srLabel, children, ...props }: BadgeProps) {
  return (
    <span className={cn(badgeVariants({ tone }), className)} {...props}>
      {icon !== undefined ? <span aria-hidden="true">{icon}</span> : null}
      {children}
      {srLabel !== undefined ? <span className="sr-only">{srLabel}</span> : null}
    </span>
  );
}
