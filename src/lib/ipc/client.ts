/**
 * Tauri IPC 的底层客户端。
 *
 * 这是**唯一**允许 `import '@tauri-apps/api/*'` 的地方（由 `eslint.config.js`
 * 的 `no-restricted-imports` 强制，规则覆盖 `src/lib/ipc/**`）。
 *
 * 业务代码只能通过本模块与 `repository.ts` / `jobs.ts` 暴露的具名函数访问后端，
 * 好处是：
 *
 *  1. 所有 IPC 调用点可被静态检索（审计与重构成本可控）。
 *  2. 命令名与 DTO 类型集中定义，避免各处手写字符串导致拼写错误。
 *  3. 后续要统一加超时、重试、日志、错误归一化时，只改一处。
 */
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

/**
 * 取消订阅。
 *
 * 刻意不把 Tauri 的 `UnlistenFn` 类型透出去：业务代码不需要知道事件来自 Tauri，
 * 而且 `@tauri-apps/api/*` 的 import 被 ESLint 限制在本目录内。
 */
export type Unlisten = () => void;

/**
 * 前端是否运行在 Tauri 宿主中。
 *
 * 用途：`pnpm dev` 直接在普通浏览器里打开时（无 Tauri 宿主），
 * 调用 `invoke` 会抛错。调用方应据此禁用相关查询，而不是让界面报错。
 */
export function isTauriRuntime(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

/**
 * 调用一个 Tauri 命令（底层通用入口）。
 *
 * 除非确实没有对应的具名封装，否则**不要**在业务代码中直接使用它——
 * 每新增一个后端命令，请在 `repository.ts` 之类的模块里补一个具名函数。
 */
export function invokeCommand<TResult>(
  command: string,
  args?: Record<string, unknown>,
): Promise<TResult> {
  return invoke<TResult>(command, args);
}

/**
 * 订阅一个后端事件，回调直接拿到载荷（而不是 Tauri 的 `Event` 包装）。
 *
 * 返回 `unlisten`：**组件卸载时必须调用它**，否则监听器会留在全局集合里，
 * 每次挂载再叠一层（表现是同一个事件被处理多次）。
 */
export function listenEvent<TPayload>(
  event: string,
  handler: (payload: TPayload) => void,
): Promise<Unlisten> {
  return listen<TPayload>(event, (message) => handler(message.payload));
}
