/**
 * 托管平台账号（T4.3/T4.4）：命令 DTO 与具名封装。
 *
 * # 令牌的边界（红线 R8）
 *
 * 这里只有两个入口能**送出**令牌明文：`accountLoginWithPat`（用户在哪里
 * 粘贴，令牌就从哪里来）与 Device Flow 的后端轮询（明文根本不经过前端）。
 * 所有返回的 DTO（`Account`、`DeviceFlowSession`）都**不含**任何令牌材料：
 * `device_code` 留在后端会话表里，前端连序列化的机会都没有。
 *
 * # 类型来源
 *
 * 镜像 `crates/commands/src/account.rs`（docs/API.md「托管平台账号」节），
 * 形状是 camelCase；改一侧必须同步另一侧。
 */
import { invokeCommand } from './client';

/** 一个已登录的托管平台账号（不含令牌）。 */
export interface Account {
  readonly id: string;
  readonly provider: string;
  readonly host: string;
  readonly login: string;
  readonly avatarUrl?: string;
  readonly scopes: readonly string[];
  /** 首次登录时间（Unix 毫秒）。 */
  readonly createdAt?: number | null;
}

/** 一次已启动的 Device Flow（UI 引导数据）。 */
export interface DeviceFlowSession {
  /** 后端会话 id，`accountDeviceFlowWait` 凭它取会话。 */
  readonly flowId: string;
  /** 用户要输入的码（如 `WDJB-MJTK`）。 */
  readonly userCode: string;
  /** 输码页面。 */
  readonly verificationUri: string;
  /** 携带 user_code 的直链（存在时复制后打开即填）。 */
  readonly verificationUriComplete?: string;
  /** 流程过期秒数。 */
  readonly expiresInSecs: number;
  /** 轮询间隔秒数（后端负责节奏，这里供提示"大约多久"）。 */
  readonly intervalSecs: number;
}

/** 列出已登录账号（按创建时间排序）。 */
export function accountList(): Promise<Account[]> {
  return invokeCommand<Account[]>('account_list');
}

/** PAT 登录：令牌经 `/user` 校验后落 keyring 与账号表。 */
export function accountLoginWithPat(host: string, token: string): Promise<Account> {
  return invokeCommand<Account>('account_login_with_pat', { host, token });
}

/** 启动 Device Flow：返回三步引导需要的非秘密数据。 */
export function accountDeviceFlowStart(
  host: string,
  scopes?: readonly string[],
): Promise<DeviceFlowSession> {
  return invokeCommand<DeviceFlowSession>('account_device_flow_start', {
    host,
    ...(scopes === undefined ? {} : { scopes }),
  });
}

/** 等待 Device Flow 完成（长任务）：返回 jobId，结果经 `job:done` 的 `{ account }` 送达。 */
export function accountDeviceFlowWait(flowId: string): Promise<JobRef> {
  return invokeCommand<JobRef>('account_device_flow_wait', { flowId });
}

/** 删除账号（凭据库条目 + 账号行）。 */
export function accountRemove(accountId: string): Promise<void> {
  return invokeCommand<void>('account_remove', { accountId });
}

/** 长任务引用（与 `repository.ts` 的 JobRef 同形；此处独立声明避免循环导入）。 */
interface JobRef {
  readonly jobId: string;
}
