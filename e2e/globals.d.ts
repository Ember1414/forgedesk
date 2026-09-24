/**
 * E2E 专用：mock 脚本注入的调试钩子。
 *
 * 为什么不放进 `src/main.tsx` 的全局声明：那些是**生产代码**里的窗口字段
 * （`__errs` 是真实存在的未捕获错误集合），而下面这些只存在于测试注入的 mock 里。
 * 让生产类型带着测试字段，会让"这个字段到底存不存在"变得无法判断。
 */

/** mock 里的一份变更文件。 */
export interface MockFileChange {
  readonly path: string;
  readonly kind: string;
  readonly indexStatus: string;
  readonly worktreeStatus: string;
  readonly isBinary: boolean;
  readonly isLfs: boolean;
  readonly isSubmodule: boolean;
  readonly sizeBytes: number;
}

/** mock 记录的一次暂存 / 取消暂存请求。 */
export interface MockStagingCall {
  readonly command: string;
  readonly spec: {
    readonly kind: string;
    readonly path?: string;
    readonly paths?: readonly string[];
    readonly hunkIndices?: readonly number[];
    readonly selections?: readonly {
      readonly hunkIndex: number;
      readonly lines: readonly number[];
    }[];
  };
  readonly view: unknown;
  readonly paths: readonly string[];
}

declare global {
  interface Window {
    /** mock 记录收到的暂存 / 取消暂存请求（断言"界面选的粒度与下标"是否原样传到后端）。 */
    __stagingCalls?: MockStagingCall[];
    /** mock 的文件夹具（断言部分暂存后的索引 / 工作区状态）。 */
    __mockFiles?: MockFileChange[];
  }
}
