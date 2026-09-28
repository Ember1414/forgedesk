/**
 * 分支与标签管理（T2.5）：命令 DTO 与具名封装。
 *
 * # 类型来源
 *
 * 镜像 `crates/domain/src/git/spec.rs`（spec，camelCase wire）与
 * `crates/services/src/branch.rs`（`BranchComparison` / `BranchDeleteOutcome`）；
 * `Branch` / `Tag` 镜像 `domain::git::refs`。改任何一侧必须同步另一侧。
 */
import { invokeCommand } from './client';
import type { Branch } from './history';

// ---------------------------------------------------------------- DTO

/** 切换策略（与后端 `SwitchStrategy` 的 serde 形状一致）。 */
export type SwitchStrategy = 'stash' | 'force' | 'clean';

/** 新建分支参数。 */
export interface BranchCreateSpec {
  readonly name: string;
  readonly startPoint?: string | null;
  readonly checkout: boolean;
  readonly trackUpstream?: string | null;
}

/** 重命名分支参数。 */
export interface BranchRenameSpec {
  readonly old: string;
  readonly new: string;
  readonly renameRemote: boolean;
}

/** 删除分支参数。 */
export interface BranchDeleteSpec {
  readonly names: readonly string[];
  readonly force: boolean;
  readonly alsoDeleteRemote: boolean;
}

/** 设置 / 取消上游参数（`upstream` 缺省 = 取消）。 */
export interface BranchSetUpstreamSpec {
  readonly branch: string;
  readonly upstream?: string | null;
}

/** 创建标签参数（`message` 有值 = 附注标签）。 */
export interface TagCreateSpec {
  readonly name: string;
  readonly target?: string | null;
  readonly message?: string | null;
  readonly sign: boolean;
  readonly force: boolean;
}

/** 删除标签参数。 */
export interface TagDeleteSpec {
  readonly names: readonly string[];
  readonly alsoDeleteRemote: boolean;
}

/** 一个标签（镜像 `domain::git::refs::Tag`）。 */
export interface Tag {
  readonly name: string;
  readonly target: string;
  readonly commit: string | null;
  readonly annotated: boolean;
  readonly message: string | null;
  readonly createdAt: number | null;
}

/** 分支比较结果（`onlyInA` 是 `[oid, subject]` 对）。 */
export interface BranchComparison {
  readonly ahead: number;
  readonly behind: number;
  readonly onlyInA: readonly (readonly [string, string])[];
}

/** 删除分支的结果。 */
export interface BranchDeleteOutcome {
  readonly deleted: readonly string[];
}

// ---------------------------------------------------------------- 只读

/** 分支列表（当前分支置顶 → 本地 → 远端）。错误：`NOT_FOUND`。 */
export function gitBranchList(repoId: number, includeRemote?: boolean): Promise<readonly Branch[]> {
  return invokeCommand<readonly Branch[]>('git_branch_list', {
    repoId,
    ...(includeRemote === undefined ? {} : { includeRemote }),
  });
}

/** 标签列表。错误：`NOT_FOUND`。 */
export function gitTagList(repoId: number): Promise<readonly Tag[]> {
  return invokeCommand<readonly Tag[]>('git_tag_list', { repoId });
}

/** 比较 a 与 b：ahead/behind 与 a 独有的提交（删除确认清单的数据源）。 */
export function gitBranchCompare(repoId: number, a: string, b: string): Promise<BranchComparison> {
  return invokeCommand<BranchComparison>('git_branch_compare', { repoId, a, b });
}

// ---------------------------------------------------------------- 写（全部走审计；危险路径需确认参数）

/** 新建分支（`checkout=true` 时创建后干净切换）。错误：`VALIDATION`（名称非法/不干净）。 */
export function gitBranchCreate(repoId: number, spec: BranchCreateSpec): Promise<void> {
  return invokeCommand<void>('git_branch_create', { repoId, spec });
}

/**
 * 切换分支（三策略）。
 *
 * - `stash`：工作区不干净时自动储藏并在切换后恢复；
 * - `force`：**必须** `confirmForce=true`（丢弃未提交修改，后端先打快照）；
 * - `clean`：不干净时 `VALIDATION`。
 *
 * 返回 Force 路径创建的快照 id（其余为 `null`）。
 */
export function gitBranchSwitch(
  repoId: number,
  target: string,
  strategy: SwitchStrategy,
  confirmForce?: boolean,
): Promise<number | null> {
  return invokeCommand<number | null>('git_branch_switch', {
    repoId,
    target,
    strategy,
    ...(confirmForce === undefined ? {} : { confirmForce }),
  });
}

/** 重命名分支。错误：`VALIDATION`（新名非法）。 */
export function gitBranchRename(repoId: number, spec: BranchRenameSpec): Promise<void> {
  return invokeCommand<void>('git_branch_rename', { repoId, spec });
}

/**
 * 删除一批分支。
 *
 * 未合并强删**必须** `confirmUnmerged=true`——调用方应先 `gitBranchCompare`
 * 把独有提交展示给用户；后端删除前打快照。错误：`VALIDATION`（当前分支/无确认）。
 */
export function gitBranchDelete(
  repoId: number,
  spec: BranchDeleteSpec,
  confirmUnmerged?: boolean,
): Promise<BranchDeleteOutcome> {
  return invokeCommand<BranchDeleteOutcome>('git_branch_delete', {
    repoId,
    spec,
    ...(confirmUnmerged === undefined ? {} : { confirmUnmerged }),
  });
}

/** 设置 / 取消上游。 */
export function gitBranchSetUpstream(repoId: number, spec: BranchSetUpstreamSpec): Promise<void> {
  return invokeCommand<void>('git_branch_set_upstream', { repoId, spec });
}

/** 创建标签（轻量 / 附注；重名需 `force`）。错误：`VALIDATION`（名称/轻量签名）。 */
export function gitTagCreate(repoId: number, spec: TagCreateSpec): Promise<void> {
  return invokeCommand<void>('git_tag_create', { repoId, spec });
}

/** 删除一批标签（本地；远端删除走 T2.6 的 push 通道）。 */
export function gitTagDelete(repoId: number, spec: TagDeleteSpec): Promise<void> {
  return invokeCommand<void>('git_tag_delete', { repoId, spec });
}
