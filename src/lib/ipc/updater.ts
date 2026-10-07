/**
 * 自动更新（T7.1）的前端 IPC 封装。
 *
 * 两个命令的语义见 `docs/API.md`：
 *   - `update_check` 在"未配置更新源"时返回 `{ configured: false }` 而**不是**报错——
 *     源码自编译/开发构建本来就没有发布配置，报错会在界面上留下一条永远修不好的红条；
 *   - `update_install` 要求传入的版本等于最近一次检查到的版本（防"点 A 装 B"）。
 *
 * 订阅 `update:progress` 的组件必须在卸载时调用 `unlisten`（否则监听器泄漏、事件重复处理）。
 */
import { invokeCommand, listenEvent, type Unlisten } from './client';

/** 更新进度事件名（与 Rust 侧 `EVENT_UPDATE_PROGRESS` 逐字符一致）。 */
export const EVENT_UPDATE_PROGRESS = 'update:progress';

/** 可用更新（对应 Rust 侧 `UpdateInfoDto`）。 */
export interface UpdateInfo {
  readonly version: string;
  readonly currentVersion: string;
  readonly notes: string | null;
  readonly date: string | null;
}

/** 更新检查结果（对应 Rust 侧 `UpdateCheckDto`）。 */
export interface UpdateCheck {
  /** 本构建是否配置了更新源（endpoints + 公钥）。 */
  readonly configured: boolean;
  readonly update: UpdateInfo | null;
}

/** 更新进度载荷。 */
export interface UpdateProgressPayload {
  /** 阶段：下载或安装。 */
  readonly phase: 'downloading' | 'installing';
  /** 已接收字节。 */
  readonly received: number;
  /** 总字节；服务端未给 `Content-Length` 时为 null。 */
  readonly total: number | null;
}

/** 查询是否有新版本。 */
export function updateCheck(): Promise<UpdateCheck> {
  return invokeCommand<UpdateCheck>('update_check');
}

/** 下载并安装指定版本（成功后应用会重启）。 */
export function updateInstall(version: string): Promise<void> {
  return invokeCommand<void>('update_install', { version });
}

/** 订阅更新进度事件。 */
export function listenUpdateProgress(
  handler: (payload: UpdateProgressPayload) => void,
): Promise<Unlisten> {
  return listenEvent<UpdateProgressPayload>(EVENT_UPDATE_PROGRESS, handler);
}
