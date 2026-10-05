/**
 * 内嵌终端（T5.2）：`term_*` 命令族与输出 / 退出事件。
 *
 * 传输契约（docs/PTY-SPIKE.md §3.1）：
 * - 输出方向是胖载荷 → base64 字符串（16ms 合并块）；
 * - 输入方向是键盘字节 → `Uint8Array`（JSON 数字数组，一次几字节，体积无所谓）。
 *
 * 事件订阅的组件必须在卸载时调用 `unlisten`（否则监听器泄漏、事件重复处理）。
 */
import { invokeCommand, listenEvent, type Unlisten } from './client';

/** 终端输出事件（base64 块）。 */
export const EVENT_TERM_OUTPUT = 'term:output';
/** 终端退出事件。 */
export const EVENT_TERM_EXIT = 'term:exit';

/** `term_create` 的返回。 */
export interface TermCreated {
  readonly termId: string;
  readonly program: string;
}

/** `term_create` 的请求。 */
export interface TermCreateRequest {
  readonly repoId: number;
  readonly shell?: string;
  readonly cwd?: string;
  readonly cols: number;
  readonly rows: number;
  readonly env?: Readonly<Record<string, string>>;
}

/** `term_list` 的元素。 */
export interface TermSummary {
  readonly id: string;
  readonly repoId: number;
  readonly program: string;
  readonly exited: boolean;
}

/** `term_shell_list` 的元素（展示名由 i18n 按 id 提供）。 */
export interface TermShell {
  readonly id: string;
  readonly program: string;
}

/** 输出事件载荷。 */
export interface TermOutputPayload {
  readonly termId: string;
  readonly data: string;
}

/** 退出事件载荷（`code` 为 null 表示后端拿不到退出码）。 */
export interface TermExitPayload {
  readonly termId: string;
  readonly code: number | null;
}

/** 创建终端会话。 */
export function termCreate(request: TermCreateRequest): Promise<TermCreated> {
  return invokeCommand<TermCreated>('term_create', { request });
}

/** 向会话写入键盘字节。 */
export function termWrite(termId: string, data: Uint8Array): Promise<void> {
  return invokeCommand<void>('term_write', { termId: termId, data: Array.from(data) });
}

/** 调整会话尺寸（前端 xterm 视图变化时调用）。 */
export function termResize(termId: string, cols: number, rows: number): Promise<void> {
  return invokeCommand<void>('term_resize', { termId, cols, rows });
}

/** 关闭并移除会话。 */
export function termClose(termId: string): Promise<void> {
  return invokeCommand<void>('term_close', { termId });
}

/** 列出全部会话（跨仓库）。 */
export function termList(): Promise<TermSummary[]> {
  return invokeCommand<TermSummary[]>('term_list');
}

/** 读取会话尾部输出（会话退出后依然可读；后端上限 1000 行）。 */
export function termOutputTail(termId: string, lines?: number): Promise<string[]> {
  return invokeCommand<string[]>('term_output_tail', {
    termId,
    ...(lines === undefined ? {} : { lines }),
  });
}

/** 列出本平台可选的 shell（"+" 菜单数据源）。 */
export function termShellList(): Promise<TermShell[]> {
  return invokeCommand<TermShell[]>('term_shell_list');
}

/** 订阅输出事件。 */
export function listenTermOutput(handler: (payload: TermOutputPayload) => void): Promise<Unlisten> {
  return listenEvent<TermOutputPayload>(EVENT_TERM_OUTPUT, handler);
}

/** 订阅退出事件。 */
export function listenTermExit(handler: (payload: TermExitPayload) => void): Promise<Unlisten> {
  return listenEvent<TermExitPayload>(EVENT_TERM_EXIT, handler);
}

/**
 * 用系统默认浏览器打开 http(s) 链接（终端里的链接识别用）。
 *
 * 后端只接受 http/https 且无空白/控制字符的 URL，其余返回 `VALIDATION`——
 * 终端输出里的"链接"是任意字符串，不能把它原样交给操作系统。
 */
export function systemOpenUrl(url: string): Promise<void> {
  return invokeCommand<void>('system_open_url', { url });
}
