/**
 * Rebase 面板的计划状态与推导（纯函数，无 React 依赖，全部可单测）。
 *
 * 三层关注点：
 * 1. **entries**：区间提交清单 → 可拖拽的步骤列表（动作 + 新信息草案）；
 * 2. **issues**：与后端 `RebasePlan::validate` 对齐的即时校验（UI 先拦、
 *    后端兜底）。规则短名沿用后端 `PlanError::as_str` 的取值，i18n 按名取
 *    文案——两端同一份规则名，避免"界面说 A、后端说 B"；
 * 3. **previewRows**：把后端 preview 与本地 entries 合成"执行后历史"的
 *    可渲染行（存活行含 squash/fixup 归组，丢弃行划掉）。
 *
 * 刻意**不做**拓扑顺序校验：git rebase -i 接受任意重排（线性链交换相邻
 * 提交是常见操作，重放冲突时自然暂停）。把重排当非法会挡住用户的合法
 * 拖拽——这是 T3.5 域模型的既定决策，面板不得重新引入。
 */
import type { RebasePreview, RebaseRangeCommit, RebaseStepRequest, ReorderAction } from '@/lib/ipc';

/** 面板里的一条步骤：区间提交 + 当前动作 + 新信息草案。 */
export interface PlanEntry {
  readonly oid: string;
  readonly subject: string;
  /** 父提交 oid（识别 merge 提交用）。 */
  readonly parents: readonly string[];
  readonly action: ReorderAction;
  /** reword / squash 的新信息草案（在面板里编辑）。 */
  readonly newMessage?: string;
}

/** 一条与后端规则对齐的校验问题。 */
export interface EntryIssue {
  /** 出问题的步骤索引（全局问题归到触发它的第一条）。 */
  readonly index: number;
  /** 规则短名（`PlanError::as_str` 的取值：squashAsFirst / squashOnMerge / allDropped …）。 */
  readonly rule: string;
}

/** 校验选项（与后端 `RebasePlan` 的开关一一对应）。 */
export interface PlanOptions {
  readonly allowFlattenMerges?: boolean;
}

/** 该条目是否为 merge 提交。 */
export function isMergeEntry(entry: PlanEntry): boolean {
  return entry.parents.length >= 2;
}

/** 区间清单 → 初始步骤（全部 pick，顺序保持"从旧到新"）。 */
export function initialEntries(range: readonly RebaseRangeCommit[]): PlanEntry[] {
  return range.map((commit) => ({
    oid: commit.oid,
    subject: commit.subject,
    parents: [...commit.parents],
    action: 'pick' as const,
  }));
}

/**
 * 把 `from` 处的条目移动到 `to` 处（拖拽与 Alt+↑/↓ 共用同一实现）。
 *
 * 越界或位置不变时原样返回（React 侧可用引用相等跳过渲染）。
 */
export function moveEntry(
  entries: readonly PlanEntry[],
  from: number,
  to: number,
): readonly PlanEntry[] {
  if (from === to || from < 0 || to < 0 || from >= entries.length || to >= entries.length) {
    return entries;
  }
  const next = [...entries];
  const [moved] = next.splice(from, 1);
  if (moved === undefined) {
    return entries;
  }
  next.splice(to, 0, moved);
  return next;
}

/** 设置某条的动作（reword / squash 允许同时给出新信息草案）。 */
export function setAction(
  entries: readonly PlanEntry[],
  index: number,
  action: ReorderAction,
  newMessage?: string,
): readonly PlanEntry[] {
  if (index < 0 || index >= entries.length) {
    return entries;
  }
  return entries.map((entry, current) => {
    if (current !== index) {
      return entry;
    }
    // 动作切走时清掉旧的信息草案：pick 上挂一条消息在后端会被忽略，
    // 留着只会让下一次切回 reword 时出现"幽灵"旧文案
    if (action === 'reword' || action === 'squash') {
      return { ...entry, action, ...(newMessage === undefined ? {} : { newMessage }) };
    }
    return { oid: entry.oid, subject: entry.subject, parents: entry.parents, action };
  });
}

/**
 * 该 squash / fixup 会并入哪一条（向前找第一条非 drop 的存活条目）。
 *
 * 没有可并入的目标时返回 null——UI 据此说明"它前面没有可并入的提交"。
 */
export function mergeTargetFor(entries: readonly PlanEntry[], index: number): PlanEntry | null {
  for (let cursor = index - 1; cursor >= 0; cursor -= 1) {
    const candidate = entries[cursor];
    if (candidate !== undefined && candidate.action !== 'drop') {
      return candidate;
    }
  }
  return null;
}

/**
 * 即时校验（与后端 validate 的规则对齐；UI 先拦、后端兜底）。
 *
 * 比后端更严的一处：squash/fixup 的"前无存活"判定看的是**整个前缀**而
 * 不是只看 index 0（`drop` + `squash` 的开头在后端仍算"第一条"，但 git
 * 执行时会报 "cannot squash without a previous commit"）。更严的 UI 不
 * 会放过非法计划，只是更早说人话。
 */
export function computeIssues(
  entries: readonly PlanEntry[],
  options: PlanOptions = {},
): readonly EntryIssue[] {
  const issues: EntryIssue[] = [];
  entries.forEach((entry, index) => {
    const merging = entry.action === 'squash' || entry.action === 'fixup';
    if (merging && mergeTargetFor(entries, index) === null) {
      issues.push({ index, rule: 'squashAsFirst' });
    }
    if (entry.action === 'squash' && !options.allowFlattenMerges && isMergeEntry(entry)) {
      issues.push({ index, rule: 'squashOnMerge' });
    }
  });
  if (entries.length > 0 && entries.every((entry) => entry.action === 'drop')) {
    issues.push({ index: 0, rule: 'allDropped' });
  }
  return issues;
}

/** 计划是否可以执行（无任何问题）。 */
export function isExecutable(entries: readonly PlanEntry[], options: PlanOptions = {}): boolean {
  return computeIssues(entries, options).length === 0;
}

/** 转成 IPC 请求步骤：只有 reword / squash 携带信息草案。 */
export function toRequestEntries(entries: readonly PlanEntry[]): RebaseStepRequest[] {
  return entries.map((entry) => {
    const keepMessage = entry.action === 'reword' || entry.action === 'squash';
    return {
      oid: entry.oid,
      action: entry.action,
      ...(keepMessage && entry.newMessage !== undefined ? { newMessage: entry.newMessage } : {}),
    };
  });
}

/** 预览区的一行（存活行或丢弃行）。 */
export interface PreviewRow {
  readonly kind: 'survivor' | 'dropped';
  readonly oid: string;
  readonly subject: string;
  /** 被执行该动作的提交（reword 标记 / edit 标记）。 */
  readonly action: ReorderAction;
  /** 被 squash / fixup 并入本行的提交 oid（按 todo 顺序）。 */
  readonly mergedOids: readonly string[];
  /** 信息被改写（reword，且草案与原信息不同）。 */
  readonly reworded: boolean;
}

/**
 * 合成"执行后历史"的行。
 *
 * 主干取后端 `preview.surviving`（顺序 = 新历史顺序），归组信息取本地
 * entries（哪个 squash/fixup 归入哪条）——后端 preview 的 `squashed`
 * 是给人看的字符串（`"<oid> -> <目标>"`），解析它既脆又没意义。
 *
 * 行序：先全部存活行（新顺序），再丢弃行——丢弃的提交在新历史里不存在，
 * 列在最后并划掉，比插在中间更少误导。
 */
export function buildPreviewRows(
  entries: readonly PlanEntry[],
  preview: RebasePreview,
): readonly PreviewRow[] {
  const merged = new Map<string, string[]>();
  let lastSurvivor: string | null = null;
  for (const entry of entries) {
    if (entry.action === 'squash' || entry.action === 'fixup') {
      if (lastSurvivor !== null) {
        const bucket = merged.get(lastSurvivor);
        if (bucket === undefined) {
          merged.set(lastSurvivor, [entry.oid]);
        } else {
          bucket.push(entry.oid);
        }
      }
    } else if (entry.action !== 'drop') {
      lastSurvivor = entry.oid;
    }
  }

  const reworded = new Set(preview.reworded);
  const actionByOid = new Map(entries.map((entry) => [entry.oid, entry]));
  const subjectByOid = new Map(entries.map((entry) => [entry.oid, entry.subject]));

  const rows: PreviewRow[] = preview.surviving.map((commit) => {
    const action = actionByOid.get(commit.oid)?.action ?? 'pick';
    return {
      kind: 'survivor' as const,
      oid: commit.oid,
      subject: commit.subject,
      action,
      mergedOids: merged.get(commit.oid) ?? [],
      reworded: reworded.has(commit.oid),
    };
  });
  for (const oid of preview.dropped) {
    rows.push({
      kind: 'dropped' as const,
      oid,
      subject: subjectByOid.get(oid) ?? oid,
      action: 'drop' as const,
      mergedOids: [],
      reworded: false,
    });
  }
  return rows;
}

/** 从历史页的选中集合计算 rebase 区间（新→旧的行序输入）。 */
export interface RangeSelection {
  /** 新的基点：最旧选中提交的第一个父（git rebase 的区间左端，不含本身）。 */
  readonly base: string;
  /** 区间右端：最新的选中提交。 */
  readonly head: string;
}

/**
 * 选中集合 → 区间。
 *
 * `rowsNewestFirst` 是历史图的行序（新在上）。选中集合可能不连续
 * （Ctrl 点选）：取"最新选中"与"最旧选中"之间的整体作为区间——区间里的
 * 非选中提交会成为初始 pick 项，用户可以在面板里继续调整。
 *
 * 边界：
 * - 空选中 → null；
 * - 最旧选中提交没有父（它是根提交）→ null（没有可用的基点）。
 *   这时界面应提示用户把更旧的提交一起选中。
 * - merge 提交取第一父作为基点：区间可能因此带上第二父侧的历史，
 *   面板展示的就是后端 `git_rebase_range` 的返回，用户能看到全部条目。
 */
export function rangeFromSelection(
  rowsNewestFirst: readonly { oid: string; parents: readonly string[] }[],
  selectedOids: readonly string[],
): RangeSelection | null {
  const selected = new Set(selectedOids);
  const picked = rowsNewestFirst.filter((row) => selected.has(row.oid));
  if (picked.length === 0) {
    return null;
  }
  const head = picked[0]?.oid;
  const oldest = picked[picked.length - 1];
  if (head === undefined || oldest === undefined) {
    return null;
  }
  const base = oldest.parents[0];
  if (base === undefined) {
    return null;
  }
  return { base, head };
}
