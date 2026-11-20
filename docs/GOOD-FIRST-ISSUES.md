# good first issue 候选清单（M8 / T8.7）

> 用途：给**第一次参与本项目**的外部贡献者的 15 个任务。每条都写明了背景、
> 步骤、涉及文件、验收标准与"如何运行测试"，目标是**不看额外上下文也能完成**。
>
> 创建方式：由人类（或被授权的维护者）在 GitHub 上逐条创建为 Issue，并打上
> `good first issue` + 对应 `area/*` 标签（标签体系见文末 §3）。
> 本文件是**待创建的草稿**，不代表 Issue 已存在。
>
> 相关：[CONTRIBUTING.md](../CONTRIBUTING.md)、[CODE_OF_CONDUCT.md](../CODE_OF_CONDUCT.md)、
> [docs/DEV-ENV.md](./DEV-ENV.md)、[docs/I18N-GLOSSARY.md](./I18N-GLOSSARY.md)

## 1. 总览

| # | 类别 | 标题 | 建议标签 | 规模 |
| --- | --- | --- | --- | --- |
| 1 | 文档 | 新增 3 条故障排查条目 | `area/docs` | S |
| 2 | 文档 | 补充中文手册「amend」与「文件监听」两节 | `area/docs` | S |
| 3 | 文档 | 英文界面文案审校（common + errors 两个语言包） | `area/i18n` `area/docs` | M |
| 4 | 文档 | 术语表补 5 条（detached HEAD / cherry-pick / reflog / hunk / upstream） | `area/i18n` | S |
| 5 | i18n | 新增 `ja-JP` 语言包骨架 | `area/i18n` | L |
| 6 | i18n | 把术语表的"禁用词"变成可执行门禁脚本 | `area/i18n` `area/tests` | M |
| 7 | 测试 | 为 `parse_diff_numstat` 补边界 fixture | `area/tests` | M |
| 8 | 测试 | 为命令面板补无障碍测试 | `area/tests` `area/a11y` | M |
| 9 | 测试 | 为诊断规则补"误报反例" | `area/tests` `area/diagnostics` | M |
| 10 | 功能 | 新增诊断规则：`commit-signing-failed` | `area/diagnostics` | M |
| 11 | 功能 | 快捷键键位预设机制（含一个内置预设） | `area/shortcuts` | L |
| 12 | 功能 | 新增一个原创配色主题 | `area/themes` | M |
| 13 | 插件 | 新增只读统计插件（分支概览面板） | `area/plugins` | M |
| 14 | 插件 | 提交信息校验钩子插件（只改写/告警，不拦截） | `area/plugins` | M |
| 15 | 工程 | 为提交图基准补"数值断言 + 基线记录" | `area/perf` | M |

规模仅为预估（S < 半天 / M ≈ 1–2 天 / L ≈ 2–3 天），不构成工期承诺。

---

## 2. 各条详情

### 1. 新增 3 条故障排查条目（文档）

**背景**：`docs/TROUBLESHOOTING.md` 目前 21 条（编号 1–21，跨章节全局连续）。
维护者收到过下面三类提问，但文档里没有对应条目。

**目标**：在对应章节内新增 3 条（编号 22、23、24），保持现有格式：
`**N. 标题**` + `原因：…`（可省）+ `处理：…`。

建议主题（先在全文件里检索确认不重复）：
1. 「检查更新失败 / 点检查更新没有反应」（放入「应用与更新」章）；
2. 「便携版与安装版混用时设置"丢失"」（说明两者的数据目录差异，放入「Windows」章）；
3. 「快捷键不生效」（被输入法/其他应用占用时的排查路径，放入「应用与更新」章）。

**涉及文件**：`docs/TROUBLESHOOTING.md`

**步骤**：
1. 阅读文件顶部说明与第 21 条，确认格式；
2. 依据 `docs/install/windows.md`、`docs/FAQ.md` 与源码行为写准确的原因与处理（不确定的结论宁可不写）；
3. 不改动现有条目编号。

**验收标准**：
- [ ] 新增 3 条且编号连续（22–24）；格式与既有条目一致；
- [ ] `pnpm check:docs` 通过（文档链接有效）；
- [ ] PR 说明中给出每条结论的来源（源码/实测/既有文档）。

**运行测试**：`pnpm check:docs`

**不确定时问谁**：在本 Issue 下留言；或到 GitHub Discussions → Q&A 提问。

---

### 2. 补充中文手册「amend」与「文件监听」两节（文档）

**背景**：`docs/manual/01-work-with-a-repository.md` 里
「修改最后一次提交（amend）」与「文件监听」两节各只有 3 行，新用户看不懂能做什么、风险在哪。

**目标**：各扩写到 8–15 行，覆盖：什么时候用 / 界面在哪 / 与快照的关系 / 常见误用。

**涉及文件**：`docs/manual/01-work-with-a-repository.md`；参考实现
`src/features/commit/`（amend 流程）与 `src/stores/settingsStore.ts`
的 `WATCH_AUTO_REFRESH_KEY` / `WATCH_DEBOUNCE_KEY`（文件监听的开关与去抖动）。

**验收标准**：
- [ ] 两节均说明"界面位置 + 典型操作 + 风险提示"；
- [ ] 与界面实际文案一致（截图文字、按钮名不臆造）；
- [ ] `pnpm check:docs` 通过。

**运行测试**：`pnpm check:docs`（手册是静态站与官网同源构建，
如想本地看效果见 `docs/DEV-ENV.md`）。

---

### 3. 英文界面文案审校（i18n）

**背景**：`en-US` 语言包由中文原稿翻译而来，部分表达生硬或术语不一致
（对照 `docs/I18N-GLOSSARY.md` 的约定）。

**目标**：审校 `common.json`（23 key）与 `errors.json`（92 key）两个文件，
把不自然、不地道或与术语表冲突的表达改好；**不改 key、不删 key、不加 key**。

**涉及文件**：
- `src/lib/i18n/locales/en-US/common.json`
- `src/lib/i18n/locales/en-US/errors.json`
- 对照：`docs/I18N-GLOSSARY.md`

**验收标准**：
- [ ] key 集合与 zh-CN 完全一致（`pnpm i18n:check` 通过）；
- [ ] 每处改动在 PR 描述里给出理由（一行一个）；
- [ ] 占位符（`{{count}}` 等）零改动。

**运行测试**：`pnpm i18n:check && pnpm exec vitest run src/lib/i18n`

---

### 4. 术语表补 5 条（i18n 文档）

**背景**：`docs/I18N-GLOSSARY.md` 现有 40 条三列表格（`中文 | 英文 | 备注`）。
新界面反复出现 5 个还没有约定的词，翻译时各写各的。

**目标**：为 detached HEAD、cherry-pick、reflog、hunk、upstream 各补一条，
备注写清"为什么这样译 / 不这样译"。

**涉及文件**：`docs/I18N-GLOSSARY.md`

**步骤**：先在 `src/lib/i18n/locales/zh-CN/`、`en-US/` 中检索这 5 个词
**当前实际怎么译**（保持一致，除非你的改动同时更新了两侧语言包），再落表。

**验收标准**：
- [ ] 5 条新条目，格式与既有 40 条一致；
- [ ] 与现有界面文案一致（不一致处要么改文案并说明，要么在备注写明差异原因）；
- [ ] `pnpm check:docs` 通过。

---

### 5. 新增 `ja-JP` 语言包骨架（i18n）

**背景**：界面已支持 zh-CN / en-US；语言机制是"整套 key 对齐 + 逐命名空间加载"。
第一个新语言需要把骨架搭起来，后续翻译才能增量化推进。

**目标**：新增 `ja-JP` 骨架：`common.json` 由你**完整翻译**（23 key），
`shell.json` / `errors.json` 先**整体复制 en-US 的原文**（保证 key 集合与无空文案）；
同时把语言接进选择界面。骨架阶段的界面上会出现英文占位，这是预期行为
（在 `docs/I18N-GLOSSARY.md` 或 PR 里注明"翻译进行中"）。

**涉及文件**（一处不落，否则静默回落或测试红）：
- 新建 `src/lib/i18n/locales/ja-JP/{common,shell,errors}.json`
- `src/lib/i18n/index.ts`：`SUPPORTED_LANGUAGES`、静态 import、`resources`
- `src/lib/i18n/i18n.test.ts`：`BUNDLES` 与断言 `SUPPORTED_LANGUAGES` 的用例
- `scripts/i18n/check-i18n-keys.mjs`：`LOCALES` 常量
- `src/features/settings/AppearanceSettingsPage.tsx`：语言选择项（显示名写 `日本語`）

**验收标准**：
- [ ] `pnpm i18n:check` 通过（三套语言 key 完全一致、无空文案、占位符一致）；
- [ ] `pnpm exec vitest run src/lib/i18n` 通过；
- [ ] 设置 → 外观 能切换到日本語，界面不崩、无缺失 key 报错。

**不确定时问谁**：命名空间/加载机制的问题请在本 Issue 下留言，不要自行改机制。

---

### 6. 把术语表的"禁用词"变成可执行门禁（工程 + i18n）

**背景**：`docs/I18N-GLOSSARY.md` 明确写了"暂存不用'缓存/加入索引'、快照与 stash
区分"等约定，但**没有机器检查**——约定靠人记。

**目标**：新增 `scripts/i18n/check-glossary.mjs`：
从术语表的备注列提取"禁用词"（建议在表格中引入一个可解析的标记，如
`| 中文 | 英文 | 不用：缓存/加入索引；原因… |`），扫描
`src/lib/i18n/locales/zh-CN/**.json` 的值，发现禁用词则以非零码退出并列出行号。

**涉及文件**：新增脚本 + `package.json` 的 `i18n:check` 链路（可挂为
`i18n:glossary` 并在 `.github/workflows/i18n.yml` 中调用）。

**验收标准**：
- [ ] 脚本对当前语言包运行通过（没有残留的禁用词，或先修掉）；
- [ ] 故意写入一个禁用词时脚本以非零码退出（PR 里贴出这段输出）；
- [ ] 脚本有最小单测（`scripts/` 下的既有脚本风格：纯 Node、无新依赖）。

---

### 7. 为 `parse_diff_numstat` 补边界 fixture（测试）

**背景**：`crates/git-engine/tests/fixtures/` 有 24 个从真实 git 输出重定向的
fixture；`diff_numstat` 的 6 个用例尚未覆盖 `-z` 形式下的转义与怪路径。

**目标**：新增 3 个 fixture + 对应用例：
① 文件名含制表符/换行的转义；② 纯重命名（0 行变更）；③ CRLF 文件。

**涉及文件**：
- `crates/git-engine/tests/fixtures/diff_numstat_*.txt`（新文件；
  按现有文件头的"fixture ↔ 生成命令"注释写清如何用真实 git 生成，
  建议版本与现有 fixture 一致）
- `crates/git-engine/tests/parsers_fixtures.rs`（照抄同文件的现有 `#[test]` 模板）

**验收标准**：
- [ ] `cargo test -p forgedesk-git-engine` 全绿；
- [ ] 每个新 fixture 标注生成命令；
- [ ] 至少一个用例是"改坏解析实现会红"的有效用例（PR 里说明并展示）。

---

### 8. 为命令面板补无障碍测试（测试）

**背景**：`src/features/commands/CommandPalette.tsx` 已实现键盘与 ARIA 语义，
但**没有任何测试**（`src/features/commands/` 全目录无测试文件）。

**目标**：新建 `src/features/commands/CommandPalette.test.tsx`，至少覆盖：
以 `role=listbox`/`role=option` 查询面板项；`aria-selected` 随上下键移动；
回车执行选中命令；`Escape` 关闭；无匹配时展示空态文案。

**涉及文件**：新增测试文件；被测组件 `src/features/commands/CommandPalette.tsx`、
`commandRegistry.ts`（mock 建议参考 `src/ui/components/__tests__/menus.test.tsx`
的 role 查询风格与 `src/test/renderApp.tsx` 的真实路由渲染）。

**验收标准**：
- [ ] `pnpm exec vitest run src/features/commands` 全绿；
- [ ] 断言用 role 与可访问名（不断言 class/内部状态）；
- [ ] 不 mock 被测组件内部的子组件。

---

### 9. 为诊断规则补"误报反例"（测试）

**背景**：诊断规则以正则匹配 git stderr；宽正则容易在无关输出上误报
（例如 `nothing to commit` 出现在**钩子自己的日志**里）。
现有 `crates/diagnostics/tests/rules.rs` 有一条"20 个无关 stderr 不得产生 primary"
的用例（`fn` 名以文件中实际为准）——本任务就是往它补样本。

**目标**：补 2 个反例，断言不产生 primary（或在 `alternatives` 里且
confidence 明显低于正例）：① 包含 `timed out` 的无关输出（非网络命令）；
② 钩子输出里包含 `nothing to commit`。

**涉及文件**：`crates/diagnostics/tests/rules.rs`

**验收标准**：
- [ ] `cargo test -p forgedesk-diagnostics` 全绿；
- [ ] 如果反例确实命中了规则（说明规则过宽），**不要改测试蒙混**——
      在本 Issue 下说明并附输出，维护者决定是收紧规则还是换反例；
- [ ] PR 标注两条例子的来源。

---

### 10. 新增诊断规则：`commit-signing-failed`（功能）

**背景**：`crates/diagnostics/rules/` 现有 51 条规则，**没有**提交签名失败
（GPG/SSH 签名）这一类——而它是自建密钥用户的高频阻塞点。

**目标**：新增一条规则：匹配 `gpg failed to sign the data` /
`error: gpg failed to sign the data` / `failed to write commit object`（签名失败路径），
给出"检查 GPG 密钥与 git 配置"的处理建议。规则按域放进 `commit.yaml`。

**涉及文件**：
- `crates/diagnostics/rules/commit.yaml`（追加条目；只写 key，不写中文文案）
- `src/lib/i18n/locales/{zh-CN,en-US}/shell.json`：`diag.<id>.title/.explanation/.cause*/fix_*`
- `crates/diagnostics/tests/rules.rs`：`FIXTURES` 追加样本（每条规则必须被自己的 fixture 命中为 primary）

**验收标准**：
- [ ] `cargo test -p forgedesk-diagnostics` 全绿；
- [ ] `pnpm i18n:check` 通过（中英 key 同步）；
- [ ] 规则 id 先检索现有 51 条确认不重复。

---

### 11. 快捷键键位预设机制（功能）

**背景**：快捷键系统（`src/lib/shortcutKeys.ts` + `src/features/commands/`）支持
逐命令覆盖（`shortcuts.overrides.<命令id>`），但**没有"预设"概念**——
新用户只能拿到 8 条默认键。本任务引入第一个预设（例如 `vscode-like`）。

**目标**：新增 `src/lib/shortcutPresets.ts`，定义一个预设（覆盖 5–8 个高频命令，
键位自拟且不与现有默认键冲突），在 设置 → 快捷键 提供一个"应用预设"入口；
应用即写入 `shortcuts.overrides.*` 并即时生效（沿用 `ShortcutManager` 的读取路径）。

**涉及文件**：新增 `src/lib/shortcutPresets.ts`（+ 测试）、
`src/features/settings/ShortcutsSettingsPage.tsx`、`src/features/commands/commandDefs.ts`
（只读引用命令 id）、i18n `shell.json` 的 `settings.shortcuts.*`。

**验收标准**：
- [ ] 预设应用后，`ShortcutManager` 分发到对应命令（测试覆盖）；
- [ ] 预设内部无冲突（`detectConflicts` 断言为空）；
- [ ] i18n 中英同步；`pnpm lint && pnpm typecheck` 通过。

**注意**：这是"新功能"而非"补充预设"——仓库里此前没有预设机制（见
`docs/I18N-GLOSSARY.md` 与 `src/stores/layoutStore.ts` 的"布局预设"是另一回事）。

---

### 12. 新增一个原创配色主题（功能）

**背景**：内置主题有 4 个（Light / Dark / Sandstone Dawn / Pine Nocturne）。
主题定义是纯数据，适合作为第一个视觉向贡献。

**目标**：新增一个**原创**深色主题（建议名 "Deep Harbor"，也可自拟），
提供完整 19 个颜色 token 与字体/动效字段。

**涉及文件**：
- `src/features/themes/builtinThemes.ts`（照 Sandstone Dawn 的字段结构）
- 对照白名单：`src/features/themes/themeModel.ts` 的 `THEME_COLOR_TOKENS`（未知 key 会报错）
- 测试：`src/features/themes/themeModel.test.ts` / `AppearanceSettingsPage.test.tsx`（补一条激活用例）

**验收标准**：
- [ ] `pnpm check:contrast` 通过（正文对比度 ≥ 4.5，非文本 ≥ 3）；
- [ ] `pnpm check:colors` 通过（不引入 token 之外的硬编码颜色）；
- [ ] 设置 → 外观 能激活并全局生效（测试断言色板真的被应用）；
- [ ] 配色 **100% 原创**，不得复刻任何竞品配色（红线 R3，PR 里声明）。

---

### 13. 新增只读统计插件：分支概览（插件）

**背景**：示例插件 `repo-stats`（源码 `plugins/repo-stats/src/lib.rs`）演示了
只读面板的最小骨架；第二个示例会让插件文档更容易被理解。

**目标**：新增 `plugins/branch-overview/`：面板展示每个本地分支的
"最后提交时间 + 领先/落后 upstream"（用宿主 `get_git_log` / `get_repo_info` op，
见 `docs/PLUGIN-API.md` §0 的能力表），产物同步到 `plugins/examples/branch-overview/`。

**涉及文件**：`plugins/branch-overview/{Cargo.toml,src/lib.rs,plugin.json,README.md}`、
`plugins/Cargo.toml`（workspace 成员）、`plugins/examples/branch-overview/`（构建产物）、
`scripts/build-plugins.mjs`（如需登记）。

**验收标准**：
- [ ] `rustup target add wasm32-wasip1` 后 `node scripts/build-plugins.mjs` 通过；
- [ ] 权限只用 `git:read` + `ui:panel`（最小权限，不得申请写权限）；
- [ ] 应用内启用后能看到面板（附截图或文字描述实测步骤）。

---

### 14. 提交信息校验钩子插件（插件）

**背景**：`docs/PLUGIN-API.md` §6 定义了提交钩子（op 16/17）：
pre 钩子可以**改写**提交信息首行，但**本期不能阻断提交**（`can_block` 恒不授予）。

**目标**：新增 `plugins/commit-lint/`：pre 钩子里检查首行是否匹配
`<type>(<scope>): <subject>`；不匹配时**追加一行提醒**到提交信息体（或按 API
能力给出告警），**不阻断提交**。

**涉及文件**：`plugins/commit-lint/**`（同 #13 的结构）、
`plugins/examples/commit-lint/`、参考 `plugins/commit-template/src/lib.rs`。

**验收标准**：
- [ ] 构建通过；权限最小（`git:read` + `ui:*` 之一，不需要 `git:write`）；
- [ ] 实测：消息不合规时提交**仍然成功**，且信息被改写/提示（附输出）；
- [ ] README 写明"受宿主能力限制，本插件不拦截提交"。

---

### 15. 提交图基准：数值断言 + 基线记录（工程）

**背景**：`e2e/graph-bench.spec.ts` 目前只打印 `[BENCH ...]` 数字、无断言
（见该文件顶部注释），数字靠人工抄进 `docs/PERF-BASELINE.md`。

**目标**：让基准能自我判定：把"5 万节点首屏 < 1.5s"落成断言，
并输出一行机器可读的基线行（便于后续对账）。

**涉及文件**：`e2e/graph-bench.spec.ts`、`docs/PERF-BASELINE.md`（补"如何跑"小节）。

**步骤**：
1. 本地跑现状：`$env:PERF_BENCH='1'; npx playwright test e2e/graph-bench.spec.ts`
2. 记录当前数值；**若超过 1.5s，把阈值先定为"现状 × 1.3"并在 PR 说明**，
   同时开一个 `area/perf` 的优化 issue（不在本任务里优化）；
3. 断言用 `expect(...).toBeLessThan(...)`，失败信息带上实测值。

**验收标准**：
- [ ] 基准在 CI 可跳过（`PERF_BENCH=1` 门控保留）、本地可跑；
- [ ] 断言失败时信息包含实测数字与阈值（PR 附一次故意失败的输出）；
- [ ] `docs/PERF-BASELINE.md` 记录本次数值与运行方法。

---

## 3. 标签与创建方式（给维护者）

创建这 15 条时建议新建以下标签（GitHub → Issues → Labels；本仓库当前
没有标签定义文件）：

| 标签 | 说明 |
| --- | --- |
| `good first issue` | GitHub 内置；标注后会自动出现在"Good first issues"聚合页 |
| `area/docs` / `area/i18n` / `area/tests` / `area/diagnostics` / `area/themes` / `area/plugins` / `area/perf` / `area/shortcuts` / `area/a11y` | 按上表逐条打 |

批量创建可参考 `gh` CLI：

```bash
gh label create area/docs --color 0E8A16 --description "文档类改动"
gh issue create --title "<标题>" --body-file <(sed -n '/### N\./,/^---$/p' docs/GOOD-FIRST-ISSUES.md) --label "good first issue,area/docs"
```

（逐条创建时直接从本文件复制对应小节作为正文即可。）
