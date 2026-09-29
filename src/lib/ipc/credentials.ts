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

/** 凭据存储当前的形态（界面据此在"添加"与"解锁"之间切换）。 */
export type CredentialMode = 'systemKeyring' | 'vaultUnlocked' | 'vaultLocked';

/** 凭据状态（设置页的顶部说明用）。 */
export interface CredentialsStatus {
  readonly backend: CredentialBackend;
  /** 当前形态。 */
  readonly mode: CredentialMode;
  /**
   * 已保存的凭据数量；保险库未解锁时为 `null`。
   *
   * 注意不是 0：0 的意思是"没有凭据"，而 `null` 是"还不知道"。
   */
  readonly count: number | null;
  /** 索引文件路径（keyring 不可用时用户需要知道回退文件在哪）。 */
  readonly indexPath?: string;
  /** 加密保险库文件路径（已存在时）。 */
  readonly vaultPath?: string;
  /** 保险库文件是否已存在（决定展示"新建"还是"解锁"）。 */
  readonly vaultExists: boolean;
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

/** 一把本地 SSH 密钥的元信息（**不含私钥内容**）。 */
export interface SshKeyInfo {
  /** 公钥文件路径（存在时）。 */
  readonly publicPath?: string;
  /** 私钥文件路径（存在时；只表示"文件在"，不代表内容可读或被读过）。 */
  readonly privatePath?: string;
  /** 密钥类型（来自公钥首行，如 `ssh-ed25519`）。 */
  readonly keyType?: string;
  /** 公钥里的注释（通常是 `user@host`）。 */
  readonly comment?: string;
}

/** agent 里的一把密钥。 */
export interface SshAgentKey {
  /** 位长。 */
  readonly bits?: number;
  /** 指纹（`SHA256:...`）：用户拿它去跟托管平台上的指纹核对。 */
  readonly fingerprint: string;
  /** 注释。 */
  readonly comment?: string;
}

/**
 * `ssh-add -l` 的结果。
 *
 * `noIdentities`（agent 在跑但没加载密钥）与 `notRunning`（agent 没开）
 * 必须分开：两者的下一步动作完全不同。
 */
export type SshAgentStatus =
  | { readonly kind: 'ready'; readonly keys: readonly SshAgentKey[] }
  | { readonly kind: 'noIdentities' }
  | { readonly kind: 'notRunning' }
  | { readonly kind: 'unknown'; readonly reason: string };

/** 本地 SSH 盘点。 */
export interface SshInventory {
  /** 扫描的目录（`~/.ssh`；拿不到 HOME 时缺省）。 */
  readonly directory?: string;
  /** 目录里的密钥（按文件名排序）。 */
  readonly keys: readonly SshKeyInfo[];
  /** agent 状态。 */
  readonly agent: SshAgentStatus;
}

/**
 * 盘点本地 SSH 密钥与 ssh-agent（只读）。
 *
 * 只回答"本地有什么"：公私钥是否配对、agent 里加载了哪几把。服务端是否接受公钥
 * 只能靠 {@link credentialTestRemote} 实际连一次。
 */
export function credentialsSshInventory(): Promise<SshInventory> {
  return invokeCommand<SshInventory>('credentials_ssh_inventory');
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
 * 新建加密保险库并切换到它（系统凭据库不可用时的回退方案）。
 *
 * 口令只用于派生密钥，**不会**被保存：忘记口令等于里面的凭据不可恢复。
 * 已存在保险库文件时后端拒绝（覆盖等于把已有凭据悄悄清空）。
 */
export function credentialsVaultCreate(passphrase: string): Promise<void> {
  return invokeCommand<void>('credentials_vault_create', { passphrase });
}

/** 解锁已有加密保险库并切换到它。口令错时返回本地存储类错误。 */
export function credentialsVaultUnlock(passphrase: string): Promise<void> {
  return invokeCommand<void>('credentials_vault_unlock', { passphrase });
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
