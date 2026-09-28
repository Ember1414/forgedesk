/**
 * 提交详情（T2.4）：`git_commit_detail` 的 DTO 与具名封装。
 *
 * # 为什么单独一个模块
 *
 * 详情是"一次一提交"的形态，与历史分页（`history.ts`）、工作区 diff
 * （`workspace.ts`）的消费节奏都不同；单独成模块让"改详情契约只动一处"。
 * 文件清单的**行级内容**不在这里：它复用 `workspace_diff` 的 `between`
 * 目标（父 oid 由详情下发），见 `CommitDetailPanel`。
 *
 * # 类型的来源
 *
 * 镜像 `crates/services/src/commit_detail.rs` 的 serde 形状（camelCase）；
 * `CommitSignature` / `SignatureStatus` 与 `history.ts` 共用（同一份
 * `domain::git::commit` 的线格式，不重复定义）。
 */
import { invokeCommand } from './client';
import type { CommitSignature, SignatureStatus } from './history';

/** 文件在 diff 中的变更类别（后端 `DiffChangeKind`，serde 小驼峰）。 */
export type CommitFileChangeKind =
  'added' | 'deleted' | 'modified' | 'renamed' | 'copied' | 'typeChanged' | 'unknown';

/** 提交元数据（`show` 的结果 + 短 oid）。 */
export interface CommitMeta {
  readonly oid: string;
  readonly shortOid: string;
  /** 父提交 oid，顺序与 Git 一致（第一个是 first-parent）；根提交为空数组。 */
  readonly parents: readonly string[];
  readonly author: CommitSignature;
  readonly committer: CommitSignature;
  readonly subject: string;
  /** 提交信息正文（详情查询带 `%b`；列表查询没有这个字段）。 */
  readonly body: string | null;
  readonly signature: SignatureStatus;
}

/** 变更统计汇总（相对所选父提交；二进制文件不计行数）。 */
export interface CommitStats {
  readonly filesChanged: number;
  readonly insertions: number;
  readonly deletions: number;
}

/** 文件清单里的一项（文件级统计；行级内容按需经 `workspace_diff` 拉取）。 */
export interface CommitFileChange {
  readonly path: string;
  /** 重命名/复制的来源路径。 */
  readonly oldPath: string | null;
  readonly kind: CommitFileChangeKind;
  readonly binary: boolean;
  readonly additions: number;
  readonly deletions: number;
  readonly truncated: boolean;
}

/** 一次提交详情查询的结果。 */
export interface CommitDetail {
  readonly meta: CommitMeta;
  /** 指向该提交的引用（`%D` 原文，如 `HEAD -> main`、`tag: v1.0.0`）。 */
  readonly refs: readonly string[];
  readonly stats: CommitStats;
  readonly files: readonly CommitFileChange[];
  /** 是否为合并提交（父提交数 > 1）。 */
  readonly isMerge: boolean;
  /** 是否为当前 HEAD。 */
  readonly isHead: boolean;
  /** 是否已被某个远端分支包含。 */
  readonly isPushed: boolean;
  /**
   * 由 origin 的 fetch URL 推断的网页 URL（仅 GitHub / GitLab.com / Bitbucket.org；
   * 自建实例形态无法保证，猜错比不给更糟）。提交页路径由前端拼接。
   */
  readonly webUrl: string | null;
  /** 本次文件清单实际使用的父下标（`parents` 的下标）。 */
  readonly parentIndex: number;
}

/**
 * 读取提交详情。
 *
 * 错误：`NOT_FOUND`（repoId 无效或提交不存在）、`VALIDATION`（oid 形状非法、
 * `parentIndex` 越界——非根提交 ≥ 父数，根提交 ≠ 0）。
 */
export function gitCommitDetail(
  repoId: number,
  oid: string,
  parentIndex?: number,
): Promise<CommitDetail> {
  return invokeCommand<CommitDetail>('git_commit_detail', {
    repoId,
    oid,
    ...(parentIndex === undefined ? {} : { parentIndex }),
  });
}
