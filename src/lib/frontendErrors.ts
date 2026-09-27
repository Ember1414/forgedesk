/**
 * 未捕获错误的收集与上报（PLAN §10 的 DoD 必过项）。
 *
 * 两条去路，按构建类型分开：
 *
 * - **开发与 E2E**（`import.meta.env.DEV`）：收进 `window.__errs`。
 *   测试结束时断言它为空，是"没有错误被悄悄吞掉"的最后一道闸。
 *   执行 E2E 的 dev server 也走这条（`import.meta.env.DEV` 为真），
 *   因此断言在 E2E 里同样有效。
 * - **生产**：写进本地日志（`log_frontend_error`）。那个数组在生产里没人看，
 *   用户只会遇到"某处坏了一下"，而我们的日志里一行都没有——偶发白屏就永远查不了。
 *
 * 为什么抽成模块而不是写在 `main.tsx` 里：这里是**可测的逻辑**（怎么描述一个
 * 任意值、上报限流、监听安装），而 `main.tsx` 的职责只是"在渲染前调用一次"。
 */

import { logFrontendError } from '@/lib/ipc';

declare global {
  interface Window {
    /**
     * 未捕获错误集合（E2E 与人工验收断言其为空；仅保留最近 50 条）。
     *
     * **只在开发与 E2E 构建里存在**：生产环境这些错误走本地日志
     * （见 `installFrontendErrorHooks`）。因此 E2E 的断言必须在 dev server 下跑，
     * 否则 `window.__errs ?? []` 会变成一个永远为空的假闸门。
     */
    __errs?: unknown[];
  }
}

/** 保留的最近错误条数：错误数量不该成为内存问题。 */
export const KEEP_ERRORS = 50;

/**
 * 生产环境每次会话最多上报多少条。
 *
 * 渲染循环里出错会以每帧一条的速度触发；没有上限时日志会被一个 bug 刷爆，
 * 反而把发生顺序里最早、最有价值的那几条挤掉。
 */
export const REPORT_LIMIT = 20;

/** 把任意抛出物描述成可读的 `{ message, stack }`。 */
export function describeError(error: unknown): { message: string; stack?: string } {
  if (error instanceof Error) {
    return error.stack === undefined
      ? { message: error.message }
      : { message: error.message, stack: error.stack };
  }
  if (typeof error === 'string') {
    return { message: error };
  }
  try {
    // Promise 拒绝里什么都可能有（`throw undefined` 是合法 JS）：
    // JSON 化失败（循环引用）时要退回 String()，而不是在错误处理里再抛一次
    return { message: JSON.stringify(error) ?? String(error) };
  } catch {
    return { message: String(error) };
  }
}

/**
 * 从事件对象里取出真正被抛出的东西。
 *
 * `error` 事件带 `error`（现代浏览器）或 `message`（`window.onerror` 风格）；
 * `unhandledrejection` 带 `reason`。三种形状都在这里收敛，
 * 调用方不需要知道监听的是哪个事件。
 */
export function extractError(event: unknown): unknown {
  if (typeof event !== 'object' || event === null) {
    return event;
  }
  const record = event as { error?: unknown; message?: unknown; reason?: unknown };
  if (record.reason !== undefined) {
    return record.reason;
  }
  if (record.error !== undefined) {
    return record.error;
  }
  if (record.message !== undefined) {
    return record.message;
  }
  return event;
}

/** 事件目标：`window` 天然满足它，测试可以换成自己的假目标。 */
export interface ErrorTarget {
  readonly addEventListener: (
    type: 'error' | 'unhandledrejection',
    listener: (event: unknown) => void,
  ) => void;
  readonly removeEventListener: (
    type: 'error' | 'unhandledrejection',
    listener: (event: unknown) => void,
  ) => void;
}

export interface InstallOptions {
  /** 是否把错误收进 `window.__errs`（开发与 E2E 为真）。 */
  readonly collect: boolean;
  /** 生产环境的上报出口；缺省用 Tauri 命令，测试里可以注入。 */
  readonly report?: (message: string, stack: string | undefined) => void;
  readonly limit?: number;
  /**
   * 事件目标；缺省 `window`。
   *
   * 测试注入自己的假目标，是为了**不派发真实的 `error` 事件**：
   * jsdom 会把没人处理的 `error` 事件上报给 vitest，输出里就多出
   * "可能有假阳性"的噪音，而那是测试脚手架的副作用，不是被测行为。
   */
  readonly target?: ErrorTarget;
}

/**
 * 安装监听，返回卸载函数。
 *
 * 全局只允许装一份（模块级标记）：`main.tsx` 之外将来可能还有入口
 * （例如崩溃恢复页），重复安装会让同一条错误被记两次。
 *
 * **返回卸载函数**不是为了让生产代码去卸载，而是：测试的 `window` 在同一文件里
 * 是同一个对象，监听器会跨用例累积，于是"一次错误"被记成三次——这种失败看起来
 * 像逻辑错，实际上是测试隔离问题。有了卸载入口，两边都干净。
 */
let installed = false;

export function installFrontendErrorHooks(options: InstallOptions): () => void {
  if (installed || typeof window === 'undefined') {
    return () => {};
  }
  installed = true;

  const target = options.target ?? window;

  if (options.collect) {
    window.__errs = [];
  }

  const limit = options.limit ?? REPORT_LIMIT;
  let reported = 0;

  const report = options.report ?? logFrontendError;
  const handle = (error: unknown): void => {
    if (options.collect) {
      const errors = window.__errs ?? (window.__errs = []);
      errors.push(error);
      if (errors.length > KEEP_ERRORS) {
        errors.splice(0, errors.length - KEEP_ERRORS);
      }
    }

    if (options.collect || reported >= limit) {
      return;
    }
    reported += 1;

    const described = describeError(error);
    try {
      report(described.message, described.stack);
    } catch {
      // 上报本身出错时什么都不要做：在错误处理里再抛会让错误无限繁殖
    }
  };

  // 两个事件走同一个处理体：`extractError` 负责把三种形状收敛成"被抛出的东西"
  const listener = (event: unknown): void => {
    handle(extractError(event));
  };

  target.addEventListener('error', listener);
  target.addEventListener('unhandledrejection', listener);

  return () => {
    target.removeEventListener('error', listener);
    target.removeEventListener('unhandledrejection', listener);
    installed = false;
  };
}
