# M8 计划：零成本信任加固 / 包管理器分发 / 社区启动

> 起点：**v1.0.0 已发布**（2026-10-08，Windows + macOS universal）。
> 修订记录：2026-10-09 —— 编制本轮工作时的线上版本已是 **v1.1.3**
> （1.1.0–1.1.3 的修复都来自真实用户反馈：更新清单别名 404、检查失败不可见、
> Monaco 装配时序等），T8.1/T8.2 与 T8.3–T8.7 的代理侧交付已按 §3 的状态推进。
> 上游依据：`docs/PLAN.md` §M8（本节按现状修订——原计划写的"GitHub Pages 官网"已由
> `docs/adr/ADR-003` 的 **Cloudflare Pages** 取代）。
> 核心约束不变：**全流程零支出**（不买签名证书、不买商店、不买域名）。

## 1. 目标与判断

M8 要解决的只有一个问题：**让陌生人敢下载、能装上、装完能用**。
无付费证书时的三个抓手：① 把校验做扎实（SHA256 + GPG + 随站公钥）；② 把摩擦写清楚
（去隔离 / SmartScreen 一步步说明）；③ 走包管理器（来源更可信、安装更省事）。

## 2. 现状盘点（2026-10-09 修订）

| 计划项 | 现状 |
| --- | --- |
| D8.4 官网 + 下载页 | ✅ 已上线（Cloudflare Pages，多页站点 + 版本矩阵 + 同源校验和直显 + GPG 公钥随站） |
| D8.5 GPG 签名与校验和自动化 | ✅ `SHA256SUMS` + `SHA256SUMS.asc` 由 `release.yml` 的 `publish` 作业统一生成并签名 |
| D8.2 Windows 信任加固（校验/指引/便携版） | ✅ 便携版 zip 随发布；下载页有三平台校验命令与 SmartScreen 指引；**新增** SignPath 申请材料（[`install/signpath-application.md`](./install/signpath-application.md)） |
| D8.1 macOS 安装指引 + 签名断言 | ✅ [`install/macos.md`](./install/macos.md) 就绪；`release.yml` 增加 ad-hoc 签名断言（T8.1），v1.1.x 的每次 macOS 构建都经过该断言 |
| D8.3 包管理器 | 🟡 清单全部就绪（`packaging/`）：**Scoop 已在本机真实验证安装**；Winget/Homebrew 草稿 + 提交流程齐备；自有 bucket / tap 仓库与第三方 PR 待人类创建/提交 |
| D8.6 / D8.7 / D8.8 社区与推广 | 🟡 代理侧文案就绪（[`COMMUNITY.md`](./COMMUNITY.md)、[`GOOD-FIRST-ISSUES.md`](./GOOD-FIRST-ISSUES.md)、[回复模板库](./templates/replies/README.md)）；分类欢迎贴、Issue 创建、推广素材发布待人类 |
| D8.1+ **应用内首启引导页**（T8.2 关键交付） | ✅ `FirstRunTrustHint` 落地（含 i18n 与测试；设置 → 隐私 提供"重新显示"入口） |
| 清单更新自动化（T8.3 第 7 点） | ✅ `scripts/ci/update-packaging.mjs`（`--check` 可门禁）+ `release.yml` 发布后自动开清单同步 PR（仅自有仓库） |

## 3. 任务分解（按"能否由编码代理独立完成"排序）

| ID | 任务 | 谁做 | 状态 |
| --- | --- | --- | --- |
| T8.1 | **macOS ad-hoc 签名断言**：在 `release.yml` 的 `build-macos` 里加一步 `codesign -dv --verbose=2 ForgeDesk.app`，断言输出含 `Signature=adhoc` | 代理 | ✅ 已落地（v1.1.x 的每次 macOS 构建都经该断言） |
| T8.2 | **安装可信性引导页**（应用内首次启动）：说明未付费签名的事实，给出校验入口与关闭记忆 | 代理 | ✅ 已落地（`FirstRunTrustHint` + i18n + 6 条测试 + 隐私页重置入口） |
| T8.3 | **Scoop manifest**：`scoop install` 本机验证 + 清单更新自动化 | 代理（本机可验证） | ✅ 清单 + **本机实测安装/卸载**（`docs/acceptance/M8.md` §3）+ `update-packaging.mjs` 与发布后自动开 PR；bucket 仓库待人类创建 |
| T8.4 | **Winget manifest 草稿 + PR 流程说明**：`winget-pkgs` 的 PR 需要人类账号与审核 | 代理起草 / 人类提交 | 📋 草稿就绪（`packaging/winget/`），待人类校验并提交 |
| T8.5 | **Homebrew Cask 草稿（自有 tap）**：`brew install --cask` 的验证需要一台 Mac | 代理起草 / 人类验证 | 📋 草稿就绪（`packaging/homebrew/`），真机验证待 Mac |
| T8.6 | **SignPath / 免费 OSS 签名计划**申请材料（`docs/install/signpath-application.md`） | 代理起草 / 人类提交 | ✅ 材料就绪，提交待人类 |
| T8.7 | **15 个 good first issue 候选清单**（含引导说明与标签建议） | 代理 | ✅ `docs/GOOD-FIRST-ISSUES.md`，创建待人类 |
| T8.8 | **社区运营**（Discussions 分类、欢迎贴、响应 SLA、回复模板） | 人类（代理可写文案） | 🟡 文案就绪（`docs/COMMUNITY.md` + 模板库），创建待人类 |
| T8.9 | **推广材料**（演示视频 ×2、技术文章 ×3、文字对比表） | 人类为主 | ⏳ 未开始（需真机演示与个人表达） |
| T8.10 | **升级 + 回滚演练**（M7 遗留的"部分通过"） | 代理准备 / 人类执行发布 | 🟡 升级链路已被 1.1.x 实证；完整演练留档待下次发布 |

## 4. 明确"先不弄"的项（与理由）

按维护者指示：**难以在当前条件下验证的项先不做**，避免为了打勾而堆砌无法验证的产物。

| 项 | 为什么先不做 |
| --- | --- |
| macOS / Linux **真机安装与自动更新实测** | 手上没有 Mac 与 Linux 桌面环境；写了"已验证"就是假话。等有真机或用户反馈再说（`docs/acceptance/M7.md` 已如实标注） |
| Flathub / Snapcraft / AUR | 依赖 Linux 打包链（AppImage/deb/rpm）与各自的真机验证，而 Linux 本身按 ADR-005 暂缓 |
| Winget 官方收录 | 需要人类账号提 PR 且审核周期不可控；先做 Scoop（本机可验证）更划算 |
| 付费签名 / 公证 / 商店 | 与零成本原则直接冲突（红线 R5），不列入计划 |
| 自定义域名 | 同上；Cloudflare Pages 子域 + 品牌化落地页已够用 |

## 5. 验收标准（沿用 PLAN §M8，按现实调整；逐条证据见 [`acceptance/M8.md`](./acceptance/M8.md)）

- [x] macOS 产物经 `codesign -dv --verbose=2` 验证含 **adhoc** 签名（CI 断言；v1.1.x 每次构建都经过）
- [x] Windows：安装包与便携版均提供 SHA256，下载页给出可复制命令
- [x] `SHA256SUMS` 与 `SHA256SUMS.asc` 由 CI 自动生成并随 Release 发布
- [x] 官网提供 SHA256 + GPG 公钥 + 三平台安装指引（公开密钥指纹随页显示）
- [x] `scoop install` 成功 —— **本机实测**（下载/校验/解压/卸载全链路；bucket 仓库待人类创建）
- [ ] Homebrew / Winget / Flathub / Snap / AUR：**至少 1 个**完成并被真机/官方渠道验证（Winget/Homebrew 清单就绪，提交与真机验证待人类；Flathub/Snap/AUR 按 §4 不做）
- [x] 应用内首次启动的"安装可信性"引导页可用（含 i18n 与测试）
- [x] 15 个 good first issue 候选清单就绪（≥3 个可立即标记；创建待人类）
- [ ] 社区 Discussions 分类与欢迎贴就绪（文案就绪，创建待人类）
- [ ] 升级 + 回滚演练完成（升级链路已实证；完整留档随下一次版本发布）
- [x] **全流程零支出**：无付费账号、无付费证书、无付费商店

## 6. 建议的推进顺序

1. **T8.1**（macOS 签名断言）——小、能在下一次 dry-run 里立刻验证；
2. **T8.10 + 版本节奏**：发下一个补丁版本，同时完成升级/回滚演练留档
   （升级链路已随 v1.1.0–v1.1.3 发布被真实验证，缺的只是演练记录）；
3. **T8.3**（Scoop，本机可验证）→ **T8.2**（引导页）→ **T8.6/T8.7**（材料与清单）；
4. 需要人类账号的（Winget PR、Homebrew tap、SignPath、Discussions、推广）集中一批交给维护者执行。
