# ForgeDesk 架构、功能与体验审查（2026-11-20）

审查版本：`5848df4`，产品版本 `1.1.4`。以下保留修复前的审查证据；
用户随后授权直接修复，实施结果与最终验证见 [修复记录](./FIXES-2026-11-20.md)。

## 1. 结论与边界

当前最值得优先处理的是：编辑器跨仓库保存、历史图视口约束失效、终端离屏缓冲不受上限约束，以及 HTTP 缓存缺少账号隔离。这些都已有最小复现证据。

已有架构基础可以保留：前后端 IPC 边界集中、Git 读写双引擎、纯领域布局、SQLite WAL、快照与操作审计、后台任务、语义设计 token、国际化，以及工作区/diff 的虚拟列表。主要缺陷集中在模块之间的接线、状态生命周期和缓存身份，而不是缺少这些基础设施。

阅读覆盖了工程约定、计划相关章节、架构/API/编码风格、前端全部功能域入口、Rust workspace 的各层职责与关键实现，以及测试、构建和 CI。重点深入检查了打开仓库、状态事件、提交历史、编辑器、终端、同步任务、账号/HTTP 缓存、设置和布局链路。仓库约有 16 万行前后端及 Rust 代码（包含测试）；本报告不声称每行源码均已人工逐行审读，也不构成所有功能无缺陷的保证。

运行验证使用 Windows + Edge 无头浏览器、真实前端组件、内存 IPC mock，以及链接仓库真实 Rust 库的本地探针。没有对真实用户仓库执行写入或远程操作。macOS/Linux WebView、真实托管平台账号、真实桌面冷启动和交互 p95 尚未测量。

## 2. 架构理解

| 层 | 实际职责与主要入口 | 观察 |
| --- | --- | --- |
| 应用外壳 | `src/app`、`RepoLayout`：路由、导航、仓库上下文、详情位 | 编辑器按路由延迟加载；大多数其余页面仍静态导入 |
| 功能界面 | `src/features`：工作区、提交、历史、分支、冲突、终端、编辑器、托管平台、设置等 | 本地 Git 主要用 Query；托管平台列表部分自己管理请求状态 |
| 前端共享层 | `src/lib/ipc`、查询键、仓库变化失效器、Zustand stores、UI primitives | 接口集中，但缓存键、UI 状态和生命周期仍存在分叉 |
| 桌面宿主 | `src-tauri`：窗口、插件、服务装配、命令注册、退出清理 | 业务客户端的启动配置与设置更新未完全接通 |
| IPC 边界 | `crates/commands`：参数收敛、DTO、事件、任务接线 | 静态 IPC 参数检查通过；不能覆盖生命周期和事件时序 |
| 用例编排 | `crates/services`：仓库、提交/分支/同步、冲突、历史缓存、文件、终端等 | 负责快照与审计编排；历史前缀缓存仍重新从 tip 遍历 |
| 纯领域逻辑 | `crates/domain`：错误、Git 模型、补丁、计划、DAG 布局 | 不依赖 IO，性质测试与独立 DAG 断言较完整 |
| 基础设施 | `git-engine`、`snapshot`、`storage`、`provider`、`credentials`、`jobs`、`platform`、`diagnostics`、`plugin-host` | 读 libgit2、写系统 Git；原生监听；WASM 插件限额；错误/日志脱敏 |

需要维持的核心数据流是：UI → 具名 IPC → command → service → domain/infra → 事件 → Query 失效。编辑器/终端的会话内容则需要独立于页面挂载生命周期持有。

## 3. 有复现证据的问题

P1 表示应优先修复的功能、数据正确性或明显资源问题；P2 表示条件性错误和体验问题。以下优先级针对本次已验证的触发条件，不将条件性问题扩大为无条件故障。

### F01 · P1：编辑器标签跨仓库共享，保存目标会跟随当前路由改变

- 位置：[editorStore.ts](../src/stores/editorStore.ts)、[EditorPanel.tsx](../src/features/editor/EditorPanel.tsx)、[RepoEditorPage.tsx](../src/features/editor/RepoEditorPage.tsx)。
- 原因：标签只有相对 `path`，没有 `repoId`；Monaco 也以相对路径作为 model URI。页面给全部旧标签传入当前路由的 `repoId`。
- 复现：在仓库 1 打开并修改 `src/main.rs`，切到仓库 2 编辑器后点击保存。捕获到 `fs_write({ request: { repoId: 2, path: 'src/main.rs', content: 'repository ONE edited content\n' } })`。
- 影响：旧仓库内容可被写到新仓库的同名文件。这是本轮最高优先级之一。
- 建议：以 `(repoId, path)` 标识标签、草稿及 Monaco URI；保存从标签读取仓库身份。文件异步打开完成时也要核对原始仓库上下文。

### F02 · P1：历史图容器高度失控，使虚拟化实际退化为全量渲染

- 位置：[HistoryPage.tsx](../src/features/history/HistoryPage.tsx)、[GraphCanvas.tsx](../src/features/history/GraphCanvas.tsx)、[GraphOverlay.tsx](../src/features/history/GraphOverlay.tsx)。
- 原因：页面 `min-h-full` 与图容器 `min-h-[22rem] flex-1` 没有建立确定的可用高度；`h-full` 不能从这条祖先链得到固定视口。ResizeObserver 随后把全内容高度当作视口高度，Canvas 和 DOM 行窗口都随之放大。
- 复现：1280×800 浏览器中，200 行使 overlay/listbox 高度达到 5600px，200 个 option 全部挂载。5 万行时高度为 1,400,000px，5 万个 option 全部挂载。
- 根因对照：仅在浏览器中将图的父容器限定为 360px 后，相同 200 行只挂载 20 个 option。没有更改应用源码。
- 影响：大仓库内存、布局和脚本成本显著增加；滚动发生在外层，图内的分页哨兵、搜索定位和键盘滚动也可能失去预期作用。超大 Canvas 还可能超过浏览器尺寸限制，但本次没有将其列为已验证的绘制故障。
- 建议：从仓库内容出口到历史图建立连续的 `flex-1 min-h-0` 高度约束，给图一个真实视口；较矮窗口采用明确布局策略。验收应断言 DOM 行数和 Canvas 后备尺寸随视口而非提交总数变化。

### F03 · P1：终端离屏输出缓冲的 1 MiB 上限没有生效

- 位置：[manager.ts](../src/features/terminal/manager.ts) 的 `deliverOutput`。
- 原因：入队后写回的是 `queued`，没有增加本次 `bytes.length`。常规小块输出会使计数持续为 0，截断条件不成立。
- 复现：没有终端视图时推送 40 个 64 KiB 输出块，再注册接收实例。收到 2,621,440 字节，超过声明的 1,048,576 字节上限，且没有截断标记。
- 影响：离开终端页后，持续输出的任务可能无限积累前端内存；重新进入时集中回放也会造成卡顿。
- 建议：修正入队/出队字节记账，处理单块超过上限的情况；关闭会话后清理 pending，并添加多块输出超限回归测试。

### F04 · P1：HTTP 离线缓存没有隔离认证身份

- 位置：[client.rs](../crates/provider/src/client.rs)、[etag_cache.rs](../crates/provider/src/etag_cache.rs)、[host_repos.rs](../crates/services/src/host_repos.rs)。
- 原因：缓存键只有 URL 和查询参数；多账号请求共享克隆的 HTTP 底座。认证身份和影响表示的请求头没有参与缓存分区。
- 复现：本地 HTTP 服务以合成账号 A 返回带 ETag 的 `{"owner":"synthetic-a"}`。关闭该服务后，以合成账号 B 请求相同 URL，真实 `GitHubHttp` 返回 A 的缓存内容。
- 影响：切换账号、改变仓库绑定账号、离线或请求失败时，可能展示另一账号上次读取的数据。没有使用真实凭据。
- 建议：缓存按 host、账号标识/凭据代次和表示类型分区；退出账号或撤销凭据时清除对应分区。不得将原始 token 写入缓存键日志。

### F05 · P1：历史查询签名漏掉四类筛选，导致结果错误复用

- 位置：[useGraphQuery.ts](../src/features/history/useGraphQuery.ts) 的 `SIGNATURE_FIELDS` / `filtersSignature`。
- 复现：`revisions=['main']` 与 `['feature']`、`mergesOnly` 开关、`myCommitsOnly` 开关、相同关键词的 `caseInsensitive` 开关，分别得到完全相同的签名。
- 影响：这些控件修改 URL 后仍可能读取旧缓存，且游标分桶也复用旧筛选结果。旧结果新鲜时尤其明显，不能依靠稍后过期掩盖。
- 建议：签名覆盖全部后端查询语义；对每个 `HistoryQuery` 筛选字段建立差异测试，并明确数组的规范化规则。

### F06 · P1：编辑器离开页面时静默丢失未保存内容

- 位置：[EditorPanel.tsx](../src/features/editor/EditorPanel.tsx) 的 `EditorTabView`、[editorStore.ts](../src/stores/editorStore.ts)。
- 原因：新内容放在组件局部 `model`，store 只保留上次保存基线。离开路由导致组件卸载，重新进入以基线重建 model，没有未保存确认。
- 复现：输入 `UNSAVED content to retain`，切到工作区再返回，内容退回上次保存的 `repository ONE edited content`。
- 建议：将文档草稿和 Monaco model 生命周期提升到仓库文档会话层；添加内存脏状态、关闭/切仓确认和明确的恢复行为。仅保留标签元信息不足以保留编辑会话。

### F07 · P2：ETag 304 回放丢失分页 Link 头

- 位置：[etag_cache.rs](../crates/provider/src/etag_cache.rs) 的 `CacheEntry`、[client.rs](../crates/provider/src/client.rs) 的 `stale_fallback`。
- 原因：缓存保存 body、ETag、Content-Type；回放只重建 Content-Type 与缓存标记。仓库/PR/Issue 页却依赖 `Link` 判断下一页。
- 复现：真实 HTTP 底座第一次收到 200 + `Link: <...page=2>; rel="next"`，第二次收到 304；第二次上层响应的 Link 为 `None`。
- 影响：内容没变，刷新后“下一页”反而可能消失；离线/限流回放同样受影响。
- 建议：缓存并恢复必要响应元数据；区分“304 验证成功”与“过期回退”的新鲜度标记。

### F08 · P2：仓库事件节流会吞掉类别与路径

- 位置：[repoChanged.ts](../src/lib/repoChanged.ts) 的 `createRepoChangeInvalidator`。
- 原因：节流窗口中每次事件替换上次待处理的 kind 和 paths，没有求并集。
- 复现：先处理 workspace，再在同一窗口发送 refs，最后发送另一次 workspace。尾随调用只处理 workspace，没有任何历史 log 失效请求。
- 影响：提交/切分支同时发生文件变化时，历史、分支、ahead/behind 或先前文件的 diff 可能停留旧状态。staleTime 到期本身不会让已挂载查询自动重新读取。
- 建议：按仓库累计类别和路径集合，`large` 覆盖所有类别；用事件序列测试验证最终状态。

### F09 · P2：Git 子进程的 stdin 写入阶段不受超时与取消保护

- 位置：[process.rs](../crates/git-engine/src/process.rs) 的 `GitProcess::run`。
- 原因：先 `write_stdin(...).await`，再进入超时/取消/退出的 `tokio::select!`。
- 复现：用 `GitProcess::with_program` 注入不读 stdin、1.5 秒后退出的 Node 子进程；传入 8 MiB stdin，超时设置 100ms。实际等待约 1568ms，并返回成功 `Ok(Some(0))`。
- 影响：补丁等大输入遇到不读取管道的进程或 hook 时，取消/超时不按承诺生效。这里验证的是进程执行器，而非断言每次 Git 写入都会触发。
- 建议：将 stdin 写入与进程等待放在同一个取消/截止时间范围，退出时统一回收管道任务。

### F10 · P2：布局状态分裂、预设空转，底部高度恢复错误

- 位置：[layoutStore.ts](../src/stores/layoutStore.ts)、[uiStore.ts](../src/stores/uiStore.ts)、[RepoLayout.tsx](../src/features/repo/RepoLayout.tsx)、[LayoutSettingsPage.tsx](../src/features/settings/LayoutSettingsPage.tsx)。
- 复现 1：通过 UI store 将面板设为 hidden，持久化 layout 的位置仍为 right；hydrate bottom 后，实际 UI 仍是 hidden。
- 复现 2：解析 bottom + 112px，结果变成 240px。`parseLayout` 固定用 horizontal 的 240..640 边界，而底部应使用 vertical 的 96..480。
- 静态确认：preset 只被设置页展示，`setPreset` 只改名称，没有应用审查/编辑器预设的尺寸与位置。
- 建议：面板位置只有一个来源；按位置解析尺寸，最好分别保存左右宽度与底部高度；预设应应用一组完整布局值并验证实际 DOM。

### F11 · P2：终端切页后此前已显示的输出消失

- 位置：[TerminalView.tsx](../src/features/terminal/TerminalView.tsx)、[manager.ts](../src/features/terminal/manager.ts)。
- 原因：卸载时销毁 xterm；pending 只缓冲卸载之后到达的输出，既有缓冲没有保存或重放。后端虽提供 `term_output_tail`，此链路没有使用。
- 复现：终端先显示 `VISIBLE_OUTPUT_BEFORE_NAVIGATION`，切到编辑器再返回，终端可见文本只剩空行。
- 建议：保留按会话管理的终端实例/缓冲，或以带序号的后端输出重建；重建时避免丢输出与重复回放。

### F12 · P2：编辑器保存失败成为未捕获 Promise 拒绝

- 位置：[EditorPanel.tsx](../src/features/editor/EditorPanel.tsx) 的 `save` 与保存按钮。
- 原因：`save` 只有 try/finally，按钮 `void save()` 没有 catch 或统一错误出口。
- 复现：mock `fs_write` 拒绝为 `PERMISSION_DENIED`。保存前浏览器 pageerror 为空，保存后新增未捕获错误。
- 建议：接入 `useAppError`，失败时保留草稿、给出明确反馈；外部修改未处理时也应明确保存行为。

### F13 · P2：较窄窗口里详情面板挤掉编辑器，内容被裁切

- 位置：[RepoLayout.tsx](../src/features/repo/RepoLayout.tsx)、[RepoEditorPage.tsx](../src/features/editor/RepoEditorPage.tsx)、[resizable.tsx](../src/ui/components/resizable.tsx)。
- 复现：1024×768，右侧面板调到允许的 640px，文件树保持 264px；截图显示编辑器只剩很窄区域，大块空间用于空详情。编辑器测得的右边界进入详情区域，外层将其裁切。
- 原因：详情/树宽只按固定常量钳制，没有按容器可用宽度和主编辑区最小宽度约束；详情在全部仓库页默认保留。
- 建议：根据实际内容宽度约束面板上限；主编辑区设最小可用宽度；编辑器/终端页采用合适的详情默认策略。空详情不应长期占据关键工作空间。

## 4. 静态链路已确认、尚未做真实宿主端到端复现的问题

### F14 · P1：代理设置与正常业务客户端未接通

- 位置：[settings.rs](../crates/commands/src/settings.rs)、[network.rs](../crates/commands/src/network.rs)、[main.rs](../src-tauri/src/main.rs)、[NetworkSettingsPage.tsx](../src/features/settings/NetworkSettingsPage.tsx)。
- `settings_set` 只对 watch 类设置应用副作用；`apply_proxy_settings` 在仓库中没有调用点；宿主用 `HttpConfig::default()` 创建共享 provider。Git 引擎同样没有在此设置路径收到配置更新。
- 测试按钮会临时读取当前设置创建客户端，所以可能出现“测试通过、实际业务仍失败”。重启也仍从默认 provider 配置装配。
- 直连模式把 `Some("")` 当成禁用代理，但真实 `GitHubHttp::new` 会将其作为 URL 解析并返回错误，已通过原生探针确认。`noProxy` 读取后被 `_no_proxy` 忽略。
- URL 草稿仅在初次渲染从尚未完成的查询初始化，随后没有同步，重新打开设置页可能显示空输入。
- 建议：让代理配置进入唯一客户端装配/更新路径；明确“无代理”与“使用环境”的类型语义；支持 bypass 列表；完成加载后初始化草稿并保护用户正在编辑的值。

### F15 · P2：仓库事件失效订阅依赖当前页面，部分页面不能持续刷新

- 位置：[repoChanged.ts](../src/lib/repoChanged.ts)、[RepoLayout.tsx](../src/features/repo/RepoLayout.tsx)、[RepoConflictPage.tsx](../src/features/repo/RepoConflictPage.tsx)、[FileTreePanel.tsx](../src/features/editor/FileTreePanel.tsx)。
- 统一失效 hook 分散挂在工作区、提交、历史、分支等页面，公共 RepoLayout 没有订阅；冲突、编辑器、仓库设置等页面没有同等失效通路。编辑器的外部文件监听仅处理当前文档内容，并不刷新 Query 或文件树。
- 全局又关闭了窗口聚焦刷新，没有统一轮询兜底。因此外部终端修改仓库时，这些页面和常驻同步/合并信息可能长期不更新。
- 建议：每个打开仓库在公共层只保留一个失效订阅，页面只提供额外查询需求；文件树按目录变化失效，并覆盖 watcher 失效的降级行为。

### F16 · P2：Actions 日志重开/切换目标时沿用上一份日志

- 位置：[ActionsLogDialog.tsx](../src/features/github/ActionsLogDialog.tsx)。
- `lines` 和 `phase` 在组件创建时初始化；target effect 切换只退订旧事件，没有清空 lines、重置 phase，也没有取消旧下载任务。后续新块持续 append 到旧数组。
- 建议：以 run/job 身份隔离日志会话，切目标显式重置；关闭时取消任务。大日志还应使用分块存储、批量刷新和内存上限，避免每个事件复制累计数组。

## 5. 流畅度、布局与性能改进顺序

| 顺序 | 改进 | 原因及验收依据 |
| --- | --- | --- |
| 1 | 先修 F02，再评估 Canvas 绘制优化 | 当前真实视口已失控，继续优化描边算法收益有限。200/50k/100k 三档 DOM 行数应保持在视口行数 + overscan 附近 |
| 2 | 保护编辑器/终端会话生命周期 | 切页不丢草稿、不清输出、跨仓库不串文档；缓冲超过上限必须裁剪 |
| 3 | 修缓存键、账号隔离和事件累计 | 筛选正确、刷新可靠之后，才能减少重复请求；避免用过度刷新掩盖一致性问题 |
| 4 | 约束历史已观察页数、隔离每仓库游标 | `useQueries` 观察所有已累积页，活跃页不会按 gcTime 淘汰；refs 失效会重取全部这些页。分页 store 又只按筛选签名分桶，没有 repoId，也没有调用 resetAll 的生产入口 |
| 5 | 历史模型增量合并 | 5 万条模型一次合并约 32ms；按 200 行逐页累计反复合并约 3.3s（250 次合计，非单帧）。这是合成数据、当前机器的观察，不是桌面 p95；应减少全量排序、建索引与重复结构复制 |
| 6 | 限制后端深分页重复遍历 | `page_via_cache` 扩展前缀仍调用 `log(skip, limit)`；libgit2 从 tip 重新 revwalk。完整顺序翻页累计成本会放大，跨缓存上限后又回直连；优先测真实分支/筛选场景再选择增量 walker 或批量读取 |
| 7 | 路由拆包 | Windows 生产构建首屏 JS 约 1.93MB（gzip 555KB），编辑器延迟块约 2.30MB（gzip 591KB）。终端、托管平台、插件等仍静态进入路由依赖；按功能延迟加载并测本地 WebView 解析/执行成本 |
| 8 | 减少布局固定成本 | 左侧导航与仓库十个 tabs 重复；较小窗口会换行，加上同步条、提示条、空详情挤占内容。保留主任务入口，将低频项收纳；长文件路径与工具栏应截断/折叠并保留可访问名称 |
| 9 | 拆开危险操作的视觉权重 | 历史页上方默认呈现三个 reset 模式，查阅历史的主要空间被低频操作占用。可收进选中提交后的操作区，预览/快照/回滚的安全流程继续保留 |
| 10 | 补性能的有效测量 | 现有 graph bench 在发送 wheel 后才调用 countFrames，因此 `fps-scroll` 实际统计后续静止窗口；同时一次 mock 返回 5 万行不覆盖真实分页链路。应在同一采样窗口持续滚动，记录帧耗时分位数、长任务、DOM 数、主/渲染进程内存和取消延迟 |

另一个功能体验缺口是顶部 [GlobalSearch](../src/app/shell/GlobalSearch.tsx)：目前输入只更新局部 state 和提示，尚未执行仓库/提交/命令搜索。建议先把入口接入已有命令面板并明确搜索范围；新增索引能力如超出计划范围，再按仓库规则单独审批。

## 6. 验证证据与现有测试盲区

| 检查 | 本次结果 |
| --- | --- |
| `pnpm lint`、`pnpm i18n:lint`、`pnpm i18n:check`、`pnpm typecheck` | 通过 |
| `pnpm test` | 首次默认并发：928 通过、1 个等待元素超时；该文件单独复跑 11/11 通过；完整限制并发重跑 `pnpm test --maxWorkers=2`：92 文件、929 测试通过 |
| `pnpm format:check`、`pnpm check:contrast`、`pnpm check:workflows`、`pnpm check:repo` | 业务代码基线检查通过；报告新增后另行格式/文档复检 |
| `pnpm check:docs`、`pnpm check:site`、`pnpm check:ipc`、`pnpm compliance` | 基线通过；报告新增后文档检查复跑 |
| `cargo fmt --all -- --check` | 通过 |
| `cargo clippy --all-targets --all-features -- -D warnings` | 构建前端 dist 后通过 |
| `cargo test --workspace --all-features` | 通过，57 个测试结果组，1324 passed，8 ignored（含文档测试结果） |
| Windows 前端生产构建 | `TAURI_ENV_PLATFORM=windows pnpm build` 通过；有大 chunk 和无效动态导入提示 |
| 现有 E2E：layout/history/editor 三文件 | 27 项全部失败；主要入口被 `TitleBar` 的 null 更新结果击穿，无法把这些失败直接归因到目标功能 |
| 本次自建浏览器探针 | 使用真实页面/Monaco/xterm，修正 mock 请求包装并提供合法 update DTO，复现文档/终端/布局/历史问题；正常动作没有 pageerror，故障注入保存失败时出现 pageerror |
| 本次原生探针 | 调用真实 Rust HTTP/进程执行器，仅访问本地服务与合成子进程，验证 ETag、认证分区、直连配置和 stdin 超时 |

Rust 首次执行遇到缺 dist 和高并发编译器内部错误；补齐前端构建、设置 `CARGO_BUILD_JOBS=2` 重跑通过。没有将环境/资源问题记作产品 Bug。

E2E 的具体盲区：旧 mock 对新增 `update_check` 默认返回 null，`TitleBar` 只判断 undefined，读取 `configured` 时整个路由报错；编辑器 mock 还按平铺 payload 读取字段，与现在的 `{ request: ... }` 契约不符。应建立共享且按契约校验的 mock，未知命令显式报错，所有用例统一采集 pageerror/unhandledrejection 和断言 `window.__errs`。当前 E2E 工作流仅手动触发，问题可能长期不暴露。

单测全绿没有覆盖以下组合：真实 CSS 高度链、页面卸载后的草稿/终端内容、不同账号的同 URL 缓存、ETag 二次读取与分页、混合类别事件突发、stdin 反压与取消。这些应成为修复时的回归重点。

本次原始证据与可复跑探针保存在工作区 `target/review-20261120/`（被 Git 忽略，不作为发布内容）：`probe-results.json`、`graph-results.json`、`graph-results-200.json`、`terminal-results.json`、`native-results.txt`、`validation-summary.json`、截图，以及 `probe.mjs` / `graph_probe.mjs` / `terminal_probe.mjs` / `native_probe.rs`。这些是本机审查证据，清理 target 会移除。

## 7. 建议的修复批次与审批

1. 文档身份与生命周期：F01、F06、F12，同时补跨仓库/切页/关闭/失败回归。
2. 历史视口与数据正确性：F02、F05、F08，随后再做有界分页和增量合并。
3. 终端资源与连续性：F03、F11，测离屏持续输出和会话关闭清理。
4. 网络正确性：F04、F07、F14，验证多账号、断网、304、多页、手动代理与直连。
5. 布局统一与信息层级：F10、F13，覆盖 1024/1280/1440 宽度及不同系统缩放。
6. 进程截止时间、统一事件订阅和日志会话：F09、F15、F16。

以上均可在现有技术栈与已定义功能内修复。初始审查没有执行修复、发布或远程写入；
后续经用户授权实施的修复见文首链接。无需付费、第三方登录、改计划范围、替换核心技术栈或不可逆操作审批。
