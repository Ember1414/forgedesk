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
| M0 / T0.4 应用外壳与路由 | 未开始 |
| M0 / T0.5–T0.9、T0.11、T0.12 | 未开始 |
