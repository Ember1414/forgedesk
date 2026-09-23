/**
 * 唯一的 Tauri IPC 出口。
 *
 * 为什么必须集中在这里：`eslint.config.js` 中的 `no-restricted-imports` 规则
 * 禁止 `@tauri-apps/api/*` 出现在本目录之外的任何文件里（AGENTS.md §6「前后端严格分层」）。
 * 业务代码只能通过本模块暴露的具名函数访问后端，好处是：
 *
 *  1. 所有 IPC 调用点可被静态检索（审计与重构成本可控）。
 *  2. 命令名与 DTO 类型集中定义，避免前端各处手写字符串导致拼写错误。
 *  3. 后续要在这一层统一加超时、重试、日志、错误归一化时，只改一处。
 *
 * 约定：本文件中的类型必须与 Rust 侧 DTO 的 `serde(rename_all = "camelCase")` 一一对应。
 */
import { invoke } from '@tauri-apps/api/core';

/** 应用版本与构建信息（对应 Rust 侧 `forgedesk_commands::AppVersion`）。 */
export interface AppVersion {
  readonly version: string;
  readonly gitSha: string;
  readonly target: string;
  readonly profile: string;
}

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
 * 每新增一个后端命令，请在本文件补一个具名函数。
 */
export function invokeCommand<TResult>(
  command: string,
  args?: Record<string, unknown>,
): Promise<TResult> {
  return invoke<TResult>(command, args);
}

/** 获取应用版本与构建信息。 */
export function appVersion(): Promise<AppVersion> {
  return invokeCommand<AppVersion>('app_version');
}

/**
 * 触发一个受控失败的演示错误（仅开发构建注册该命令）。
 *
 * 用途：验证"后端分类 → 脱敏 → IPC → 前端 i18n → Toast → 动作按钮"整条链路。
 * 它是基础设施的自检入口：链路坏掉时不会有任何业务功能报错，
 * 只会在真正出错那天集体失效，所以需要能随时主动触发。
 */
export function debugThrowError(code: string): Promise<void> {
  return invokeCommand<void>('debug_throw_error', { code });
}
