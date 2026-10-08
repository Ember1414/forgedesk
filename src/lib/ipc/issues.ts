/**
 * Issue（T4.8）：命令 DTO 与具名封装。
 *
 * # 类型来源
 *
 * 镜像 `crates/commands/src/issues.rs` 的 `IssueDetailDto` 与
 * `crates/provider/src/issues.rs`（docs/API.md「Issue」节）。
 *
 * # 描述是消毒过的
 *
 * `bodyHtml` 来自后端白名单渲染（与 README/PR 描述同一规则）；原始
 * Markdown **不越过** IPC——类型上就没有那个字段。
 *
 * # 评论是纯文本渲染
 *
 * 评论正文是 Markdown 原文，与 PR 时间线评论同一规则：前端用 React
 * 文本节点展示，不做任何 HTML 注入。
 */
import { invokeCommand } from './client';

/** Issue 列表条目。 */
export interface IssueSummary {
  readonly number: number;
  readonly title: string;
  readonly state: 'open' | 'closed' | string;
  readonly author: string;
  /** 标签名（展示用）。 */
  readonly labels: readonly string[];
  /** 指派人 login。 */
  readonly assignees: readonly string[];
  /** 评论数。 */
  readonly comments: number;
  readonly createdAt?: string;
  readonly updatedAt?: string;
  readonly closedAt?: string;
}

/** Issue 列表分页。 */
export interface IssuePage {
  readonly items: readonly IssueSummary[];
  readonly nextPage: number | null;
}

/** Issue 详情（描述为消毒 HTML）。 */
export interface IssueDetail {
  readonly number: number;
  readonly title: string;
  readonly state: 'open' | 'closed' | string;
  readonly author: string;
  readonly labels: readonly string[];
  readonly assignees: readonly string[];
  readonly comments: number;
  readonly bodyHtml?: string;
  readonly createdAt?: string;
  readonly updatedAt?: string;
  readonly closedAt?: string;
}

/** Issue 评论（与 PR 时间线评论同形）。 */
export type IssueComment = {
  readonly id: number;
  readonly author: string;
  /** Markdown 原文（展示层消毒）。 */
  readonly body: string;
  readonly createdAt?: string;
};

/** 可指派人。 */
export interface Assignee {
  readonly login: string;
}

/** Issue 列表的请求体（type 别名以获得隐式索引签名）。 */
export type IssueListRequest = {
  readonly host: string;
  readonly owner: string;
  readonly repo: string;
  readonly repoId?: number;
  readonly stateFilter?: 'open' | 'closed' | 'all';
  readonly page?: number;
  readonly perPage?: number;
};

/** 列出 Issue（不含 PR）。 */
export function repoIssueList(request: IssueListRequest): Promise<IssuePage> {
  return invokeCommand<IssuePage>('repo_issue_list', { request });
}

/** Issue 详情（描述已消毒）。 */
export function repoIssueGet(
  host: string,
  owner: string,
  repo: string,
  number: number,
  repoId?: number,
): Promise<IssueDetail> {
  return invokeCommand<IssueDetail>('repo_issue_get', {
    host,
    owner,
    repo,
    number,
    ...(repoId === undefined ? {} : { repoId }),
  });
}

/** 描述原文（Markdown）——只供编辑器预填进 textarea；展示用 repoIssueGet。 */
export function repoIssueBody(
  host: string,
  owner: string,
  repo: string,
  number: number,
  repoId?: number,
): Promise<string> {
  return invokeCommand<string>('repo_issue_body', {
    host,
    owner,
    repo,
    number,
    ...(repoId === undefined ? {} : { repoId }),
  });
}

/** 创建 Issue（标题必填）。 */
export function repoIssueCreate(request: {
  readonly host: string;
  readonly owner: string;
  readonly repo: string;
  readonly title: string;
  readonly body?: string;
  readonly repoId?: number;
}): Promise<IssueDetail> {
  return invokeCommand<IssueDetail>('repo_issue_create', { request });
}

/** 编辑标题/描述（null 字段不动）。 */
export function repoIssueEdit(request: {
  readonly host: string;
  readonly owner: string;
  readonly repo: string;
  readonly number: number;
  readonly title?: string;
  readonly body?: string;
  readonly repoId?: number;
}): Promise<IssueDetail> {
  return invokeCommand<IssueDetail>('repo_issue_edit', { request });
}

/** 关闭 / 重新开启。 */
export function repoIssueStateSet(request: {
  readonly host: string;
  readonly owner: string;
  readonly repo: string;
  readonly number: number;
  readonly open: boolean;
  readonly repoId?: number;
}): Promise<IssueDetail> {
  return invokeCommand<IssueDetail>('repo_issue_state_set', { request });
}

/** 整体替换指派人（空数组 = 全部取消）。 */
export function repoIssueAssigneesSet(request: {
  readonly host: string;
  readonly owner: string;
  readonly repo: string;
  readonly number: number;
  readonly assignees: readonly string[];
  readonly repoId?: number;
}): Promise<IssueDetail> {
  return invokeCommand<IssueDetail>('repo_issue_assignees_set', { request });
}

/** Issue 评论列表。 */
export function repoIssueCommentsList(
  host: string,
  owner: string,
  repo: string,
  number: number,
  repoId?: number,
): Promise<IssueComment[]> {
  return invokeCommand<IssueComment[]>('repo_issue_comments_list', {
    host,
    owner,
    repo,
    number,
    ...(repoId === undefined ? {} : { repoId }),
  });
}

/** 发表一条 Issue 评论。 */
export function repoIssueCommentCreate(
  host: string,
  owner: string,
  repo: string,
  number: number,
  body: string,
  repoId?: number,
): Promise<IssueComment> {
  return invokeCommand<IssueComment>('repo_issue_comment_create', {
    host,
    owner,
    repo,
    number,
    body,
    ...(repoId === undefined ? {} : { repoId }),
  });
}

/** 可指派人列表。 */
export function repoIssueAssignees(
  host: string,
  owner: string,
  repo: string,
  repoId?: number,
): Promise<Assignee[]> {
  return invokeCommand<Assignee[]>('repo_issue_assignees', {
    host,
    owner,
    repo,
    ...(repoId === undefined ? {} : { repoId }),
  });
}
