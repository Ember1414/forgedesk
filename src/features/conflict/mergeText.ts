/**
 * 冲突块 → 结果文本的纯函数层（T3.2）。
 *
 * 为什么独立成模块：结果文本的组装规则（未解决块保留 git 标记、空块不产生
 * 残留行）与"残留标记检查"是**保存正确性**的一部分，独立出来才能在
 * 不挂载组件的情况下穷举测试（Vitest 直接测纯函数）。
 */
import type { MergeBlock } from '@/lib/ipc';

/** 一个冲突块的解决状态（纯前端交互状态，不属于后端契约）。 */
export type BlockResolution =
  'unresolved' | 'ours' | 'theirs' | 'bothOursFirst' | 'bothTheirsFirst' | 'custom';

export interface BlockState {
  readonly resolution: BlockResolution;
  /** `resolution === 'custom'` 时的编辑文本。 */
  readonly customText?: string;
}

/** 初始状态：未解决。 */
export const INITIAL_BLOCK_STATE: BlockState = { resolution: 'unresolved' };

/** 与块序列对齐的全空状态数组。 */
export function initialStates(count: number): (BlockState | undefined)[] {
  return Array.from({ length: count }, () => undefined);
}

/** git 标准冲突标记包裹的原文（未解决块在结果中的形态，也是手动编辑的起点）。 */
export function conflictMarkerLines(block: Extract<MergeBlock, { type: 'conflict' }>): string[] {
  return ['<<<<<<< ours', ...block.ours, '=======', ...block.theirs, '>>>>>>> theirs'];
}

/** 单个块解析出的行；未解决块保留标记（保存时会被残留检查点名）。 */
function blockLines(block: MergeBlock, state: BlockState | undefined): readonly string[] {
  if (block.type !== 'conflict') {
    return block.lines;
  }
  const resolution = state?.resolution ?? 'unresolved';
  switch (resolution) {
    case 'ours':
      return block.ours;
    case 'theirs':
      return block.theirs;
    case 'bothOursFirst':
      return [...block.ours, ...block.theirs];
    case 'bothTheirsFirst':
      return [...block.theirs, ...block.ours];
    case 'custom':
      return state?.customText === undefined
        ? conflictMarkerLines(block)
        : state.customText.split('\n');
    default:
      return conflictMarkerLines(block);
  }
}

/**
 * 把块序列与每块状态组装成完整结果文本（LF 换行；EOL / BOM 由后端按原文件形状重建）。
 *
 * 用"收集行再 join"而不是"段间补换行"：空段（例如删除一侧后内容为空）
 * 必须贡献**零行**，段间补换行的写法会在删除处留下凭空的空行。
 */
export function assembleResult(
  blocks: readonly MergeBlock[],
  states: readonly (BlockState | undefined)[],
): string {
  const lines = blocks.flatMap((block, index) => blockLines(block, states[index]));
  return lines.join('\n');
}

/** 结果文本中残留的 git 冲突标记行（保存前的警告依据；不阻断保存）。 */
export function findConflictMarkers(text: string): readonly string[] {
  return text
    .split('\n')
    .filter(
      (line) => line.startsWith('<<<<<<<') || line.startsWith('>>>>>>>') || line === '=======',
    );
}

// ---------------------------------------------------------------- 行内词级差异

/** 一段词级差异：`changed` 为真的 token 只在两个版本之一出现。 */
export interface WordSpan {
  readonly text: string;
  readonly changed: boolean;
}

const MAX_DIFF_WORDS = 40;

function tokenize(line: string): string[] {
  // 保留空白 token：高亮渲染时词与词的间隔不能消失
  return line.split(/(\s+)/).filter((token) => token.length > 0);
}

/** 经典 DP LCS 表（`table[i][j]` = tokens[i..] 与 other[j..] 的公共词数）。 */
function lcsTable(tokens: string[], other: string[]): number[][] {
  const table: number[][] = Array.from({ length: tokens.length + 1 }, () =>
    new Array<number>(other.length + 1).fill(0),
  );
  for (let i = tokens.length - 1; i >= 0; i -= 1) {
    const row = table[i];
    const next = table[i + 1];
    if (row === undefined || next === undefined) {
      continue;
    }
    for (let j = other.length - 1; j >= 0; j -= 1) {
      row[j] =
        tokens[i] === other[j] ? (next[j + 1] ?? 0) + 1 : Math.max(next[j] ?? 0, row[j + 1] ?? 0);
    }
  }
  return table;
}

/** 一侧的词序列 → 高亮 span：LCS 里的词不动，落单的词高亮。 */
function spansFor(tokens: string[], other: string[]): WordSpan[] {
  const table = lcsTable(tokens, other);
  const spans: WordSpan[] = [];
  const push = (text: string, changed: boolean) => {
    const last = spans[spans.length - 1];
    if (last && last.changed === changed) {
      spans[spans.length - 1] = { text: last.text + text, changed };
    } else {
      spans.push({ text, changed });
    }
  };
  let i = 0;
  let j = 0;
  while (i < tokens.length && j < other.length) {
    const token = tokens[i];
    const row = table[i];
    const next = table[i + 1];
    if (token === undefined || row === undefined || next === undefined) {
      break;
    }
    if (token === other[j]) {
      push(token, false);
      i += 1;
      j += 1;
    } else if ((next[j] ?? 0) >= (row[j + 1] ?? 0)) {
      // token 不在 LCS 里：它只出现在本侧
      push(token, true);
      i += 1;
    } else {
      // other[j] 只出现在对方侧：不进入本侧的渲染
      j += 1;
    }
  }
  while (i < tokens.length) {
    const token = tokens[i];
    if (token === undefined) {
      break;
    }
    push(token, true);
    i += 1;
  }
  return spans;
}

/** 两行之间的词级对照（公共词不高亮）。行太长（> 40 词）返回 null——
 *  编辑器据此关闭该行的行内高亮（T3.2 任务书第 4 条的性能闸）。 */
export function wordDiffLine(
  oursLine: string,
  theirsLine: string,
): { readonly ours: readonly WordSpan[]; readonly theirs: readonly WordSpan[] } | null {
  const a = tokenize(oursLine);
  const b = tokenize(theirsLine);
  if (a.length > MAX_DIFF_WORDS || b.length > MAX_DIFF_WORDS) {
    return null;
  }
  return { ours: spansFor(a, b), theirs: spansFor(b, a) };
}
