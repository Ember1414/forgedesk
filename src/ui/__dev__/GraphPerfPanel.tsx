/**
 * 提交图性能面板（T2.2，仅 DEV）。
 *
 * # 它消费谁
 *
 * `graphPerfStore` 的采样点分散在三处互不相通的地方：`useGraphQuery`（IPC 往返）、
 * `GraphCanvas`（静态层 / 动态层绘制耗时、fps、堆占用）。本面板是它们**唯一**的消费者，
 * 通过 store 解耦，因此可以独立挂载 / 卸载，而不必让 `HistoryPage` 把一堆数字用 props 传出。
 *
 * # 关于"布局耗时"的口径（务必如实）
 *
 * 面板上的 `layoutMs` 量的是 **`git_log_page` 的 IPC 往返时间**（含后端对**这一页**
 * ≤500 行的布局），**不是** Rust 全量布局耗时。后端基准（随机 DAG 最坏情况，
 * 2026-09-28，`cargo test --release -p forgedesk-domain -- --ignored`）：
 * 5000 节点 ~52ms、50000 ~7.2s、100000 ~26.4s——那组数字与首屏无关，
 * 绝不能拿来当"这个界面要 30 秒"的依据（真实仓库的并发泳道远少于随机 DAG）。
 * 面板底部固定显示这条说明，避免读数的人误判。
 *
 * # 关于堆内存
 *
 * `performance.memory` 是非标准扩展（Chromium 系才有）；不可用时 `readHeapMb()` 返回 `null`，
 * 面板显示 n/a（而不是印出 `NaN MB`）。这是任务明确要求的降级表现。
 *
 * 本文件所有可见文案都走 i18n key（`history.perf.*`），因此**不需要** `i18n-ignore-file`。
 */
import { useTranslation } from 'react-i18next';
import { X } from 'lucide-react';

import { cn } from '@/lib/utils';

import { IconButton } from '@/ui/components/icon-button';
import { useGraphPerfStore } from '@/features/history/graphPerfStore';

/** 把可空数值格式化成固定小数位；`null` / 非有限数显示 n/a。 */
function formatMetric(value: number | null, digits = 1): string {
  return value === null || !Number.isFinite(value) ? 'n/a' : value.toFixed(digits);
}

/** 面板正文：把当前快照渲染成一组"标签 + 数值"的行。 */
function PerfStats() {
  const { t } = useTranslation('shell');
  const snapshot = useGraphPerfStore((state) => state.snapshot);

  const lines = [
    t('history.perf.nodesLoaded', { count: snapshot.rowCount }),
    t('history.perf.nodesDrawn', { count: snapshot.nodeCount }),
    t('history.perf.edges', { count: snapshot.edgeCount }),
    t('history.perf.lanes', { count: snapshot.laneCount }),
    t('history.perf.scale', { percent: Math.round(snapshot.scale * 100) }),
    t('history.perf.layoutMs', { ms: formatMetric(snapshot.layoutMs) }),
    t('history.perf.staticDrawMs', { ms: formatMetric(snapshot.staticDrawMs) }),
    t('history.perf.dynamicDrawMs', { ms: formatMetric(snapshot.dynamicDrawMs) }),
    t('history.perf.fps', { fps: formatMetric(snapshot.fps, 0) }),
    snapshot.heapMb === null
      ? t('history.perf.heapUnavailable')
      : t('history.perf.heapMb', { mb: formatMetric(snapshot.heapMb) }),
  ];

  return (
    <div className="flex flex-col gap-0.5">
      {lines.map((line) => (
        <div key={line} className="flex items-baseline justify-between gap-3 font-mono text-11">
          <span className="whitespace-nowrap">{line}</span>
        </div>
      ))}
      <p className="mt-1 max-w-64 text-10 leading-snug text-fg-subtle">{t('history.perf.note')}</p>
    </div>
  );
}

export interface GraphPerfPanelProps {
  readonly className?: string;
  /** 关闭回调；省略时回退到"关掉采样开关"（悬浮形态用的就是它）。 */
  readonly onClose?: () => void;
}

/**
 * 悬浮形态：固定在右下角，浮在画布之上。
 *
 * 由 `HistoryPage` 在 `import.meta.env.DEV && enabled` 时挂载；生产构建里
 * 这个分支被静态替换成 `false`，面板代码不会进入产物。
 */
export function GraphPerfPanel({ className, onClose }: GraphPerfPanelProps) {
  const { t } = useTranslation('shell');
  const setEnabled = useGraphPerfStore((state) => state.setEnabled);

  return (
    <section
      aria-label={t('history.perf.title')}
      data-testid="graph-perf-panel"
      className={cn(
        'fixed bottom-4 right-4 z-50 w-72 rounded-lg border border-line bg-surface/95 p-3 shadow-lg backdrop-blur',
        className,
      )}
    >
      <header className="mb-2 flex items-center justify-between gap-2">
        <h2 className="text-12 font-semibold tracking-tight">{t('history.perf.title')}</h2>
        <IconButton
          label={t('history.perf.close')}
          size="sm"
          onClick={() => {
            if (onClose !== undefined) {
              onClose();
            } else {
              setEnabled(false);
            }
          }}
        >
          <X aria-hidden="true" className="size-3.5" />
        </IconButton>
      </header>
      <PerfStats />
    </section>
  );
}

/**
 * 独立路由形态（`__dev__/graph-perf`）：静态卡片，始终可见。
 *
 * 说明：性能采样发生在历史页的画布上，离开历史页后 rAF 循环停止，
 * 因此这里显示的是**最近一次**采样（若本次会话还没打开过历史页，则全为初始值 n/a）。
 */
export function GraphPerfRoute() {
  const { t } = useTranslation('shell');
  const enabled = useGraphPerfStore((state) => state.enabled);
  const toggleEnabled = useGraphPerfStore((state) => state.toggleEnabled);

  return (
    <section className="flex h-full flex-col gap-3">
      <header className="flex flex-wrap items-center justify-between gap-2">
        <h1 className="text-20 font-semibold tracking-tight">{t('history.perf.title')}</h1>
        <button
          type="button"
          aria-pressed={enabled}
          onClick={toggleEnabled}
          className="rounded-md border border-line px-2.5 py-1 text-12 hover:bg-surface-sunken"
        >
          {t('history.perf.toggle')}
        </button>
      </header>
      <div className="w-80 rounded-lg border border-line bg-surface p-3">
        <PerfStats />
      </div>
    </section>
  );
}
