/**
 * 终端实例管理器（T5.2）。
 *
 * # 为什么不是 React 状态
 *
 * xterm 的 `Terminal` 是命令式的重对象（一个 canvas + 内部缓冲区）。React 的
 * 生命周期（尤其 StrictMode 的双挂载）与"输出事件流不等人"的现实放在一起，
 * 最稳的结构是：实例与待输出缓冲住在**模块级注册表**里，React 组件只在
 * 挂载/卸载时注册/注销自己那个实例。
 *
 * # 三个职责
 *
 * 1. **输出路由**：全局的 `term:output` 监听只有一份（懒建立、活到应用退出），
 *    按 termId 路由到已注册的实例；实例不存在（标签页没挂载）时进 pending
 *    缓冲（上限 1MB，超限丢最旧并插入截断标记），注册时排干——保证
 *    "切到别的页面再回来"不丢输出、不重复输出。
 * 2. **事件接线**：`term:exit` 直接进 store（标签角标与关闭提示的来源）。
 * 3. **全局生效项**：主题切换 / 字号变化要作用到**所有**实例，调用方（页面）
 *    不需要逐个找到它们。
 */
import type { Terminal } from '@xterm/xterm';
import type { FitAddon } from '@xterm/addon-fit';
import type { SearchAddon } from '@xterm/addon-search';

import { isTauriRuntime, listenTermExit, listenTermOutput, systemOpenUrl } from '@/lib/ipc';
import { useTerminalStore } from '@/stores/terminalStore';

/** 单会话待输出缓冲上限；超过即丢最旧（真实输出以 100k 行/s 计，1MB 约几秒的量）。 */
const MAX_PENDING_BYTES = 1024 * 1024;

// i18n-ignore：插进终端输出流的原始字节标记，不在 React 渲染树里，无处走 t()
const TRUNCATION_MARK = '\r\n[ForgeDesk: 输出过多，较早的内容已省略]\r\n'; // i18n-ignore

/** 一个受管终端实例。 */
export interface ManagedTerminal {
  readonly term: Terminal;
  readonly fit: FitAddon;
  readonly search: SearchAddon;
}

const instances = new Map<string, ManagedTerminal>();
const pending = new Map<string, Uint8Array[]>();
const pendingBytes = new Map<string, number>();
let listenersReady = false;

/** base64 → 字节（输出方向；一次一个合并块，几十 KB 量级）。 */
function base64ToBytes(data: string): Uint8Array {
  const binary = atob(data);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  return bytes;
}

function deliverOutput(termId: string, bytes: Uint8Array): void {
  const managed = instances.get(termId);
  if (managed) {
    managed.term.write(bytes);
    return;
  }
  let queued = pendingBytes.get(termId) ?? 0;
  const queue = pending.get(termId) ?? [];
  if (queued + bytes.length > MAX_PENDING_BYTES) {
    // 丢最旧的块直到装得下：缓冲是"离屏暂存"，不是完整的回滚记录
    let dropped = 0;
    while (queue.length > 0 && queued + bytes.length - dropped > MAX_PENDING_BYTES) {
      const oldest = queue.shift();
      if (!oldest) {
        break;
      }
      dropped += oldest.length;
    }
    const mark = new TextEncoder().encode(TRUNCATION_MARK);
    queue.unshift(mark);
    queued = queued - dropped + mark.length;
  }
  queue.push(bytes);
  pending.set(termId, queue);
  pendingBytes.set(termId, queued);
}

/**
 * OSC 0/2 标题序列：`ESC ] 0;<title> BEL`（或 ST 结尾）。
 *
 * shell 在执行命令时会改窗口标题（cmd / PowerShell 都会），把它用作标签标题
 * 能天然显示"当前运行的命令"。跨块切开的序列扫不到——标题短且低频，
 * 下一帧同一标题会再来，漏一次无感（刻意不做跨块拼接状态机）。
 */
// 终端协议本身就是控制字符：正则匹配 ESC/BEL 正是它的职责
// eslint-disable-next-line no-control-regex -- 见上
const OSC_TITLE_PATTERN = /\x1b\](?:0|2);([^\x07\x1b]{1,120})(?:\x07|\x1b\\)/;

/**
 * 导出仅供测试：输出路径在 jsdom 里无法端到端驱动，纯函数部分直接断言。
 */
export function scanOscTitle(termId: string, bytes: Uint8Array): void {
  if (!bytes.includes(0x1b)) {
    return;
  }
  const match = OSC_TITLE_PATTERN.exec(new TextDecoder('utf-8', { fatal: false }).decode(bytes));
  if (match?.[1]) {
    useTerminalStore.getState().setTitleAuto(termId, match[1]);
  }
}

/** 建立全局事件监听（幂等；非 Tauri 运行时跳过——浏览器预览与 jsdom 无事件源）。 */
export function ensureTerminalListeners(): void {
  if (listenersReady || !isTauriRuntime()) {
    return;
  }
  listenersReady = true;
  void listenTermOutput(({ termId, data }) => {
    const bytes = base64ToBytes(data);
    deliverOutput(termId, bytes);
    scanOscTitle(termId, bytes);
  });
  void listenTermExit(({ termId, code }) => {
    useTerminalStore.getState().setExited(termId, code);
  });
}

/** 注册实例并排干待输出缓冲（TerminalView 挂载时调用）。 */
export function registerTerminal(termId: string, managed: ManagedTerminal): void {
  instances.set(termId, managed);
  const queue = pending.get(termId);
  if (queue) {
    for (const bytes of queue) {
      managed.term.write(bytes);
    }
    pending.delete(termId);
    pendingBytes.delete(termId);
  }
}

/** 注销实例（TerminalView 卸载时调用；之后输出回到 pending 缓冲）。 */
export function unregisterTerminal(termId: string): void {
  instances.delete(termId);
}

/** 主题切换后刷新所有实例的颜色（含未挂载的没有实例，无需处理）。 */
export function applyThemeToAll(): void {
  // 延迟 require 避免循环依赖？——不，theme 派生是纯函数，直接 import。
  // 这里动态 import 只是为了让 jest/jsdom 环境不必解析 xterm 主包。
  void import('./xtermTheme').then(({ deriveXtermTheme }) => {
    const theme = deriveXtermTheme();
    for (const managed of instances.values()) {
      managed.term.options.theme = theme;
    }
  });
}

/** 字号 / 行高变化后应用到所有实例（随后由调用方重新 fit）。 */
export function applyFontToAll(fontSize: number, lineHeight: number): void {
  for (const managed of instances.values()) {
    managed.term.options.fontSize = fontSize;
    managed.term.options.lineHeight = lineHeight;
  }
}

/** 会话是否已打开（E2E 与测试用）。 */
export function hasTerminalInstance(termId: string): boolean {
  return instances.has(termId);
}

/** 取受管实例（菜单动作、搜索、测试用）；不存在时为 undefined。 */
export function findTerminalInstance(termId: string): ManagedTerminal | undefined {
  return instances.get(termId);
}

/**
 * 命令历史（"在新标签中恢复上次命令"）：按仓库保存在 localStorage。
 *
 * 任务书明确"应用重启后不恢复会话"，但历史命令是纯文本、恢复无害——
 * 用 localStorage 而不是 settings 表：它是终端的便签性质数据，不值得占
 * 一条 IPC 往返；上限 30 条，只在此模块读写。
 */
const HISTORY_LIMIT = 30;

function historyKey(repoId: number): string {
  return `forgedesk.terminal.history.${repoId}`;
}

/** 记录一条命令（Enter 提交时调用；空行与纯空白不记）。 */
export function recordCommand(repoId: number, line: string): void {
  const trimmed = line.trim();
  if (trimmed === '' || typeof localStorage === 'undefined') {
    return;
  }
  try {
    const history = readHistory(repoId).filter((entry) => entry !== trimmed);
    history.push(trimmed);
    const clipped = history.slice(-HISTORY_LIMIT);
    localStorage.setItem(historyKey(repoId), JSON.stringify(clipped));
  } catch {
    // localStorage 满 / 被禁用：历史是便利功能，失败即放弃
  }
}

/** 最近一次执行的命令（没有则 null）。 */
export function getLastCommand(repoId: number): string | null {
  const history = readHistory(repoId);
  return history.at(-1) ?? null;
}

function readHistory(repoId: number): string[] {
  try {
    const raw = localStorage.getItem(historyKey(repoId));
    if (raw === null) {
      return [];
    }
    const parsed: unknown = JSON.parse(raw);
    return Array.isArray(parsed)
      ? parsed.filter((entry): entry is string => typeof entry === 'string')
      : [];
  } catch {
    return [];
  }
}

/**
 * 读取会话当前缓冲里的全部文本（E2E 断言与问题诊断用；挂在 window 上）。
 *
 * 为什么需要：xterm 的 WebGL / canvas 渲染器把文字画进 canvas，
 * DOM 里没有可断言的文本节点——E2E（`e2e/terminal.spec.ts`）只能从
 * 缓冲对象读内容。只读、无副作用。
 */
export function terminalVisibleText(termId: string): string {
  const managed = instances.get(termId);
  if (!managed) {
    return '';
  }
  const buffer = managed.term.buffer.active;
  const lines: string[] = [];
  for (let index = 0; index < buffer.length; index += 1) {
    const line = buffer.getLine(index);
    if (line) {
      lines.push(line.translateToString(true));
    }
  }
  return lines.join('\n');
}

if (typeof window !== 'undefined') {
  window.__forgedeskTermText = terminalVisibleText;
}

/** 链接点击的统一出口：http(s) 交给系统默认浏览器（后端二次校验协议）。 */
export function openExternalUrl(url: string): void {
  void systemOpenUrl(url).catch((error) => {
    // 打不开的链接（非法 URL 被 VALIDATION 拒绝）不阻断终端
    console.warn('open external url rejected', url, error);
  });
}
