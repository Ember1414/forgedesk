# 参与 ForgeDesk

感谢你有兴趣参与。ForgeDesk 是一个**开源、跨平台、可自由分发**的 Git 桌面客户端，
目标是"把复杂的 Git 与 GitHub 终端操作变成可视化界面、向导与可交互图表"。

在动手之前，请务必先读 [`AGENTS.md`](./AGENTS.md)（本项目最高优先级公约）。
其中 §2 的八条红线（尤其是"不含任何 AI 推理功能""不复制竞品 UI""不使用受限商标"）
是硬性要求，违反的提交不会被合并。

设计说明（**阅读顺序建议**）：

| 想了解                                 | 看这份                                           |
| -------------------------------------- | ------------------------------------------------ |
| 为什么做、做什么、做到哪一步           | [`docs/PLAN.md`](./docs/PLAN.md)                 |
| 代码怎么分层、数据怎么流动、在哪里扩展 | [`docs/ARCHITECTURE.md`](./docs/ARCHITECTURE.md) |
| 代码风格（Rust / TS-React / 提交信息） | [`docs/CODING_STYLE.md`](./docs/CODING_STYLE.md) |
| 前后端契约（命令与事件登记表）         | [`docs/API.md`](./docs/API.md)                   |
| 本地怎么跑起来、踩过的坑               | [`docs/DEV-ENV.md`](./docs/DEV-ENV.md)           |

---

## 1. 开发环境

完整步骤（含三平台差异、镜像与工具链脚本、排错）见 **[`docs/DEV-ENV.md`](./docs/DEV-ENV.md)**。

最低要求：

| 依赖             | 版本                            | 说明                                                                       |
| ---------------- | ------------------------------- | -------------------------------------------------------------------------- |
| Rust             | stable（`rust-version = 1.85`） | 用 `rustup` 安装                                                           |
| Node.js          | ≥ 20.19                         | `package.json` 的 `engines` 约束                                           |
| pnpm             | 11.x                            | 用 `corepack enable pnpm`                                                  |
| 系统 Git         | ≥ 2.30                          | 应用会调用系统 `git`（写操作走 CLI）                                       |
| Tauri 2 系统依赖 | —                               | Windows 需 MSVC Build Tools（见 `scripts/setup/`）；Linux 需 webkit2gtk 等 |

Windows 用户可一键准备工具链：

```bash
scripts\setup\windows-toolchain.cmd
```

国内网络环境可生成本地 cargo 镜像配置（该文件不入库）：

```bash
pnpm setup:cargo-mirror
```

---

## 2. 常用命令

```bash
# 开发
pnpm dev                    # 启动前端开发服务器（Vite）
pnpm tauri dev              # 启动完整桌面应用（含 Rust 侧）

# 质量门禁（提交前必须全绿，与 AGENTS.md §4 一致）
pnpm lint                   # ESLint（含架构护栏：禁止在 ipc 封装之外直接调 Tauri API）
pnpm i18n:lint              # 禁止硬编码用户可见文案
pnpm typecheck              # TypeScript 类型检查
pnpm test                   # Vitest（前端单测）
pnpm format:check           # Prettier 校验
pnpm check:contrast         # 设计 token 的 WCAG AA 对比度
pnpm check:workflows        # 校验 .github/workflows/*.yml
pnpm check:repo             # 仓库一致性（workspace 成员存在且被 git 跟踪）
pnpm check:docs             # 文档内部链接与锚点有效
pnpm check:site             # 官网落地页自检（下载入口与校验和的渲染）

cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# 构建与预览
pnpm build                  # tsc --noEmit + vite build
pnpm tauri build            # 打包桌面应用
```

辅助脚本（按需，不进入门禁）：

```bash
pnpm lint:fix               # 自动修复 lint
pnpm format                 # 自动格式化
node scripts/setup/scaffold-crates.mjs   # 幂等生成 workspace 骨架（新增 crate 后运行）
node scripts/ci/pin-actions.mjs          # 把 workflow 里的 action 固定为 commit SHA（人工确认后执行）
```

---

## 3. 提交规范

使用 **Conventional Commits**，subject 用英文祈使句，正文写"为什么这样改"。

```text
feat(logging): M0 T0.8 -- file logging, rotation, panic capture and log viewing

Until now logs existed only in a terminal. That is useless on the day a user
reports a problem: the window is closed and the evidence is gone.

- Rotation resolves name collisions instead of failing: Windows rename does not
  overwrite an existing target, so two rotations within the same second would
  otherwise silently stop rotating.
```

- 类型：`feat` / `fix` / `chore` / `docs` / `test` / `refactor` / `perf` / `ci`
- scope：模块名（`logging`、`app-shell`、`ci`）
- 破坏性变更：类型后加 `!`，并在正文写迁移方式
- 一个提交一件事，可独立回滚

---

## 4. PR 流程

1. **开 issue 先对齐**：较大的改动（新功能、新依赖、接口变更）先在 issue 或 RFC 表单里讨论，
   避免写完了才发现方向不对。RFP/RFC 模板见 `.github/ISSUE_TEMPLATE/`。
2. **分支**：`feat/<简短描述>`、`fix/<简短描述>`、`docs/<简短描述>`。
   `main` 始终可发布，不要直接推 `main`。
3. **自检**：把 §2 的门禁全部跑一遍；CI 会在 PR 上跑 Linux 质量门禁（三平台矩阵只在打标签时跑）。
4. **填 PR 模板**：模板里的勾选项（未复制竞品 UI、未使用受限商标、未引入 AI 依赖等）
   必须逐条确认，这些对应 [`AGENTS.md`](./AGENTS.md) §2 的红线。
5. **评审关注点**：分层是否被破坏（见 `docs/ARCHITECTURE.md` §3）、错误与文案是否按
   `docs/CODING_STYLE.md` 的分工、新增命令是否登记到 `docs/API.md`、测试是否覆盖失败路径。
6. **合并**：squash 合并到 `main`，提交信息沿用 PR 标题。

---

## 5. 各类贡献怎么做

### 5.1 翻译与文案（i18n）

- 文案文件：`src/lib/i18n/locales/<语言>/{common,shell,errors}.json`
- 新增语言：复制 `zh-CN/`（或 `en-US/`）为新语言目录，在 `src/lib/i18n/index.ts` 的
  `SUPPORTED_LANGUAGES` 里登记——**不需要改任何组件**。
- 规则：
  - 中英（以及任何新语言）的 **key 必须完全一致且没有空值**，由 `src/lib/i18n/i18n.test.ts` 断言；
  - 组件里的用户可见文案必须走 `t('key')`，硬编码会被 `pnpm i18n:lint` 拦住；
  - 例外的写法与理由见 `docs/CODING_STYLE.md` §3.4（`// i18n-ignore`）。
- 自检：`pnpm i18n:lint && pnpm i18n:check && pnpm test`。
- **术语表**：`docs/I18N-GLOSSARY.md` 是中英对照的术语决定表——新文案先查表；
  同一概念两种译法比翻译错误更糟。要新增术语，改表并在 PR 里说明。
- **占位符**：`{{count}}` 会触发 i18next 的复数机制（要求 `_one`/`_other` 后缀键）；
  除非真的要做复数，插值参数请避开 `count`（如用 `{{n}}`）。
- **日期/数字**：一律用 `src/lib/i18n/intl.ts` 的 `formatDateTime` / `formatDate` /
  `formatRelative`（按当前语言走 Intl），禁止散落的 `toLocaleString()`。
- 英文文案必须**地道**（不是中文直译）；抽查基准与反例见术语表"语气与风格"一节。

### 5.2 诊断规则（M5 起）

诊断规则把 Git 的原始报错变成"人话原因 + 可执行修复动作"。

- 规则文件：`crates/diagnostics/rules/*.yaml`
- 规则里**只写 i18n key，不写文案**（文案在 `errors.json` 里，便于翻译）
- 新增一条规则请同时补：一条 fixture（真实 `stderr` 样本）与断言；
- `kind = dangerous` 的修复动作必须走危险确认对话框，不允许一键执行（红线 R7）。

### 5.3 文档

- 文档清单与定位见 `PLAN.md` §8.7。
- 写完请跑 `pnpm check:docs`：它校验仓库内文档的**相对链接与锚点**是否存在
  （外部链接不做网络探测，见脚本头部说明）。
- 若你改了架构或契约，请同步更新 `docs/ARCHITECTURE.md` / `docs/API.md`，
  否则下一个读代码的人会被过期文档误导。

### 5.4 新增命令、页面或 crate

- 命令：按 `docs/API.md` §4 的检查清单执行（定义位置、参数校验、错误转换、登记、单测）。
- 页面：路由条目（`src/app/routes.tsx`）+ 导航项（`src/app/shell/navItems.ts`）+ 全部文案走 i18n。
- crate：更新 `Cargo.toml` 的 `members`、`docs/ARCHITECTURE.md` §2 的表，运行
  `pnpm check:repo` 确认它在仓库里真实存在（不是只在你本机）。

### 5.5 新增依赖

依赖是需要评审的：请在 PR 描述里说明**理由、体积/构建时间影响、替代方案、许可证**。
项目要求：

- 许可证必须与 Apache-2.0 兼容；GPL/AGPL 一律不接受（libgit2 的链接例外除外）；
- 锁文件（`Cargo.lock`、`pnpm-lock.yaml`）必须一起提交；
- 不得引入任何 AI/ML 推理依赖（红线 R1）与第三方遥测 SDK（红线 R6）。

---

## 6. 行为准则与许可

- 参与本项目即表示你同意遵守 `CODE_OF_CONDUCT.md`（Contributor Covenant v2.1，随转公开阶段一并加入仓库）；
  在那之前，请保持基本的专业与尊重。
- 本项目采用 **Apache-2.0**（见 [`LICENSE`](./LICENSE)）。提交代码即表示你同意以同一许可证授权，
  **不需要签署 CLA**。
- 安全问题请**不要**公开提 issue，走 GitHub 的私密漏洞报告（Security → Report a vulnerability）；
  `SECURITY.md` 会在后续里程碑补齐。
