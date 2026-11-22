/** Actions 日志的有界尾部缓冲；事件到达只更新环形队列，视图按批次取快照。 */
export const MAX_LOG_LINES = 20_000;
/** 按 UTF-16 code unit 记账，字符串内容最多约 8 MiB，不包含对象开销。 */
export const MAX_LOG_CHARACTERS = 4 * 1024 * 1024;
/** 超长单行也要限制，避免一个虚拟行包含数 MB 的文本节点。 */
export const MAX_LOG_LINE_LENGTH = 16_384;

export interface ActionsLogSnapshot {
  readonly lines: readonly string[];
  readonly totalLines: number;
  readonly truncated: boolean;
}

/** 保留末尾文本且不从 UTF-16 代理对中间截断。 */
export function logTextTail(text: string, limit: number): string {
  let start = Math.max(0, text.length - limit);
  const first = text.charCodeAt(start);
  if (start > 0 && first >= 0xdc00 && first <= 0xdfff) start += 1;
  // slice 的结果可能继续引用原始大字符串。复制截断后的文本，避免尾部视图
  // 仍间接持有整个下载块；JSON 往返也保留转义字符与 Unicode 原文。
  return start === 0 ? text : (JSON.parse(JSON.stringify(text.slice(start))) as string);
}

export class ActionsLogBuffer {
  private readonly entries: (string | undefined)[] = new Array<string>(MAX_LOG_LINES);
  private head = 0;
  private length = 0;
  private characters = 0;
  private totalLines = 0;
  private truncated = false;

  /** 早到事件队列溢出时同样必须明确告知用户。 */
  markTruncated(): void {
    this.truncated = true;
  }

  /** 输入以完整行结束，最后一个网络块允许没有末尾换行。 */
  append(text: string, totalLines: number): void {
    if (text.length > MAX_LOG_CHARACTERS) {
      text = logTextTail(text, MAX_LOG_CHARACTERS);
      this.truncated = true;
    }
    let start = 0;
    let added = 0;
    while (start < text.length) {
      const newline = text.indexOf('\n', start);
      let end = newline === -1 ? text.length : newline;
      if (end > start && text[end - 1] === '\r') end -= 1;
      const lineStart = Math.max(start, end - MAX_LOG_LINE_LENGTH);
      let line = text.slice(lineStart, end);
      if (lineStart > start) {
        line = logTextTail(text.slice(Math.max(start, lineStart - 1), end), MAX_LOG_LINE_LENGTH);
        this.truncated = true;
      }
      while (
        this.length > 0 &&
        (this.length === MAX_LOG_LINES || this.characters + line.length > MAX_LOG_CHARACTERS)
      ) {
        this.characters -= this.entries[this.head]?.length ?? 0;
        this.entries[this.head] = undefined;
        this.head = (this.head + 1) % MAX_LOG_LINES;
        this.length -= 1;
        this.truncated = true;
      }
      this.entries[(this.head + this.length) % MAX_LOG_LINES] = line;
      this.length += 1;
      this.characters += line.length;
      added += 1;
      start = newline === -1 ? text.length : newline + 1;
    }
    this.totalLines = Math.max(this.totalLines + added, totalLines);
    if (this.totalLines > this.length) this.truncated = true;
  }

  /** 只在批量刷新或终态时复制当前保留窗口。 */
  snapshot(): ActionsLogSnapshot {
    return {
      lines: Array.from(
        { length: this.length },
        (_, index) => this.entries[(this.head + index) % MAX_LOG_LINES] ?? '',
      ),
      totalLines: this.totalLines,
      truncated: this.truncated,
    };
  }
}
