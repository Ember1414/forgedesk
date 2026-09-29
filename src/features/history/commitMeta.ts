/**
 * 提交的展示元数据（纯函数，T2.2）。
 *
 * # 边界说明（重要）
 *
 * 这里**不做 Git 语义判断**：不解析 revision 表达式、不推断分支拓扑、不计算
 * 可达性。所有函数的输入都已经是后端给出的确定值（`Commit` DTO），
 * 输出只是"怎么把它显示出来"。
 *
 * 唯一的灰色地带是 `parseRefs`：`Commit.refs` 是 `git log --format=%D` 的原文
 * （形如 `HEAD -> main, origin/main, tag: v1.0.0`），要画出三种 ref 胶囊就必须
 * 把这段字符串分类。这是对**显示字符串**的词法切分，不是对仓库状态的推断——
 * 权威的分类（哪些是本地分支、哪些是远端跟踪）属于 T2.3 的分支列表，
 * 到那时本函数应改为直接消费结构化数据。
 *
 * 另一件曾经要如实说明的事（T2.10 已修复）：libgit2 没有 `%D` 的等价物，
 * 此前 `Commit.refs` 恒为空、ref 胶囊在真实数据下从未出现；现在引擎侧用
 * `RefDecorations` 给出与 `%D` 同形的 token（差分测试钉住两侧一致）。
 * 渲染层仍必须优雅地处理"没有 ref"，这一点由 `layoutRefCapsules` 保证。
 */
import type { Commit, GraphRow } from '@/lib/ipc/history';
import type { RefLabel } from '@/features/history/graphGeometry';

/** 短 oid 的默认长度（7 位；与 `git log --oneline` 的习惯一致）。 */
export const SHORT_OID_LENGTH = 7;

/** 取短 oid（不足 7 位时原样返回，不补零——补零会造出一个假的哈希）。 */
export function shortOid(oid: string, length: number = SHORT_OID_LENGTH): string {
  return oid.slice(0, Math.max(1, length));
}

/**
 * 节点胶囊里的首字母。
 *
 * 取值顺序：作者名 → 作者邮箱 → oid。
 * 之所以一路兜到 oid 而不是用一个占位符号：占位符号是**用户可见文案**，
 * 就得进 i18n；而 oid 的首字符一定是十六进制字符，天然与语言无关，
 * 且保证"每个节点都有字母"（视觉节奏不会因为一个空作者名断掉）。
 *
 * 用 `codePointAt` 而不是 `[0]`：作者名可能是 emoji 或代理对字符，
 * 按下标切会得到半个码点，Canvas 画出来是方框。
 */
export function authorInitial(commit: Commit): string {
  const name = commit.author.name.trim();
  const email = commit.author.email.trim();
  const source = name.length > 0 ? name : email.length > 0 ? email : commit.oid;
  const first = source.codePointAt(0);
  if (first === undefined) {
    return '';
  }
  return String.fromCodePoint(first).toUpperCase();
}

/** 完整提交信息（subject + 空行 + body；没有 body 时只有 subject）。 */
export function fullMessage(commit: Commit): string {
  const body = commit.body?.trim() ?? '';
  return body === '' ? commit.subject : `${commit.subject}\n\n${body}`;
}

/** 相对时间的分档（键名直接对应 i18n key 的后缀）。 */
export type RelativeTimeUnit = 'justNow' | 'minutes' | 'hours' | 'days' | 'months' | 'years';

/** 相对时间的解析结果：i18n key 后缀 + 插值参数。 */
export interface RelativeTime {
  readonly unit: RelativeTimeUnit;
  readonly count: number;
}

/**
 * 把 Unix 秒时间戳解析成"多久之前"。
 *
 * 为什么返回结构而不是字符串：文案必须由调用方用 `t()` 生成
 * （纯函数拿不到 i18n 实例，也不该拿——那样就没法测了）。
 *
 * 未来时间（时钟偏差、或提交者手写的日期）统一按 `justNow` 处理：
 * 显示"-3 分钟后"会让人怀疑是不是数据坏了，而"刚刚"至多是含糊。
 */
export function relativeTime(seconds: number | null, nowSeconds: number): RelativeTime {
  if (seconds === null || !Number.isFinite(seconds)) {
    return { unit: 'justNow', count: 0 };
  }
  const delta = Math.max(0, nowSeconds - seconds);
  const minutes = Math.floor(delta / 60);
  if (minutes < 1) {
    return { unit: 'justNow', count: 0 };
  }
  if (minutes < 60) {
    return { unit: 'minutes', count: minutes };
  }
  const hours = Math.floor(minutes / 60);
  if (hours < 24) {
    return { unit: 'hours', count: hours };
  }
  const days = Math.floor(hours / 24);
  if (days < 31) {
    return { unit: 'days', count: days };
  }
  const months = Math.floor(days / 30.44);
  if (months < 12) {
    return { unit: 'months', count: months };
  }
  return { unit: 'years', count: Math.floor(months / 12) };
}

/** 本地时区的绝对时间（详情面板与 tooltip 里用；表格里不需要秒以下精度）。 */
export function absoluteTime(seconds: number | null): string | null {
  if (seconds === null || !Number.isFinite(seconds)) {
    return null;
  }
  return new Date(seconds * 1000).toLocaleString();
}

/** `%D` 里 `tag:` 前缀（Git 的固定输出格式，不是本地化文案）。 */
const TAG_PREFIX = 'tag:';
/** `HEAD -> branch` 里的箭头（同样是 Git 的固定输出格式）。 */
const HEAD_ARROW = '->';

/**
 * 把 `Commit.refs` 分类成三种胶囊。
 *
 * 分类规则（对显示字符串做词法判断，见文件头的边界说明）：
 *   1. `tag: <name>` → tag，标签取 `<name>`；
 *   2. `HEAD -> <branch>` → local，标签取 `<branch>`（`HEAD` 本身没有信息量）；
 *   3. `HEAD`（游离状态）→ local，标签保留 `HEAD`；
 *   4. 含 `/` 且不是 `tag:` → remote（`origin/main` 这类远端跟踪引用）；
 *   5. 其余 → local。
 *
 * 保持原顺序：Git 的输出已经把 HEAD 放在第一位，打乱它会让"当前分支"不显眼。
 */
export function parseRefs(refs: readonly string[]): readonly RefLabel[] {
  const labels: RefLabel[] = [];
  for (const raw of refs) {
    const entry = raw.trim();
    if (entry === '') {
      continue;
    }
    if (entry.startsWith(TAG_PREFIX)) {
      const name = entry.slice(TAG_PREFIX.length).trim();
      if (name !== '') {
        labels.push({ label: name, kind: 'tag' });
      }
      continue;
    }
    const arrow = entry.indexOf(HEAD_ARROW);
    if (arrow >= 0) {
      const name = entry.slice(arrow + HEAD_ARROW.length).trim();
      if (name !== '') {
        labels.push({ label: name, kind: 'local' });
        continue;
      }
    }
    if (entry === 'HEAD') {
      labels.push({ label: entry, kind: 'local' });
      continue;
    }
    labels.push({ label: entry, kind: entry.includes('/') ? 'remote' : 'local' });
  }
  return labels;
}

/** 签名状态对应的 i18n key 后缀（`history.signature.<suffix>`）。 */
export type SignatureKeySuffix =
  | 'good'
  | 'bad'
  | 'untrustedGood'
  | 'expired'
  | 'expiredKey'
  | 'revokedKey'
  | 'missingKey'
  | 'unsigned'
  | 'unknown';

/**
 * 把 `Commit.signature` 映射成 i18n key 后缀。
 *
 * 显式穷举（而不是直接把字符串拼进 key）：拼出来的 key 是运行期字符串，
 * `i18n.test.ts` 的"源码里 t() 引用的 key 必须存在"检查看不到它，
 * 漏一个分支就会在界面上印出 key 原文。穷举 + `satisfies` 让漏项变成编译错误。
 */
export function signatureKeySuffix(signature: Commit['signature']): SignatureKeySuffix {
  switch (signature) {
    case 'good':
      return 'good';
    case 'bad':
      return 'bad';
    case 'untrustedGood':
      return 'untrustedGood';
    case 'expired':
      return 'expired';
    case 'expiredKey':
      return 'expiredKey';
    case 'revokedKey':
      return 'revokedKey';
    case 'missingKey':
      return 'missingKey';
    case 'unsigned':
      return 'unsigned';
    default:
      return 'unknown';
  }
}

/** 作者显示名（`Name <email>`；邮箱为空时只有名字）。 */
export function authorDisplay(commit: Commit): string {
  const name = commit.author.name.trim();
  const email = commit.author.email.trim();
  if (email === '') {
    return name;
  }
  return name === '' ? email : `${name} <${email}>`;
}

// ---------------------------------------------------------------- 节点文案

/**
 * 一个节点在画布上需要的全部文案。
 *
 * 定义在这里（而不是渲染层）：它完全是从 `Commit` 派生出来的展示元数据，
 * 与"怎么画"无关。放在渲染层会让数据层为了构造它而反向依赖绘制模块。
 */
export interface NodeLabel {
  /** 胶囊里的作者首字母（空串表示不画）。 */
  readonly initial: string;
  /** 已分类的 ref 标签（libgit2 引擎下恒为空，见文件头）。 */
  readonly refs: readonly RefLabel[];
  /** 该行折叠掉的提交数（0 表示没有折叠）。 */
  readonly collapsed: number;
}

/**
 * 由提交与行数据算出节点文案。
 *
 * `commit` 允许为 `undefined`：分页边界上可能先拿到布局、后拿到提交
 * （两页的数据到达顺序不保证）。那时退化成"没有首字母、没有 ref"，
 * 而不是抛错——抛错会让整张图因为一行数据晚到而白屏。
 *
 * 折叠数取 `row.collapsed.length`：后端只给被折叠分支的 tip oid 清单，
 * **个数**就是徽标上要显示的 "+N"。注意这个 N 是"被折叠的分支数"
 * 而不是"被隐藏的提交数"，两者在多层合并时不相等；文案（i18n）必须
 * 写成不会让人误以为是后者的措辞。
 */
export function nodeLabelFor(commit: Commit | undefined, row: GraphRow): NodeLabel {
  return {
    initial: commit === undefined ? '' : authorInitial(commit),
    refs: commit === undefined ? [] : parseRefs(commit.refs),
    collapsed: row.collapsed.length,
  };
}

// ---------------------------------------------------------------- DOM 侧的行文案

/**
 * 一行在 DOM 侧（图模式的文本列 / 列表模式）需要的全部文案。
 *
 * 为什么要预先算好而不是在组件里现算：两个视图共用同一批字段，
 * 各自算一遍就会出现"列表里写 3 天前、文本列里写 3 天前多"这种细微不一致；
 * 而且时间格式化（`toLocaleString`）并不便宜，在窗口化渲染里重复做会拖慢滚动。
 */
export interface RowText {
  readonly oid: string;
  /** 全局行号（后端已平移，与图上的 y 一一对应）。 */
  readonly row: number;
  readonly lane: number;
  readonly colorIndex: number;
  readonly isMerge: boolean;
  /** 被折叠的合并分支上的提交（画得淡，但仍占一行）。 */
  readonly hidden: boolean;
  readonly subject: string;
  readonly author: string;
  /** 已格式化的相对时间；拿不到提交或时间戳时为 `pending`。 */
  readonly time: string;
  readonly shortOid: string;
  readonly refs: readonly RefLabel[];
  readonly collapsed: number;
}

/**
 * 行文案的格式化器（由组件注入）。
 *
 * 为什么是参数而不是在本文件里调 `t()`：本模块是**纯函数层**，
 * 拿了 i18n 实例就没法在单测里断言（而且会把 jsdom 外的调用方全部拖下水）。
 */
export interface RowTextFormat {
  readonly relativeTime: (value: RelativeTime) => string;
  readonly collapsed: (count: number) => string;
  /** 提交对象尚未到达时的占位文案。 */
  readonly pending: string;
}

/**
 * 把布局行与提交合成 DOM 可直接渲染的行文案。
 *
 * `commit` 允许缺失：分页边界上可能先拿到布局、后拿到提交（两页的
 * 到达顺序不保证）。那时用 `format.pending` 占位而不是抛错——抛错会让
 * 整页因为一行数据晚到而白屏。
 */
export function buildRowTexts(
  rows: readonly GraphRow[],
  commitByOid: ReadonlyMap<string, Commit>,
  nowSeconds: number,
  format: RowTextFormat,
): readonly RowText[] {
  return rows.map((row) => {
    const commit = commitByOid.get(row.oid);
    const label = nodeLabelFor(commit, row);
    return {
      oid: row.oid,
      row: row.row,
      lane: row.lane,
      colorIndex: row.colorIndex,
      isMerge: row.isMerge,
      hidden: row.hidden,
      subject: commit === undefined ? format.pending : commit.subject,
      author: commit === undefined ? '' : authorDisplay(commit),
      time:
        commit === undefined || commit.author.time === null
          ? format.pending
          : format.relativeTime(relativeTime(commit.author.time, nowSeconds)),
      shortOid: shortOid(row.oid),
      refs: label.refs,
      collapsed: label.collapsed,
    };
  });
}
