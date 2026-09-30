/**
 * 远端托管仓库（T4.5）：命令 DTO 与具名封装。
 *
 * # 分页是页码游标
 *
 * 后端把 GitHub 的 `Link` 头解析成 `nextPage`：`null` 就是最后一页，
 * UI 的"加载更多"每次带 `page` 追加一页（不放进无限查询缓存，
 * 理由与 `graphPagingStore` 相同——累积列表属于页面状态而非服务端缓存）。
 *
 * # 类型来源
 *
 * 镜像 `crates/commands/src/remote_repos.rs`（docs/API.md「远端仓库与账号绑定」节）。
 */
import { invokeCommand } from './client';
import type { Account } from './accounts';

/** 远端平台上的一份仓库。 */
export interface RemoteRepo {
  readonly id: number;
  readonly owner: string;
  readonly name: string;
  readonly fullName: string;
  readonly description?: string;
  readonly htmlUrl: string;
  readonly defaultBranch?: string;
  readonly private: boolean;
  readonly fork: boolean;
  readonly stars: number;
  readonly pushedAt?: string;
}

/** 一页远端仓库 + 下一页游标（`null` = 没有更多）。 */
export interface RemoteRepoPage {
  readonly items: readonly RemoteRepo[];
  readonly nextPage: number | null;
}

/** 列表归属（对应后端 affiliation：`owned` / `all`）。 */
export type RemoteRepoScope = 'owned' | 'all';

/** 列出账号可见的仓库（需要已登录账号）。 */
export function repoRemoteList(
  host: string,
  options: {
    readonly scope?: RemoteRepoScope;
    readonly page?: number;
    readonly perPage?: number;
  } = {},
): Promise<RemoteRepoPage> {
  return invokeCommand<RemoteRepoPage>('repo_remote_list', {
    host,
    ...(options.scope === undefined ? {} : { scope: options.scope }),
    ...(options.page === undefined ? {} : { page: options.page }),
    ...(options.perPage === undefined ? {} : { perPage: options.perPage }),
  });
}

/** 列出账号星标的仓库（需要已登录账号）。 */
export function repoRemoteStarred(
  host: string,
  options: { readonly page?: number; readonly perPage?: number } = {},
): Promise<RemoteRepoPage> {
  return invokeCommand<RemoteRepoPage>('repo_remote_starred', {
    host,
    ...(options.page === undefined ? {} : { page: options.page }),
    ...(options.perPage === undefined ? {} : { perPage: options.perPage }),
  });
}

/** 搜索仓库（匿名可用；有账号走高配额）。 */
export function repoRemoteSearch(
  host: string,
  query: string,
  options: { readonly page?: number; readonly perPage?: number } = {},
): Promise<RemoteRepoPage> {
  return invokeCommand<RemoteRepoPage>('repo_remote_search', {
    host,
    query,
    ...(options.page === undefined ? {} : { page: options.page }),
    ...(options.perPage === undefined ? {} : { perPage: options.perPage }),
  });
}

/** 加星 / 取消加星。 */
export function repoRemoteStar(
  host: string,
  owner: string,
  repo: string,
  starred: boolean,
): Promise<void> {
  return invokeCommand<void>('repo_remote_star', { host, owner, repo, starred });
}

/** fork 到当前账号名下（GitHub 202：副本异步创建中）。 */
export function repoRemoteFork(host: string, owner: string, repo: string): Promise<RemoteRepo> {
  return invokeCommand<RemoteRepo>('repo_remote_fork', { host, owner, repo });
}

/** 读取仓库绑定的账号；未绑定为 `null`。 */
export function repoAccountBindingGet(repoId: number): Promise<Account | null> {
  return invokeCommand<Account | null>('repo_account_binding_get', { repoId });
}

/** 设置/解除仓库绑定的账号（解除传 `null`）。 */
export function repoAccountBindingSet(
  repoId: number,
  accountId: string | null,
): Promise<Account | null> {
  return invokeCommand<Account | null>('repo_account_binding_set', {
    repoId,
    accountId,
  });
}
