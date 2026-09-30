/**
 * rebase 命令（T3.5 计划模型 / T3.7 执行引擎的 IPC 封装；T3.6 面板使用）。
 *
 * 契约要点：
 * - 两段式：preview 只读（校验 + 预测结果），execute 才动仓库；
 * - 暂停是**结果不是错误**：`pausedConflict` 走冲突页（T3.1 状态机），
 *   `pausedEdit` 由 `gitRebaseContinueEdit` 恢复；
 * - 形状与 Rust 侧 `RebasePreviewDto` / `RebaseOutcomeDto` 的 serde 契约
 *   一一对应（camelCase；`Option` 序列化为 **null** 不是 undefined）；
 * - 区间清单必须来自 `gitRebaseRange`（不能用界面已加载的分页数据推断：
 *   todo 漏列的区间提交会被 git 静默丢弃）。
 */
import { invokeCommand } from './client';

/** 重排动作（Rust 侧 `ReorderAction` 的 camelCase）。 */
export type ReorderAction = 'pick' | 'reword' | 'edit' | 'squash' | 'fixup' | 'drop';

/** 计划里一条步骤（顺序 = 执行顺序，从旧到新）。 */
export interface RebaseStepRequest {
  readonly oid: string;
  /** reword / squash 的新信息草案；其余动作为 undefined。 */
  readonly newMessage?: string;
  readonly action: ReorderAction;
}

/** preview / execute 的请求体（`{ base, head, steps[], allowFlattenMerges?, autosquash? }`）。 */
export interface RebaseExecuteRequest {
  readonly base: string;
  readonly head: string;
  readonly steps: readonly RebaseStepRequest[];
  /** 允许压平 merge 提交（会丢弃合并结构；默认关闭）。 */
  readonly allowFlattenMerges?: boolean;
  /** todo 启用 autosquash 语义。 */
  readonly autosquash?: boolean;
}

/** 预演里一条存活提交（`oid` 是重写前占位）。 */
export interface RebaseSurvivingCommit {
  readonly oid: string;
  readonly subject: string;
}

/** 预览结果（`RebasePreviewDto`）。 */
export interface RebasePreview {
  readonly surviving: readonly RebaseSurvivingCommit[];
  /** 被丢弃的提交 oid。 */
  readonly dropped: readonly string[];
  /** 信息被改写的提交 oid。 */
  readonly reworded: readonly string[];
  /** 被并入其他提交的记录（"本条 -> 归入哪条"）。 */
  readonly squashed: readonly string[];
  /** 受影响的提交总数。 */
  readonly affectedCount: number;
  /** 区间内有已推送提交将被重写（界面必须提示需要 force-with-lease）。 */
  readonly touchesPushed: boolean;
  /** 等价 `git rebase -i` todo 内容（面板底部展示）。 */
  readonly todoText: string;
}

/** 区间清单的一条提交（`git_rebase_range`）。 */
export interface RebaseRangeCommit {
  readonly oid: string;
  readonly parents: readonly string[];
  readonly subject: string;
  /** 作者名（面板每项展示）。 */
  readonly author: string;
  /** 作者时间（Unix 秒；用 `absoluteTime` 之类按本地时区格式化）。 */
  readonly authorTime: number;
}

/** rebase 结局：三种都是正常返回值（暂停不是错误）。 */
export type RebaseOutcome =
  | {
      readonly kind: 'completed';
      readonly oid: string;
      /** 执行前的 pre-head-move 快照 id；null = 快照失败，没有回滚点。 */
      readonly snapshotId: number | null;
    }
  | {
      readonly kind: 'pausedConflict';
      readonly conflicts: readonly string[];
      readonly snapshotId: number | null;
    }
  | {
      readonly kind: 'pausedEdit';
      readonly oid: string;
      readonly snapshotId: number | null;
    };

/** 预演 rebase 计划（只读：装区间图 → 校验 → 预览）。 */
export function gitRebasePreviewOnly(
  repoId: number,
  spec: RebaseExecuteRequest,
): Promise<RebasePreview> {
  return invokeCommand<RebasePreview>('git_rebase_preview_only', { repoId, spec });
}

/** 列出区间的全部提交（只读；从旧到新、拓扑序）。 */
export function gitRebaseRange(
  repoId: number,
  base: string,
  head: string,
): Promise<RebaseRangeCommit[]> {
  return invokeCommand<RebaseRangeCommit[]>('git_rebase_range', { repoId, base, head });
}

/** 执行 rebase 计划（重写历史；执行前打 pre-head-move 快照）。 */
export function gitRebaseExecute(
  repoId: number,
  spec: RebaseExecuteRequest,
): Promise<RebaseOutcome> {
  return invokeCommand<RebaseOutcome>('git_rebase_execute', { repoId, spec });
}

/** edit 暂停的恢复（amend 接住暂存改动 + rebase --continue；幂等）。 */
export function gitRebaseContinueEdit(repoId: number): Promise<RebaseOutcome> {
  return invokeCommand<RebaseOutcome>('git_rebase_continue_edit', { repoId });
}
