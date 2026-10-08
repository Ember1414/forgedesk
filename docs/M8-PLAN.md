# M8 计划：零成本信任加固 / 包管理器分发 / 社区启动

> 起点：**v1.0.0 已发布**（2026-10-08，Windows + macOS universal）。
> 上游依据：`docs/PLAN.md` §M8（本节按现状修订——原计划写的"GitHub Pages 官网"已由
> `docs/adr/ADR-003` 的 **Cloudflare Pages** 取代）。
> 核心约束不变：**全流程零支出**（不买签名证书、不买商店、不买域名）。

## 1. 目标与判断

M8 要解决的只有一个问题：**让陌生人敢下载、能装上、装完能用**。
无付费证书时的三个抓手：① 把校验做扎实（SHA256 + GPG + 随站公钥）；② 把摩擦写清楚
（去隔离 / SmartScreen 一步步说明）；③ 走包管理器（来源更可信、安装更省事）。

## 2. 现状盘点（M7 已顺带完成的部分）

| 计划项 | 现状 |
| --- | --- |
| D8.4 官网 + 下载页 | ✅ 已上线（Cloudflare Pages，多页站点 + 版本矩阵 + 同源校验和直显 + GPG 公钥随站） |
| D8.5 GPG 签名与校验和自动化 | ✅ `SHA256SUMS` + `SHA256SUMS.asc` 由 `release.yml` 的 `publish` 作业统一生成并签名 |
| D8.2 Windows 信任加固（校验/指引/便携版） | ✅ 便携版 zip 随发布；下载页有三平台校验命令与 SmartScreen 指引 |
| D8.1 macOS 安装指引 | ✅ `docs/install/macos.md` 本轮补齐（去隔离、右键打开、`codesign` 校验、数据目录）；macOS 已进入发布矩阵（universal） |
| D8.3 包管理器 | ⏳ 未开始（Scoop 可在本机验证；Homebrew/Winget 需要外部环境与 PR 审核） |
| D8.6 / D8.7 / D8.8 社区与推广 | ⏳ 未开始（多数需要人类账号与内容创作） |

## 3. 任务分解（按"能否由编码代理独立完成"排序）

| ID | 任务 | 谁做 | 难点 |
| --- | --- | --- | --- |
| T8.1 | **macOS ad-hoc 签名断言**：在 `release.yml` 的 `build-macos` 里加一步 `codesign -dv --verbose=2 ForgeDesk.app`，断言输出含 `Signature=adhoc` | 代理 | 需要在 macOS runner 上跑（dry-run 即可验证） |
| T8.2 | **安装可信性引导页**（应用内首次启动）：检测隔离标记 / SmartScreen 来源，给出对应指引与校验入口 | 代理 | 需要 UI + i18n；「是否被隔离」在 macOS 上可查（`xattr`），Windows 上只能引导 |
| T8.3 | **Scoop manifest**（自有 bucket 或 `scoop` 官方 bucket 草稿）：`scoop install` 可在本机真实验证 | 代理（本机可验证） | 需要新版本发布后才好用（manifest 指向 Release 资产） |
| T8.4 | **Winget manifest 草稿 + PR 流程说明**：`winget-pkgs` 的 PR 需要人类账号与审核 | 代理起草 / 人类提交 | 审核周期不可控 |
| T8.5 | **Homebrew Cask 草稿（自有 tap）**：`brew install --cask` 的验证需要一台 Mac | 代理起草 / 人类验证 | 真机验证 |
| T8.6 | **SignPath / 免费 OSS 签名计划**申请材料（`docs/install/signpath-application.md`） | 代理起草 / 人类提交 | 申请要账号与项目信息 |
| T8.7 | **15 个 good first issue 候选清单**（从现有 TODO / 已知缺口整理，含引导说明） | 代理 | 需要判断"外部贡献者能在不看上下文的情况下完成" |
| T8.8 | **社区运营**（Discussions 分类、欢迎贴、响应 SLA、月度节奏） | 人类（代理可写文案） | 需要仓库设置权限 |
| T8.9 | **推广材料**（演示视频 ×2、技术文章 ×3、文字对比表） | 人类为主 | 视频与文章需要真机演示与个人表达 |
| T8.10 | **升级 + 回滚演练**（M7 遗留的"部分通过"）：发 1.0.1 时走一遍完整链路 | 代理准备 / 人类执行发布 | 需要一个真实升级场景 |

## 4. 明确"先不弄"的项（与理由）

按维护者指示：**难以在当前条件下验证的项先不做**，避免为了打勾而堆砌无法验证的产物。

| 项 | 为什么先不做 |
| --- | --- |
| macOS / Linux **真机安装与自动更新实测** | 手上没有 Mac 与 Linux 桌面环境；写了"已验证"就是假话。等有真机或用户反馈再说（`docs/acceptance/M7.md` 已如实标注） |
| Flathub / Snapcraft / AUR | 依赖 Linux 打包链（AppImage/deb/rpm）与各自的真机验证，而 Linux 本身按 ADR-005 暂缓 |
| Winget 官方收录 | 需要人类账号提 PR 且审核周期不可控；先做 Scoop（本机可验证）更划算 |
| 付费签名 / 公证 / 商店 | 与零成本原则直接冲突（红线 R5），不列入计划 |
| 自定义域名 | 同上；Cloudflare Pages 子域 + 品牌化落地页已够用 |

## 5. 验收标准（沿用 PLAN §M8，按现实调整）

- [ ] macOS 产物经 `codesign -dv --verbose=2` 验证含 **adhoc** 签名（CI 断言，dry-run 覆盖）
- [ ] Windows：安装包与便携版均提供 SHA256，下载页给出可复制命令（✅ 已具备）
- [ ] `SHA256SUMS` 与 `SHA256SUMS.asc` 由 CI 自动生成并随 Release 发布（✅ 已具备）
- [ ] 官网提供 SHA256 + GPG 公钥 + 三平台安装指引（✅ 已具备；公开密钥指纹随页显示）
- [ ] `scoop install`（自有 bucket）成功 —— 可在本机验证
- [ ] Homebrew / Winget / Flathub / Snap / AUR：**至少 1 个**完成并被真机/官方渠道验证（其余记录为待办）
- [ ] 应用内首次启动的"安装可信性"引导页可用（含 i18n 与测试）
- [ ] 15 个 good first issue 候选清单就绪（≥3 个可立即标记）
- [ ] 社区 Discussions 分类与欢迎贴就绪（人类执行，代理提供文案）
- [ ] 升级 + 回滚演练完成（随下一次版本发布）
- [ ] **全流程零支出**：无付费账号、无付费证书、无付费商店

## 6. 建议的推进顺序

1. **T8.1**（macOS 签名断言）——小、能在下一次 dry-run 里立刻验证；
2. **T8.10 + 版本节奏**：发 `1.0.1`，同时完成升级/回滚演练与 macOS 首个"可升级"版本；
3. **T8.3**（Scoop，本机可验证）→ **T8.2**（引导页）→ **T8.6/T8.7**（材料与清单）；
4. 需要人类账号的（Winget PR、Homebrew tap、SignPath、Discussions、推广）集中一批交给维护者执行。
