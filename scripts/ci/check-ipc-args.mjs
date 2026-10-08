#!/usr/bin/env node
/**
 * IPC 参数形状校验（2026-10-08）。
 *
 * # 为什么需要这个检查
 *
 * Tauri 按**形参名**从 payload 里取每个参数（`CommandItem::key`）。于是
 * `fn fs_tree(state, request: FsTreeRequest)` 要求前端传 `{ request: {...} }`；
 * 平铺传 `{ repoId, path }` 会得到 `invalid args request for command fs_tree`。
 *
 * 这条规则**在单测与 e2e 里都测不出来**：两侧都 mock 掉了 `invoke`，
 * mock 只关心"传了什么对象"，不关心宿主能不能反序列化它。结果是 2026-10-08
 * 一次性查出 19 处调用点全废（编辑器文件树永远为空、新建文件永远失败、
 * GitHub 议题/PR 与仪表盘整页失效），而测试全绿。
 *
 * # 检查内容
 *
 * 1. 每个 `invokeCommand('cmd', ...)` 的命令名必须在 `crates/commands/src` 里存在；
 * 2. 命令的每个**非注入**参数都必须在调用点作为对象字面量的键出现
 *    （snake_case ↔ camelCase 双向容忍，因为 Tauri 默认做这个映射）；
 * 3. 参数不是对象字面量（例如 `invokeCommand('x', request)`）直接报错——
 *    那正是被漏掉的那类写法。
 *
 * 启发式边界（刻意保守，宁可漏报不误报）：
 *   - `Option<T>` 参数视为可选，缺键不算错；
 *   - 标量类型（String/i64/bool/…）允许缺省，因为 Tauri 对 `Option` 与带
 *     `#[serde(default)]` 的字段都容忍缺键，静态判不出默认值，交给运行时；
 *   - 对象字面量里的 `...spread` 无法静态展开：只要命令的**结构体**参数
 *     全都以显式键出现即可，不检查 spread 的内容。
 *
 * 退出码：0 通过；1 有错（CI 与本地同款）。
 */
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');

/** 递归收集指定后缀的文件。 */
function walk(dir, extensions, out = []) {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    const stat = statSync(path);
    if (stat.isDirectory()) {
      walk(path, extensions, out);
    } else if (extensions.some((extension) => name.endsWith(extension))) {
      out.push(path);
    }
  }
  return out;
}

/** 按尖括号/圆括号深度切分（泛型里的逗号不是分隔符）。 */
function splitTopLevel(raw) {
  const parts = [];
  let depth = 0;
  let token = '';
  for (const char of raw) {
    if ('<([{'.includes(char)) depth += 1;
    if ('>)]}'.includes(char)) depth -= 1;
    if (char === ',' && depth === 0) {
      parts.push(token);
      token = '';
    } else {
      token += char;
    }
  }
  parts.push(token);
  return parts.map((part) => part.trim()).filter(Boolean);
}

const toCamel = (value) => value.replace(/_([a-z])/g, (_, char) => char.toUpperCase());

/** Tauri 注入的参数（不在 payload 里出现）。 */
const INJECTED = /^(state|app|app_handle|window|webview_window|handle|runtime|manager)$/i;

/** 标量与等价包装：能直接从 JSON 值反序列化，不需要对象键。 */
const SCALAR =
  /^(String|&str|str|bool|u8|u16|u32|u64|usize|i8|i16|i32|i64|isize|f32|f64|Value|JsonValue)$/;

/**
 * 本仓库里声明为 `enum` 的类型名。
 *
 * 枚举（如 `CredentialKindDto`）在 payload 里是字符串/简单值，键就是参数名本身，
 * 与结构体不是一回事。不把它们排除出去，`credentials_save` 这类命令会被误报。
 */
const enumNames = new Set();

/** 剥掉 Option/Vec 包装，返回（内层类型, 是否可选）。 */
function unwrap(type) {
  let inner = type.trim();
  let optional = false;
  let match = /^(Option|std::option::Option)\s*<([\s\S]*)>$/.exec(inner);
  if (match) {
    optional = true;
    inner = match[2].trim();
  }
  match = /^(Vec|std::vec::Vec)\s*<([\s\S]*)>$/.exec(inner);
  if (match) {
    inner = match[2].trim();
  }
  return { inner, optional };
}

/** 是否为"必须以对象传入"的参数。 */
function isStructParam(type) {
  const { inner } = unwrap(type);
  const base = inner.split('::').pop().trim();
  return !SCALAR.test(base) && !enumNames.has(base);
}

// ---------------------------------------------------------------- 1. Rust 侧

const rustFiles = [
  ...walk(join(repoRoot, 'crates', 'commands', 'src'), ['.rs']),
  ...walk(join(repoRoot, 'crates', 'domain', 'src'), ['.rs']),
];
for (const file of rustFiles) {
  const source = readFileSync(file, 'utf8');
  for (const match of source.matchAll(/(?:pub\s+)?enum\s+(\w+)/g)) {
    enumNames.add(match[1]);
  }
}

const rustCommands = new Map();
for (const file of walk(join(repoRoot, 'crates', 'commands', 'src'), ['.rs'])) {
  const source = readFileSync(file, 'utf8');
  // `#[tauri::command]` 与 `fn` 之间可能还夹着别的属性与注释（如 `#[allow(clippy::panic)]`
  // 上方那段"为什么这里可以 panic"的说明）——`debug_panic` 就是这样漏掉过一次
  const pattern =
    /#\[tauri::command[^\]]*\](?:\s*(?:#\[[^\]]*\]|\/\/[^\n]*))*\s*(?:pub\s+)?(?:async\s+)?fn\s+(\w+)\s*\(([\s\S]*?)\)\s*(?:->|\{|where)/g;
  let match;
  while ((match = pattern.exec(source)) !== null) {
    const name = match[1];
    const params = splitTopLevel(match[2])
      .map((entry) => {
        const index = entry.indexOf(':');
        return index < 0
          ? null
          : { name: entry.slice(0, index).trim(), type: entry.slice(index + 1).trim() };
      })
      .filter((entry) => entry !== null && !INJECTED.test(entry.name));
    rustCommands.set(name, { params, file: relative(repoRoot, file) });
  }
}

// ---------------------------------------------------------------- 2. 前端调用点

const FRONTEND_DIRS = ['src/lib/ipc', 'src/features', 'src/stores', 'src/app', 'src/ui'];
const calls = [];
for (const dir of FRONTEND_DIRS) {
  const absolute = join(repoRoot, dir);
  for (const file of walk(absolute, ['.ts', '.tsx'])) {
    const source = readFileSync(file, 'utf8');
    const pattern = /invokeCommand<[^>]*>\(\s*'([a-z0-9_]+)'/g;
    let match;
    while ((match = pattern.exec(source)) !== null) {
      const name = match[1];
      const rest = source.slice(match.index + match[0].length).replace(/^\s*,?\s*/, '');
      let keys = [];
      let literal = false;
      if (rest.startsWith('{')) {
        const start = source.indexOf('{', match.index + match[0].length);
        let depth = 0;
        let end = start;
        for (; end < source.length; end += 1) {
          if (source[end] === '{') depth += 1;
          else if (source[end] === '}') {
            depth -= 1;
            if (depth === 0) break;
          }
        }
        literal = true;
        keys = splitTopLevel(source.slice(start + 1, end))
          .map((entry) => entry.split(':')[0].trim())
          .filter((key) => /^[A-Za-z_$][\w$]*$/.test(key))
          .map(toCamel);
      }
      calls.push({
        name,
        keys: new Set(keys),
        literal,
        where: `${relative(repoRoot, file).replaceAll('\\', '/')}:${source.slice(0, match.index).split('\n').length}`,
      });
    }
  }
}

// ---------------------------------------------------------------- 3. 比对

const problems = [];
const unknown = [];
for (const call of calls) {
  const command = rustCommands.get(call.name);
  if (command === undefined) {
    unknown.push(call);
    continue;
  }
  const required = command.params.filter(
    (param) => isStructParam(param.type) && !unwrap(param.type).optional,
  );
  if (required.length === 0) {
    continue;
  }
  if (!call.literal) {
    problems.push(
      `${call.where}  ${call.name}\n    ` +
        `结构体参数必须写成对象字面量：{ ${required.map((p) => p.name).join(', ')} }`,
    );
    continue;
  }
  for (const param of required) {
    if (!call.keys.has(toCamel(param.name))) {
      problems.push(
        `${call.where}  ${call.name}\n    ` +
          `缺少键 \`${toCamel(param.name)}\`（Rust: ${param.name}: ${param.type}）`,
      );
    }
  }
}

// ---------------------------------------------------------------- 4. 结果

if (problems.length > 0) {
  console.error(`IPC 参数形状校验失败：${problems.length} 处\n`);
  for (const problem of problems) {
    console.error(`✗ ${problem}\n`);
  }
  console.error(
    '提示：Tauri 按形参名取值，结构体参数要包在同名键里（详见 src/lib/ipc/fs.ts 的说明）。',
  );
  process.exit(1);
}

if (unknown.length > 0) {
  // 命令名对不上多半是拼写错误；但也可能是命令定义在别处（插件/测试夹具）
  console.error(`IPC 校验：${unknown.length} 个调用点的命令名在 crates/commands 里不存在\n`);
  for (const call of unknown) {
    console.error(`✗ ${call.where}  ${call.name}`);
  }
  process.exit(1);
}

console.log(
  `[PASS] IPC 参数形状：${rustCommands.size} 个命令、${calls.length} 个调用点，键名与形参名一一对应。`,
);
