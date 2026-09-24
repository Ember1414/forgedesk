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
  /** 快照时刻的未跟踪文件路径（v1 只记录不恢复）。 */
  readonly untrackedPaths: readonly string[];
}

/** 快照与当前状态的差异摘要。 */
export interface SnapshotDiff {
  readonly headChanged: boolean;
  readonly indexChanged: boolean;
  readonly currentHeadOid: string | null;
  readonly currentIndexTreeOid: string | null;
  /** 锚点丢失——这个快照不可恢复。 */
  readonly refMissing: boolean;
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
