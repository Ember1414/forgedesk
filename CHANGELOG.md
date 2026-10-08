# 更新日志

本项目的版本号遵循 [Semantic Versioning](https://semver.org/lang/zh-CN/)；
格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)。

> 说明：M0–M6 在开发期未逐一打 tag，下方日期对应各里程碑的**验收日期**
> （见 `docs/acceptance/`）。正式对外分发从 M7（v1.0）开始。

## [未发布]

### 新增

- 官网升级：首页主视觉（纯 CSS/SVG 界面示意 + 「三步开始」+ 真实数字条）、下载页顶部推荐卡与
  GPG 指纹复制；导航毛玻璃、键盘焦点可见性、尊重系统「减少动态效果」
- **GPG 公钥随站发布**：`build-site.mjs` 把 `docs/keys/forgedesk-release.pub` 复制为站点路径
  `updates/gpg-pubkey.asc`；下载页的 GPG 公钥入口由此从"探测必失败、永远隐藏"变为可用。
  `check:site` 增加三条断言（副本存在 / 与真相源逐字节一致 / 源缺失时不留旧副本）
- 自动更新的**「篡改包被拒」自动化测试**（M7 验收第 2 条）：与 `tauri-plugin-updater` 相同的
  验签库与调用方式，覆盖"原样通过 / 改一字节拒绝 / 长度变化拒绝 / 非签发者公钥拒绝"

### 文档

- `docs/RELEASE.md` §3.1：写明公钥副本是**构建时生成**，换公钥只改 `docs/keys/` 一处
- `docs/acceptance/M7.md`：按 v1.0.0 实况复检（8 通过 / 2 部分通过 / 0 未执行）

## [1.0.0] - 2026-10-07

**首个对外正式版**。M7 里程碑：自动更新 / CI / 多平台打包 / 文档 / 首个正式发布。
范围决策：**Windows 优先**；macOS 与 Linux 用户少、真机验证成本高，暂缓（保留代码与 CI 骨架）。
详细计划见 `docs/M7-PLAN.md`，逐条验收结论见 `docs/acceptance/M7.md`。

### 新增

- 自动更新：`update_check` / `update_install` 命令（签名校验由 `tauri-plugin-updater` 执行，**不提供跳过校验的降级路径**；未配置更新源时返回 `configured: false` 而不是报错）、状态栏更新横幅（含"跳过该版本"）、设置页渠道（stable / beta）与自动检查开关
- 崩溃恢复与安全模式：上次异常退出提示（残留会话标记，标记损坏也算异常）、一键"以安全模式重启"（禁用插件与终端）、安全模式请求为**一次性**语义
- 操作历史导出：CSV / JSON，导出前弹系统保存对话框选择位置（取消则不写任何文件）；路径必须绝对、扩展名须与格式一致；CSV 带 UTF-8 BOM 供 Excel 直接打开
- 隐私说明页（设置 → 隐私）：与 `docs/PRIVACY.md` 逐条一致的数据清单与对外域名
- 官网落地页（`site/`）与 Cloudflare Pages 部署工作流（未配置凭据时优雅跳过而非失败）

### 变更

- 版本号由 `0.0.1` 提升到 `0.7.0`（`package.json` / `Cargo.toml` / `tauri.conf.json` 三处同步），并补齐 M1–M6 的更新日志
- 界面修正：侧栏与命令面板的「插件」入口此前指向未实现占位页，现指向真实插件管理页
- 界面修正：清理"命令面板与全局搜索将在 M4 实现""在编辑器打开（M5 可用）""将在 T2.8 接通"等未随里程碑更新的过期文案
- 移除「设置 → Git」占位页（PLAN 未规划，且标注的任务号有误）
- 宿主能力按需收紧：新增 `dialog:allow-save`（导出另存为所需），消息框等其余 dialog 子权限仍未授予

### 安全

- 32 处 GitHub Actions 引用固定到 commit SHA（保留 `# v7` 形式的可读标签），`compliance.yml` 补 `concurrency`；工作流校验由 33 警告降到 **0 警告**
- 修复 2 处高危依赖（均在 dev 链路、不进产物）：`sharp` 提升到 `^0.35.5`（CVE-2026-96889 / librsvg）、`source-map-js` 经 `pnpm-workspace.yaml` 的 `overrides` 钉到 `1.2.2`（GHSA-68fv-2mgg-jv7q）；`pnpm audit` 归零
- CI 的 cargo 构建与测试步骤一律加 `--locked`：CI 必须跑在提交进仓库的 `Cargo.lock` 上，锁文件过期即失败
- 文档脱敏：`docs/adr/ADR-004` 中回显的个人邮箱已改为描述（转公开前的 A 组审计发现）

### 文档

- `README.md` 按 M1–M6 实况重写（此前仍宣称"Git 核心尚未实现"）
- 新增：`docs/install/windows.md`、用户手册 `docs/manual/`（五篇）、`docs/FAQ.md`（24 条）、`docs/TROUBLESHOOTING.md`（21 条）、`docs/PRIVACY.md`、`docs/RELEASE.md`、`docs/SIGNING.md`、`docs/M7-PLAN.md`、`docs/acceptance/M7.md`
- 社区文件：`CONTRIBUTING.md`、`CODE_OF_CONDUCT.md`、`SECURITY.md`、`GOVERNANCE.md`、Issue 表单九件、PR 模板、讨论区模板四件

### 已知待办（v1.0.0 之后）

- 发布演练与回滚演练：用已发布版本实测「检测新版本 → 下载 → 签名校验 → 重启为最新版」，以及「篡改包被拒」（见 `docs/acceptance/M7.md` §4·G）
- Windows 安装包在干净环境的实机验证：安装 → 打开仓库 → 提交 → push，以及便携版 zip 解压可启动
- macOS / Linux 的安装与更新真机验证（ADR-005，推迟到 M8）

## [0.6.0] - 2026-10-06

M6 里程碑：插件系统 / 主题 / 多平台适配 / i18n。

### 新增

- 插件运行时：wasmi 沙箱引擎、封闭权限白名单、fuel 与内存上限、独立栈限制；越权调用返回结构化拒绝且宿主不崩溃
- 插件崩溃隔离：trap / 超时 / 内存超限即隔离，其他插件与宿主照常，每实例保留环形日志供排查
- 插件管理器：安装 / 逐项授权（危险权限二次确认）/ 启用禁用 / 卸载 / 热重载 / SHA256 校验，含开发者模式与权限用量统计
- 示例插件与 SDK：commit-template / repo-stats / repo-audit 三个示例，配套 plugin-sdk、面板 DSL 与提交钩子扩展点
- 主题系统：33 token 白名单校验、导入导出、非法主题回退默认；新增两套原创主题（Sandstone Dawn / Pine Nocturne），全部通过 WCAG AA 对比度
- 网络与密钥面板：代理设置（同时对 fetch/push 与托管 API 生效、支持 no_proxy 覆盖）、连通性测试、SSH 与 GPG 管理
- 平台适配层：路径规范化（大小写 / UNC / NFC）、shell 探测、三平台开机自启与「在文件管理器显示」、平台环境检查
- 提交签名与验证状态展示（Verified 徽章）

### 变更

- i18n 全覆盖：源文件硬编码文案 lint + 中英 key 对齐校验 + 术语表，窗口标题跟随语言

## [0.5.0] - 2026-10-06

M5 里程碑：终端 / 命令解释 / 错误诊断 / 编辑器增强。

### 新增

- 内嵌终端：xterm.js 多标签、交互式命令、中文与 emoji 不乱码、退出事件、关闭仓库时拦截活跃终端
- 危险命令拦截：`git reset --hard` 等给出非阻塞提示条与「转到图形安全操作」，执行后补打快照
- 本地 Git 命令解释器 / 词典（81 条）：解释命令含义，`git rebase -i` 引导走图形界面
- 错误诊断引擎（51 条规则）：解析 stderr 给出原因与可点击修复动作，含诊断历史
- 文件树 + 懒加载 Monaco 编辑器：外部变更三选一（重载 / 保留 / 并排对比）、保留 EOL 与 BOM、blame 与文件历史
- 命令面板与可自定义快捷键（43 条命令，冲突检测并阻止保存，可恢复默认）
- 布局系统：面板预设、持久化，损坏 JSON 回退默认

## [0.4.0] - 2026-10-05

M4 里程碑：代码托管集成（GitHub）。

### 新增

- 账号登录：OAuth Device Flow 与 PAT，凭据入系统 keyring、重启保持，多账号并存并可绑定仓库
- 令牌过期 / 被撤销时明确提示并重新登录，不出现无限 401 重试
- 仓库面板：我的 / 星标 / 组织 / 搜索、Clone（可选择账号）、Fork、Star、Watch、README 安全渲染（防 XSS）
- 拉取请求：列表筛选 / 详情 / 时间线 / 行内评论（定位到行，越界本地拒绝）/ 提交 Review / 合并（merge、squash、rebase + 删除源分支选项）
- 议题：列表 / 筛选 / 创建 / 编辑 / 评论 / 关闭 / 指派
- 流水线（Actions）：workflow、运行记录、状态、日志流式加载、重跑与取消
- 限流处理：显示剩余额度与重置时间，自动降级为缓存数据（ETag）

### 安全

- 未登录时给出友好引导；错误详情不泄露令牌（脱敏）

## [0.3.0] - 2026-10-01

M3 里程碑：冲突解决 / Rebase 可视化 / 快照回滚（差异化核心）。

### 新增

- 三栏冲突解决向导：逐块「采用本地 / 采用远端 / 两者保留 / 手动编辑」，冲突块卡片流，二进制冲突提供专门处理路径
- 冲突状态机覆盖 merge / rebase / cherry-pick / revert，支持 continue / abort / skip，中止前自动打快照
- 拖拽式交互 rebase：拖拽排序 + reword / squash / fixup / drop / edit，实时预览新历史，逐步执行并在冲突时跳转向导
- 合并可视化：执行前预览合并计划，进行中显示状态横幅
- 快照 v2：未跟踪文件内容备份、磁盘配额与按仓库锁；快照页展示用量、手动快照、清理与还原报告
- 回滚校验：仓库指纹校验、分阶段还原、紧急指引与崩溃恢复提示
- 操作历史页：时间线 + 筛选 + 一键回滚，状态栏常驻当前回滚点

### 安全

- 破坏性操作安全测试矩阵：reset / rebase / clean / checkout -f / stash drop 等场景全部验证「可回滚」

## [0.2.0] - 2026-09-29

M2 里程碑：历史 DAG / 分支 / 远端同步。

### 新增

- 提交历史 DAG 图：Canvas 绘制、分支泳道、命中检测、列表模式与性能面板；同分支刷新后同色不抖动
- 提交详情面板：双父 diff 与固定对比；历史筛选与关键词搜索跳转，可拖拽迷你地图
- 分支与标签管理：创建 / 切换 / 重命名 / 删除 / 跟踪设置
- 远端管理 + fetch / pull / push 编排与同步条；push 被拒走 `--force-with-lease`（先 fetch 再确认），**不提供裸 `--force`**
- pull 产生冲突时正确进入冲突状态并跳转冲突向导
- stash 面板、cherry-pick、revert、reset（含计划预览）、reflog 恢复
- 凭据体系：keyring 存储 + askpass 注入 + SSH 密钥清单 + 加密保险库回退
- 大仓库性能模式：自动降级字符级 diff、压缩上下文、减半页大小

## [0.1.0] - 2026-09-27

M1 里程碑：Git 核心闭环（打开 → 状态 → diff → 暂存 → 提交）。

### 新增

- 仓库打开 / 克隆 / 初始化与「最近仓库」列表；非仓库路径给出明确引导
- 工作区状态面板：已暂存 / 未暂存 / 未跟踪分组、树与平铺视图、批量操作、万行虚拟列表
- diff 查看器：统一与并排模式、hunk 折叠、虚拟滚动、复制与导出
- 行级 / 块级暂存与取消暂存（结果与 `git diff --cached` 逐字节一致）
- 两段式提交：提交预览 + 等价 git 命令 + 索引指纹防串改；amend、钩子清单，hook 拒绝时展示原始输出并可「跳过钩子重试」
- 「放弃修改」预览与二次确认；提交前自动快照 + 一键回滚
- 文件监听自动刷新；写操作审计记录（脱敏、导出、保留策略）

## [0.0.1] - 2026-09-23

M0 里程碑：工程地基与首个可打包的空壳应用。此版本仅供开发与验收，不对外分发。

### 新增

- 工程骨架：Tauri 2 + React 19 + TypeScript + Vite + Tailwind 4 前端，Rust workspace（12 个分层 crate）；`pnpm tauri dev` 可直接启动桌面窗口
- 应用外壳：顶栏（仓库切换器 / 全局搜索 / 账号 / 更新位）、可折叠侧栏、状态栏，20 条路由覆盖仪表盘、仓库五页、代码托管、插件与设置；全部页面文案走 i18n（中 / 英）
- 设计系统：语义 token（色彩 / 字号 / 间距 / 圆角）+ 明暗主题 + WCAG AA 对比度校验，29 个基于 Radix 原语的基础组件与组件展示页
- 统一错误模型：稳定错误码、脱敏详情、可执行修复动作与统一 Toast 呈现
- 本地存储：SQLite（bundled）+ 版本化迁移（迁移前备份、可回滚）+ 设置持久化
- 日志系统：JSON 文件日志（写入前脱敏）、按天与 10MB 双阈值轮转、保留 7 天、崩溃留档（panic 日志）与会话标记；设置页可查看日志目录与最近日志
- 工程门禁：lint / i18n / 类型 / 单测 / 对比度 / workflow / 仓库一致性 / 文档链接 / 合规红线（名称、免责声明、图标、依赖许可、AI 依赖）共 12 项，全部接入本地与 CI
- CI：Linux 质量门禁随每次推送；三平台打包矩阵在打 tag 时触发，产物按 `ForgeDesk_版本_平台_架构` 归一化命名并附 SHA256SUMS
- 文档：ARCHITECTURE / API / CODING_STYLE / CONTRIBUTING / DEV-ENV / README（含免责声明）

### 安全

- 令牌 / 密码 / 私钥写入日志前统一脱敏（写入层强制，含 JSON 字段与 panic 报告）
- 固定已知的传递依赖漏洞：ansi-regex@<5.0.1 升至 5.0.1（GHSA-93q8-gq69-wqmw）

[1.0.0]: https://github.com/Ember1414/forgedesk/releases/tag/v1.0.0
[0.6.0]: https://github.com/Ember1414/forgedesk/releases/tag/v0.6.0
[0.5.0]: https://github.com/Ember1414/forgedesk/releases/tag/v0.5.0
[0.4.0]: https://github.com/Ember1414/forgedesk/releases/tag/v0.4.0
[0.3.0]: https://github.com/Ember1414/forgedesk/releases/tag/v0.3.0
[0.2.0]: https://github.com/Ember1414/forgedesk/releases/tag/v0.2.0
[0.1.0]: https://github.com/Ember1414/forgedesk/releases/tag/v0.1.0
[0.0.1]: https://github.com/Ember1414/forgedesk/releases/tag/v0.0.1
