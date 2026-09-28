/**
 * 凭据与认证（T2.7）：命令 DTO 与具名封装。
 *
 * # 与后端的分工
 *
 * 密文存在**系统凭据库**里，前端只能：
 *   1. 写入（`credentialsSave`：明文只经这一次调用，之后只留在 keyring）；
 *   2. 列出元数据（`credentialsList`：**没有**密文字段，后端也不返回）；
 *   3. 删除；
 *   4. 读状态（`credentialsStatus`：存在哪里、系统凭据库能不能用）。
 *
 * # 类型来源
 *
 * 镜像 `crates/commands/src/credentials.rs` 与 `crates/credentials/src/model.rs`，
 * 形状是 **camelCase**（docs/API.md §1）。改一侧必须同步另一侧。
 */
import { invokeCommand } from './client';

/** 凭据类型。 */
export type CredentialKind = 'pat' | 'oauth' | 'password';

/** 一条凭据的引用（不含密文）。 */
export interface CredentialRef {
  readonly provider: string;
  readonly host: string;
  readonly login: string;
}

/** 列表中一条凭据的元数据。 */
export interface CredentialMeta {
  readonly key: CredentialRef;
  readonly kind: CredentialKind;
  /** 写入时间（Unix 毫秒）。 */
  readonly createdAtMs: number;
}

/** 密文实际存放的位置。 */
export type CredentialBackend = 'systemKeyring' | 'encryptedVault' | 'memory';

/** 凭据状态（设置页的顶部说明用）。 */
export interface CredentialsStatus {
  readonly backend: CredentialBackend;
  readonly count: number;
  /** 索引文件路径（keyring 不可用时用户需要知道回退文件在哪）。 */
  readonly indexPath?: string;
  /**
   * 系统凭据库不可用的原因（平台原文，是**数据**不是建议）。
   *
   * 有值时应提示用户：凭据将无法保存到系统凭据库。
   */
  readonly keyringUnavailableReason?: string;
}

/** 保存凭据的入参。 */
export interface CredentialInput {
  readonly provider: string;
  readonly host: string;
  readonly login: string;
  readonly kind: CredentialKind;
  /** 明文令牌/密码；**只**经这一个参数进入后端。 */
  readonly secret: string;
}

/** "测试连接"的结果。 */
export interface RemoteProbe {
  /** 远端引用条数（空仓库为 0）。 */
  readonly refs: number;
}

/** 列出已保存的凭据（不含密文）。 */
export function credentialsList(): Promise<readonly CredentialMeta[]> {
  return invokeCommand<readonly CredentialMeta[]>('credentials_list');
}

/** 保存（同引用覆盖）。成功即清空该主机的连续失败计数。 */
export function credentialsSave(input: CredentialInput): Promise<CredentialMeta> {
  return invokeCommand<CredentialMeta>('credentials_save', { ...input });
}

/** 删除（幂等）。 */
export function credentialsDelete(key: CredentialRef): Promise<void> {
  return invokeCommand<void>('credentials_delete', {
    provider: key.provider,
    host: key.host,
    login: key.login,
  });
}

/** 凭据状态：存在哪里、有多少条、系统凭据库是否可用。 */
export function credentialsStatus(): Promise<CredentialsStatus> {
  return invokeCommand<CredentialsStatus>('credentials_status');
}

/**
 * 测试连接（`git ls-remote`，5s 超时，只读）。
 *
 * 二选一：直接给 `url`，或给 `repoId`（+ 可选 `remote`，缺省当前分支的上游远端）。
 * 失败时错误码可区分 SSH 主机指纹 / 公钥被拒 / 证书 / 代理，界面据此给不同建议。
 */
export function credentialTestRemote(target: {
  readonly url?: string;
  readonly repoId?: number;
  readonly remote?: string;
}): Promise<RemoteProbe> {
  return invokeCommand<RemoteProbe>('credential_test_remote', { ...target });
}

/**
 * 从凭据推导一个默认的测试地址。
 *
 * 为什么需要：用户手里往往只有"我保存了 github.com 的令牌"，而 `git ls-remote`
 * 需要一个具体仓库地址。给一个主机根地址是**有意义的探测**——它足以区分
 * "能连上（可能 404）""需要认证""证书/代理有问题"，而这正是用户想知道的。
 */
export function probeUrlFor(host: string): string {
  return `https://${host}`;
}
