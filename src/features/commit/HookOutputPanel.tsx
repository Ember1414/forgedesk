/**
 * 钩子输出面板（T1.8）。
 *
 * 三件事必须同时出现，缺一个用户就只能来问人：
 *
 * 1. **看起来是哪个阶段** —— 并且写明这是**推断**（git 不告诉调用方哪个钩子失败）；
 * 2. **哪几行是错误** —— 着色，但原文一字不改；
 * 3. **一个可操作的出口** —— 跳过钩子重试。
 *
 * 原文一字不改很重要：用户会把这段贴给同事或搜索引擎找答案，任何"整理"
 * 都会让那段文本搜不到自己。着色只是附加信息，不是替代品。
 */
import { useMemo } from 'react';

import { useTranslation } from 'react-i18next';

import { analyzeHookOutput } from '@/features/commit/hookOutput';
import type { HookLineKind, HookPhase } from '@/features/commit/hookOutput';
import { Button } from '@/ui/components/button';
import { cn } from '@/lib/utils';

/** 推断阶段 → i18n key（措辞里都带"看起来"，因为它确实是推断）。 */
const PHASE_LABEL_KEYS: Readonly<Record<HookPhase, string>> = {
  'code-check': 'commit.hooksPhaseCodeCheck',
  'message-check': 'commit.hooksPhaseMessageCheck',
  dependencies: 'commit.hooksPhaseDependencies',
  unknown: 'commit.hooksPhaseUnknown',
};

/** 行的类别 → 颜色 token 类（语义色，不写死颜色值）。 */
const LINE_TONE: Readonly<Record<HookLineKind, string>> = {
  error: 'text-danger',
  warning: 'text-warning',
  info: 'text-fg-subtle',
  plain: 'text-fg-muted',
};

export interface HookOutputPanelProps {
  /** git 的原始输出（已脱敏）。 */
  readonly output: string;
  /** 这次计划里会执行的钩子（来自计划本身，不是推断出来的）。 */
  readonly hooks: readonly string[];
  /** 提供后显示"跳过钩子重试"。 */
  readonly onSkipHooks?: () => void;
  readonly busy?: boolean;
}

export function HookOutputPanel({
  output,
  hooks,
  onSkipHooks,
  busy = false,
}: HookOutputPanelProps) {
  const { t } = useTranslation('shell');
  const report = useMemo(() => analyzeHookOutput(output), [output]);

  return (
    <section
      role="group"
      aria-label={t('commit.hooksTitle')}
      className="flex flex-col gap-2 rounded-md border border-danger bg-surface p-3"
    >
      <div className="flex flex-wrap items-baseline gap-2">
        <h3 className="text-13 font-medium text-fg">{t('commit.hooksTitle')}</h3>
        <span className="text-11 text-fg-subtle">{t('commit.hooksInferred')}</span>
      </div>

      <p className="text-13 text-fg">{t(PHASE_LABEL_KEYS[report.phase])}</p>

      <div className="flex flex-wrap items-center gap-3 text-11 text-fg-subtle">
        {report.errorCount > 0 ? (
          <span className="text-danger">
            {t('commit.hooksErrors', { count: report.errorCount })}
          </span>
        ) : null}
        {report.warningCount > 0 ? (
          <span className="text-warning">
            {t('commit.hooksWarnings', { count: report.warningCount })}
          </span>
        ) : null}
        {report.tools.length > 0 ? (
          <span>{t('commit.hooksTools', { list: report.tools.join(' · ') })}</span>
        ) : null}
        {hooks.length > 0 ? (
          <span>{t('commit.hooksPlanned', { list: hooks.join(' · ') })}</span>
        ) : null}
      </div>

      <pre className="max-h-64 overflow-auto rounded-md border border-line bg-surface-sunken px-3 py-2 font-mono text-12">
        {report.lines.map((line, index) => (
          <div key={index} className={cn(LINE_TONE[line.kind])}>
            {/* 空行也要占一行高度，否则输出会看起来"少了几行" */}
            {line.text === '' ? ' ' : line.text}
          </div>
        ))}
      </pre>

      {onSkipHooks !== undefined ? (
        <div className="flex items-center gap-2">
          <Button variant="secondary" size="sm" loading={busy} onClick={onSkipHooks}>
            {t('commit.hooksSkip')}
          </Button>
        </div>
      ) : null}
    </section>
  );
}
