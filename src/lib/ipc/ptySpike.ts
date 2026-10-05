/**
 * PTY Spike 调试通道（T5.1）。
 *
 * 后端命令**只在开发构建注册**（见 `src-tauri/src/main.rs` 的 debug 列表），
 * 正式构建调用会直接报"命令不存在"。配套页面是 `src/ui/__dev__/PtySpikePanel.tsx`。
 *
 * # 传输方式：base64 字符串（T5.1 的结论）
 *
 * 终端是字节流，Tauri 2 的 IPC 载荷默认走 JSON 序列化。两个候选：
 * - `Vec<u8>`：JSON 数字数组，每字节约 3.9 字节（`123,` + 数组括号）；
 * - base64 字符串：每字节约 1.33 字符，代价是一次 O(n) 编解码。
 * 吞吐瓶颈在 PTY 与渲染，不在编解码（实测数据见 `docs/PTY-SPIKE.md`），
 * 因此选传输体积更小的 base64——键盘输入与输出块共用同一条通道。
 */
import { invokeCommand, listenEvent, type Unlisten } from './client';

/** 输出块事件（16ms 合并，base64 载荷）。 */
export const EVENT_PTY_SPIKE_OUTPUT = 'pty-spike:output';
/** 会话退出事件。 */
export const EVENT_PTY_SPIKE_EXIT = 'pty-spike:exit';

/** `pty_spike_create` 的返回。 */
export interface PtySpikeInfo {
  readonly id: string;
  readonly program: string;
}

/** `pty_spike_throughput` 的返回（后端统计口径）。 */
export interface PtySpikeThroughput {
  readonly lines: number;
  readonly bytes: number;
  readonly elapsedMs: number;
}

/** 输出块事件载荷。 */
export interface PtySpikeOutputPayload {
  readonly id: string;
  readonly data: string;
}

/** 退出事件载荷（`code` 为 null 表示 ConPTY 未报告退出码）。 */
export interface PtySpikeExitPayload {
  readonly id: string;
  readonly code: number | null;
}

/** 创建 spike 会话（无头 `<pre>` 输出区，后端自动应答 DSR）。 */
export function ptySpikeCreate(cols: number, rows: number): Promise<PtySpikeInfo> {
  return invokeCommand<PtySpikeInfo>('pty_spike_create', { cols, rows });
}

/** 向会话写入输入（`dataBase64` 为原始字节的 base64）。 */
export function ptySpikeWrite(id: string, dataBase64: string): Promise<void> {
  return invokeCommand<void>('pty_spike_write', { id, data: dataBase64 });
}

/** 调整会话尺寸。 */
export function ptySpikeResize(id: string, cols: number, rows: number): Promise<void> {
  return invokeCommand<void>('pty_spike_resize', { id, cols, rows });
}

/** 关闭并移除会话。 */
export function ptySpikeClose(id: string): Promise<void> {
  return invokeCommand<void>('pty_spike_close', { id });
}

/** 吞吐量测试：向会话灌 10 万行，阻塞至完成或 90s 超时。 */
export function ptySpikeThroughput(id: string): Promise<PtySpikeThroughput> {
  return invokeCommand<PtySpikeThroughput>('pty_spike_throughput', { id });
}

/** 订阅输出块事件（组件卸载时必须调用返回的 unlisten）。 */
export function listenPtySpikeOutput(
  handler: (payload: PtySpikeOutputPayload) => void,
): Promise<Unlisten> {
  return listenEvent<PtySpikeOutputPayload>(EVENT_PTY_SPIKE_OUTPUT, handler);
}

/** 订阅退出事件。 */
export function listenPtySpikeExit(
  handler: (payload: PtySpikeExitPayload) => void,
): Promise<Unlisten> {
  return listenEvent<PtySpikeExitPayload>(EVENT_PTY_SPIKE_EXIT, handler);
}

/**
 * UTF-8 文本 → base64（键盘输入路径）。
 *
 * 分块 `String.fromCharCode`：大输入一次性 spread 会触发参数个数上限
 * （约 65536），分块后稳定。
 */
export function utf8ToBase64(text: string): string {
  const bytes = new TextEncoder().encode(text);
  let binary = '';
  const CHUNK = 0x8000;
  for (let offset = 0; offset < bytes.length; offset += CHUNK) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + CHUNK));
  }
  return btoa(binary);
}

/**
 * 创建一个流式 base64 → 文本解码器（输出路径）。
 *
 * 为什么要"流式"：输出块按 16ms 合并，一个多字节字符（中文/emoji）会被
 * 从中间切开；`TextDecoder` 的 stream 模式会自己攒住半个字符，
 * 非流式的 `new TextDecoder().decode()` 每块都从头解，会产出替换符。
 */
export function createUtf8StreamDecoder(): (chunkBase64: string) => string {
  const decoder = new TextDecoder('utf-8');
  return (chunkBase64: string) => {
    const binary = atob(chunkBase64);
    const bytes = new Uint8Array(binary.length);
    for (let index = 0; index < binary.length; index += 1) {
      bytes[index] = binary.charCodeAt(index);
    }
    return decoder.decode(bytes, { stream: true });
  };
}
