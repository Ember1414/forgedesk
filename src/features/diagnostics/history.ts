/**
 * 诊断历史（T5.6）：最近 20 次命中的诊断，供设置页排查"反复出现的同一问题"。
 *
 * 为什么在 localStorage 而不是 settings 表：这是**诊断性的便签数据**（与
 * 终端命令历史同一性质）——不值得占一条 IPC 往返与数据库行；上限 20 条，
 * 写满淘汰最旧。stderr 只留**尾部 200 字符**（已是脱敏文本）用于辨认，
 * 不是完整日志。
 */
import type { DiagnosticReport } from '@/lib/ipc';

/** 一条历史记录。 */
export interface DiagnosisRecord {
  /** 记录时间（Unix 毫秒）。 */
  readonly at: number;
  /** 命中的规则 id（无 primary 时为 null）。 */
  readonly ruleId: string | null;
  /** 备选规则的 id（可能原因）。 */
  readonly alternatives: readonly string[];
  /** stderr 尾部（已脱敏；辨认用）。 */
  readonly stderrTail: string;
}

const HISTORY_KEY = 'forgedesk.diagnostics.history';
const HISTORY_LIMIT = 20;
const STDERR_TAIL_LIMIT = 200;

/** 记录一次诊断（只在 primary 命中时调用；静默失败）。 */
export function recordDiagnosis(report: DiagnosticReport, stderr: string): void {
  try {
    const record: DiagnosisRecord = {
      at: Date.now(),
      ruleId: report.primary?.id ?? null,
      alternatives: report.alternatives.map((d) => d.id),
      stderrTail: stderr.slice(-STDERR_TAIL_LIMIT),
    };
    const history = readDiagnosisHistory();
    history.push(record);
    localStorage.setItem(HISTORY_KEY, JSON.stringify(history.slice(-HISTORY_LIMIT)));
  } catch {
    // localStorage 满 / 被禁用：历史是排查便利功能，失败即放弃
  }
}

/** 读取全部历史（旧 → 新）。 */
export function readDiagnosisHistory(): DiagnosisRecord[] {
  try {
    const raw = localStorage.getItem(HISTORY_KEY);
    if (raw === null) {
      return [];
    }
    const parsed: unknown = JSON.parse(raw);
    return Array.isArray(parsed) ? parsed.filter(isRecord) : [];
  } catch {
    return [];
  }
}

function isRecord(value: unknown): value is DiagnosisRecord {
  if (typeof value !== 'object' || value === null) {
    return false;
  }
  const record = value as Record<string, unknown>;
  return (
    typeof record['at'] === 'number' &&
    (typeof record['ruleId'] === 'string' || record['ruleId'] === null) &&
    typeof record['stderrTail'] === 'string'
  );
}

/** 清空历史（设置页按钮）。 */
export function clearDiagnosisHistory(): void {
  try {
    localStorage.removeItem(HISTORY_KEY);
  } catch {
    // 忽略：清不掉的历史只是多留一会儿
  }
}
