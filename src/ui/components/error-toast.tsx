import type { ReactNode } from 'react';

import { Button } from '@/ui/components/button';
import { cn } from '@/lib/utils';

/**
 * 错误提示的**内容部分**（标题 / 建议 / 可折叠详情 / 修复动作）。
 *
 * 为什么单独成组件而不写死在 Toaster 里：
 *   同一份内容有两种使用场景——① 全局提示（Toaster 渲染 Radix Toast 外壳 + 本内容）；
 *   ② 无法弹全局提示的位置（抽屉内、对话框内）需要就地展示。
 *   拆出来后两边共用同一份排版与无障碍结构，不会出现"抽屉里的错误提示"格式不一致。
 *
 * 安全约定（红线 R8）：`detail` 必须是**已经脱敏**的文本（后端经 sanitize_log 处理，
 * 见 crates/diagnostics）。本组件不做脱敏，因为它拿不到必要的上下文；
 * 它只保证以纯文本渲染（`<pre>{detail}</pre>`，绝不 `dangerouslySetInnerHTML`）。
 *
 * 无障碍：详情用原生 `<details>`，键盘可达且读屏软件会播报展开状态；
 * 修复动作是真正的 `<button>`，可 Tab 到达并被 Enter/Space 触发。
 */
export interface ErrorToastContentProps {
  readonly title: string;
  /** 建议 / 可能原因。 */
  readonly hint?: string;
  /** 原始详情（已脱敏）。 */
  readonly detail?: string;
  /** "详情"二字由调用方传入，避免组件里写死语言。 */
  readonly detailsLabel?: string;
  readonly actions?: readonly {
    readonly id: string;
    readonly label: string;
    readonly disabled?: boolean;
  }[];
  onAction?(actionId: string): void;
  /** 标题前的内容（图标等）。 */
  readonly leading?: ReactNode;
  readonly className?: string;
}

export function ErrorToastContent({
  title,
  hint,
  detail,
  detailsLabel,
  actions,
  onAction,
  leading,
  className,
}: ErrorToastContentProps) {
  const hasActions = actions !== undefined && actions.length > 0;

  return (
    <div className={cn('flex min-w-0 flex-1 flex-col gap-0.5', className)}>
      <div className="flex items-start gap-2">
        {leading}
        <p className="min-w-0 text-13 font-medium text-fg">{title}</p>
      </div>

      {hint !== undefined ? <p className="text-12 text-fg-muted">{hint}</p> : null}

      {detail !== undefined ? (
        <details className="mt-1">
          <summary className="cursor-pointer text-12 text-fg-subtle">
            {detailsLabel ?? 'details'}
          </summary>
          <pre className="mt-1 max-h-40 overflow-auto whitespace-pre-wrap break-all rounded-sm border border-line bg-surface-sunken p-2 font-mono text-12 text-fg-muted">
            {detail}
          </pre>
        </details>
      ) : null}

      {hasActions ? (
        <div className="mt-1 flex flex-wrap items-center gap-2">
          {actions.map((action) => (
            <Button
              key={action.id}
              size="sm"
              variant="secondary"
              disabled={action.disabled ?? false}
              onClick={() => {
                onAction?.(action.id);
              }}
            >
              {action.label}
            </Button>
          ))}
        </div>
      ) : null}
    </div>
  );
}
