import type { ReactNode } from 'react';

import { cn } from '@/lib/utils';

/**
 * 空态。
 *
 * 设计约定：空态必须回答两个问题——"这里为什么是空的"和"我接下来能做什么"。
 * 因此 `title` 与 `description` 是必填的，`action` 强烈建议提供：
 * 一个只有插画和"暂无数据"的空页面，用户除了关掉它没有别的选择。
 */
export interface EmptyStateProps {
  /** 标题（必填）：说明这里本该有什么。 */
  readonly title: string;
  /** 说明（必填）：解释为什么现在是空的。 */
  readonly description: string;
  /** 主操作按钮（如"打开仓库""创建第一个标签"）。 */
  readonly action?: ReactNode;
  /** 次要说明（如命令行等价写法）。 */
  readonly footnote?: ReactNode;
  readonly className?: string;
}

export function EmptyState({ title, description, action, footnote, className }: EmptyStateProps) {
  return (
    <div
      className={cn(
        'flex flex-col items-center justify-center gap-2 rounded-lg border border-dashed border-line bg-surface px-6 py-10 text-center',
        className,
      )}
    >
      <h3 className="text-14 font-medium text-fg">{title}</h3>
      <p className="max-w-md text-13 text-fg-muted">{description}</p>
      {action !== undefined ? <div className="mt-1">{action}</div> : null}
      {footnote !== undefined ? (
        <p className="mt-1 font-mono text-12 text-fg-subtle">{footnote}</p>
      ) : null}
    </div>
  );
}
