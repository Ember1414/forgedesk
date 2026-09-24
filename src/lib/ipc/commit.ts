/**
 * 提交命令封装（T1.7）。
 *
 * 契约见 `docs/API.md` 的 `commit_prepare` / `commit_execute` / `commit_message_hint`。
 * 这里只做类型与调用，不含任何业务判断：文件清单、等价命令、钩子列表都由后端给出，
 * 前端不做"再算一遍"——同一件事两种算法迟早会不一致（而用户会同时看到两份）。
 *
 * `planId` 是**一次性**的：`commit_execute` 成功后它立即失效，重放会得到 `PLAN_STALE`。
 */
import { invokeCommand } from './client';

/** 作者身份（`Name <email>`）。 */
export interface CommitIdentity {
  readonly name: string;
  readonly email: string;
}

/** GPG 签名模式：`auto` 跟随仓库/全局配置。 */
export type CommitSignMode = 'auto' | 'yes' | 'no';

/** 准备提交计划的请求。 */
export interface PrepareCommitRequest {
  /** 提交信息**首行**（subject）。 */
  readonly message: string;
  /** 正文；空字符串与不传等价。 */
  readonly description?: string;
  /** 是否 amend 上一个提交。 */
  readonly amend?: boolean;
  /** 是否追加 `Signed-off-by`（与 GPG 签名是两件事）。 */
  readonly signOff?: boolean;
  /** 是否跳过钩子（必须由用户显式选择）。 */
  readonly noVerify?: boolean;
  /** GPG 签名模式。 */
  readonly sign?: CommitSignMode;
  /** 覆盖作者身份。 */
  readonly author?: CommitIdentity;
}

/** 计划里的一个文件。 */
export interface PlannedFile {
  readonly path: string;
  /** 索引侧状态字符（`A` / `M` / `D` / `R` / `U`），界面据此分组。 */
  readonly indexStatus: string;
}

/** 提交计划（提交前预览对话框的数据源）。 */
export interface CommitPlan {
  readonly planId: string;
  readonly repoId: number;
  readonly files: readonly PlannedFile[];
  readonly message: string;
  readonly description: string | null;
  readonly author: CommitIdentity | null;
  readonly sign: CommitSignMode;
  readonly signOff: boolean;
  readonly noVerify: boolean;
  readonly amend: boolean;
  /** 将要执行的钩子（按 git 的调用顺序）。 */
  readonly hooks: readonly string[];
  /** 可直接粘贴到终端的等价 git 命令。 */
  readonly equivalentCommand: string;
  readonly headOid: string | null;
  readonly indexFingerprint: string;
  readonly createdAtMs: number;
  readonly expiresAtMs: number;
  readonly subject: string;
  readonly subjectChars: number;
  /** 不阻断的建议（稳定短名，走 i18n）。 */
  readonly warnings: readonly string[];
}

/** 提交结果。 */
export interface CommitOutcome {
  readonly oid: string;
  readonly subject: string;
  /** 关联的快照 id（M3 之前为 `null`）。 */
  readonly snapshotId: number | null;
  readonly paths: readonly string[];
}

/** 提交信息的风格提示（纯本地规则，无 AI）。 */
export interface CommitMessageHint {
  readonly recentMessages: readonly string[];
  readonly template: string | null;
  readonly branchStyle: string | null;
}

/** 生成提交计划（不创建提交）。 */
export function commitPrepare(repoId: number, spec: PrepareCommitRequest): Promise<CommitPlan> {
  // 缺省字段直接留 `undefined`：JSON 序列化会丢掉它们，后端 `#[serde(default)]` 接住。
  // 逐字段条件展开会让这里多出十几行毫无信息量的代码。
  return invokeCommand<CommitPlan>('commit_prepare', { repoId, spec });
}

/** 执行提交计划（一次性；成功后发布 `repo:changed`）。 */
export function commitExecute(planId: string): Promise<CommitOutcome> {
  return invokeCommand<CommitOutcome>('commit_execute', { planId });
}

/** 读取提交信息的风格提示。 */
export function commitMessageHint(repoId: number): Promise<CommitMessageHint> {
  return invokeCommand<CommitMessageHint>('commit_message_hint', { repoId });
}
