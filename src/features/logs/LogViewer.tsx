import { useMemo } from 'react';

import { useQuery } from '@tanstack/react-query';
import { RefreshCw } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { Button } from '@/ui/components/button';
import { EmptyState } from '@/ui/components/empty-state';
import { ErrorState } from '@/ui/components/error-state';
import { Skeleton } from '@/ui/components/skeleton';
import { logsTail } from '@/lib/ipc';
import type { LogLine } from '@/lib/ipc';
import { normalizeError } from '@/lib/errors';
import { cn } from '@/lib/utils';

/**
 * 日志查看器。
 *
 * 设计要点：
 *
 * 1. **高亮"错误发生时间附近"的行**：用户从错误提示点进来时，最想知道的是
 *    "刚才那一下到底发生了什么"。窗口取 ±60 秒——足够覆盖一次失败操作的完整过程
 *    （含重试与前后依赖），又不至于把整屏都标黄。
 * 2. **错误提示不吞掉原始信息**：这里是"原始日志"的出口，因此保留完整行（`raw`），
 *    结构化字段（时间/级别/target）只是用来排版与高亮。
 * 3. **脱敏提示常驻**：让用户放心地把日志贴到反馈里——这是"日志不含令牌"这一承诺
 *    在界面上的可见部分（实现见 crates/diagnostics）。
 */
const DEFAULT_LINES = 300;

/** 高亮窗口（毫秒）。 */
export const HIGHLIGHT_WINDOW_MS = 60_000;

/** 判断一行是否落在高亮窗口内（导出以便单测，避免只靠渲染断言）。 */
export function isNearTimestamp(
  lineTimestamp: number | null,
  anchor: number | null | undefined,
): boolean {
  if (lineTimestamp === null || anchor === null || anchor === undefined) {
    return false;
  }
  return Math.abs(lineTimestamp - anchor) <= HIGHLIGHT_WINDOW_MS;
}

/** 格式化器缓存（按 locale）：构造 Intl.DateTimeFormat 很贵，300 行日志逐行
 * 新建就是 300 次构造——这是设置页"日志区一滚就卡"的实测热点，必须复用。 */
const timeFormatters = new Map<string, Intl.DateTimeFormat>();

function timeFormatter(locale: string): Intl.DateTimeFormat {
  const cached = timeFormatters.get(locale);
  if (cached !== undefined) {
    return cached;
  }
  const formatter = new Intl.DateTimeFormat(locale, {
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    fractionalSecondDigits: 3,
    hour12: false,
  });
  timeFormatters.set(locale, formatter);
  return formatter;
}

/** 格式化时间：只显示到毫秒，日志里精确到秒往往不够定位。 */
function formatTime(timestamp: number | null, locale: string): string {
  if (timestamp === null) {
    return '—';
  }
  return timeFormatter(locale).format(new Date(timestamp));
}

const LEVEL_CLASS: Record<string, string> = {
  ERROR: 'text-danger',
  WARN: 'text-warning',
  INFO: 'text-info',
  DEBUG: 'text-fg-subtle',
  TRACE: 'text-fg-subtle',
};

export interface LogViewerProps {
  /** 需要高亮的时间点（通常是错误发生时间）。 */
  readonly nearTimestamp?: number | null;
  readonly className?: string;
}

export function LogViewer({ nearTimestamp, className }: LogViewerProps) {
  const { t, i18n } = useTranslation('shell');
  const locale = i18n.resolvedLanguage ?? 'zh-CN';

  const { data, isPending, isError, error, refetch, isFetching } = useQuery({
    queryKey: ['logs', 'tail', DEFAULT_LINES],
    queryFn: () => logsTail(DEFAULT_LINES),
    // 日志一直在增长：短期缓存足够避免重复往返，又不会显示过期内容
    staleTime: 5_000,
  });

  const highlighted = useMemo(() => {
    if (data === undefined || nearTimestamp === undefined || nearTimestamp === null) {
      return 0;
    }
    return data.filter((line) => isNearTimestamp(line.timestamp, nearTimestamp)).length;
  }, [data, nearTimestamp]);

  if (isError) {
    const normalized = normalizeError(error);
    // 非 AppError 的失败（例如 IPC 抛出的字符串）没有 detail 字段，
    // 这时 message 就是唯一的原始信息——不能因为"字段名不对"就把它丢掉
    const details = normalized.detail ?? normalized.message;
    return (
      <ErrorState
        title={t('logs.error')}
        hint={t('logs.errorHint')}
        {...(details === undefined || details === '' ? {} : { details })}
        retryLabel={t('logs.retry')}
        retryLoading={isFetching}
        {...(className === undefined ? {} : { className })}
        onRetry={() => {
          void refetch();
        }}
      />
    );
  }

  if (isPending) {
    return (
      <div className={cn('flex flex-col gap-1.5', className)}>
        <Skeleton className="h-4 w-2/3" />
        <Skeleton className="h-4 w-1/2" />
        <Skeleton className="h-4 w-3/4" />
      </div>
    );
  }

  if (data.length === 0) {
    return (
      <EmptyState
        title={t('logs.empty')}
        description={t('logs.emptyHint')}
        {...(className === undefined ? {} : { className })}
      />
    );
  }

  return (
    <div className={cn('flex min-h-0 flex-col gap-2', className)}>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <p className="text-12 text-fg-subtle">
          {t('logs.sanitizedNotice')}
          {highlighted > 0 ? (
            <span className="ml-1 text-brand">{t('logs.highlighted', { count: highlighted })}</span>
          ) : null}
        </p>
        <Button
          size="sm"
          variant="secondary"
          loading={isFetching}
          onClick={() => {
            void refetch();
          }}
        >
          <RefreshCw aria-hidden="true" className="size-3.5" />
          {t('logs.refresh')}
        </Button>
      </div>

      <ol className="min-h-0 flex-1 overflow-auto rounded-md border border-line bg-surface-sunken p-2 font-mono text-12 leading-relaxed">
        {data.map((line: LogLine, index) => {
          const near = isNearTimestamp(line.timestamp, nearTimestamp);
          return (
            <li
              key={`${String(line.timestamp ?? index)}-${String(index)}`}
              className={cn(
                'flex gap-2 whitespace-pre-wrap break-all border-l-2 border-transparent pl-2',
                near && 'border-brand bg-brand-subtle',
              )}
            >
              <span className="shrink-0 text-fg-subtle">{formatTime(line.timestamp, locale)}</span>
              <span
                className={cn(
                  'w-12 shrink-0',
                  line.level === null
                    ? 'text-fg-subtle'
                    : (LEVEL_CLASS[line.level] ?? 'text-fg-muted'),
                )}
              >
                {line.level ?? '—'}
              </span>
              <span className="min-w-0 flex-1 text-fg">{line.message}</span>
            </li>
          );
        })}
      </ol>
    </div>
  );
}
