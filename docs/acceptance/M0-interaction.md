﻿# M0 交互级验收（OPS-7）

- 日期：2026-09-23 ｜ 环境：Windows 11 + Playwright（系统 Edge，Chromium 内核）+ Vite dev server（localhost:1420）
- 用例：`e2e/shell.spec.ts` + `e2e/states.spec.ts`，共 **11 例，11 通过 / 0 失败**（12.8s）
- 运行方式：`npx playwright test`（配置见 `playwright.config.ts`；CI 接入按 ADR-002 配额策略另行安排）

## 逐项结论

| # | OPS-7 要求 | 结论 | 证据（测试名 / 说明） |
| --- | --- | --- | --- |
| 1 | 悬停与命中（elementFromPoint，禁止目测） | ✅ 通过 | `sidebar entries and titlebar buttons are genuinely hittable`：对侧栏 4 个链接 + 仓库切换器 + 账号按钮 + 搜索输入框逐一取中心点做 elementFromPoint，断言命中点落在目标子树内 |
| 2 | 点击进入预期界面/状态 | ✅ 通过 | `every sidebar entry opens its own page`：8 个侧栏条目逐一点击并断言各自 h1；`route table entries resolve`：路由表全部条目渲染非空 |
| 3 | 面板联动 | ✅ 通过（M0 范围内） | `detail panel switches right -> bottom -> hidden`：详情面板三态；仓库切换 → 侧栏仓库级条目禁用态 ↔ 可用态联动。提交→详情→diff 联动属 M1，尚无该面板，不适用 |
| 4 | 状态一致性（名称/ID 映射断言） | ✅ 通过 | `repository-scoped entries …`：断言 URL 段 `#/repo/example-forgedesk/status` 与 store 的 repoId `example-forgedesk` 同源（该 id 来自 `src/features/repo/recentRepos.ts` 唯一定义）；`route table entries resolve` 断言导航项与路由表一一对应 |
| 5 | 空态 / 错误态 / 加载态 | ✅ 通过 | 错误态：浏览器无 IPC 时设置页与日志页呈现统一错误态且重试稳定（states 第 7/8 例）；加载态：`LogViewer.test.tsx` 的 pending→Skeleton 断言；空态：仪表盘"后台任务"空态由单测覆盖（E2E 下 IPC 缺失直接进错误态，属预期） |
| 6 | 键盘可达 | ✅ 通过 | `keyboard: skip link first, Ctrl+K focuses search, Esc closes the switcher menu`：首 Tab 落在"跳到主内容"；Ctrl+K 聚焦搜索；Esc 关菜单且焦点归还触发按钮 |
| 7 | `window.__errs` 为空 | ✅ 通过 | `uncaught error collection stays empty after all interactions (DoD gate)`：跨 3 个页面 + 错误态 + 重试 + 菜单操作后断言 `window.__errs === []`（钩子实现在 `src/main.tsx`，捕获未捕获异常与未处理拒绝，保留最近 50 条） |
| 8 | 跨引擎（WebView2 / WKWebView / WebKitGTK） | ⏸ 延后（ADR-005） | 维护者决定：三平台验证降级为按需触发（首次对外预发布或怀疑平台缺陷时）。E2E 目前跑在 Chromium 内核（Edge），DOM 级断言与 WebView 品牌无关；桌面运行时的跨引擎验证随首个矩阵触发执行 |

## 过程发现与处置（首跑 5 失败 → 修复后 11 通过）

| # | 发现 | 定性 | 处置 |
| --- | --- | --- | --- |
| F1 | 无本地语言偏好时 UI **跟随浏览器语言**（Playwright 浏览器为 en-US，界面渲染成英文） | 符合设计（i18n 回退策略），但此前无任何测试覆盖该行为 | E2E 固定 zh-CN 断言主链路，并新增 `language detection` 一例把"跟随浏览器"固化为受测行为 |
| F2 | Radix 单选 ToggleGroup 在真实浏览器暴露为 **radiogroup/radio**，而 jsdom 里的单测按 `button` 断言通过 | 测试环境差异（jsdom 与浏览器可访问名实现不同） | E2E 以浏览器为准用 radio 断言；单测保持现状（两组件库行为已在组件级单测覆盖） |
| F3 | 仓库页存在**两个 h1**（仓库名 + 页面名） | P2 可访问性改进项（不阻塞 M0） | 记录：M1 重构仓库页时降为 h1+h2 |
| F4 | `ansi-regex@5.0.0` 已知 ReDoS（high） | 安全（DoD 第 6 项） | `pnpm-workspace.yaml` overrides 固定到 5.0.1，audit 清零；jest-dom/pretty-format 升级后可移除 |
| F5 | E2E 断言三次纠正均为**测试自身与真实文案/可访问名不一致**（禁用原因文案、区域入口默认页标题、菜单命名模式），应用行为无缺陷 | 测试校准 | 修正断言；菜单命名（由触发器派生，符合 ARIA 菜单模式）已在源码注释中说明 |

## 说明

- E2E 用例名使用英文：Playwright 规格经脚本管道落盘（规避编码风险），且与 Rust 测试命名约定一致；
  Vitest 用例名仍为中文（见 CODING_STYLE §3.6）。
- 悬停（hover）路径：M0 的交互元素全部有 hover 样式但无 hover 行为（无 tooltip 行为逻辑），
  因此命中测试即等价于悬停可用性；带行为的悬停（如提交图 tooltip）从 M2 起纳入本验收。
