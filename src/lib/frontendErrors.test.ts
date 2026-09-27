import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import {
  describeError,
  extractError,
  installFrontendErrorHooks,
  KEEP_ERRORS,
  REPORT_LIMIT,
} from '@/lib/frontendErrors';

/**
 * `window.__errs` 是 PLAN §10 的 DoD 闸门，所以它自己必须被测：
 *
 *   - 开发 / E2E：错误进数组，且**有上限**（错误数量不该成为内存问题）；
 *   - 生产：数组不存在，错误走本地日志——但**有上报上限**，
 *     否则渲染循环里的 bug 会以每帧一条的速度刷爆日志；
 *   - 描述任意抛出物时不许再抛（`throw undefined`、循环引用都是合法 JS）。
 *
 * 测试用**自己的假事件目标**而不是真实 `window`：jsdom 会把没人处理的
 * `error` 事件上报给 vitest，那是脚手架的副作用，不是被测行为。
 */

/** 记录监听器的假事件目标（可注入，避免触碰 jsdom 的错误机制）。 */
function fakeTarget() {
  const listeners = new Map<string, (event: unknown) => void>();
  return {
    addEventListener: (type: string, listener: (event: unknown) => void) => {
      listeners.set(type, listener);
    },
    removeEventListener: (type: string) => {
      listeners.delete(type);
    },
    /** 派发一个事件（形状与浏览器一致：`error` 带 error，拒绝带 reason）。卸载后无人监听，什么也不做。 */
    fire: (type: 'error' | 'unhandledrejection', payload: unknown) => {
      const listener = listeners.get(type);
      if (listener === undefined) {
        return;
      }
      listener(type === 'unhandledrejection' ? { reason: payload } : { error: payload });
    },
    get size() {
      return listeners.size;
    },
  };
}

let cleanup: (() => void) | undefined;
// 一个文件里共用同一个假目标：卸载函数会把监听清干净，不必每个用例换一个对象
const target = fakeTarget();

beforeEach(() => {
  delete window.__errs;
});

afterEach(() => {
  cleanup?.();
  cleanup = undefined;
  delete window.__errs;
});

/**
 * 装一份监听并登记卸载。
 *
 * jsdom 的 `window` 在整个文件里是同一个对象：不卸载的话，前一个用例装上的
 * 监听器会继续处理后一个用例派发的事件，"一次错误"被记成两次或三次——
 * 那种失败看起来像逻辑错，实际是测试隔离问题。
 */
function install(options: Parameters<typeof installFrontendErrorHooks>[0]): void {
  cleanup = installFrontendErrorHooks(options);
}

describe('describeError', () => {
  it('Error 取消息与调用栈', () => {
    const described = describeError(new Error('boom'));
    expect(described.message).toBe('boom');
    expect(described.stack).toContain('boom');
  });

  it('字符串原样使用', () => {
    expect(describeError('plain failure').message).toBe('plain failure');
  });

  it('对象 JSON 化，循环引用时退回 String（不许在错误处理里再抛）', () => {
    expect(describeError({ code: 7 }).message).toBe('{"code":7}');

    const circular: Record<string, unknown> = {};
    circular['self'] = circular;
    expect(() => describeError(circular)).not.toThrow();
    expect(describeError(circular).message).toContain('Object');
  });

  it('undefined / null 也描述得出来', () => {
    expect(describeError(undefined).message).toBe('undefined');
    expect(describeError(null).message).toBe('null');
  });
});

describe('extractError', () => {
  it('error 事件取 error，缺省退回 message', () => {
    expect(extractError({ error: 'boom' })).toBe('boom');
    // 老式 window.onerror 只给 message
    expect(extractError({ message: 'legacy' })).toBe('legacy');
  });

  it('拒绝事件取 reason（哪怕 reason 是 undefined）', () => {
    expect(extractError({ reason: 'nope' })).toBe('nope');
    expect(extractError({ reason: undefined, error: 'ignored' })).toBe('ignored');
  });

  it('不是对象时原样返回', () => {
    expect(extractError('plain')).toBe('plain');
    expect(extractError(undefined)).toBeUndefined();
  });
});

describe('收集模式（开发与 E2E）', () => {
  it('未捕获异常与未处理的拒绝都进 __errs', () => {
    install({ target, collect: true });
    // 两个事件都要装上：只装 `error` 会漏掉所有"没人接的 Promise 拒绝"
    expect(target.size).toBe(2);

    target.fire('error', new Error('first'));
    target.fire('unhandledrejection', 'second');

    expect(window.__errs?.length).toBe(2);
    expect((window.__errs?.[0] as Error).message).toBe('first');
    expect(window.__errs?.[1]).toBe('second');
  });

  it('只保留最近 KEEP_ERRORS 条', () => {
    install({ target, collect: true });

    for (let index = 0; index < KEEP_ERRORS + 10; index += 1) {
      target.fire('error', `err-${index}`);
    }

    expect(window.__errs?.length).toBe(KEEP_ERRORS);
    // 丢的是最早的：用户最关心刚刚发生了什么
    expect(window.__errs?.[0]).toBe(`err-10`);
  });

  it('收集模式下不占用上报通道', () => {
    const report = vi.fn();
    install({ target, collect: true, report });

    target.fire('error', new Error('dev error'));

    expect(report).not.toHaveBeenCalled();
  });
});

describe('上报模式（生产）', () => {
  it('不创建 __errs，把错误交给上报出口', () => {
    const report = vi.fn();
    install({ target, collect: false, report });

    target.fire('error', new Error('production error'));

    expect(window.__errs).toBeUndefined();
    expect(report).toHaveBeenCalledTimes(1);
    expect(report.mock.calls[0]?.[0]).toBe('production error');
  });

  it('上报次数有上限（渲染循环出错时不能刷爆日志）', () => {
    const report = vi.fn();
    install({ target, collect: false, report, limit: 3 });

    for (let index = 0; index < 10; index += 1) {
      target.fire('error', `err-${index}`);
    }

    expect(report).toHaveBeenCalledTimes(3);
  });

  it('缺省上限是 REPORT_LIMIT', () => {
    const report = vi.fn();
    install({ target, collect: false, report });

    for (let index = 0; index < REPORT_LIMIT + 5; index += 1) {
      target.fire('unhandledrejection', `err-${index}`);
    }

    expect(report).toHaveBeenCalledTimes(REPORT_LIMIT);
  });

  it('上报出口自己抛错时不再往上抛（否则错误会无限繁殖）', () => {
    install({
      collect: false,
      report: () => {
        throw new Error('reporter is broken');
      },
    });

    expect(() => target.fire('error', new Error('original'))).not.toThrow();
  });
});

describe('安装', () => {
  it('重复调用只安装一次（同一条错误不会被记两次）', () => {
    // 第二次调用返回的是空卸载函数：登记"真正装上"的那一次给 afterEach
    const first = installFrontendErrorHooks({ target, collect: true });
    install({ target, collect: true });
    cleanup = first;

    target.fire('error', 'once');

    expect(window.__errs?.length).toBe(1);
  });

  it('卸载后不再收集（否则同一条错误会被后来的监听重复记录）', () => {
    install({ target, collect: true });
    target.fire('error', 'before');
    expect(window.__errs?.length).toBe(1);

    cleanup?.();
    cleanup = undefined;

    target.fire('error', 'after');
    expect(window.__errs?.length).toBe(1);
  });
});
