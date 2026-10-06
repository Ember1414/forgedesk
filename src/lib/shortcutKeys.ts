/**
 * 快捷键框架（T5.9）：纯逻辑层。
 *
 * # 键位表示
 *
 * 统一用**规范化字符串**（`Mod+Shift+P`、`Ctrl+S`）：`Mod` 是平台修饰键
 * （macOS = Cmd，其余 = Ctrl），比较在规范化后进行。解析与匹配是纯函数，
 * 表驱动单测锁定。
 *
 * # 冲突检测
 *
 * 冲突 = 两个命令的**规范化键相同且 when 条件可能同时成立**。保守策略：
 * 只要 when 集合有交集就算潜在冲突（精确的运行时互斥无法静态判定），
 * 自定义保存时由调用方阻止。
 *
 * # when 条件
 *
 * 用字符串标识（`repoOpen` / `editorActive` …）而非表达式语言——
 * 求值表由调用方注入，框架只做集合判断。
 */

/** 平台修饰键名（macOS = Cmd）。 */
export const MOD_KEY: 'Cmd' | 'Ctrl' =
  typeof navigator !== 'undefined' && /mac/i.test(navigator.platform ?? navigator.userAgent)
    ? 'Cmd'
    : 'Ctrl';

/** 快捷键定义的 when 条件集合（空 = 全局可用）。 */
export type WhenTags = readonly string[];

/** 规范化一个快捷键字符串：`ctrl+shift+p` → `Ctrl+Shift+P`（Mod 归一）。 */
export function normalizeShortcut(raw: string): string {
  const parts = raw
    .split('+')
    .map((part) => part.trim())
    .filter((part) => part !== '');
  const modIndex = parts.findIndex((part) => part.toLowerCase() === 'mod');
  const ctrlIndex = parts.findIndex((part) => part.toLowerCase() === 'ctrl');
  // `Ctrl` 在定义里就是"平台修饰键"的 Windows 写法——归一为 Mod
  // （macOS 上用户写 Cmd 或 Ctrl 都指向同一个动作）。
  const mod: string | null = modIndex >= 0 || ctrlIndex >= 0 ? 'Mod' : null;
  const modifiers: string[] = [];
  const rest: string[] = [];
  for (const [index, part] of parts.entries()) {
    const lower = part.toLowerCase();
    if (index === modIndex) {
      continue;
    }
    if (index === ctrlIndex) {
      continue;
    }
    if (lower === 'shift') {
      modifiers.push('Shift');
    } else if (lower === 'alt') {
      modifiers.push('Alt');
    } else if (lower === 'mod' || lower === 'ctrl') {
      // 已处理
    } else {
      // 键名首字母大写（字母）；其余原样（F1-F12、方向键名等）
      rest.push(part.length === 1 ? part.toUpperCase() : part);
    }
  }
  const ordered = [...(mod !== null ? [mod] : []), ...modifiers, ...rest];
  return ordered.join('+');
}

/** 规范化后的键是否包含平台修饰键。 */
export function hasPlatformMod(normalized: string): boolean {
  return normalized.split('+').some((part) => part === 'Mod' || part === 'Ctrl');
}

/** 键盘事件是否命中规范化键。 */
export function matchesEvent(
  normalized: string,
  event: {
    readonly key: string;
    readonly ctrlKey: boolean;
    readonly metaKey: boolean;
    readonly shiftKey: boolean;
    readonly altKey: boolean;
  },
): boolean {
  const parts = normalized.split('+');
  const wantMod = parts.includes('Mod');
  const wantCtrl = parts.includes('Ctrl');
  const wantShift = parts.includes('Shift');
  const wantAlt = parts.includes('Alt');
  const key = parts[parts.length - 1];
  if (key === undefined) {
    return false;
  }
  const eventMod = MOD_KEY === 'Cmd' ? event.metaKey : event.ctrlKey;
  const eventCtrl = event.ctrlKey;
  if (wantMod && !eventMod) {
    return false;
  }
  if (wantCtrl && !eventCtrl) {
    return false;
  }
  if (!wantMod && !wantCtrl && (event.metaKey || (MOD_KEY === 'Cmd' ? false : event.ctrlKey))) {
    return false;
  }
  if (wantShift !== event.shiftKey) {
    return false;
  }
  if (wantAlt !== event.altKey) {
    return false;
  }
  // 单字母键名大小写不敏感（Shift+X 时 event.key = 'X'）
  return event.key.toUpperCase() === key.toUpperCase();
}

/** 冲突检测的结果。 */
export interface ShortcutConflict {
  readonly key: string;
  readonly firstId: string;
  readonly secondId: string;
}

/** 一个命令的快捷键信息（框架只认这些字段）。 */
export interface KeyedCommand {
  readonly id: string;
  readonly normalizedKey: string | null;
  readonly when: WhenTags;
}

/**
 * 找出规范化后的重复键（潜在冲突）。
 *
 * 无键（null）与不同 when 集合的命令不算冲突；when 集合有交集的才算。
 */
export function detectConflicts(commands: readonly KeyedCommand[]): ShortcutConflict[] {
  const conflicts: ShortcutConflict[] = [];
  for (const [i, first] of commands.entries()) {
    if (first.normalizedKey === null) {
      continue;
    }
    for (const second of commands.slice(i + 1)) {
      if (second.normalizedKey === null) {
        continue;
      }
      if (first.normalizedKey !== second.normalizedKey) {
        continue;
      }
      const overlap = first.when.some((tag) => second.when.includes(tag));
      if (overlap || (first.when.length === 0 && second.when.length === 0)) {
        conflicts.push({
          key: first.normalizedKey,
          firstId: first.id,
          secondId: second.id,
        });
      }
    }
  }
  return conflicts;
}

/** 求值 when 集合：全部 tag 都在运行时上下文里才可用。 */
export function evaluateWhen(when: WhenTags, context: ReadonlySet<string>): boolean {
  return when.every((tag) => context.has(tag));
}

/** 为展示格式化键名：Mod → 平台修饰键。 */
export function displayShortcut(normalized: string): string {
  return normalized.replace('Mod', MOD_KEY);
}

/** 导出 Markdown 的表头（调用方按语言传入）。 */
export interface MarkdownHeader {
  readonly keyColumn: string;
  readonly actionColumn: string;
}

/** 导出 Markdown（帮助页）。 */
export function shortcutsToMarkdown(
  rows: readonly { readonly title: string; readonly normalized: string | null }[],
  header: MarkdownHeader,
): string {
  const lines = [`| ${header.keyColumn} | ${header.actionColumn} |`, '| --- | --- |'];
  for (const row of rows) {
    lines.push(
      `| ${row.normalized === null ? '—' : displayShortcut(row.normalized)} | ${row.title} |`,
    );
  }
  return lines.join('\n');
}
