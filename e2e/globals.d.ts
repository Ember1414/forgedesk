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

/** mock 记录的一次提交命令调用（prepare / execute）。 */
export interface MockCommitCall {
  readonly command: string;
  readonly args: {
    readonly repoId?: number;
    readonly planId?: string;
    readonly spec?: {
      readonly message?: string;
      readonly description?: string;
      readonly amend?: boolean;
      readonly signOff?: boolean;
      readonly noVerify?: boolean;
      readonly sign?: string;
    };
  };
}

/** mock 记录的一次快照命令调用。 */
export interface MockSnapshotCall {
  readonly command: string;
  readonly args: { readonly repoId?: number; readonly snapshotId?: number };
}

/** mock 里的一条提交记录。 */
export interface MockCommit {
  readonly oid: string;
  readonly subject: string;
}

/** mock 记录的一次审计命令调用（list / export / prune）。 */
export interface MockAuditCall {
  readonly command: string;
  readonly args: {
    readonly repoId?: number;
    readonly opType?: string;
    readonly format?: string;
    readonly limit?: number;
    readonly offset?: number;
  };
}

/**
 * M1 闭环用例（`e2e/loop.spec.ts`）的 mock 状态。
 *
 * 与其它 spec 的钩子分开命名：闭环用例要同时断言"界面传了什么"与"仓库状态怎么变"，
 * 因此它需要一份能读写的共享状态，而不是只记录调用的数组。
 */
export interface MockLoopState {
  readonly calls: {
    readonly command: string;
    readonly args?: unknown;
    readonly spec?: unknown;
    readonly paths?: readonly string[];
  }[];
  readonly files: readonly {
    readonly path: string;
    indexStatus: string;
    worktreeStatus: string;
  }[];
  readonly commits: readonly { readonly oid: string; readonly subject: string }[];
  readonly hookRejects: boolean;
  prepared?: string;
}

/** mock 记录的一次同步命令（`sync.spec.ts` 断言界面传的策略与被拒后的重推标志）。 */
export interface MockSyncCall {
  readonly command: string;
  readonly args?: {
    readonly repoId?: number;
    readonly spec?: {
      readonly remote?: string | null;
      readonly strategy?: string;
      readonly setUpstream?: boolean;
      readonly forceWithLease?: boolean;
    };
  };
}

/**
 * mock 记录的一次凭据命令（`credentials.spec.ts` 断言保存的载荷、删除的目标与探活地址）。
 *
 * `secret` / `passphrase` 在这里被记录下来是**测试专用**的：真实运行里它们的唯一去处是
 * 系统凭据库（红线 R8），而 e2e 要断言的正是"明文只经一次 IPC、之后不再出现在界面上"。
 */
export interface MockCredentialCall {
  readonly command: string;
  readonly args?: {
    readonly provider?: string;
    readonly host?: string;
    readonly login?: string;
    readonly kind?: string;
    readonly secret?: string;
    readonly passphrase?: string;
    readonly url?: string;
  };
}

declare global {
  interface Window {
    /** Tauri v2 的 IPC 入口（mock 注入；e2e 直接驱调用它来准备状态）。 */
    __TAURI_INTERNALS__?: {
      readonly transformCallback: (callback: unknown) => number;
      readonly unregisterListener: () => void;
      readonly invoke: (command: string, args?: unknown) => Promise<unknown>;
    };
    /** M1 闭环用例的 mock 状态（见上）。 */
    __loop?: MockLoopState;
    /** mock 记录收到的暂存 / 取消暂存请求（断言"界面选的粒度与下标"是否原样传到后端）。 */
    __stagingCalls?: MockStagingCall[];
    /** mock 的文件夹具（断言部分暂存后的索引 / 工作区状态）。 */
    __mockFiles?: MockFileChange[];
    /** mock 记录收到的提交命令（断言预览与执行用的是同一份数据）。 */
    __commitCalls?: MockCommitCall[];
    /** mock 的提交历史（断言 amend 后提交数不变、oid 变化）。 */
    __mockCommits?: MockCommit[];
    /** mock 记录收到的快照命令（断言回滚用的是预览的那一个快照）。 */
    __snapshotCalls?: MockSnapshotCall[];
    /**
     * 手动投递一次 `repo:changed`（模拟文件监听发出的外部变化，T1.10）。
     *
     * `kind` 缺省 `workspace`；传 `large` 可以验证"大量变更"的界面说明。
     */
    __emitRepoChanged?: (paths: string[], kind?: string) => void;
    /** mock 记录收到的审计命令（断言导出带的格式、列表带的筛选）。 */
    __auditCalls?: MockAuditCall[];
    /** mock 记录收到的同步命令（T2.6：断言策略、`--set-upstream`、force-with-lease）。 */
    __syncCalls?: MockSyncCall[];
    /** 手动投递一个 `job:*` 事件（T2.6：长任务的进度 / 完成 / 失败由事件到达）。 */
    __emitJob?: (event: string, payload: unknown) => void;
    /** mock 记录收到的凭据命令（T2.7：断言明文只发一次、删除目标、探活地址）。 */
    __credCalls?: MockCredentialCall[];
    /** stash e2e 的 mock 调用记录（T2.10：断言 apply / drop 到达后端）。 */
    __calls?: readonly { readonly command: string; readonly index?: number }[];
    /** 冲突页 e2e 的 mock 调用记录（T3.1：断言 resolve / continue / abort 到达后端）。 */
    __conflictCalls?: readonly { readonly command: string; readonly args?: unknown }[];
    /** 合并 e2e 的 mock 调用记录（T3.4：断言 prepare / execute / abort 到达后端）。 */
    __mergeCalls?: readonly { readonly command: string; readonly args?: unknown }[];
    /** reset e2e 的执行计数（T2.10：断言取消后 execute 从未被调用）。 */
    __resetExecuted?: number;
    /** 快照页 e2e：自定义手动打点的结果（T3.8 的超限告警场景）。 */
    __createOutcome?: unknown;
    /** 快照页 e2e：自定义回滚报告（T3.9 的紧急模式场景）。 */
    __restoreOutcome?: unknown;
    /** 快照页 e2e：注入一个"上次回滚没走完"的标记（T3.9 崩溃恢复）。 */
    __pendingRestore?: unknown;
    /** 操作历史 e2e：mock 的记录清单（T3.10，可随时替换以模拟"刚做完一次操作"）。 */
    __opRecords?: unknown[];
    /** 操作历史 e2e：回滚执行次数（断言"再滚一次"真的到了后端）。 */
    __restoreCount?: number;
    /**
     * rebase 面板 e2e 的 mock 调用记录（T3.6）。
     *
     * 记录 preview 与 execute 的 steps：验收要求"执行的新历史与预览一致"，
     * 在 E2E 层的等价断言是两份 steps 逐字相同（界面不能执行时换计划）。
     */
    __rebaseCalls?: readonly {
      readonly command: string;
      readonly steps?: readonly {
        readonly oid: string;
        readonly action: string;
        readonly newMessage?: string;
      }[];
    }[];
    /** rebase 面板 e2e：自定义执行结局（冲突 / edit 暂停场景）。 */
    __rebaseExecuteOutcome?: unknown;
    /** rebase 面板 e2e：自定义 continue_edit 结局。 */
    __rebaseContinueOutcome?: unknown;
    /**
     * GitHub 区域 e2e 的 mock 调用记录（T4.12）。
     *
     * 断言"界面把什么发给了后端"：PR 列表的站点/所有者/状态过滤、
     * 行内评论的锚点（路径 + 侧 + 行号）、review 的结论与正文。
     */
    __githubCalls?: readonly {
      readonly command: string;
      readonly args?: {
        readonly host?: string;
        readonly owner?: string;
        readonly repo?: string;
        readonly number?: number;
        readonly stateFilter?: string;
        readonly page?: number;
        readonly path?: string;
        readonly side?: string;
        readonly line?: number;
        readonly body?: string;
        readonly event?: string;
      };
    }[];
    /**
     * GitHub 区域 e2e：把托管接口切成"未登录"（T4.12 / M4 验收第 9 条）。
     *
     * 放在 mock 上而不是改数据：要验证的是**同一个错误码在多个页面都走同一条引导**，
     * 因此需要在运行中切换，而不是换个夹具再跑一遍。
     */
    __githubAuthRequired?: boolean;
    /** 终端 e2e 的 mock 调用记录（T5.2：term_write / resize / close / create）。 */
    __termWrites?: readonly { readonly termId: string; readonly text: string }[];
    __termResize?: readonly {
      readonly termId: string;
      readonly cols: number;
      readonly rows: number;
    }[];
    __termClosed?: readonly string[];
    __createdSessions?: readonly { readonly termId: string; readonly request: unknown }[];
    /** 编辑器实例（EditorPanel onMount 挂载；E2E 用公共 API 编辑 model）。 */
    __forgedeskEditor?: {
      getModel?: () => {
        getFullModelRange: () => unknown;
        applyEdits: (edits: unknown) => void;
      } | null;
    };
    /** 编辑器 e2e：git_blame 调用计数（T5.8）。 */
    __blameCalls?: number;
    /** 编辑器 e2e：mock 记录的 fs_write 请求（T5.7）。 */
    __fsWrites?: readonly {
      readonly path: string;
      readonly content: string;
      readonly eol: string;
      readonly hasBom: boolean;
    }[];
    /** mock 记录的强制登记请求（T5.3：断言 term_report_command 到达后端）。 */
    __termReports?: readonly {
      readonly repoId: number;
      readonly line: string;
      readonly kind: string;
      readonly autoSnapshot: boolean;
    }[];
    /** mock 的扫描级别（T5.3：可被用例切换）。 */
    __scanLevel?: string;
    /** 手动投递一次终端输出 / 退出事件（mock 后端的 push 通道）。 */
    __emitTermOutput?: (termId: string, text: string) => void;
    __emitTermExit?: (termId: string, code: number | null) => void;
    /** 终端缓冲文本（xterm canvas 渲染器无 DOM 文本，E2E 从缓冲断言；manager.ts 挂载）。 */
    __forgedeskTermText?: (termId: string) => string;
    /** 诊断 e2e 的 mock 记录（T5.6：system_diagnose_error 的输入与打开的 URL）。 */
    __diagCalls?: readonly string[];
    __openUrls?: readonly string[];
  }
}
