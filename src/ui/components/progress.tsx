import type { ReactNode } from 'react';

import { cn } from '@/lib/utils';

/**
 * 进度条。
 *
 * 两种形态在行为上刻意不同：
 *   - 确定进度（value 为 0–1）：`role="progressbar"` + aria-valuenow，屏幕阅读器可读出百分比。
 *   - 不确定进度（value 为 null）：**不渲染** aria-valuenow，并使用条纹动画。
 *     后端还没给出总量时（如 fetch），进度条绝不能瞎动一个假数值——
 *     那会让用户误判还要等多久。
 */
export interface ProgressProps {
  /** 0–1 的进度；null 表示"进行中但总量未知"。 */
  readonly value: number | null;
  /** 无障碍名称（必填，来自上层 i18n）。 */
  readonly label: string;
  /** 是否显示右侧百分比文本。 */
  readonly showValue?: boolean;
  /** 附加说明（如"3 / 8 个对象"）。 */
  readonly hint?: ReactNode;
  readonly className?: string;
}

export function Progress({ value, label, showValue = true, hint, className }: ProgressProps) {
  const clamped = value === null ? null : Math.min(1, Math.max(0, value));

  return (
    <div className={cn('flex w-full flex-col gap-1', className)}>
      <div className="flex items-center justify-between gap-2">
        <span className="truncate text-12 text-fg-muted">{label}</span>
        {showValue ? (
          <span className="shrink-0 font-mono text-12 text-fg-subtle">
            {clamped === null ? '—' : `${String(Math.round(clamped * 100))}%`}
          </span>
        ) : null}
      </div>

      <div
        role="progressbar"
        aria-label={label}
        // 不确定进度时不给 aria-valuenow：读屏软件会据此播报"忙碌"而不是某个百分比
        {...(clamped === null
          ? {}
          : {
              'aria-valuenow': Math.round(clamped * 100),
              'aria-valuemin': 0,
              'aria-valuemax': 100,
            })}
        className="h-1.5 w-full overflow-hidden rounded-full bg-surface-sunken"
      >
        <div
          className={cn(
            // 用 tokens.css 里的慢速时长（320ms）：进度变化需要一点过渡才不显得突兀
            'h-full rounded-full bg-brand transition-[width] duration-300 ease-out',
            clamped === null && 'w-1/3 animate-pulse',
          )}
          style={clamped === null ? undefined : { width: `${String(clamped * 100)}%` }}
        />
      </div>

      {hint !== undefined ? <span className="text-12 text-fg-subtle">{hint}</span> : null}
    </div>
  );
}
