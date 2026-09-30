/**
 * Pull Request（T4.7）：命令 DTO 与具名封装。
 *
 * # 类型来源
 *
 * 镜像 `crates/commands/src/remote_repos.rs` 的 `PullDetailDto` 与
 * `crates/provider/src/pulls.rs`（docs/API.md「Pull Request」节）。
 *
 * # 描述是消毒过的
 *
 * `bodyHtml` 来自后端白名单渲染（与 README 同一规则）；原始 Markdown
 * **不越过** IPC——类型上就没有那个字段。
 *
 * # 合并失败的"可读原因"是 hint
 *
 * `hint ∈ "not-mergeable" | "conflict" | "head-changed"`（见 docs/API.md
 * 错误表）：合并对话框按它区分"有冲突 / 有新提交 / 分支保护"三种文案，
 * 而不是把一条错误信息糊给用户。
 */
import { invokeCommand } from './client';

/** PR 列表条目。 */
export interface PullSummary {
  readonly number: number;
  readonly title: string;
  readonly state: 'open' | 'closed';
  readonly draft: boolean;
  readonly merged: boolean;
  readonly author: string;
  /** 源分支标签（`owner:branch`）。 */
  readonly headLabel: string;
  /** 目标分支标签。 */
  readonly baseLabel: string;
  readonly htmlUrl: string;
  readonly createdAt?: string;
  readonly updatedAt?: string;
}

/** PR 列表分页。 */
export interface PullPage {
  readonly items: readonly PullSummary[];
  readonly nextPage: number | null;
}

/** PR 详情（描述为消毒 HTML；含合并条件判断的全部输入）。 */
export interface PullDetail {
  readonly number: number;
  readonly title: string;
  readonly state: 'open' | 'closed';
  readonly draft: boolean;
  readonly merged: boolean;
  readonly author: string;
  readonly headLabel: string;
  readonly baseLabel: string;
  /** 当前 head sha——合并预检的输入（远端版 PLAN_STALE 的判定基准）。 */
  readonly headSha: string;
  readonly htmlUrl: string;
  readonly bodyHtml?: string;
  readonly changedFiles: number;
  readonly additions: number;
  readonly deletions: number;
  readonly mergeable: boolean | null;
  /** `clean` / `dirty` / `blocked` / `unstable`…（原样透出，UI 分档）。 */
  readonly mergeableState?: string;
  readonly createdAt?: string;
  readonly updatedAt?: string;
}

/** 一条 review。 */
export interface PullReview {
  readonly id: number;
  readonly author: string;
  readonly state: 'APPROVED' | 'CHANGES_REQUESTED' | 'COMMENTED' | string;
  readonly body?: string;
  readonly submittedAt?: string;
}

/** 合并结果。 */
export type PullMergeOutcome = {
  readonly merged: boolean;
  readonly sha?: string;
  readonly message?: string;
  readonly branchDeleted: boolean;
};

/** PR 列表的请求体（`docs/API.md` 的 request 形态；type 别名以获得隐式索引签名）。 */
export type PullListRequest = {
  readonly host: string;
  readonly owner: string;
  readonly repo: string;
  readonly repoId?: number;
  readonly stateFilter?: 'open' | 'closed' | 'all';
  readonly page?: number;
  readonly perPage?: number;
};

/** 列出 PR。 */
export function repoPullList(request: PullListRequest): Promise<PullPage> {
  return invokeCommand<PullPage>('repo_pull_list', request);
}

/** PR 详情（描述已消毒）。 */
export function repoPullGet(
  host: string,
  owner: string,
  repo: string,
  number: number,
  repoId?: number,
): Promise<PullDetail> {
  return invokeCommand<PullDetail>('repo_pull_get', {
    host,
    owner,
    repo,
    number,
    ...(repoId === undefined ? {} : { repoId }),
  });
}

/** PR 的 review 列表。 */
export function repoPullReviews(
  host: string,
  owner: string,
  repo: string,
  number: number,
  repoId?: number,
): Promise<PullReview[]> {
  return invokeCommand<PullReview[]>('repo_pull_reviews', {
    host,
    owner,
    repo,
    number,
    ...(repoId === undefined ? {} : { repoId }),
  });
}

/** 合并 PR（三策略 + 可选删源分支 + head 预检）。 */
export function repoPullMerge(request: {
  readonly host: string;
  readonly owner: string;
  readonly repo: string;
  readonly number: number;
  readonly strategy: 'merge' | 'squash' | 'rebase';
  readonly repoId?: number;
  readonly commitTitle?: string;
  readonly commitMessage?: string;
  readonly expectedHeadSha?: string;
  readonly deleteBranch?: boolean;
  readonly headBranch?: string;
}): Promise<PullMergeOutcome> {
  return invokeCommand<PullMergeOutcome>('repo_pull_merge', request);
}
