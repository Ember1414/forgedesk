import { describe, expect, it } from 'vitest';

import i18n from '@/lib/i18n';
import {
  ERROR_CODES,
  errorHintKey,
  errorTitleKey,
  isAppError,
  isErrorCode,
  normalizeError,
} from '@/lib/errors';

/**
 * 错误归一化的测试重点：
 *
 * 1. **形态收敛**：Tauri 的 `invoke` 在命令不存在、参数错误等情况下抛的是字符串或对象，
 *    不是 AppError。漏掉任何一种形态，用户就会看到 `[object Object]`。
 * 2. **契约对齐**：错误码清单必须与 Rust `ErrorCode` 一致，
 *    且每个码在两种语言下都有标题与建议——否则界面会显示裸 key
 *    （这比硬编码中文更糟：用户完全看不懂）。
 */
describe('normalizeError', () => {
  it('原样保留后端返回的 AppError', () => {
    const error = normalizeError({
      code: 'NETWORK',
      message: 'fetch failed',
      detail: 'fatal: could not resolve host',
      hint: '检查网络',
      actions: [{ id: 'retry', labelKey: 'errors.actions.refresh', command: 'git_status' }],
      retryable: true,
    });

    expect(error.code).toBe('NETWORK');
    expect(error.detail).toBe('fatal: could not resolve host');
    expect(error.hint).toBe('检查网络');
    expect(error.actions).toHaveLength(1);
    expect(error.actions[0]?.command).toBe('git_status');
    expect(error.retryable).toBe(true);
  });

  it('兼容缺少可选字段的 AppError', () => {
    const error = normalizeError({ code: 'VALIDATION', message: 'bad input' });

    expect(error.code).toBe('VALIDATION');
    expect(error.detail).toBeUndefined();
    expect(error.hint).toBeUndefined();
    expect(error.actions).toEqual([]);
    expect(error.retryable).toBe(false);
  });

  it('把 Tauri 的字符串错误收敛为 INTERNAL 并保留原文', () => {
    const error = normalizeError('Command debug_throw_error not found');

    expect(error.code).toBe('INTERNAL');
    expect(error.message).toBe('Command debug_throw_error not found');
  });

  it('把 JS 异常收敛为 INTERNAL', () => {
    const error = normalizeError(new TypeError('x is not a function'));
    expect(error.code).toBe('INTERNAL');
    expect(error.message).toBe('x is not a function');
  });

  it('未知形状不会被序列化成 [object Object]', () => {
    expect(normalizeError({ weird: true }).message).toBe('{"weird":true}');
    expect(normalizeError(undefined).message).toBe('unknown error');
    expect(normalizeError(null).message).toBe('null');
  });

  it('丢弃字段不完整的动作（缺 command 点了没反应，缺 labelKey 会显示裸 key）', () => {
    const error = normalizeError({
      code: 'INTERNAL',
      message: 'x',
      actions: [
        { id: 'a', labelKey: 'errors.actions.refresh' },
        { id: 'b', command: 'app_version' },
        { id: 'c', labelKey: 'errors.actions.refresh', command: 'app_version' },
      ],
    });

    expect(error.actions).toHaveLength(1);
    expect(error.actions[0]?.id).toBe('c');
  });
});

describe('错误码判定与 key 映射', () => {
  it('isErrorCode 只接受清单内的码', () => {
    expect(isErrorCode('GIT_CONFLICT')).toBe(true);
    expect(isErrorCode('git_conflict')).toBe(false);
    expect(isErrorCode('NEW_CODE_FROM_FUTURE_BACKEND')).toBe(false);
    expect(isErrorCode(123)).toBe(false);
  });

  it('isAppError 以 code 是否合法为准', () => {
    expect(isAppError({ code: 'INTERNAL' })).toBe(true);
    // code 非法时不能当成 AppError：否则前端会用错误码去查 i18n，显示裸 key
    expect(isAppError({ code: 'WHATEVER' })).toBe(false);
    expect(isAppError('boom')).toBe(false);
  });

  it('key 映射保持 errors 命名空间前缀', () => {
    expect(errorTitleKey('PATH_NOT_REPO')).toBe('PATH_NOT_REPO.title');
    expect(errorHintKey('PATH_NOT_REPO')).toBe('PATH_NOT_REPO.hint');
  });
});

/**
 * 契约对齐：这是本文件最重要的断言。
 * Rust 侧新增错误码而前端漏加文案时，这条会立刻红——而不是等用户看到裸 key。
 */
describe('错误码与 i18n 契约', () => {
  it.each([...ERROR_CODES])('%s 在两种语言下都有 title 与 hint', (code) => {
    for (const language of ['zh-CN', 'en-US'] as const) {
      const title = i18n.getFixedT(language, 'errors')(errorTitleKey(code));
      const hint = i18n.getFixedT(language, 'errors')(errorHintKey(code));

      expect(title).not.toBe(errorTitleKey(code));
      expect(hint).not.toBe(errorHintKey(code));
      expect(title.length).toBeGreaterThan(0);
      expect(hint.length).toBeGreaterThan(0);
    }
  });

  it('错误码清单无重复', () => {
    expect(new Set(ERROR_CODES).size).toBe(ERROR_CODES.length);
  });

  it('通用兜底文案存在（未知错误码时使用）', () => {
    const t = i18n.getFixedT('zh-CN', 'errors');
    expect(t('unknownTitle')).not.toBe('unknownTitle');
    expect(t('unknownHint')).not.toBe('unknownHint');
  });
});
