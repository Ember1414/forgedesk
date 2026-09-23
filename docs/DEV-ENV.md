# ForgeDesk 开发环境说明

> 面向所有执行本项目的编码代理与人类贡献者。**开工前请先读本文件。**
> 最后更新：2026-09-23（M0 / T0.1–T0.3 阶段）
>
> 本文档已做**脱敏处理**（使用 `%USERPROFILE%`、`<repo-root>` 等占位符，不含任何本机绝对路径），
> 可直接随仓库公开。涉及具体机器与 IDE 的细节请勿写入本文。

---

## 1. 参考环境（本机实测通过）

| 组件 | 版本 | 备注 |
| --- | --- | --- |
| 操作系统 | Windows 11 (x64) | WebView2 运行时已安装 |
| Node.js | v24.15.0 | 也支持 >= 20.19（见 package.json engines） |
| pnpm | 11.7.0 | 与 `packageManager` 字段一致 |
| npm | 11.12.1 | 仅用于查询包版本 |
| git | 2.54.0.windows.1 | |
| Rust | stable 1.98.1（`x86_64-pc-windows-msvc`） | 含 rustfmt 1.9.0、clippy 0.1.98 |
| MSVC | Visual Studio 2022 Build Tools 17.14（MSVC 14.44） | Tauri 在 Windows 编译必需 |
| winget | 可用 | 用于安装缺失组件 |

macOS / Linux 的等价环境未在本机验证，但 `scripts/setup/` 下的脚本与 CI 工作流已按三平台编写。

---

## 2. ⚠️ 环境陷阱清单（务必遵守）

以下 7 条都是在真实开发中踩过并修复的问题。它们中的多数只在特定环境出现，
但一旦踩到会浪费大量时间，因此固化在此。

### 陷阱 1：`NODE_OPTIONS` 被注入 `safe-delete` 垫片，导致 pnpm 安装中断

**现象**：pnpm 在替换/删除依赖包时抛出：

```text
[ERROR] [safe-delete] 操作失败: ERROR ...\node_modules\<pkg>:
Error during a `trash` operation: Unknown { description: "Some operations were aborted" }
```

**根因**：某些 IDE / 终端集成会通过环境变量 `NODE_OPTIONS` 注入一个 Node 垫片
（形如 `--require="<path>/node-language-shim.cjs"`），该垫片把 `fs.rm` 替换为"移入回收站"。
pnpm 依赖 `fs.rm` 做原子替换，垫片失败即导致安装中断。

**约定**：任何会修改 `node_modules` 的 pnpm 命令（`add` / `remove` / `install` / `update`）
都必须在**清空 `NODE_OPTIONS`** 的前提下执行：

```bash
# PowerShell
$env:NODE_OPTIONS=''; pnpm add -D <packages>

# cmd
cmd /c "set NODE_OPTIONS= && pnpm add -D <packages>"
```

只读命令（`typecheck` / `lint` / `build` / `test`）不受影响。

### 陷阱 2：长时间命令可能被上层工具判定为 "watch 服务" 并终止

**现象**：命令输出被截断；依赖下载出现 `UND_ERR_DESTROYED`（连接被销毁）。

**约定**：耗时可能超过 10 秒的命令（依赖安装、全量构建、测试套件、工具链安装）
以**脱离进程 + 日志文件**方式运行，再轮询日志：

```powershell
# 启动（立即返回）
Start-Process -FilePath cmd.exe -WorkingDirectory '<repo-root>' -WindowStyle Hidden `
  -ArgumentList '/c','set NODE_OPTIONS= && pnpm add -D <pkgs> > %TEMP%\install.log 2>&1'

# 轮询
Get-Content "$env:TEMP\install.log" -Tail 25
```

### 陷阱 3：命令外壳可能是 PowerShell 包装层

- 用 `cd <repo-root>`，**不要**用 `cd /d ...`（PowerShell 会报 `Set-Location` 参数错误，cmd 则相反）。
- 传入的命令里 `$变量名` 可能被剥离/破坏，**避免在单行命令中使用 shell 变量**；
  需要变量时写成脚本文件。
- 读取 GBK 输出的日志时加 `-Encoding Default`，否则中文乱码。

### 陷阱 4：`.cmd` / `.bat` 脚本必须**纯 ASCII**

cmd.exe 按 OEM 代码页解析脚本文件，UTF-8 的中文注释会被解码成乱码，
其中一个字节序列会被当作命令分隔，产生类似错误：

```text
'Tools（Tauri' 不是内部或外部命令，也不是可运行的程序
```

**约定**：`scripts/setup/*.cmd` 内**禁止出现中文**（包括 REM 注释与 echo 文案）。中文说明放在本文件里。

### 陷阱 5：`rustup-init -y` 可能留下损坏的工具链；并发安装会卡死

**现象一（损坏）**：

```text
error: missing manifest in toolchain 'stable-x86_64-pc-windows-msvc'
help: this may happen if the toolchain installation was interrupted
```

**现象二（卡死）**：`rustup-init.exe` 的 CPU 时间停止增长（不是网络等待，而是争锁），
原因是同时发起了第二个 rustup 安装/设置命令。

**约定**：

1. **同一时刻只允许一个 rustup 安装类命令运行**，禁止并发。
2. 部分网络环境下直连 `static.rust-lang.org` 极慢甚至不可用，**建议使用镜像**：
   设置 `RUSTUP_DIST_SERVER` / `RUSTUP_UPDATE_ROOT`（模板见 `scripts/setup/rust-toolchain.cmd`）。
3. 修复/重装统一走 `scripts/setup/rust-toolchain.cmd`（内置"先卸载再安装"逻辑），不要手工敲零散命令。

### 陷阱 6：Vite 8 不再内置 esbuild，`minify: 'esbuild'` 会构建失败

**现象**（只在 `pnpm tauri build` 时出现，独立 `pnpm build` 正常，因为该分支只在 Tauri 环境变量存在时生效）：

```text
[plugin vite:esbuild-transpile]
Error: Failed to load `transformWithEsbuild`. It is deprecated and it now requires
esbuild to be installed separately.
Caused by: Error: Cannot find package 'esbuild'
```

**根因**：Vite 8 使用 rolldown 内核，默认压缩器为 oxc，esbuild 已改为可选依赖。

**约定**：`vite.config.ts` 中的 `build.minify` 只用 `true` / `false`，
**禁止**写 `'esbuild'`；如需自定义压缩器请使用 `'oxc'`。

### 陷阱 7：新写入的文件是 CRLF，与 lint 工具的 LF 要求冲突

**现象**：`cargo fmt --check` 报 `Incorrect newline style in <file>`；
`prettier --check` 把大量文件列为待格式化。

**约定**：新增文件后运行 `cargo fmt --all`（Rust）与 `pnpm format`（前端/脚本/YAML）。
两者分别按 `rustfmt.toml` 的 `newline_style = "Unix"` 与 `prettier.config.mjs` 的 `endOfLine: 'lf'` 统一。

### 陷阱 8：`.gitignore` 的通配误伤源码目录 → CI 报错指不到根因

**现象**：本地 `cargo clippy/test` 全绿，CI 却失败，报错是

```text
failed to load manifest for workspace member `/home/runner/work/forgedesk/forgedesk/src-tauri`
```

**成因**：报错的 `src-tauri` 只是 cargo 正在加载的那个成员，真正的断点在依赖链末端。
典型事故：`.gitignore` 用「任意层级的 credentials 目录」通配对密钥目录做拦截，
这条通配同时命中了源码目录 `crates/credentials/`，于是该 crate 从未进入仓库；
本地磁盘上有文件（所以本地一切正常），CI checkout 后却少了这个目录。

**约定**：
1. 在 `.gitignore` 里用目录名通配拦截密钥时，必须紧跟 `!crates/<name>/` 与 `!crates/<name>/**` 例外；
2. 新增 crate 后运行 `node scripts/setup/scaffold-crates.mjs`，再执行 `pnpm check:repo`
   （断言所有 workspace 成员存在、未被忽略、已被 git 跟踪）；
3. 看到 "failed to load manifest for workspace member X" 时，先怀疑**依赖链末端**缺少目录，
   用 `git ls-tree -r --name-only HEAD -- crates` 对比 `Get-ChildItem crates` 的差集。

### 陷阱 9：Radix 原语在测试里"点不动"，多数是触发时机问题而非实现有 bug

**现象**：`fireEvent.click` 打不开 DropdownMenu / 不激活 Tabs，
报错通常是 "Unable to find role=menu / 断言 aria-selected 仍为 false"。

**成因**（都是 Radix 刻意对齐浏览器原生行为）：
- 菜单/选择器在 **pointerdown** 打开（这样"按住拖选"不会误开菜单）；
- Tabs 在 **mousedown** 激活（与原生标签页一致）；
- Switch/Checkbox/RadioGroup 点击的是 Root 元素，要用 `role="switch" | "checkbox" | "radio"` 定位；
- ToggleGroup 的单选项是 **radio 语义**（radiogroup + radio），不是 button；
- 菜单的可访问名称来自**触发按钮**（Radix 把 aria-labelledby 指向它），传 `aria-label` 会被覆盖。

**约定**：
1. 测试里用 `fireEvent.pointerDown(el, { pointerId: 1, pointerType: 'mouse', button: 0 })` + `fireEvent.click(el)` 打开菜单；
2. jsdom 缺失的浏览器 API（ResizeObserver、Pointer Capture、scrollIntoView、PointerEvent）
   已在 `src/test/setup.ts` 统一补齐，**不要**在单个测试文件里各补一次；
3. Radix 内部组件的少量 "not wrapped in act" 警告来自其自带的 presence/定时器逻辑，
   属于上游噪音；我们自己的组件若出现同类警告必须修（通常是"挂载状态下改 store"，
   见 `src/app/shell/AppShell.test.tsx` 的 afterEach）。

### 陷阱 10：SQLite 的 `NULL` 在唯一约束里互不相等

**现象**：`settings` 表里 `repo_id` 为 `NULL` 的全局设置可以插入任意多行；
`INSERT OR REPLACE` 也覆盖不掉，表现为"改了设置但读出来还是旧值"。

**成因**：SQL 标准里 `NULL != NULL`，而 SQLite 的 `PRIMARY KEY` / `UNIQUE` 沿用这一点，
因此 `PRIMARY KEY (scope, repo_id, key)` 对 `repo_id IS NULL` **完全失效**。

**约定**：迁移里额外建了一个唯一索引把 `NULL` 折叠成具体值：

```sql
CREATE UNIQUE INDEX idx_settings_unique ON settings(scope, COALESCE(repo_id, -1), key);
```

`crates/storage/src/migrations.rs` 的 `global_scope_settings_are_unique` 测试锁住了这个行为。
新增任何"可空的唯一键"表时，都要照此处理，否则会得到一张能塞重复行的表。

### 陷阱 11：迁移前必须先 `wal_checkpoint`

**现象**：迁移前备份了 `.db` 文件，恢复时发现少了最近几次操作。

**成因**：WAL 模式下最近的写入还在 `-wal` 文件里，直接 `copy` 主文件拿到的是一份旧快照。

**约定**：备份前调用 `Database::checkpoint()`（`PRAGMA wal_checkpoint(TRUNCATE)`），
由 `crates/storage/src/migrations.rs` 的 `backup_database` 统一处理，不要在别处手写备份。

### 陷阱 12：日志脱敏必须在**写入层**，不能包在事件格式化层

**现象**：想给文件日志用 JSON 格式，于是把脱敏器（`FormatEvent` 实现）套在
`tracing_subscriber::fmt::format::Json` 外面，编译报
`the trait bound Format<Json>: Default is not satisfied`，
或 `Json: FormatEvent<_, JsonFields> is not satisfied`。

**成因**：tracing-subscriber 里 `Json` 只是给 `Format` 用的类型标记，真正实现
`FormatEvent` 的是 `Format<Json, T>`，而它**没有 `Default`**（只有 `Format<Full, _>` 有）。
也就是说第三方格式化器无法被"包一层再交给 `event_format`"。

**约定**：脱敏做成 `Write` 层（`forgedesk_diagnostics::SanitizingMakeWriter`），
放在最靠近落盘的位置：

```text
tracing 事件 → 格式化器（可读 / JSON）→ 非阻塞写入 → SanitizingMakeWriter（按行脱敏）→ 文件
```

好处不止是能兼容 JSON：它按**整行**脱敏，因此"秘密被拆成两次 write"也能抹掉，
而这在按事件脱敏时是看不见的。

### 陷阱 13：JSON 日志里的键是带引号的，键值对脱敏规则会整体失效

**现象**：文件日志里出现 `{"password":"hunter2"}`，密码没有被抹掉。

**成因**：脱敏规则原来要求"键后面紧跟 `=` 或 `:`"，而 JSON 是 `"password":"hunter2"`
——键后面先出现的是**收尾引号**，于是整条规则一条都没匹配上。
值得注意的是：单元测试（针对文本模式）当时是全绿的，问题只在**端到端断言整条日志**时才暴露。

**约定**：
1. 键后允许"空白 + 一个收尾引号 + 空白"再出现分隔符（见 `crates/diagnostics/src/sanitize.rs`）；
2. 新增脱敏规则时，必须在 `sanitizing_writer` 里补一条**端到端**断言
   （真实订阅者 + 真实格式 + 断言输出文本），只测纯函数不足以发现这类"格式差异"问题。

### 陷阱 14：`i18n:lint` 的豁免只能写在"文件头部"或"命中行"

**现象**：明明加了 `// i18n-ignore-file`，`pnpm i18n:lint` 仍报该文件。

**成因**：检查脚本只读文件**前 6 行**判断整文件豁免（避免有人把标记塞到某个中间片段里
"局部豁免整个文件"）；而行级豁免 `// i18n-ignore` 必须与命中内容在**同一行**。

**约定**：
1. 整文件豁免（开发专用页面，如 `src/ui/__dev__/ComponentsPage.tsx`）：
   标记放文件第一行，并写明理由；
2. 单行豁免（如 `src/main.tsx` 的引导期致命错误）：标记与该行同级写在同一行；
3. 新页面不要图省事整文件豁免——`i18n:lint` 的价值就在于把"以后再做 i18n"挡在门外。

### 陷阱 15：文档锚点要按 GitHub 的算法写，凭直觉写会错

**现象**：`[设置命令](docs/API.md#settings_get--settings_set--settings_all)` 看起来没问题，
但 `pnpm check:docs` 报"锚点不存在"；或者反过来，自己算出一个"更合理"的锚点却点不过去。

**成因**：GitHub 的锚点规则是把标题转小写、去掉标点后，**把每个空白字符各自替换成一个连字符**
（不合并连续空白）。`### settings_get / settings_set / settings_all` 里的斜杠被去掉后留下两个空格，
锚点因此是 `#settings_get--settings_set--settings_all`（两个连字符），而不是凭直觉的单连字符。

**约定**：
1. 给标题写锚点时不要手算，改标题后记得同步引用处，用 `pnpm check:docs` 兜底；
2. `check:docs` 只校验**仓库内**的相对链接与锚点（外部链接不做网络校验，
   原因是 CI 需要确定性、且私有仓库的链接外部不可见），写文档时别依赖"链接能点"来判断正确性。

---

## 3. 已固化的工具链版本

| 包 | 版本 | 说明 |
| --- | --- | --- |
| typescript | **6.0.3** | 7.x 暂不被 typescript-eslint 支持，见 `docs/adr/ADR-001-typescript-version.md` |
| vite | 8.3.0 | rolldown 内核 |
| @vitejs/plugin-react | 6.1.1 | |
| react / react-dom | 19.3.0 | |
| react-router-dom | 7.18.4 | |
| @tanstack/react-query | 5.103.2 | 服务端状态 |
| zustand | 5.0.15 | 客户端 UI 状态 |
| zod | 4.6.5 | 运行时校验 |
| tailwindcss / @tailwindcss/vite | 4.3.3 | CSS-first，设计 token 在 `src/ui/tokens.css` |
| lucide-react | 1.47.0 | UI 图标（ISC 许可，需登记到第三方许可清单） |
| i18next / react-i18next | 26.4.2 / 17.0.15 | |
| eslint / typescript-eslint | 10.11.0 / 8.70.1 | 扁平配置 |
| eslint-plugin-react-hooks | 7.1.1 | 含 `set-state-in-effect` 等规则 |
| vitest / @vitest/coverage-v8 | 5.0.1 | jsdom 环境 |
| @playwright/test | 1.63.0 | E2E |
| @tauri-apps/cli / api | 2.11.5 / 2.11.1 | |
| sharp | 0.35.4 | 图标栅格化 |
| yaml | 2.9.1 | 校验 CI 工作流（后续诊断规则也会用到） |
| @radix-ui/react-* | 1.x / 2.x | 无样式无障碍原语（MIT）。T0.5 引入 15 个包：dialog / alert-dialog / select / checkbox / radio-group / switch / slider / tabs / toggle-group / tooltip / popover / dropdown-menu / context-menu / toast / slot |
| class-variance-authority | 0.7.1 | 组件变体声明（MIT，shadcn/ui 生态标准做法） |

---

## 4. 常用命令

```bash
# ---- 日常开发 ----
pnpm dev                    # 启动前端开发服务器（端口 1420，strictPort）
pnpm build                  # tsc --noEmit && vite build
pnpm typecheck              # tsc --noEmit
pnpm lint                   # eslint .
pnpm test                   # vitest run --passWithNoTests
pnpm tauri dev              # 需要 Rust + MSVC 工具链
pnpm tauri build --debug --no-bundle   # 只出可执行文件，不打包安装器（避免下载 WiX/NSIS）

# ---- 质量门禁（CI 会跑，本地也应跑）----
pnpm check:contrast         # 设计 token 的 WCAG AA 对比度
pnpm check:workflows        # 校验 .github/workflows/*.yml
pnpm check:repo             # 仓库一致性：workspace 成员存在、未被 .gitignore 忽略、已被 git 跟踪
pnpm check:docs             # 文档内部链接与锚点有效（不访问网络）
pnpm i18n:lint              # 扫描未走 i18n key 的用户可见中文（T0.6 起纳入 CI）
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# ---- 资产管理 ----
node scripts/brand/render-icon.mjs                            # 矢量图标 → 1024 PNG
pnpm tauri icon docs/brand/icon-1024.png -o src-tauri/icons   # 生成各平台图标
node scripts/setup/scaffold-crates.mjs                        # 生成 Rust crate 骨架（幂等）
pnpm setup:cargo-mirror                                       # 生成本地 Cargo 镜像配置（不提交）
node scripts/ci/pin-actions.mjs --dry-run                     # 转公开前固定 action 到 SHA
```

> `.cargo/config.toml`（Cargo 镜像）**已被 .gitignore 排除**，属本机环境配置：
> 提交它会让 GitHub Actions 的 runner 也去访问国内镜像，反而更慢。
> 需要时用 `pnpm setup:cargo-mirror` 生成。

---

## 5. 工具链安装步骤（权威流程）

```powershell
# 1) 安装 rustup 与 MSVC Build Tools（MSVC 需要管理员权限，约 4-7 GB）
scripts\setup\windows-toolchain.cmd

# 2) 安装 / 修复 Rust stable 工具链（损坏时同样用这条）
scripts\setup\rust-toolchain.cmd

# 3) 关闭并重新打开终端后验证
rustc --version
cargo --version
cargo fmt --version
cargo clippy --version
where link.exe
```

预期：rustc / cargo / rustfmt / clippy 均输出版本号，`where link.exe` 能找到 MSVC 链接器。
若 `cargo` 提示 "not recognized"，说明 shell 未重启或 PATH 未生效，用绝对路径
`%USERPROFILE%\.cargo\bin\cargo.exe` 亦可。

---

## 6. 当前进度快照

| 里程碑 / 任务 | 状态 |
| --- | --- |
| M0 / T0.1 前端脚手架 | ✅ typecheck + lint + build 全绿 |
| M0 / T0.1 Tauri 宿主 + IPC 通路 | ✅ 构建并实际启动成功（窗口标题 ForgeDesk） |
| M0 / T0.2 Rust workspace（12 个 crate） | ✅ check / test / fmt / clippy 全绿 |
| M0 / T0.3 设计 token + 预览页 + 原创图标 | ✅ 34 项对比度通过；17 个图标文件 |
| M0 / T0.10 CI 两阶段工作流 + 校验器 | ✅ 本地校验通过 |
| M0 / T0.4 应用外壳与路由 | ✅ 20 条路由可跳转；外壳交互测试 13 项 + 路由表 23 项 |
| M0 / T0.5 基础组件库 | ✅ 29 个组件（Radix 原语）+ 85 项组件测试；展示页 `/__dev__/components` |
| M0 / T0.6 错误模型 + i18n + 错误展示 | ✅ 错误分类/脱敏（Rust 单测）+ `normalizeError`/ErrorToast + `pnpm i18n:lint` 门禁 |
| M0 / T0.7 SQLite + 迁移 + 设置持久化 | ✅ 7 张表 + 版本化迁移（含备份/回滚）+ 设置读写命令 + 界面密度落库 |
| M0 / T0.8 日志与日志查看 | ✅ 文件日志（JSON+脱敏）+ 按天/10MB 轮转 + panic 留档 + 会话标记 + `logs_open`/`logs_tail` |
| M0 / T0.9 规范文档五件套 | ✅ ARCHITECTURE / API（含事件表）/ CODING_STYLE / CONTRIBUTING / README + PR 与 Issue 模板 + `pnpm check:docs` |
| M0 / T0.11、T0.12 | 未开始 |
| 审批 | ✅ T0.4 主界面布局已确认（红线 R3）；⏳ T0.9 的 AGENTS.md 与 README 免责声明待确认 |

补充说明（T0.4 顺带落地的两项前置能力，后续任务直接复用）：

- **i18n 骨架已就位**（`src/lib/i18n`，命名空间 `common` / `shell`）。
  组件文案一律 `t('key')`，中英 key 一致性由 `i18n.test.ts` 断言。
  新增语言只需补一个 locales 目录，不需要改组件。
- **主题统一走 `src/app/theme.ts` + uiStore**。入口在渲染前写入 `<html data-theme>`（避免暗色闪白），
  `system` 模式会在运行时解析并订阅系统外观变化。
  注意：不要再在组件里自行读写 `forgedesk.theme` 这个 key（T0.3 的预览页曾这么干，已收敛）。

组件库使用约定（T0.5 之后新页面一律照此写）：

- 一律从 `@/ui/components/*` 取组件，**不要**为了"就一个小按钮"自己写一份；
  曾经的 T0.4 临时 `SegmentedControl` 就是这样留下的，T0.5 已收敛为 `ToggleGroup`。
- 颜色只用语义类（`bg-surface` / `border-danger` / `text-fg-muted`…），禁止十六进制色值与固定 px 字号；
  新增语义色请加到 `src/ui/tokens.css`（并考虑是否纳入对比度校验）。
- 组件**不产生**用户可见文案：标题、`aria-label`、关闭按钮名称都由调用方用 i18n 文案传入
  （例如 `DialogContent` 的 `closeLabel` 是必填项）。
- 破坏性操作的确认框用 `AlertDialog`，`impact`（影响说明）是必填项——这是红线 R7 在 UI 层的闸门。
