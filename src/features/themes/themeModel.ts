/**
 * 主题 JSON 模型与校验（T6.6）。
 *
 * 设计要点：
 *  1. `colors` 的 key 是**封闭白名单**（与 tokens.css 的 --fd-* 一一对应），
 *     未知 key 直接报错而不是忽略——否则一个拼错的 token 会造成"调了半天没变化"的
 *     静默失败，用户永远不知道原因。
 *  2. 颜色值允许缺省：缺失的 token 继承该外观（light/dark）的内置值，
 *     自定义主题只需覆盖想改的部分（导入体验友好，且新 token 加入时旧主题不失效）。
 *  3. 校验结果是**逐字段**的错误列表（field + code），由 UI 映射为 i18n 文案；
 *     本模块只产生数据，不产生用户可见字符串（CODING_STYLE §2.1 同源原则）。
 *  4. 对比度计算放在这里而不是 UI：主题详情（T6.6 §7）与导入检查共用同一实现，
 *     避免两处算法漂移。
 */

/** 主题可覆盖的颜色 token（camelCase，映射到 tokens.css 的 --fd-* 变量）。 */
export const THEME_COLOR_TOKENS = [
  'canvas',
  'surface',
  'surfaceRaised',
  'surfaceSunken',
  'line',
  'lineStrong',
  'scrim',
  'fg',
  'fgMuted',
  'fgSubtle',
  'fgInverted',
  'brand',
  'brandHover',
  'brandFg',
  'brandSubtle',
  'spark',
  'success',
  'warning',
  'danger',
  'info',
  'graphLane0',
  'graphLane1',
  'graphLane2',
  'graphLane3',
  'graphLane4',
  'graphLane5',
  'graphLane6',
  'graphLane7',
  'graphRefLocal',
  'graphRefRemote',
  'graphRefTag',
  'graphSelected',
  'graphHover',
] as const;

export type ThemeColorToken = (typeof THEME_COLOR_TOKENS)[number];

const COLOR_TOKEN_SET: ReadonlySet<string> = new Set(THEME_COLOR_TOKENS);

/** xterm 主题允许的键（T6.6 §6：终端主题由同一主题派生）。 */
export const XTERM_KEYS = [
  'background',
  'foreground',
  'cursor',
  'cursorAccent',
  'selectionBackground',
  'black',
  'red',
  'green',
  'yellow',
  'blue',
  'magenta',
  'cyan',
  'white',
  'brightBlack',
  'brightRed',
  'brightGreen',
  'brightYellow',
  'brightBlue',
  'brightMagenta',
  'brightCyan',
  'brightWhite',
] as const;

export type XtermKey = (typeof XTERM_KEYS)[number];

const XTERM_KEY_SET: ReadonlySet<string> = new Set(XTERM_KEYS);

/** 主题外观。 */
export type ThemeAppearance = 'light' | 'dark';

/** 主题定义（导入 / 导出 / 内置共用同一形状）。 */
export interface ThemeDefinition {
  readonly id: string;
  readonly name: string;
  readonly appearance: ThemeAppearance;
  /** 主题自身版本（与插件 API 版本无关）。 */
  readonly version: string;
  /** 覆盖的颜色；缺失项继承该外观的内置值。 */
  readonly colors: Readonly<Partial<Record<ThemeColorToken, string>>>;
  /** 字体覆盖（可选）。sizeScale 目前为预留字段：字阶是固定 px，暂不缩放。 */
  readonly fonts?: { readonly ui?: string; readonly mono?: string; readonly sizeScale?: number };
  /** 终端配色覆盖（可选，键见 XTERM_KEYS）。 */
  readonly xterm?: Readonly<Partial<Record<XtermKey, string>>>;
}

/** 逐字段校验错误（code 由 UI 映射为 i18n 文案）。 */
export interface ThemeFieldError {
  readonly field: string;
  readonly code: string;
  readonly value?: string;
}

export type ThemeValidateResult =
  | { readonly ok: true; readonly theme: ThemeDefinition }
  | { readonly ok: false; readonly errors: readonly ThemeFieldError[] };

/** 内置主题 id（不可被导入主题占用，也不可删除）。 */
export const BUILTIN_THEME_IDS = ['forgedesk-light', 'forgedesk-dark'] as const;

const ID_PATTERN = /^[a-z][a-z0-9-]{0,63}$/;
const HEX_PATTERN = /^#(?:[0-9a-fA-F]{3,4}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})$/;
// rgba() 允许 0-255 与百分比两种写法；scrim（遮罩）这类带透明度的值会用到
const RGB_PATTERN =
  /^rgba?\(\s*(?:\d{1,3}%|\d{1,3})\s+(?:\d{1,3}%|\d{1,3})\s+(?:\d{1,3}%|\d{1,3})(?:\s+\/\s+(?:0(?:\.\d+)?|1(?:\.0+)?|\d{1,3}%))?\s*\)$/;
const SEMVER_PATTERN = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/;

function isValidColorValue(value: string): boolean {
  return HEX_PATTERN.test(value) || RGB_PATTERN.test(value);
}

function err(field: string, code: string, value?: string): ThemeFieldError {
  return value === undefined ? { field, code } : { field, code, value };
}

/**
 * 校验并归一化主题 JSON（入口为已 JSON.parse 的 unknown）。
 *
 * 返回所有错误而不是首个错误：导入对话框需要一次性标出全部问题，
 * "修一个错重试一次"的循环对用户是折磨。
 */
export function validateThemeJson(input: unknown): ThemeValidateResult {
  const errors: ThemeFieldError[] = [];

  if (typeof input !== 'object' || input === null || Array.isArray(input)) {
    return { ok: false, errors: [err('', 'notAnObject')] };
  }
  const raw = input as Record<string, unknown>;

  const id = typeof raw.id === 'string' ? raw.id : '';
  if (!ID_PATTERN.test(id)) {
    errors.push(err('id', 'invalidId', id));
  } else if ((BUILTIN_THEME_IDS as readonly string[]).includes(id)) {
    errors.push(err('id', 'builtinId', id));
  }

  const name = typeof raw.name === 'string' ? raw.name : '';
  if (name.trim().length === 0 || name.length > 100) {
    errors.push(err('name', 'invalidName', name));
  }

  const appearance: ThemeAppearance | null =
    raw.appearance === 'light' || raw.appearance === 'dark' ? raw.appearance : null;
  if (appearance === null) {
    errors.push(err('appearance', 'invalidAppearance', String(raw.appearance)));
  }

  const version = typeof raw.version === 'string' ? raw.version : '';
  if (!SEMVER_PATTERN.test(version)) {
    errors.push(err('version', 'invalidVersion', version));
  }

  const colors: Record<string, string> = {};
  if (raw.colors !== undefined) {
    if (typeof raw.colors !== 'object' || raw.colors === null || Array.isArray(raw.colors)) {
      errors.push(err('colors', 'notAnObject'));
    } else {
      for (const [key, value] of Object.entries(raw.colors as Record<string, unknown>)) {
        if (!COLOR_TOKEN_SET.has(key)) {
          errors.push(err(`colors.${key}`, 'unknownToken', key));
          continue;
        }
        if (typeof value !== 'string' || !isValidColorValue(value)) {
          errors.push(err(`colors.${key}`, 'invalidColor', String(value)));
          continue;
        }
        colors[key] = value;
      }
    }
  }

  let fonts: { ui?: string; mono?: string; sizeScale?: number } | undefined = undefined;
  if (raw.fonts !== undefined) {
    if (typeof raw.fonts !== 'object' || raw.fonts === null || Array.isArray(raw.fonts)) {
      errors.push(err('fonts', 'notAnObject'));
    } else {
      const rawFonts = raw.fonts as Record<string, unknown>;
      fonts = {};
      for (const key of ['ui', 'mono'] as const) {
        const value = rawFonts[key];
        if (value === undefined) {
          continue;
        }
        if (typeof value !== 'string' || value.trim().length === 0 || value.length > 500) {
          errors.push(err(`fonts.${key}`, 'invalidFont', String(value)));
        } else {
          fonts[key] = value;
        }
      }
      if (rawFonts.sizeScale !== undefined) {
        const scale = rawFonts.sizeScale;
        if (typeof scale !== 'number' || !Number.isFinite(scale) || scale < 0.8 || scale > 1.5) {
          errors.push(err('fonts.sizeScale', 'invalidSizeScale', String(scale)));
        } else {
          fonts.sizeScale = scale;
        }
      }
    }
  }

  const xterm: Record<string, string> = {};
  if (raw.xterm !== undefined) {
    if (typeof raw.xterm !== 'object' || raw.xterm === null || Array.isArray(raw.xterm)) {
      errors.push(err('xterm', 'notAnObject'));
    } else {
      for (const [key, value] of Object.entries(raw.xterm as Record<string, unknown>)) {
        if (!XTERM_KEY_SET.has(key)) {
          errors.push(err(`xterm.${key}`, 'unknownXtermKey', key));
          continue;
        }
        if (typeof value !== 'string' || !isValidColorValue(value)) {
          errors.push(err(`xterm.${key}`, 'invalidColor', String(value)));
          continue;
        }
        xterm[key] = value;
      }
    }
  }

  if (errors.length > 0) {
    return { ok: false, errors };
  }
  // appearance 为 null 时上面已经返回错误；TS 无法跨变量追踪，这里显式兜底
  const safeAppearance: ThemeAppearance = appearance ?? 'light';
  return {
    ok: true,
    theme: {
      id,
      name,
      appearance: safeAppearance,
      version,
      colors,
      ...(fonts !== undefined && Object.keys(fonts).length > 0 ? { fonts } : {}),
      ...(Object.keys(xterm).length > 0 ? { xterm } : {}),
    },
  };
}

/** 解析 #hex / rgb() 为 RGB 分量；无法解析返回 null（调用方决定如何降级）。 */
export function parseColor(value: string): { r: number; g: number; b: number } | null {
  if (HEX_PATTERN.test(value)) {
    let hex = value.slice(1);
    if (hex.length === 3 || hex.length === 4) {
      hex = hex
        .slice(0, 3)
        .split('')
        .map((c) => c + c)
        .join('');
    }
    return {
      r: Number.parseInt(hex.slice(0, 2), 16),
      g: Number.parseInt(hex.slice(2, 4), 16),
      b: Number.parseInt(hex.slice(4, 6), 16),
    };
  }
  const rgb = RGB_PATTERN.exec(value);
  if (rgb === null) {
    return null;
  }
  const channel = (raw: string): number => {
    if (raw.endsWith('%')) {
      return Math.round((Number.parseFloat(raw) / 100) * 255);
    }
    return Number.parseInt(raw, 10);
  };
  const parts = value
    .slice(value.indexOf('(') + 1, value.lastIndexOf(')'))
    .split(/[\s,/]+/)
    .filter((part) => part.length > 0);
  const r = parts[0];
  const g = parts[1];
  const b = parts[2];
  if (r === undefined || g === undefined || b === undefined) {
    return null;
  }
  return { r: channel(r), g: channel(g), b: channel(b) };
}

/** WCAG 相对亮度。 */
export function relativeLuminance(color: { r: number; g: number; b: number }): number {
  const channel = (raw: number): number => {
    const v = raw / 255;
    return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * channel(color.r) + 0.7152 * channel(color.g) + 0.0722 * channel(color.b);
}

/** WCAG 对比度（两色的亮度比，≥1）。 */
export function contrastRatio(a: string, b: string): number | null {
  const ca = parseColor(a);
  const cb = parseColor(b);
  if (ca === null || cb === null) {
    return null;
  }
  const la = relativeLuminance(ca);
  const lb = relativeLuminance(cb);
  const [lighter, darker] = la >= lb ? [la, lb] : [lb, la];
  return (lighter + 0.05) / (darker + 0.05);
}

/**
 * 主题详情必须展示的对比度对（T6.6 §7：至少正文/背景与按钮文字/按钮背景两组）。
 * label 是 i18n key 后缀（themes.contrast.<label>），不在本模块写文案。
 */
export const REQUIRED_CONTRAST_PAIRS: readonly {
  readonly label: string;
  readonly fg: ThemeColorToken;
  readonly bg: ThemeColorToken;
  readonly min: number;
}[] = [
  { label: 'body', fg: 'fg', bg: 'canvas', min: 4.5 },
  { label: 'button', fg: 'brandFg', bg: 'brand', min: 4.5 },
  { label: 'muted', fg: 'fgMuted', bg: 'surface', min: 4.5 },
];

/**
 * 计算主题的对比度报告（T6.6 §7）。
 *
 * 只有主题**显式覆盖**了色对两端时才计算比值；缺失 token 继承内置配色，
 * 而内置配色由 `pnpm check:contrast` 在 CI 保证 AA——标记为 inherited
 * 而不是复制一份 tokens.css 的值来算（那是第二份真相，迟早漂移）。
 */
export function contrastReport(
  theme: Pick<ThemeDefinition, 'appearance' | 'colors'>,
): { label: string; ratio: number | null; min: number; state: 'pass' | 'fail' | 'inherited' }[] {
  return REQUIRED_CONTRAST_PAIRS.map(({ label, fg, bg, min }) => {
    const fgValue = theme.colors[fg];
    const bgValue = theme.colors[bg];
    if (fgValue === undefined || bgValue === undefined) {
      // 两端任一继承内置：AA 由 check:contrast 对 tokens.css 的 CI 校验保证
      return { label, ratio: null, min, state: 'inherited' as const };
    }
    const ratio = contrastRatio(fgValue, bgValue);
    return {
      label,
      ratio,
      min,
      state: ratio !== null && ratio >= min ? ('pass' as const) : ('fail' as const),
    };
  });
}
