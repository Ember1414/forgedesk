import type { ReactNode } from 'react';

import { TriangleAlert } from 'lucide-react';

import { Button } from '@/ui/components/button';
import { cn } from '@/lib/utils';

/**
 * 错误态。
 *
 * 三件事必须同时出现，缺一个用户就只能来问人：
 *   1. **发生了什么**（title）；
 *   2. **可能的原因 / 建议动作**（hint）；
 *   3. **一个可点的出口**（retry / actions）。
 *
 * 安全约定（AGENTS.md 红线 R8）：`details` 只用于展示**已脱敏**的原始信息，
 * 调用方必须先把凭证类内容过滤掉（脱敏器在 M0/T0.6 落地，
 * 见 crates/domain 的 AppError.detail 约定）。组件不做脱敏，因为组件看不到上下文。
 */
export interface ErrorStateProps {
  readonly title: string;
  /** 可能原因或下一步建议。 */
  readonly hint?: string;
  /** 已脱敏的详细信息（默认折叠，避免技术细节淹没普通用户）。 */
  readonly details?: string;
  /** 可执行的动作按钮（重试、打开日志…）。 */
  readonly actions?: ReactNode;
  /** 重试回调（提供后显示"重试"按钮）。 */
  readonly onRetry?: () => void;
  /** 重试按钮的文案（必填才渲染按钮，文案来自上层 i18n）。 */
  readonly retryLabel?: string;
  readonly retryLoading?: boolean;
  readonly className?: string;
}

export function ErrorState({
  title,
  hint,
  details,
  actions,
  onRetry,
  retryLabel,
  retryLoading = false,
  className,
}: ErrorStateProps) {
  return (
    <div
      role="alert"
      className={cn(
        'flex flex-col gap-3 rounded-lg border border-danger bg-surface p-4',
        className,
      )}
    >
      <div className="flex items-start gap-2">
        <TriangleAlert aria-hidden="true" className="mt-0.5 size-4 shrink-0 text-danger" />
        <div className="flex flex-col gap-1">
          <h3 className="text-14 font-medium text-fg">{title}</h3>
          {hint !== undefined ? <p className="text-13 text-fg-muted">{hint}</p> : null}
        </div>
      </div>

      {details !== undefined ? (
        <details className="rounded-md border border-line bg-surface-sunken px-3 py-2">
          <summary className="cursor-pointer text-12 text-fg-muted">details</summary>
          <pre className="mt-2 overflow-x-auto whitespace-pre-wrap break-all font-mono text-12 text-fg-muted">
            {details}
          </pre>
        </details>
      ) : null}

      {onRetry !== undefined || actions !== undefined ? (
        <div className="flex flex-wrap items-center gap-2">
          {onRetry !== undefined && retryLabel !== undefined ? (
            <Button variant="secondary" size="sm" loading={retryLoading} onClick={onRetry}>
              {retryLabel}
            </Button>
          ) : null}
          {actions}
        </div>
      ) : null}
    </div>
  );
}
