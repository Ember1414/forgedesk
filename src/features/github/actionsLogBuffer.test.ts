/** 对真实缓冲验证资源上限与文本完整性，而非依赖 DOM 虚拟化推断内存。 */
import { describe, expect, it } from 'vitest';

import {
  ActionsLogBuffer,
  MAX_LOG_CHARACTERS,
  MAX_LOG_LINE_LENGTH,
  MAX_LOG_LINES,
  logTextTail,
} from './actionsLogBuffer';

describe('ActionsLogBuffer', () => {
  it('保留空行、CRLF 和没有末尾换行的最后一行', () => {
    const buffer = new ActionsLogBuffer();
    buffer.append('first\r\n\r\nlast', 3);
    buffer.append('', 3);
    expect(buffer.snapshot()).toEqual({
      lines: ['first', '', 'last'],
      totalLines: 3,
      truncated: false,
    });
  });

  it('持续接收日志时只保留最近行并继续累计总行数', () => {
    const buffer = new ActionsLogBuffer();
    for (let index = 0; index < MAX_LOG_LINES + 15; index += 1) {
      buffer.append(`line-${index}\n`, index + 1);
    }
    const snapshot = buffer.snapshot();
    expect(snapshot.lines).toHaveLength(MAX_LOG_LINES);
    expect(snapshot.lines[0]).toBe('line-15');
    expect(snapshot.lines.at(-1)).toBe(`line-${MAX_LOG_LINES + 14}`);
    expect(snapshot.totalLines).toBe(MAX_LOG_LINES + 15);
    expect(snapshot.truncated).toBe(true);
  });

  it('行数不多的大日志也受总字符数上限约束', () => {
    const buffer = new ActionsLogBuffer();
    const line = 'x'.repeat(MAX_LOG_LINE_LENGTH);
    for (let index = 0; index < 1000; index += 1) buffer.append(`${line}\n`, index + 1);
    const snapshot = buffer.snapshot();
    expect(snapshot.lines.reduce((sum, item) => sum + item.length, 0)).toBeLessThanOrEqual(
      MAX_LOG_CHARACTERS,
    );
    expect(snapshot.lines.length).toBeLessThan(1000);
    expect(snapshot.totalLines).toBe(1000);
    expect(snapshot.truncated).toBe(true);
  });

  it('单个超长行保留末尾且不会从表情符号中间截断', () => {
    const buffer = new ActionsLogBuffer();
    buffer.append(`prefix😀${'x'.repeat(MAX_LOG_LINE_LENGTH - 1)}\n`, 1);
    const snapshot = buffer.snapshot();
    expect(snapshot.lines[0]).toBe('x'.repeat(MAX_LOG_LINE_LENGTH - 1));
    expect(snapshot.truncated).toBe(true);
    expect(logTextTail('prefix😀end', 5)).toBe('😀end');
    expect(logTextTail('prefix😀end', 4)).toBe('end');
  });
});
