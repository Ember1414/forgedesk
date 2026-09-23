/**
 * 仓库生命周期命令的封装（对应 Rust 侧 `forgedesk_commands::repository`）。
 *
 * 类型与 Rust DTO 的 `serde(rename_all = "camelCase")` 一一对应；
 * 契约登记在 `docs/API.md`。
 */
import { invokeCommand } from './client';

/** 当前分支在界面上的呈现类别（由后端判定，前端不自行拼）。 */
export type BranchLabel =
  | { readonly kind: 'unborn' }
  | { readonly kind: 'detached' }
  | { readonly kind: 'named'; readonly name: string };

/** 一个工作区（`git worktree`）。 */
export interface Worktree {
  readonly path: string;
  readonly head: string | null;
  readonly branch: string | null;
  readonly detached: boolean;
  readonly isBare: boolean;
  readonly locked: boolean;
  readonly prunable: boolean;
}

/** 仓库基本信息。 */
export interface Repository {
  /** 工作区根目录；裸仓库为 `null`。 */
  readonly workdir: string | null;
  /** `.git` 目录（裸仓库即仓库根）。 */
  readonly gitDir: string;
  readonly isBare: boolean;
  /** 是否还没有任何提交。 */
  readonly isEmpty: boolean;
  readonly head: string | null;
  readonly detached: boolean;
  readonly upstream: string | null;
  readonly defaultBranch: string | null;
  /** 浅克隆（历史不完整，历史视图需要降级提示）。 */
  readonly isShallow: boolean;
  /** 使用 Git LFS（提示性字段，不参与数据完整性判断）。 */
  readonly isLfs: boolean;
  /** 关联工作区列表，主工作区在首位。 */
  readonly worktrees: readonly Worktree[];
  readonly branchLabel: BranchLabel;
}

/** 一条仓库配置审计发现。 */
export interface AuditFinding {
  /** 类别（`fsmonitor` / `ssh_command` / `filter_clean` / …）。 */
  readonly id: string;
  /** `info` / `warning` / `danger`。 */
  readonly severity: 'info' | 'warning' | 'danger';
  readonly key: string;
  /** 命中的配置值（**已脱敏**）。 */
  readonly value: string;
  readonly scope: string;
}

/** 仓库配置审计报告。 */
export interface RepoAudit {
  readonly findings: readonly AuditFinding[];
  /** 是否存在"会被 git 当命令执行"的配置。 */
  readonly hasDanger: boolean;
  readonly maxSeverity: string | null;
}

/** 打开 / 克隆 / 初始化之后的完整结果。 */
export interface OpenedRepository {
  /** `repositories` 表主键；后续所有 `repoId` 参数都用它。 */
  readonly recordId: number;
  readonly repository: Repository;
  readonly audit: RepoAudit;
  /** 系统 git 版本；解析不出来时为 `null`。 */
  readonly gitVersion: string | null;
  readonly gitVersionSupported: boolean;
  /** 是否应当提示用户升级 git。 */
  readonly needsGitUpgrade: boolean;
}

/** 最近列表里的一项。 */
export interface RecentRepository {
  readonly id: number;
  readonly path: string;
  readonly name: string;
  readonly defaultBranch: string | null;
  readonly lastOpenedAt: number | null;
  readonly createdAt: number;
  /** 当前会话中是否已打开。 */
  readonly isOpen: boolean;
}

/** 长任务创建的结果。 */
export interface JobRef {
  readonly jobId: string;
}

/** 克隆请求。 */
export interface CloneRequest {
  readonly url: string;
  readonly into: string;
  readonly depth?: number;
  readonly branch?: string;
  readonly bare?: boolean;
  readonly recurseSubmodules?: boolean;
  readonly singleBranch?: boolean;
}

/** 初始化请求。 */
export interface InitRequest {
  readonly path: string;
  readonly initialBranch?: string;
  readonly bare?: boolean;
  /** `.gitignore` 模板 id：`rust` / `node` / `python` / `go` / `java`。 */
  readonly gitignore?: string;
  /** 许可证模板 id：`MIT` / `Apache-2.0` / `BSD-3-Clause`。 */
  readonly license?: string;
  readonly licenseHolder?: string;
  readonly licenseYear?: number;
}

/**
 * 从任意目录向上发现仓库（不写库、不审计）。
 *
 * 路径不在任何仓库内时抛出 `PATH_NOT_REPO`，错误里带一个
 * `repo_init` 修复动作。
 */
export function repoDiscover(path: string): Promise<Repository> {
  return invokeCommand<Repository>('repo_discover', { path });
}

/**
 * 打开仓库：发现 → 配置审计 → git 版本检查 → 登记到最近列表。
 *
 * 审计与版本检查的结果不会阻塞打开。
 */
export function repoOpen(path: string): Promise<OpenedRepository> {
  return invokeCommand<OpenedRepository>('repo_open', { path });
}

/**
 * 克隆仓库（长任务）。
 *
 * 立即返回任务 id；进度经 `job:progress`、结果经 `job:done` / `job:failed`
 * 推送，可经 {@link cancelJob} 取消。
 */
export function repoClone(spec: CloneRequest): Promise<JobRef> {
  return invokeCommand<JobRef>('repo_clone', { spec });
}

/** 初始化仓库，并按需生成 `.gitignore` / `LICENSE`。 */
export function repoInit(spec: InitRequest): Promise<OpenedRepository> {
  return invokeCommand<OpenedRepository>('repo_init', { spec });
}

/** 最近打开的仓库（按最近打开时间倒序）。 */
export function repoRecentList(limit?: number): Promise<RecentRepository[]> {
  return invokeCommand<RecentRepository[]>(
    'repo_recent_list',
    limit === undefined ? {} : { limit },
  );
}

/** 从最近列表移除（**只删记录，不碰磁盘上的仓库**）。 */
export function repoForget(repoId: number): Promise<void> {
  return invokeCommand<void>('repo_forget', { repoId });
}

/** 关闭一个已打开的仓库（只影响会话内的"已打开"状态）。 */
export function repoClose(repoId: number): Promise<void> {
  return invokeCommand<void>('repo_close', { repoId });
}
