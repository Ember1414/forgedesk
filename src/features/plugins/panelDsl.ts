/**
 * 面板 DSL 的前端解析（T6.3 方案 C）。
 *
 * 为什么后端校验了还要在前端再验一次：后端是第一道门（`render_panel`
 * 返回的 DSL 必然通过 `panel_dsl::validate_panel_dsl`），但两条路径仍可能
 * 送进坏数据——开发者模式加载的旧引擎、以及 IPC 边界上的任何未来改动。
 * 前端解析失败时渲染兜底错误卡片而不是崩溃（T6.3 验收），规则与 Rust 侧
 * panel_dsl.rs 保持一致；两边规则漂移由各自的测试钉住。
 */

/** 单个字符串字段上限（与宿主 MAX_STRING_BYTES 一致）。 */
const MAX_STRING_BYTES = 8 * 1024;
/** 块数上限（与宿主 MAX_BLOCKS 一致）。 */
const MAX_BLOCKS = 200;

/** text 块允许的色调。 */
export const PANEL_TEXT_TONES = ['plain', 'muted', 'success', 'warning', 'danger'] as const;
export type PanelTextTone = (typeof PANEL_TEXT_TONES)[number];

/** 面板块的封闭集合（与宿主 BLOCK_TYPES 一致）。 */
export type PanelBlock =
  | { readonly type: 'heading'; readonly text: string }
  | {
      readonly type: 'text';
      readonly text: string;
      readonly tone?: PanelTextTone;
    }
  | { readonly type: 'keyValue'; readonly entries: readonly (readonly [string, string])[] }
  | {
      readonly type: 'table';
      readonly columns: readonly string[];
      readonly rows: readonly (readonly string[])[];
    }
  | { readonly type: 'list'; readonly items: readonly string[] }
  | { readonly type: 'progress'; readonly label: string; readonly value: number }
  | { readonly type: 'button'; readonly label: string; readonly command: string };

/** 解析失败的原因（i18n key 后缀由 PanelRenderer 映射）。 */
export class PanelDslError extends Error {
  /** 机器可读原因。 */
  readonly reason: string;

  constructor(reason: string, detail: string) {
    super(detail);
    this.name = 'PanelDslError';
    this.reason = reason;
  }
}

function asString(value: unknown, field: string): string {
  if (typeof value !== 'string' || value.length === 0) {
    throw new PanelDslError('invalidField', `${field} must be a non-empty string`);
  }
  if (value.length > MAX_STRING_BYTES) {
    throw new PanelDslError('invalidField', `${field} exceeds ${MAX_STRING_BYTES} bytes`);
  }
  return value;
}

function asArray(value: unknown, field: string): readonly unknown[] {
  if (!Array.isArray(value)) {
    throw new PanelDslError('invalidField', `${field} must be an array`);
  }
  return value;
}

function parseBlock(raw: unknown): PanelBlock {
  if (typeof raw !== 'object' || raw === null) {
    throw new PanelDslError('invalidBlock', 'block must be an object');
  }
  const block = raw as Record<string, unknown>;
  switch (block['type']) {
    case 'heading':
      return { type: 'heading', text: asString(block['text'], 'text') };
    case 'text': {
      const parsed: { type: 'text'; text: string; tone?: PanelTextTone } = {
        type: 'text',
        text: asString(block['text'], 'text'),
      };
      if (block['tone'] !== undefined) {
        const tone = block['tone'];
        if (typeof tone !== 'string' || !(PANEL_TEXT_TONES as readonly string[]).includes(tone)) {
          throw new PanelDslError('invalidField', `unknown tone: ${String(tone)}`);
        }
        parsed.tone = tone as PanelTextTone;
      }
      return parsed;
    }
    case 'keyValue': {
      const entries = asArray(block['entries'], 'entries').map((entry) => {
        const pair = asArray(entry, 'entries');
        if (pair.length !== 2) {
          throw new PanelDslError('invalidField', 'entries must be [key, value] pairs');
        }
        return [asString(pair[0], 'key'), asString(pair[1], 'value')] as const;
      });
      return { type: 'keyValue', entries };
    }
    case 'table': {
      const columns = asArray(block['columns'], 'columns').map((cell) => asString(cell, 'column'));
      if (columns.length === 0 || columns.length > 12) {
        throw new PanelDslError('invalidField', 'columns must contain 1..=12 entries');
      }
      const rows = asArray(block['rows'], 'rows').map((row) => {
        const cells = asArray(row, 'row').map((cell) => asString(cell, 'cell'));
        if (cells.length !== columns.length) {
          throw new PanelDslError('invalidField', 'row length must match column count');
        }
        return cells as readonly string[];
      });
      return { type: 'table', columns, rows };
    }
    case 'list': {
      const items = asArray(block['items'], 'items').map((item) => asString(item, 'item'));
      return { type: 'list', items };
    }
    case 'progress': {
      const label = asString(block['label'], 'label');
      const value = block['value'];
      if (typeof value !== 'number' || !Number.isInteger(value) || value < 0 || value > 100) {
        throw new PanelDslError('invalidField', 'value must be an integer in 0..=100');
      }
      return { type: 'progress', label, value };
    }
    case 'button':
      return {
        type: 'button',
        label: asString(block['label'], 'label'),
        command: asString(block['command'], 'command'),
      };
    default:
      throw new PanelDslError('unknownBlock', `unknown block type: ${String(block['type'])}`);
  }
}

/**
 * 解析面板 DSL JSON 文本；失败抛 [`PanelDslError`]，由 PanelRenderer
 * 捕获并渲染兜底错误卡片。
 */
export function parsePanelDsl(dslJson: string): readonly PanelBlock[] {
  let root: unknown;
  try {
    root = JSON.parse(dslJson);
  } catch (error) {
    throw new PanelDslError(
      'notJson',
      error instanceof Error ? error.message : 'JSON parse failed',
    );
  }
  if (!Array.isArray(root)) {
    throw new PanelDslError('notArray', 'panel DSL root must be an array');
  }
  if (root.length > MAX_BLOCKS) {
    throw new PanelDslError('invalidField', `too many blocks: ${root.length}`);
  }
  return root.map(parseBlock);
}
