/**
 * 提交历史（T2.2）：`git_log_page` 的 DTO 与具名封装。
 *
 * # 为什么单独一个模块
 *
 * 历史查询是**分页 + 游标**的形态（一次调用只拿一页），与工作区状态那种
 * "一次拿全量"的命令在参数与返回结构上都不同；把它和 `workspace.ts` 混在一起
 * 会让两边的 DTO 互相干扰（例如 `paths` 在 diff 里是 `DiffRequest` 的一部分，
 * 在这里是 `HistoryQuery` 的筛选条件）。
 *
 * # 类型的来源
 *
 * 全部镜像 Rust 侧的 serde 形状，改任何一侧都必须同步另一侧：
 *   - `HistoryQuery` / `HistoryPage` → `crates/services/src/history.rs`
 *     （`#[serde(rename_all = "camelCase")]`，`HistoryQuery` 只有 `Deserialize`）
 *   - `GraphLayout` / `GraphRow` / `GraphEdge` / `EdgeKind` →
 *     `crates/domain/src/history/layout.rs`（`EdgeKind` 是 `rename_all = "lowercase"`）
 *   - `Commit` / `CommitSignature` / `SignatureStatus` →
 *     `crates/domain/src/git/commit.rs`（同样是 camelCase）
 *
 * 路径参数沿用仓库既有先例：`RepoPath` 在线格式上就是字符串
 * （见 `workspace.ts` 的 `DiffRequest.paths`），前端不需要额外的包装类型。
 */
import { invokeCommand } from './client';

/** 作者 / 提交者身份（镜像 `domain::git::commit::Signature`）。 */
export interface CommitSignature {
  readonly name: string;
  readonly email: string;
  /** 时间（Unix 秒）；Git 无法解析时为 `null`。 */
  readonly time: number | null;
}

/**
 * 签名校验状态（`git log --format=%G?` 的语义）。
 *
 * Rust 侧是 `#[serde(rename_all = "camelCase")]` 的枚举，因此线格式是小驼峰字符串。
 */
export type SignatureStatus =
  | 'good'
  | 'bad'
  | 'untrustedGood'
  | 'expired'
  | 'expiredKey'
  | 'revokedKey'
  | 'missingKey'
  | 'unsigned'
  | 'unknown';

/** 一条提交记录（镜像 `domain::git::commit::Commit`）。 */
export interface Commit {
  readonly oid: string;
  /** 父提交 oid，顺序与 Git 一致（第一个是 first-parent）；根提交为空数组。 */
  readonly parents: readonly string[];
  readonly author: CommitSignature;
  readonly committer: CommitSignature;
  /** 指向该提交的引用（`%D` 的原文，如 `HEAD -> main`、`tag: v1.0.0`）。 */
  readonly refs: readonly string[];
  readonly signature: SignatureStatus;
  readonly subject: string;
  /** 提交信息正文；列表查询不带 `%b`，因此通常为 `null`。 */
  readonly body: string | null;
}

/** 边的画法分类（镜像 `domain::history::layout::EdgeKind`）。 */
export type GraphEdgeKind = 'straight' | 'merge' | 'branch';

/** 一个提交在图上的位置。 */
export interface GraphRow {
  readonly oid: string;
  /** 泳道（0 基，左侧为 0）。lane 在生命周期内不变——这是"分支颜色稳定"的来源。 */
  readonly lane: number;
  /** 全局行号（0 基）：服务层已把页内行号平移成跨页连续的全局序号。 */
  readonly row: number;
  /** 颜色索引 = `lane % 8`（与 `PALETTE_SIZE` 对齐）。 */
  readonly colorIndex: number;
  readonly isMerge: boolean;
  /** 该行属于被折叠的合并分支。行**不删除**，是否隐藏与如何呈现由前端决定。 */
  readonly hidden: boolean;
  /** 本行是可折叠 merge 时，被折叠分支第二父的 tip oid（可能为空数组）。 */
  readonly collapsed: readonly string[];
}

/** 孩子 → 父的一条边。 */
export interface GraphEdge {
  readonly fromOid: string;
  /** 父提交 oid；父落在分页窗口外时边照发，渲染层据此画"继续向下"的线。 */
  readonly toOid: string;
  readonly fromLane: number;
  readonly toLane: number;
  readonly kind: GraphEdgeKind;
}

/** 本页的泳道布局。 */
export interface GraphLayout {
  readonly rows: readonly GraphRow[];
  readonly edges: readonly GraphEdge[];
  /** 用到的泳道数（决定图的绘制宽度）。 */
  readonly laneCount: number;
}

/**
 * 历史查询条件。
 *
 * 全部字段可选：一个空对象就是"当前分支、首页、默认 100 条"。
 * 前端**不做 Git 语义判断**（例如不自己拼 revision 表达式），
 * 所有解释都发生在后端。
 */
export interface HistoryQuery {
  /** 起点修订（分支名 / oid / 相对引用）；缺省为仓库的 HEAD。 */
  readonly revision?: string;
  /** 是否遍历全部分支（等价 `--all`）。 */
  readonly allBranches?: boolean;
  /** 路径筛选（等价 `-- <paths>`）。 */
  readonly paths?: readonly string[];
  /** 作者筛选（子串匹配）。 */
  readonly author?: string;
  /** 起始时间（Unix 秒，闭区间）。 */
  readonly since?: number;
  /** 截止时间（Unix 秒，闭区间）。 */
  readonly until?: number;
  /** 提交信息包含（字面匹配、区分大小写）。 */
  readonly messageContains?: string;
  /** 只走第一父（等价 `--first-parent`）。 */
  readonly firstParentOnly?: boolean;
  /** 跟随重命名；开启时 `paths` 必须恰好一个，否则后端返回 `VALIDATION`。 */
  readonly followRenames?: boolean;
  /** 折叠已合并分支；只在末页且 `allBranches` 时生效，否则静默回退。 */
  readonly collapseMergedBranches?: boolean;
  /** 每页条数；缺省 100，后端钳制到 `1..=500`。 */
  readonly pageSize?: number;
  /** 下一页第一行的全局序号（0 基）；缺省表示首页。 */
  readonly cursor?: number;
}

/** 一页历史：提交、布局与下一页游标。 */
export interface HistoryPage {
  /** 本页提交（新 → 旧），与 `layout.rows` 一一对应。 */
  readonly commits: readonly Commit[];
  readonly layout: GraphLayout;
  /** 下一页游标；`null` 表示末页。 */
  readonly nextCursor: number | null;
}

/**
 * `HistoryQuery` 里"可以出现在 wire 载荷上"的字段名。
 *
 * 为什么需要这张清单：`exactOptionalPropertyTypes` 下不能把 `undefined`
 * 直接赋给可选属性，而逐字段写 12 次条件展开既啰嗦又容易漏。
 * 用清单遍历 + 丢弃 `undefined`，既保证载荷里没有 `undefined` 值
 * （Tauri 会把参数序列化成 JSON，`undefined` 会变成"字段缺失"，
 * 后端 `Option` 能接住但语义模糊），又保证新增字段时只改一处。
 */
const HISTORY_QUERY_FIELDS = [
  'revision',
  'allBranches',
  'paths',
  'author',
  'since',
  'until',
  'messageContains',
  'firstParentOnly',
  'followRenames',
  'collapseMergedBranches',
  'pageSize',
  'cursor',
] as const satisfies readonly (keyof HistoryQuery)[];

/** 把 `HistoryQuery` 压成"只含已设置字段"的 wire 载荷。 */
function toWireQuery(query: HistoryQuery): Record<string, unknown> {
  const wire: Record<string, unknown> = {};
  for (const field of HISTORY_QUERY_FIELDS) {
    const value = query[field];
    if (value !== undefined) {
      wire[field] = value;
    }
  }
  return wire;
}

/**
 * 读取一页提交历史（含泳道布局）。
 *
 * 错误：调用方用 `normalizeError` 转成 `AppError`（与其余 IPC 封装一致，
 * 本模块不自行拼文案）。可能拿到 `NOT_FOUND`（repoId 无效）、
 * `VALIDATION`（`followRenames` 但路径数 ≠ 1）、`STORAGE`（仓库读取失败）。
 */
export function gitLogPage(repoId: number, query: HistoryQuery = {}): Promise<HistoryPage> {
  return invokeCommand<HistoryPage>('git_log_page', { repoId, query: toWireQuery(query) });
}
