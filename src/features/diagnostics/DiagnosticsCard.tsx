/**
 * 诊断卡片（T5.6）：把 DiagnosticReport 渲染成"人话 + 原因 + 修复动作"。
 *
 * - 置信度可视化：高/中/低（图标 + 文字，双通道表达）；
 * - 原因列表可展开（默认显示第一条）；
 * - 修复动作按 kind 分组：command 直接执行（白名单）、guide 跳转、
 *   dangerous 转交 DangerousActionDialog（fixActions.ts 的契约）；
 * - "原始错误"可折叠区（等宽、可复制）；"复制到搜索引擎"与"这个诊断不对"
 *   都需要用户点击才会离开应用；
 * - 备选诊断折叠为"可能原因"（只有标题，点击可切换主视图）。
 *
 * 文案全部来自 `diag.<id>.*`（shell 命名空间），卡片自身不产生文案。
 */
import { useCallback, useState } from 'react';

import {
  ChevronDown,
  ChevronRight,
  CircleAlert,
  CircleCheck,
  CircleHelp,
  Copy,
  Search,
  TriangleAlert,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { systemOpenUrl } from '@/lib/ipc';
import { pushToast } from '@/stores/toastStore';
import { runFixAction } from '@/features/diagnostics/fixActions';
import type { FixExecutionContext } from '@/features/diagnostics/fixActions';
import type { Diagnostic, DiagnosticReport } from '@/lib/ipc';
import { cn } from '@/lib/utils';

export interface DiagnosticsCardProps {
  readonly report: DiagnosticReport;
  /** 修复动作执行所需的仓库上下文与回调。 */
  readonly context: FixExecutionContext;
  /** 非原始错误的默认上下文（消歧）——卡片不做诊断，只展示。 */
  readonly className?: string;
}

function ConfidenceBadge({ confidence }: { readonly confidence: number }) {
  const { t } = useTranslation('shell');
  const level = confidence >= 0.9 ? 'high' : confidence >= 0.75 ? 'medium' : 'low';
  const map = {
    high: { icon: CircleCheck, className: 'text-success' },
    medium: { icon: CircleHelp, className: 'text-warning' },
    low: { icon: CircleAlert, className: 'text-danger' },
  } as const;
  const { icon: Icon, className } = map[level];
  return (
    <span className={cn('flex items-center gap-1 text-11 font-medium', className)}>
      <Icon aria-hidden="true" className="size-3" />
      {t(`terminal.diagnostics.confidence.${level}`)}
    </span>
  );
}

function SingleDiagnosis({
  diagnosis,
  onFix,
}: {
  readonly diagnosis: Diagnostic;
  readonly onFix: (fixIndex: number) => void;
}) {
  const { t } = useTranslation('shell');
  const [causesOpen, setCausesOpen] = useState(diagnosis.causes.length <= 1);

  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-start justify-between gap-2">
        <p className="text-13 font-medium text-fg">{t(diagnosis.titleKey)}</p>
        <ConfidenceBadge confidence={diagnosis.confidence} />
      </div>

      <p className="text-fg-muted text-12 leading-relaxed">{t(diagnosis.explanationKey)}</p>

      {diagnosis.causes.length > 0 ? (
        <div>
          <button
            type="button"
            className="text-fg-subtle flex items-center gap-1 text-12"
            aria-expanded={causesOpen}
            onClick={() => setCausesOpen((open) => !open)}
          >
            {causesOpen ? (
              <ChevronDown aria-hidden="true" className="size-3" />
            ) : (
              <ChevronRight aria-hidden="true" className="size-3" />
            )}
            {t('terminal.diagnostics.causes')}
          </button>
          {causesOpen ? (
            <ul className="text-fg-muted mt-1 list-disc ps-5 text-12 leading-relaxed">
              {diagnosis.causes.map((cause) => (
                <li key={cause}>{t(cause)}</li>
              ))}
            </ul>
          ) : null}
        </div>
      ) : null}

      {diagnosis.fixes.length > 0 ? (
        <div className="flex flex-wrap items-center gap-2">
          {diagnosis.fixes.map((fix, index) => (
            <button
              key={fix.id}
              type="button"
              className={cn(
                'fd-transition rounded-md border px-2.5 py-1 text-12',
                fix.action.kind === 'dangerous'
                  ? 'border-danger bg-surface text-danger hover:bg-danger/10'
                  : 'border-line bg-surface text-fg hover:bg-surface-sunken',
              )}
              onClick={() => onFix(index)}
            >
              {fix.action.kind === 'dangerous' ? (
                <TriangleAlert aria-hidden="true" className="me-1 inline size-3" />
              ) : null}
              {t(fix.labelKey)}
            </button>
          ))}
        </div>
      ) : null}
    </div>
  );
}

export function DiagnosticsCard({ report, context, className }: DiagnosticsCardProps) {
  const { t } = useTranslation('shell');
  const primary = report.primary;
  const [showRaw, setShowRaw] = useState(false);
  const [active, setActive] = useState<Diagnostic | null>(null);

  const handleFix = useCallback(
    (diagnosis: Diagnostic, fixIndex: number) => {
      const fix = diagnosis.fixes[fixIndex];
      if (!fix) {
        return;
      }
      void runFixAction(fix, context).catch((error) => {
        // 修复动作失败：走全局错误提示（含对新错误的递归诊断）
        pushToast({ tone: 'danger', title: String(error) });
      });
    },
    [context],
  );

  const copyRaw = useCallback(() => {
    void navigator.clipboard.writeText(report.rawSummary).then(() => {
      pushToast({ tone: 'info', title: t('terminal.diagnostics.copied') });
    });
  }, [report.rawSummary, t]);

  const copySearch = useCallback(() => {
    void navigator.clipboard.writeText(report.rawSummary).then(() => {
      // 搜索引擎需要用户点击打开（不自动离开应用）
      pushToast({ tone: 'info', title: t('terminal.diagnostics.copiedForSearch') });
    });
  }, [report.rawSummary, t]);

  if (primary === null) {
    return null;
  }

  const shown = active ?? primary;

  return (
    <div className={cn('border-line bg-surface-raised rounded-md border p-3', className)}>
      <SingleDiagnosis diagnosis={shown} onFix={(fixIndex) => handleFix(shown, fixIndex)} />

      {report.alternatives.length > 0 && active === null ? (
        <div className="border-line mt-2 border-t pt-2">
          <p className="text-fg-subtle text-11 font-medium">
            {t('terminal.diagnostics.alternatives')}
          </p>
          <div className="mt-1 flex flex-wrap gap-1">
            {report.alternatives.map((alternative) => (
              <button
                key={alternative.id}
                type="button"
                className="text-fg-muted border-line hover:bg-surface-sunken rounded border px-1.5 py-0.5 text-11"
                onClick={() => setActive(alternative)}
              >
                {t(alternative.titleKey)}
              </button>
            ))}
          </div>
        </div>
      ) : null}
      {active !== null ? (
        <button
          type="button"
          className="text-brand mt-2 text-11 hover:underline"
          onClick={() => setActive(null)}
        >
          {t('terminal.diagnostics.backToPrimary')}
        </button>
      ) : null}

      <div className="border-line mt-2 flex flex-wrap items-center gap-2 border-t pt-2">
        <button
          type="button"
          className="text-fg-subtle hover:text-fg flex items-center gap-1 text-11"
          aria-expanded={showRaw}
          onClick={() => setShowRaw((open) => !open)}
        >
          {showRaw ? (
            <ChevronDown aria-hidden="true" className="size-3" />
          ) : (
            <ChevronRight aria-hidden="true" className="size-3" />
          )}
          {t('terminal.diagnostics.rawError')}
        </button>
        <button
          type="button"
          className="text-fg-subtle hover:text-fg flex items-center gap-1 text-11"
          onClick={copyRaw}
        >
          <Copy aria-hidden="true" className="size-3" />
          {t('terminal.diagnostics.copyRaw')}
        </button>
        <button
          type="button"
          className="text-fg-subtle hover:text-fg flex items-center gap-1 text-11"
          onClick={copySearch}
        >
          <Search aria-hidden="true" className="size-3" />
          {t('terminal.diagnostics.copyForSearch')}
        </button>
        <button
          type="button"
          className="text-fg-subtle hover:text-fg flex items-center gap-1 text-11"
          onClick={() => void systemOpenUrlForFeedback().catch(() => {})}
        >
          <CircleAlert aria-hidden="true" className="size-3" />
          {t('terminal.diagnostics.wrongDiagnosis')}
        </button>
      </div>
      {showRaw ? (
        <pre className="border-line bg-surface-sunken mt-1 max-h-40 overflow-auto whitespace-pre-wrap break-all rounded-sm border p-2 font-mono text-12 text-fg-muted">
          {report.rawSummary}
        </pre>
      ) : null}
    </div>
  );
}

/** "这个诊断不对"：打开预填的 GitHub Issue（标题带标记，便于检索）。 */
function systemOpenUrlForFeedback(): Promise<void> {
  return systemOpenUrl(
    'https://github.com/Ember1414/forgedesk/issues/new?labels=diagnostics&title=%5Bdiagnostics%5D%20wrong%20diagnosis',
  );
}
