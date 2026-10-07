# M7 启动准备（Windows 优先）

> 日期：2026-10-06
> 依据：`docs/PLAN.md` §M7、`docs/OPEN-SOURCE-READINESS.md`、`AGENTS.md`
> 范围决策：**以 Windows 为首发目标**；macOS 与 Linux 用户少、真机验证成本高，**暂缓**（保留代码与 CI 骨架，不阻塞 v1.0 for Windows）。

---

## 一、M0–M6 汇总核查结论

| 核查项 | 结论 |
| --- | --- |
| 代码是否汇总到主分支 | 全部在 `main`（222 提交）；远程 `origin/m6` 已合并（merge `2619d81`） |
| 是否有散落副本 / 未合并工作树 | 无。`git worktree list` 仅 `main`；M6 报告中的 `ForgeDesk-M6` 工作树已清理 |
| 工作区是否干净 | `git status` 无未跟踪/未提交改动（本次修复前） |
| 质量门禁现状 | 前端 83 文件 / 873 测试全绿；`i18n:check` 1846 key 中英对齐；`typecheck` 无错；`lint` 0 error（1 个既有 warning，见 §六） |

---

## 二、界面显示核查结论（发现并已修复）

| # | 问题 | 位置 | 处理 |
| --- | --- | --- | --- |
| 1 | **侧栏「插件」与命令面板「打开插件」指向 M0 占位页**，显示"页面还没有实现（T6.4）"；真实插件管理器只存在于 `设置 → 插件` | `src/features/plugins/PluginsPage.tsx` | 改为转发真实 `PluginSettingsPage`（与 `RepoStatusPage` 同约定） |
| 2 | 顶栏搜索提示写死"命令面板与全局搜索将在 M4 实现"（命令面板 M5 已交付） | `titleBar.search.hint` | 改为如实描述现状（中英同步） |
| 3 | 工作区操作"在编辑器打开（M5 可用）"（编辑器 M5 已交付，括注误导） | `workspace.actions.openEditor` | 去掉里程碑括注 |
| 4 | 提交详情面板按钮 tooltip"将在 T2.8 接通"（T2.8 已交付，操作在历史页操作面板可用） | `history.detail.t28Hint` | 改为指向真实入口 |
| 5 | 代码注释残留"M4 实现"（`GlobalSearch.tsx`、`navItems.ts`） | 两处注释 | 已更正 |

> 版本号显示：已由 `0.0.1` 提升到 **`0.7.0`**（`package.json` / `Cargo.toml` / `tauri.conf.json` + `Cargo.lock` 同步），并补齐 M1–M6 的 CHANGELOG 条目（见 §七）。

---

## 七、进度记录

### 2026-10-06：版本与界面收尾

| 事项 | 状态 |
| --- | --- |
| 审批项 A：版本号 → `0.7.0` + 补齐 CHANGELOG（M1–M6） | ✅ 完成 |
| 审批项 B：移除 PLAN 未规划的「设置 → Git」占位页（含路由 / 导航 / 用例 / i18n key） | ✅ 完成 |
| 既有限制：`commandDefs.ts` 的 `useMemo` 多余依赖 warning | ✅ 修复（lint 现 0 warning） |
| `CHANGELOG.md` 文件损坏（标题缺字、版本号截断） | ✅ 重写修复 |

### 2026-10-07：M7 文档与社区文件（T7.7 / T7.8 主体）

| 事项 | 状态 |
| --- | --- |
| `README.md` 更新：状态从「M0 工程底座」改为 M0–M6 已完成 / M7 进行中，补齐功能总览与文档索引 | ✅ 完成 |
| 新增 `docs/install/windows.md`（下载方式、SHA256/GPG 校验、SmartScreen 指引、卸载） | ✅ 完成 |
| 新增 `docs/PRIVACY.md`（数据清单、对外请求、遥测、用户权利 + 免责声明） | ✅ 完成 |
| 新增 `docs/FAQ.md`（24 条） | ✅ 完成 |
| 新增 `docs/TROUBLESHOOTING.md`（21 条，含三平台） | ✅ 完成 |
| 新增社区文件：`CODE_OF_CONDUCT.md`、`SECURITY.md`、`GOVERNANCE.md` | ✅ 完成 |
| 新增 `.github/ISSUE_TEMPLATE/plugin_issue.yml` 与 `.github/DISCUSSION_TEMPLATE/*`（4 个分类） | ✅ 完成 |
| `bug_report.yml` 版本占位符 `0.0.1` → `0.7.0` | ✅ 完成 |
| 门禁 | check:docs（49 文档 / 189 链接）、compliance（含 PRIVACY 免责声明断言）、check:repo、check:workflows、format:check 全绿 |

> 尚未完成：`docs/RELEASE.md`（发布流程与密钥）、`docs/install/macos.md`（M8）。
> 遗留审批项 C（`target/debug` 71.8 GB）、D（工作区外残留）、E（历史右键菜单接线）、F（更新密钥/CI Secrets）仍待人类决定。

### 2026-10-07（续）：CI 文案门禁 + 用户手册

| 事项 | 状态 |
| --- | --- |
| 新增 `.github/workflows/i18n.yml`（硬编码文案 + 中英 key 对齐，按 paths 过滤） | ✅ 完成 |
| 新增用户手册 `docs/manual/`（索引 + 4 篇：仓库/提交、历史/分支/同步、冲突/rebase/安全网、托管/终端/编辑器/扩展） | ✅ 完成 |
| `README.md` 文档索引补充用户手册入口 | ✅ 完成 |

> 未做：`release.yml` 与 `audit.yml`——前者依赖审批项 F（更新签名密钥与更新清单托管），
> 且应与 T7.1（updater 集成）一并落地；后者需要先决定 `cargo-deny` 配置（当前仓库无 `deny.toml`，
> 许可证/GPL 已由 `pnpm compliance` 覆盖）。

### 2026-10-07（续二）：T7.5 崩溃恢复与安全模式

| 事项 | 状态 |
| --- | --- |
| `crates/platform/src/session.rs`：安全模式**一次性**标记原语（请求 / 清除 / 消费）+ 4 个单测 | ✅ 完成 |
| `crates/plugin-host`：`PluginManager::new` 增加 `safe_mode`；新增 `enabled_intent`（安全模式不激活插件，但**不写坏**用户启用意愿） | ✅ 完成 |
| `crates/commands`：新增 `startup.rs`（`app_startup_report` / `app_restart` + `SessionEnder` + 3 单测）；`AppState` 增加 `startup` / `end_session` | ✅ 完成 |
| `term_create`：安全模式下拒绝创建终端（后端兜底） | ✅ 完成 |
| `src-tauri`：启动时消费安全模式标记并把报告注入 `AppState`；重启前结束会话标记（避免主动重启被误判为崩溃） | ✅ 完成 |
| 前端：`appStartupReport` / `appRestart` IPC + `StartupRecoveryNotice`（异常退出模态 + 安全模式常驻横幅）+ 中英文案 + 5 个 Vitest 用例 | ✅ 完成 |
| `docs/API.md`：登记两个新命令 | ✅ 完成 |
| 验证 | `cargo check --workspace --all-targets` 无错；`cargo test -p forgedesk-platform -p forgedesk-commands`（82+58 通过）与 `-p forgedesk-plugin-host`（76 通过）；前端相关套件通过 |

> 与验收标准的差距：PLAN M7 要求"崩溃后重启提示…可选择安全模式（禁用插件与终端）"——
> 已按此实现（插件不激活 + 终端拒绝 + 两处入口提示）。

### 2026-10-07（续三）：T7.6 应用内隐私说明页

| 事项 | 状态 |
| --- | --- |
| 新增 `src/features/settings/PrivacySettingsPage.tsx`（6 条速览：无遥测 / 无 AI / 凭据 / 日志 / 对外请求 / 本地数据与删除 + 「查看完整隐私政策」入口） | ✅ 完成 |
| 设置导航与路由注册（`/settings/privacy`）+ 路由可达用例 | ✅ 完成 |
| 中英文案 16 项；`docs/PRIVACY.md` 注明与应用内页面一致 | ✅ 完成 |
| 测试：`privacySettings.test.tsx`（2 用例：关键承诺渲染 + 打开完整政策） | ✅ 完成 |
| 验证 | typecheck / lint / i18n:check（1871 key）/ format 全绿；相关套件 26 通过 |

> 至此 OPEN-SOURCE-READINESS 的 B-8（隐私政策存在**且与应用内页面一致**）闭合。
> 仍未做：遥测（当前**不含任何遥测**，若要做属范围新增，须人类决策）；
> 审计导出的"另存为目录"（现导出到临时目录）。

### 2026-10-07（续四）：T7.4 Windows 便携版 zip

| 事项 | 状态 |
| --- | --- |
| 新增 `scripts/ci/make-portable.mjs`：Node 内建 `zlib` 直写 ZIP（零新依赖），产出 `ForgeDesk_<版本>_windows_<架构>_portable.zip` + `.sha256`；内含 exe / LICENSE / README-portable.txt | ✅ 完成 |
| `package.json` 增加 `portable:win` 脚本；`.gitignore` 忽略 `/bundles/` | ✅ 完成 |
| `docs/install/windows.md` 补充便携版内容与本地构建命令 | ✅ 完成 |
| 验证 | 用夹具 exe 实跑脚本 → 系统 `Expand-Archive` 回读成功、3 个条目大小正确、解出的 exe 与源文件哈希一致、`.sha256` 与实际 zip 哈希一致 |

> 待 F 决策后，`release.yml` 的 Windows job 应调用本脚本（并在 Release 附件中一并上传 zip 与 .sha256）。

### 2026-10-07（续五）：发布流程文档与版本号门禁

| 事项 | 状态 |
| --- | --- |
| 新增 `docs/RELEASE.md`：版本号规则、stable/beta 渠道、**两套密钥**（GPG 签 SHA256SUMS / minisign 签更新包）的生成-保管-轮换、发布步骤、**回滚**、灾难恢复、发布检查清单 | ✅ 完成 |
| 新增 `scripts/setup/sync-version.mjs`（`pnpm version:sync` / `version:check`）：以 `tauri.conf.json` 为真相源同步三处版本并刷新 `Cargo.lock`；只改版本行，不动其余字节 | ✅ 完成 |
| `ci.yml` quality job 增加 `pnpm version:check` 门禁 | ✅ 完成 |
| 验证 | 脚本实测：`--check` 一致→0；set `0.7.1` 只改 tauri.conf.json / package.json / Cargo.toml 各一行（`git diff` 确认无附带改动）+ 刷新 Cargo.lock；再 set 回 `0.7.0` 复检通过 |

> 仍未做（待 F/G）：`release.yml`（tag → 构建 → 校验和 → GPG → Release → updater 清单 → Pages）、`audit.yml`。
> `docs/RELEASE.md` §4 描述了它们**应当**做什么，实现时以该文档为准。

### 2026-10-07（续六）：T7.1 自动更新（后端接入）

| 事项 | 状态 |
| --- | --- |
| 引入 `tauri-plugin-updater`（**精确锁 2.12.0**：2.13.x 要求 rustc 1.90 > 本仓库 MSRV 1.85；依赖放入 workspace 供宿主与命令层共用） | ✅ 完成 |
| `crates/commands/src/updater.rs`：`update_check`（未配置更新源时返回 `configured:false`，**不报错**）、`update_install`（校验"要装的版本 == 本次检查到的版本"，防"点 A 装 B"）+ `update:progress` 进度事件 | ✅ 完成 |
| `src-tauri/main.rs`：注册 `tauri_plugin_updater` 插件 + 两个命令（debug/release 两侧） | ✅ 完成 |
| `docs/API.md`：登记两个命令与 `update:progress` 事件 | ✅ 完成 |
| 验证 | `cargo check --workspace --all-targets` 通过；`cargo clippy --all-targets -- -D warnings` 与 `cargo fmt --check` 通过；`pnpm compliance` 通过（`LICENSE-AUDIT` 增量更新：Rust 依赖 644→657） |

> 仍未做：`plugins.updater` 的 endpoints 与 pubkey（待 F）；"跳过该版本 / 自动检查开关"这一组设置项。
> 另注：`pnpm compliance` 提示传递依赖 `r-efi` 为 LGPL（仅 UEFI target 使用，不进入本产品链接），
> 已在 §四 决策项 I 记录确认。

### 2026-10-07（续七）：T7.1 自动更新（前端横幅）

| 事项 | 状态 |
| --- | --- |
| `src/lib/ipc/updater.ts`：`updateCheck` / `updateInstall` / `listenUpdateProgress` + DTO 类型（与 Rust 侧 camelCase 逐字段对应） | ✅ 完成 |
| `src/features/system/UpdateBanner.tsx`：有更新时在顶栏下方显示横幅（版本号 + 稍后提醒 / 立即更新 + 下载百分比），未配置或无更新时**不渲染**；卸载时 off 掉进度订阅 | ✅ 完成 |
| `AppShell` 挂载；中英文案 7 项 | ✅ 完成 |
| 测试：`UpdateBanner.test.tsx` 5 用例（显示、版本透传、稍后提醒、未配置/无更新不显示、进度百分比） | ✅ 完成 |
| 验证 | `pnpm typecheck` / `pnpm lint` / `pnpm format:check` / `pnpm i18n:check`（1878 key）全绿；完整前端套件 **885 passed（86 文件）** |

> "立即更新"会把**当前显示的版本号**交给后端；后端校验不符即拒绝（`VALIDATION`），
> 避免"用户点了 A、装上的却是 B"。

### 2026-10-07（续八）：T7.1 更新设置项

| 事项 | 状态 |
| --- | --- |
| `settingsStore`：新增 `update.autoCheck`（默认开启）与 `update.skippedVersion`（只跳过**一个**版本号，发下个版本仍提示） | ✅ 完成 |
| `UpdateBanner`：三个动作定型——「稍后提醒」（仅本次会话）/「跳过此版本」（写设置）/「立即更新」；关闭自动检查或设置未加载完时**根本不发请求** | ✅ 完成 |
| 新增 `UpdateSettingsSection` 并挂到「设置 → 高级」：自动检查开关 + 显示/清除已跳过的版本 + 渠道说明（v1 不支持应用内切渠道） | ✅ 完成 |
| 中英文案 9 项；测试：横幅 7 用例 + 设置区块 3 用例 | ✅ 完成 |
| 验证 | `pnpm typecheck` / `pnpm lint` / `pnpm format:check` / `pnpm i18n:check`（1886 key）全绿；完整前端套件 **890 passed（87 文件）** |

> 设计取舍：把"已跳过的版本"显式显示在设置里——跳过后横幅不再出现，用户很容易忘了自己跳过过，
> 之后会把"怎么不提示更新"当成故障；显示 + 可清除让它成为可发现、可撤销的状态。

### 2026-10-07（续九）：T7.6 审计导出「另存为」

| 事项 | 状态 |
| --- | --- |
| 服务层：`AuditExportRequest` 增加 `target_path`（`None` 仍退回临时目录，保持旧行为与测试可用）；`AuditExportFormat::extension()` 改为 `pub` 供命令层校验 | ✅ 完成 |
| 命令层：`audit_export` 增加 `target_path?` + `validate_export_target`（必须绝对路径、扩展名与格式一致，大小写不敏感） | ✅ 完成 |
| 宿主能力：`capabilities/default.json` 增加 `dialog:allow-save`（此前 `dialog:default` 只含打开选择器），并更新说明 | ✅ 完成 |
| 前端：`pickSavePath`（保存对话框）+ 导出前先选位置；**用户取消就不写任何文件**（不回退临时目录）；建议文件名 `forgedesk-audit-<本地时间>.<ext>` | ✅ 完成 |
| `docs/API.md`：更新 `audit_export` 的参数、校验规则、取消语义与所需宿主能力 | ✅ 完成 |
| 测试 | Rust：新增 5 个（命令层 4 + 服务层 1，`cargo test -- export` 全过）；前端：`AuditHistoryPanel.test.tsx` 8 用例（+1 取消用例，导出用例改为断言选定路径与建议文件名） |
| 验证 | `cargo check/clippy -D warnings/fmt --check` 通过；`pnpm typecheck` / `lint` / `format:check` / `i18n:check`（1887 key）/ `check:docs` 全绿；前端 **891 passed（87 文件）** |

> 至此 T7.6 的可见部分齐备：审计导出（另存为）+ 权限清理 + 应用内隐私页 + 日志查看。
> 剩 M7 只差需要外部密钥/账号的发布链路（T7.1 发布配置、T7.2/T7.3 流水线）。

### 2026-10-07（续十）：M7 逐条验收自检 + 供应链固定

| 事项 | 状态 |
| --- | --- |
| 逐条验收自检（PLAN §M7 验收标准 10 条 + §10 清单差异项） | ✅ 产出 `docs/acceptance/M7.md`：**5 通过 / 2 部分通过 / 3 未执行**，未执行项全部卡在发布凭据 |
| 供应链固定（OPEN-SOURCE-READINESS C-1） | ✅ 32 处 action 引用固定到 commit SHA（保留 `# v7` 标签）、`compliance.yml` 补 `concurrency`；`pnpm check:workflows` **33 → 0 警告** |
| `CHANGELOG.md` 的 M7 段 | ✅ 补全（新增 / 变更 / 安全 / 文档 / 待办五节，逐条可核验） |
| `docs/OPEN-SOURCE-READINESS.md` | ✅ 状态由"未开始"改为"进行中"，补三条审计记录（B 组、C-1、M7 自检） |
| 门禁 | typecheck / lint / format / i18n / check:docs / check:repo / check:workflows / check:colors / check:contrast / version:check 全绿；前端 891 测试通过 |

> M7 结论：**产品侧与文档侧已就绪**；出口只取决于 §4·F（更新签名密钥 + 更新清单托管）与 §4·G（`release.yml`）。
> 这两项之后按「F → G → 发布演练 → 安装包实机验证 → 转公开门禁」推进。

### 2026-10-07（续十一）：转公开门禁 A 组（git 历史与内容审计）

| 项 | 结果 |
| --- | --- |
| A-3 提交者身份 | ✅ 全部 222 个提交的作者与提交者邮箱**唯一且为隐私地址**；`git config user.name/email` 亦已是 `EMBER` / noreply → **P-1/P-2 实为已完成**（文档状态此前过期） |
| A-5 凭据类文件 | ✅ `git ls-files` 对 `.env` / `*.key` / `*.pem` / `*.pfx` / `*.p12` / `id_rsa` / `id_ed25519` / `.npmrc` **0 命中** |
| A-6 大文件 | ✅ 4566 个对象中**无 >1MB 的 blob** |
| A-2 本机路径 | ✅ 2924 行命中**全部为占位**（`/home/u`、`/home/octocat`、`/home/runner`、`C:\Users\…\`）；`E:\Projects` 与 `Users\TD` 均 0 命中 |
| A-1 凭据扫描 | ✅ 以特征式替代（`gitleaks` / `trufflehog` 本机未安装）：全历史唯一样本 8 个，**全部为脱敏器自身的假夹具**；工作区与 34 个未跟踪文件 0 真凭据 |
| ⚠️ 发现 | `docs/adr/ADR-004` 曾回显**个人 QQ 邮箱** → HEAD 已脱敏；历史侧仅 1 个提交引入（`08be90e`），**清洗成本低**。是否清洗/接受由人类决定 |

> 门禁：`check:docs` / `format:check` 全绿（脱敏改动仅涉及文档）。

### 2026-10-07（续十二）：转公开门禁 C 组（供应链加固）

| 项 | 结果 |
| --- | --- |
| C-2 lockfile | ✅ `Cargo.lock` / `pnpm-lock.yaml` 均被跟踪；CI 已用 `--frozen-lockfile`（3 处），本轮给 cargo 的 clippy / test / safety / example-plugins 与 nightly 的 probe 补上 `--locked`（加参数前先本地 `cargo check --locked` 验证锁文件一致） |
| C-3 漏洞扫描 | ✅ JS 侧：修复 2 处 high（`sharp` → `^0.35.5`、`source-map-js` → `1.2.2` override），`pnpm audit` **归零**；⚠️ Rust 侧 `cargo audit` 本机无工具，建议补跑 |
| C-4 CI 权限 | ✅ 5 个工作流均有顶层 `permissions`（`check:workflows` 0 警告） |
| 复用既有机制 | `source-map-js` 的 override 加在 `pnpm-workspace.yaml`（pnpm 11 不读 `package.json#pnpm`），与既有 `ansi-regex` 条目同一处 |
| 门禁 | 前端 **891 测试**（依赖变更后复跑）、`typecheck` / `lint` / `format:check` / `i18n:check` / `check:workflows` / `compliance`（含依赖许可）全绿 |

> 转公开门禁剩余：**B-3 / B-4 的 3 人盲测**、**B-10**（干净环境复现）、C-3 的 Rust 侧补跑、§1.3 托管凭据。

### 2026-10-07（续十四）：发布流水线骨架（T7.3 / D7.4）

| 事项 | 状态 |
| --- | --- |
| `.github/workflows/release.yml` | ✅ 三个作业：`preflight`（三处版本号 + **tag ↔ 版本号**）/ `build-windows`（构建 → 归一化 → 便携版 → 合并校验和 → GPG 签名 → Notes → updater 清单）/ `publish`（`gh release create --verify-tag` → 组装 `site/`+`updates/` 部署 Pages）；动作全部固定 SHA，`check:workflows` **0 警告** |
| `scripts/ci/make-build-config.mjs` | ✅ 构建时注入 `endpoints` + `pubkey` + `createUpdaterArtifacts`（**仓库配置保持"无更新源"**，开发构建不受影响）；`--pubkey` 缺失即退出 2 |
| `scripts/ci/make-checksums.mjs` | ✅ 一个目录一份 `SHA256SUMS`（安装器 + 便携版 zip），排除签名/清单自身；**幂等**（重跑逐字节一致，已实测） |
| `scripts/ci/make-updater-manifest.mjs` | ✅ 生成 Tauri updater 清单；`--target` 走白名单（写错一个字符会让"新版本永远检测不到"）；缺文件/空签名给出可读错误 |
| `scripts/ci/release-notes.mjs` | ✅ Conventional Commits 分桶（破坏性变更置顶、认不出的进"其他"不丢弃）；已用真实历史跑通 |
| 未配置凭据时的行为 | ✅ **警告并跳过**（不失败）：与 `pages.yml` 同一约定，避免"每次打 tag 都红"训练出坏习惯 |
| 文档 | ✅ `docs/RELEASE.md` §4 清单路径修正 + 新增 §4.1（作业结构、凭据表、演练方式、首次发布核对项） |
| ⚠️ 未验证 | 流水线与脚本**从未在真实 CI/发布环境执行**；首次实跑需按 §4.1 核对 `.sig` 落点与清单 URL |

### 2026-10-07（续十三）：转公开门禁 B-3 / B-4（图标原创性证据链）

| 项 | 结果 |
| --- | --- |
| 真源自检 | `docs/brand/icon-source.svg` 图元**全为基础几何**（`rect`×5 + `path`×3 + 渐变×2），无 `<image>`、无外部引用、注释外无品牌词 |
| 产物可复现 | 重跑 `node scripts/brand/render-icon.mjs` 后 `icon-1024.png` 的 SHA256 **完全不变** → 图标确由真源渲染（且与 `sharp` 版本无关） |
| 产物台账 | 真源 + 1024 PNG + 17 个平台图标的 SHA256 写入 `docs/BRAND.md` §4.1：任何图标改动都会在台账里显形 |
| 人工盲测 | ⏳ 按 ADR-005 延后至首次对外预发布前；方法与记录表见 `docs/BRAND.md` §4.2（**不得由代理代填**） |
| 门禁 | `pnpm compliance` 的「图标检查（R2）」通过；`check:docs` / `format:check` 全绿 |

---

## 三、垃圾文件清理

**工作区内（已完成，均为可再生的 gitignore 产物）**

- 根目录 12 个构建日志：`.app-run.log`、`.cargo-*.log`、`.msvc-install.log`、`.preview.log`、`.push.log`、`.rust-toolchain.log`、`.tauri-build.log`、`.toolchain-install.log`、`.wrangler.log`
- `.wrangler/`（wrangler 本地缓存）、`dist/`、`coverage/`、`test-results/`

**保留（非垃圾）**：`.cargo/`（本机镜像配置，有意保留）、`node_modules/`、`target/`（构建缓存）。

**工作区外（2026-10-07 经批准后执行）**：已删除 `E:/Projects/fd-wasm-probe`（M6 一次性对照工程）与 `E:/Projects/ForgeDesk.zip`（旧快照）；相关注释与验收记录已同步（不再引用该路径）。

**仍未清理**：`target/debug/incremental`（**14.6 GB**）。构建环境的**批量删除护栏**拒绝了该操作，故改由人类执行：

```powershell
Remove-Item -Recurse -Force e:\Projects\ForgeDesk\target\debug\incremental
# 若还想回收依赖缓存（58 GB），在"下一次必然全量重编"的时机执行：cargo clean
```

> 为什么不直接 `cargo clean`：`target/debug/deps` 有 58 GB，删掉会让下一次 `cargo build/test` 全量重编依赖
> （20–40 分钟）。建议留到 M7 的发布演练前执行——那时本来就要做一次干净构建。

---

## 四、审批项与决策记录（2026-10-07：人类确认"按建议执行"）

| # | 事项 | 结论 | 备注 |
| --- | --- | --- | --- |
| A | 版本号与 CHANGELOG | ✅ 按①执行 | `0.7.0` + 补齐 M1–M6 条目；`pnpm version:sync` 成为三处同步的固定手段 |
| B | 「设置 → Git」占位页 | ✅ 按②执行 | 移除未规划占位页；身份/行尾设置如需，作为 M8 的新范围立项 |
| C | `target/debug` 膨胀 | ⏸ 部分执行 | `incremental`（14.6 GB）由人类执行（环境批量删除护栏阻止代理操作）；`cargo clean` 建议留到发布演练前 |
| D | 工作区外残留 | ✅ 已删除 | `fd-wasm-probe`、`ForgeDesk.zip`；同步清理了对其路径的引用（`Cargo.toml`、`docs/acceptance/M6.md`） |
| E | 历史页右键菜单接线 | ✅ 决定**不接线** | 入口已在「分支」页与「历史」操作面板；右键菜单再实现一套会产生两条真相源（同一操作两处逻辑）。菜单项保留为禁用态 + tooltip 指向真实入口 |
| F | 更新/GPG 密钥与 CI Secrets | ⏸ 待人类 | 需要人类账号（GitHub Secrets、Cloudflare）与私钥保管；步骤见 `docs/RELEASE.md` §3–§4。**代理不生成、不接触发布私钥** |
| G | `deny.toml`（cargo-deny） | ✅ 决定**暂不引入** | `pnpm compliance` 已是唯一的许可门禁（`cargo metadata` + 许可证清单 + GPL/AI/商标）；再引一套 allow 列表会产生两套规则并漂移。若将来引入，必须同时把 `compliance` 的许可检查摘掉或明确分工 |
| H | 自建遥测 | ✅ 决定**不引入** | 保持"零遥测"承诺；PLAN 的 PF-11（遥测，默认关闭）本就标注为 **V1.1**，不属 M7 |
| I | 传递依赖 `r-efi` 为 LGPL | ✅ 确认接受 | 仅用于 `target_os="uefi"`，不参与本产品任何目标平台的编译与链接；`compliance` 已提示"允许但需人工确认"，此处即为确认记录 |

---

## 五、M7 计划（Windows 优先）

> 目标：把工程产物变成**可公开下载安装的 Windows v1.0**，其余平台保留骨架、暂缓真机验证。

### 阶段 1：发布前置收尾（1–2 天）
1. 按审批项 A 落定版本号 + 补齐 CHANGELOG（M1–M6 + M7）。
2. Windows 打包细节：NSIS 安装器 + 便携版 zip、文件关联、协议注册、AUMID（使 Toast 通知可弹）。
3. 崩溃恢复与安全模式（T7.5）、审计日志导出（T7.6）。

### 阶段 2：自动更新（Windows）（2–3 天）
4. `tauri-plugin-updater` 集成：`update_check` / `update_install`、stable/beta 渠道、每 24h 检查（默认开启可关）、跳过版本。
5. Ed25519（minisign）密钥生成与离线备份流程 → `docs/RELEASE.md`；公钥硬编码、私钥入 CI Secret。
6. `UpdaterBanner`（状态栏）与失败可读提示；本地 mock manifest 验证成功/签名失败两条路径。

### 阶段 3：CI/CD 与发布流水线（2–3 天）
7. 扩展 `.github/workflows/ci.yml`：把 `check:colors` 等门禁清单化；Windows 质量门禁必过。
8. `release.yml`：打 `v*` 标签 → 质量门禁 → Windows 构建（NSIS + MSI + 便携版 zip）→ **SHA256 + GPG 校验和** → 创建 GitHub Release（Release Notes 自动分类）→ 发布 updater manifest。
9. action 固定 commit SHA；`pnpm check:workflows` 0 警告。

### 阶段 4：文档与官网（2–3 天）
10. 文档：`docs/install/windows.md`（PowerShell 校验命令 + SmartScreen 处理）、用户手册、FAQ ≥ 20、TROUBLESHOOTING ≥ 15、`docs/PRIVACY.md`。
11. 社区文件：`CODE_OF_CONDUCT.md`、`SECURITY.md`、Issue/PR/RFC 模板。
12. 官网静态站（`site/`，Cloudflare Pages）→ 下载页、版本矩阵、SHA256/GPG。

### 阶段 5：发布演练（1 天）
13. 旧版本 → 检测更新 → 下载 → 校验 → 重启升级（Windows 实机全过程）。
14. 篡改签名被拒的自动化用例；回滚演练（manifest 指回旧版本）。
15. 记录结果到 `docs/OPEN-SOURCE-READINESS.md` §4 审计记录。

### 暂缓项（保留代码/CI 骨架，不做真机验证）
- macOS：ad-hoc 签名断言、`docs/install/macos.md`、DMG 打包（M8 再做）。
- Linux：AppImage/deb/rpm、无 Secret Service 回退、inotify 上限（M8 再做）。
- 包管理器分发（Homebrew/Scoop/Winget/Flathub/AUR）→ M8。

### M7 出口标准（Windows）
- `pnpm lint`、`pnpm typecheck`、`pnpm test`、`cargo fmt/clippy/test` 全绿。
- Windows 全新环境安装 → 打开仓库 → 提交 → push 全流程可用。
- 自动更新可在旧版本上完成并签名校验通过；篡改包被拒。
- 打 tag 后 CI 自动产出安装包与更新清单并创建 Release。
- 冷启动到可交互 < 2s（本地 SSD）。
- 遥测默认关闭；崩溃后提供安全模式入口。

---

## 六、风险与后续

| 风险 | 缓解 |
| --- | --- |
| target 重编耗时长 | 选非高峰执行 `cargo clean`；或只清理 `target/debug/incremental` |
| 无付费签名导致 SmartScreen 拦截 | 便携版 zip + 包管理器（M8）+ SHA256/GPG + 文档指引 |
| 版本号不一致（三处） | 用单一脚本读 `Cargo.toml` 同步到 `package.json` / `tauri.conf.json` |
| 既有 lint warning：`commandDefs.ts` `useMemo` 依赖 `queryClient` 多余 | 一并修掉（去掉该依赖或 `useMemo`） |
| 长期残留的占位/禁用 UI | 本次已修 5 处；剩余见 §四·B/E |
