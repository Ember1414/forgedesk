/**
 * 快照命令封装（T1.9）。
 *
 * 契约见 `docs/API.md` 的 `snapshot_*`。回滚是界面上破坏性最强的动作：
 * 前端的责任是"让用户先看差异摘要、再确认"；后端的闸门是回滚前自动打保护点
 * 与恢复后的一致性校验（失败会自动回到回滚前的状态）。
 */
import { invokeCommand } from './client';

/** 快照列表行。 */
export interface SnapshotMeta {
  readonly id: number;
  readonly label: string;
  /** 场景短名（`pre-commit` / `pre-restore` / `manual`…），走 i18n。 */
  readonly kind: string;
  /** 快照时刻的 HEAD oid（全量，界面自行截短）。 */
  readonly headOid: string;
  readonly branch: string | null;
  readonly detached: boolean;
  readonly createdAtMs: number;
}

/** 一次回滚的结果。 */
export interface RestoreReport {
  readonly restoredSnapshotId: number;
  readonly headOid: string;
  readonly indexTreeOid: string;
  /** 回滚前自动打的保护点——对回滚结果不满意可以再回到那里。 */
  readonly preRestoreSnapshotId: number | null;
  /** 快照时刻的未跟踪文件路径。 */
  readonly untrackedPaths: readonly string[];
  /** 从内容备份写回工作区的未跟踪文件数（T3.8；v1 快照恒为 0）。 */
  readonly untrackedRestored: number;
  /** 没能恢复的未跟踪文件（备份缺失、写不进去）。 */
  readonly untrackedFailed: readonly string[];
  /** 当前存在、快照里没有的未跟踪文件——**不会被删除**。 */
  readonly untrackedExtra: readonly string[];
  /** 恢复后的完整校验是否通过（HEAD / 索引 / 备份内容逐字节）。 */
  readonly verified: boolean;
}

/** 快照与当前状态的差异摘要。 */
export interface SnapshotDiff {
  readonly headChanged: boolean;
  readonly indexChanged: boolean;
  readonly currentHeadOid: string | null;
  readonly currentIndexTreeOid: string | null;
  /** 锚点丢失——这个快照不可恢复。 */
  readonly refMissing: boolean;
  /** 回滚会写回的未跟踪文件（有备份，且当前缺失或内容不同）。 */
  readonly untrackedRestorable: readonly string[];
  /** 快照里记录过、但没有内容备份的未跟踪文件——回滚**找不回来**。 */
  readonly untrackedMissing: readonly string[];
  /** 当前存在、快照里没有的未跟踪文件——回滚**不会删除**它们。 */
  readonly untrackedExtra: readonly string[];
}

/** 创建告警的类型短名（与后端 `SnapshotWarning::kind` 一致，界面按它走 i18n）。 */
export type SnapshotWarningKind =
  | 'untrackedBackupSkipped'
  | 'untrackedBackupPartial'
  | 'backupDirUnavailable'
  | 'spaceReclaimed'
  | 'orphansRemoved';

/**
 * 一条创建告警（**不是失败**：快照本身已经创建成功）。
 *
 * 字段是全的、按 `kind` 取用的扁平结构：界面因此不必处理"某个类型少了字段"。
 */
export interface SnapshotWarning {
  readonly kind: SnapshotWarningKind;
  readonly count: number | null;
  readonly bytes: number | null;
  readonly limit: number | null;
  readonly paths: readonly string[];
  readonly detail: string | null;
  readonly removed: readonly number[];
  readonly freedBytes: number | null;
}

/** 手动创建快照的结果（T3.8）。 */
export interface SnapshotOutcome {
  readonly id: number;
  readonly backupBytes: number;
  readonly backedUp: number;
  readonly untrackedTotal: number;
  /** 没有进入备份的未跟踪路径（超限或复制失败）。 */
  readonly skipped: readonly string[];
  readonly warnings: readonly SnapshotWarning[];
  readonly pruned: readonly number[];
}

/** 快照磁盘占用与配额。 */
export interface SnapshotUsage {
  readonly repoId: number;
  readonly snapshotCount: number;
  readonly backupBytes: number;
  /** `0` = 不限制。 */
  readonly maxSnapshotBytes: number;
  /** `0` = 不限制。 */
  readonly maxRepoBytes: number;
  /** 磁盘上存在、数据库里没有对应快照的目录。 */
  readonly orphanDirs: readonly string[];
}

/** 手动清理的结果。 */
export interface CleanupOutcome {
  readonly orphansRemoved: number;
  readonly reclaimed: readonly number[];
  readonly freedBytes: number;
  readonly remainingBytes: number;
}

/** 下一次快照的体积预估（危险操作对话框在动手前展示）。 */
export interface SnapshotEstimate {
  readonly untrackedCount: number;
  readonly untrackedBytes: number;
  readonly ignoredCount: number;
  readonly ignoredBytes: number;
  readonly includeIgnored: boolean;
  readonly limitBytes: number;
  readonly withinLimit: boolean;
  readonly wouldSkip: number;
}

/** 读取快照列表（新的在前）。 */
export function snapshotList(repoId: number, limit = 50): Promise<readonly SnapshotMeta[]> {
  return invokeCommand<readonly SnapshotMeta[]>('snapshot_list', { repoId, limit });
}

/** 读取快照与当前状态的差异。 */
export function snapshotDiff(repoId: number, snapshotId: number): Promise<SnapshotDiff> {
  return invokeCommand<SnapshotDiff>('snapshot_diff', { repoId, snapshotId });
}

/** 回滚到快照（成功后发布 `repo:changed`）。 */
export function snapshotRestore(repoId: number, snapshotId: number): Promise<RestoreReport> {
  return invokeCommand<RestoreReport>('snapshot_restore', { repoId, snapshotId });
}

/** 按保留策略清理旧快照，返回被清理的 id。 */
export function snapshotPrune(repoId: number): Promise<readonly number[]> {
  return invokeCommand<readonly number[]>('snapshot_prune', { repoId });
}

/**
 * 手动创建快照（T3.8）。
 *
 * 返回值是完整的结果——界面据此告诉用户"这次打点包含什么、不包含什么"，
 * 而不是只回一个 id 让用户自己猜。
 */
export function snapshotCreate(repoId: number, label?: string): Promise<SnapshotOutcome> {
  return invokeCommand<SnapshotOutcome>('snapshot_create', {
    repoId,
    ...(label === undefined ? {} : { label }),
  });
}

/** 快照磁盘占用与配额。 */
export function snapshotUsage(repoId: number): Promise<SnapshotUsage> {
  return invokeCommand<SnapshotUsage>('snapshot_usage', { repoId });
}

/** 下一次快照的体积预估（危险操作对话框用）。 */
export function snapshotEstimate(repoId: number): Promise<SnapshotEstimate> {
  return invokeCommand<SnapshotEstimate>('snapshot_estimate', { repoId });
}

/** 立即清理快照缓存（孤儿目录 + 保留策略 + 总占用回收）。 */
export function snapshotCleanup(repoId: number): Promise<CleanupOutcome> {
  return invokeCommand<CleanupOutcome>('snapshot_cleanup', { repoId });
}
