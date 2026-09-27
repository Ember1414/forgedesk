/**
 * 操作审计的前端封装（T1.11）。
 *
 * 三个命令：查历史、导出、按保留策略清理。**导出与清理本身也会被记录**——
 * "谁把历史倒出去过"是审计的一部分，因此界面上不需要（也不该）自己去写一条。
 */

import { invokeCommand } from './client';

/** 一条操作记录。 */
export interface AuditEntry {
  readonly id: number;
  /** 仓库 id；全局操作（克隆 / 初始化 / 导出）为 0。 */
  readonly repoId: number;
  /** 操作类型短名（`commit` / `stage` / …）。 */
  readonly opType: string;
  /** 参数摘要（已脱敏的 JSON 字符串）。 */
  readonly argsJson: string | null;
  /** 开始时间（Unix 毫秒）。 */
  readonly startedAtMs: number | null;
  /** 结束时间；`null` = 没有正常收尾（应用崩在写操作中间的特征）。 */
  readonly endedAtMs: number | null;
  /** 耗时（毫秒）。 */
  readonly durationMs: number | null;
  /** git 的退出码。 */
  readonly exitCode: number | null;
  /** 结果短名：`ok` / `failed` / `running`。 */
  readonly result: string;
  /** 失败摘要（已脱敏）。 */
  readonly stderrSummary: string | null;
  /** 关联的快照 id。 */
  readonly snapshotId: number | null;
  /** 是否可回滚（有快照才为真）。 */
  readonly reversible: boolean;
}

/** 一页操作记录。 */
export interface AuditPage {
  /** 满足筛选条件的总数（不受分页影响）。 */
  readonly total: number;
  readonly entries: readonly AuditEntry[];
}

/** 导出格式。 */
export type AuditExportFormat = 'csv' | 'json';

/** 导出结果。 */
export interface AuditExportResult {
  /** 写好的临时文件路径（用户据此另存；选目录要等 M7 的文件对话框）。 */
  readonly path: string;
  /** 导出条数。 */
  readonly rows: number;
  readonly format: string;
}

/** 清理结果。 */
export interface AuditPruneResult {
  readonly removed: number;
  /** 本次使用的保留天数。 */
  readonly retentionDays: number;
  /** 本次使用的保留条数上限。 */
  readonly retentionRows: number;
}

/** 筛选条件（缺省/`null` 表示不筛）。 */
export interface AuditFilter {
  readonly repoId?: number | null;
  readonly opType?: string | null;
  readonly fromMs?: number | null;
  readonly toMs?: number | null;
}

/** 把筛选条件转成命令参数（不传的项一律省略，后端按"不筛"处理）。 */
function filterArgs(filter: AuditFilter): Record<string, unknown> {
  const args: Record<string, unknown> = {};
  if (filter.repoId != null) {
    args.repoId = filter.repoId;
  }
  if (filter.opType != null && filter.opType !== '') {
    args.opType = filter.opType;
  }
  if (filter.fromMs != null) {
    args.fromMs = filter.fromMs;
  }
  if (filter.toMs != null) {
    args.toMs = filter.toMs;
  }
  return args;
}

/** 分页查询操作历史。 */
export function auditList(filter: AuditFilter, limit: number, offset: number): Promise<AuditPage> {
  return invokeCommand<AuditPage>('audit_list', {
    ...filterArgs(filter),
    limit,
    offset,
  });
}

/** 导出（CSV / JSON）到临时文件，返回文件路径。 */
export function auditExport(
  filter: AuditFilter,
  format: AuditExportFormat,
): Promise<AuditExportResult> {
  return invokeCommand<AuditExportResult>('audit_export', {
    ...filterArgs(filter),
    format,
  });
}

/** 按保留策略清理旧记录。 */
export function auditPrune(): Promise<AuditPruneResult> {
  return invokeCommand<AuditPruneResult>('audit_prune', {});
}
