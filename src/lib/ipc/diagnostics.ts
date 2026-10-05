/**
 * 诊断引擎（T5.5/T5.6）的前端封装。
 *
 * 返回的诊断只有 i18n key（`diag.<id>.*`，在 shell 命名空间下），
 * 文案由组件渲染；`fix.action.kind = dangerous` 的动作**必须**经
 * DangerousActionDialog（含计划预览与快照说明），绝不允许一键直执行。
 */
import { invokeCommand } from './client';

/** 修复动作（元数据；执行在前端）。 */
export interface DiagFixAction {
  readonly kind: 'command' | 'guide' | 'dangerous';
  readonly command?: string;
  readonly args?: Record<string, unknown>;
  readonly value?: string;
}

/** 修复建议。 */
export interface DiagFix {
  readonly id: string;
  readonly labelKey: string;
  readonly action: DiagFixAction;
}

/** 一条命中的诊断。 */
export interface Diagnostic {
  readonly id: string;
  readonly confidence: number;
  readonly titleKey: string;
  readonly explanationKey: string;
  readonly causes: readonly string[];
  readonly fixes: readonly DiagFix[];
}

/** 诊断报告（`system_diagnose_error` 的返回）。 */
export interface DiagnosticReport {
  readonly primary: Diagnostic | null;
  readonly alternatives: readonly Diagnostic[];
  readonly rawSummary: string;
}

/** 诊断上下文（缺省字段 = 不关心）。 */
export interface DiagContextInput {
  readonly opType?: string;
  readonly upstream?: boolean;
  readonly detached?: boolean;
  readonly shallow?: boolean;
}

/** 诊断一段 stderr（后端截断到 8KB；规则内嵌 + 用户覆盖目录）。 */
export function systemDiagnoseError(
  stderr: string,
  context?: DiagContextInput,
): Promise<DiagnosticReport> {
  return invokeCommand<DiagnosticReport>('system_diagnose_error', {
    stderr,
    ...(context === undefined ? {} : { context }),
  });
}

/** 诊断命中的规则 id 清单（"诊断历史"的轻量指纹）。 */
export function systemDiagnoseKeys(
  stderr: string,
  context?: DiagContextInput,
): Promise<{ primary: string | null; alternatives: readonly string[] }> {
  return invokeCommand<{ primary: string | null; alternatives: readonly string[] }>(
    'system_diagnose_keys',
    { stderr, ...(context === undefined ? {} : { context }) },
  );
}
