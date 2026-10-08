# 转公开前审计清单（Open-Source Readiness）

> 背景：本项目采用**私有仓库起步**（见 `docs/adr/ADR-002-private-first-two-phase-ci.md`），
> 已于 **2026-10-07 转为公开**（`visibility=public` 由匿名 API 核实，见 §3 与 §4）。
> 关键风险：**把仓库改为 public 时，全部 git 历史会一起公开，且不可撤销**（只能靠重写历史或重建仓库补救）。
> 因此本清单要求在改动 visibility **之前**逐项完成；转公开之后才发现的问题按"能改的改、不能改的记录"处理。
>
> 状态：**已转公开，收口进行中**——A-1 ~ A-6、B-1 / B-2 / B-4（声明与证据链部分）/ B-5 / B-6 / B-7 / B-8 / B-9、C-1 ~ C-4 均已通过（见 §4）；
> 尚未完成的全部是**人工或外部**项：**B-3 / B-4 的 3 人盲测**（ADR-005 延后至首次对外预发布前；证据链与记录表见 `docs/BRAND.md` §4.1–4.2）、
> **B-10**（干净环境照 CONTRIBUTING 复现）、**E-4 更新签名密钥**（Cloudflare 凭据已配置并经真实部署验证，见 §4）、§1.4 C-3 的 Rust 侧补跑（`cargo audit`，本机不可用）。
> **A 组残留已由人类决策处置**：历史中曾回显的个人邮箱选择"**接受公开**"，处置记录见 §4。
> 负责人：人类（涉及账号与法律判断的部分）+ 编码代理（可自动化的部分）

---

## 0. 现在（首次提交之前）就该定的事

这些事在**第一次 commit 之前**处理成本为零，之后处理就需要重写历史。

| # | 事项 | 决策（2026-09-23） | 状态 |
| --- | --- | --- | --- |
| P-1 | git 提交者邮箱 | 改用 GitHub 隐私邮箱 `237394828+Ember1414@users.noreply.github.com` | ✅ 已满足（2026-10-07 只读核对：`git config user.email` 已是隐私邮箱；**全部提交**的作者/提交者邮箱唯一且为该地址） |
| P-2 | git 提交者姓名 | 改用 `EMBER` | ✅ 已满足（同上，`user.name` = EMBER） |
| P-3 | 是否提交 `docs/PLAN.md` / `docs/AGENT-PROMPTS.md` | **全部提交**（透明化，接受商业化讨论与技术方法公开） | ✅ 已决定 |
| P-4 | 是否提交 `docs/DEV-ENV.md` | **提交脱敏版**：已移除本机绝对路径与 IDE 内部组件路径，改用 `%USERPROFILE%` / `<repo-root>` 占位符 | ✅ 已完成 |
| P-5 | 首次提交前的敏感信息扫描 | 首次提交前执行第 1 节 A-1 ~ A-6 | ✅ 已执行（2026-10-07，见 §4；**发现 1 处需人类决策的残留**：`docs/adr/ADR-004` 曾在历史中回显个人邮箱，HEAD 已脱敏） |

> 决策记录：仓库归属、提交身份与应用标识符的完整理由见 `docs/adr/ADR-004-repository-identity-and-identifier.md`。

---

## 1. 硬性门禁（必须全部通过才能改 visibility）

### 1.1 git 历史与内容审计

| # | 检查项 | 方法 | 判定标准 |
| --- | --- | --- | --- |
| A-1 | 历史中是否出现过凭据 | 运行 `gitleaks detect --log-opts="--all"`（或 `trufflehog git file://.`） | 0 命中；若有命中 → **不得直接公开**，先重写历史 |
| A-2 | 历史中是否出现本机绝对路径与个人身份信息 | 检索提交内容与全部文档中的用户目录路径、个人邮箱、真实姓名 | 0 命中（或确认可接受） |
| A-3 | 是否出现真实邮箱/电话/身份信息 | 人工抽查 + `git log --format='%ae %ce'` 去重 | 全部为有意公开的地址 |
| A-4 | 是否出现内部讨论、未公开的商业判断 | 人工通读 `docs/` 与提交信息 | 已由 P-3 决策覆盖 |
| A-5 | 是否提交了 `.env` / `*.key` / `*.pfx` / 凭据文件 | 对照 `.gitignore` 并 `git ls-files` 抽查 | 0 命中 |
| A-6 | 大文件/二进制是否混入（意外提交的安装包、日志） | `git rev-list --objects --all \| git cat-file --batch-check`，筛 > 1MB | 无意外大文件 |

> **若 A-1 / A-2 有命中且不可接受**：不要用 `git filter-repo` 冒险在同一仓库上操作，
> 更稳妥的做法是**新建一个仓库，只推送清洗后的历史**（或从零开始、只保留当前快照）。
> 具体方案届时以 ADR 记录。

### 1.2 工程与合规就绪

| # | 检查项 | 命令 / 位置 | 通过标准 |
| --- | --- | --- | --- |
| B-1 | LICENSE 存在且与声明一致 | `LICENSE`（Apache-2.0）+ `package.json` / `Cargo.toml` 的 `license` 字段 | 三处一致 |
| B-2 | 免责声明齐备 | `README.md` 含 PLAN §9.5 模板全文（Git / GitHub / Tauri 三个无关联声明 + 无 AI 功能声明） | 关键字齐全（由 `scripts/compliance/check.mjs` 断言） |
| B-3 | 无受限商标素材 | `scripts/compliance/check.mjs` 图标哈希检查 + 人工确认 | 通过 |
| B-4 | 应用图标为原创 | `docs/BRAND.md` §4 声明 + 盲测记录 | 通过 |
| B-5 | 第三方许可清单存在 | `docs/LICENSE-AUDIT.md`（由 `cargo deny` + `pnpm licenses` 生成） | 无 GPL/AGPL（libgit2 链接例外除外） |
| B-6 | 无 AI/推理依赖 | `scripts/compliance/check.mjs` 的依赖扫描 | 0 命中 |
| B-7 | 无第三方遥测/分析 SDK | 同上 | 0 命中 |
| B-8 | 隐私政策存在 | `docs/PRIVACY.md`（T7.6 产出） | 存在且与应用内页面一致 |
| B-9 | 社区文件齐备 | `CONTRIBUTING.md`、`CODE_OF_CONDUCT.md`、`SECURITY.md`、Issue/PR 模板（T7.8） | 齐全 |
| B-10 | 所有质量门禁在干净环境可复现 | 在一个未参与开发的机器/容器上跑通 `CONTRIBUTING.md` 的步骤 | 可复现 |

### 1.3 托管与分发凭据

| # | 检查项 | 方法 | 通过标准 |
| --- | --- | --- | --- |
| E-1 | 静态站与更新清单的托管决策已定稿 | `docs/adr/ADR-003-cloudflare-instead-of-github-pages.md` | 已定（Cloudflare Pages） |
| E-2 | Cloudflare 项目与凭据已配置 | Cloudflare Pages 项目 + API Token 存入 GitHub Secrets（`CLOUDFLARE_API_TOKEN`、`CLOUDFLARE_ACCOUNT_ID`） | 存在且权限最小（仅 Pages: Edit） |
| E-3 | 更新清单地址已确定，并在**发布构建**时写入应用 | `scripts/ci/make-build-config.mjs` 注入 `endpoints`（`https://forgedesk.pages.dev/updates/<渠道>/{{target}}.json`） | 与发布工作流写入路径**逐字一致**；**刻意不写进仓库**——理由见 `docs/RELEASE.md` §4.1：写进去会让开发构建的 `update_check` 从"静默无更新源"变成"每次检查都网络失败" |
| E-4 | 隐私政策已说明对外请求 | `docs/PRIVACY.md` 列出全部对外域名（GitHub API、`*.pages.dev`） | 与实际网络行为一致 |

> 说明：本项因 ADR-003 而**解除**了 ADR-002 中"Pages 相关任务必须后移到转公开之后"的约束——
> Cloudflare Pages 支持私有仓库场景，因此官网与更新通道在私有阶段即可完成。

### 1.4 供应链加固

| # | 检查项 | 方法 | 通过标准 |
| --- | --- | --- | --- |
| C-1 | 所有 action 固定到 commit SHA | `node scripts/ci/pin-actions.mjs` 然后 `pnpm check:workflows` | 校验器输出 0 个"未固定"警告 |
| C-2 | lockfile 已提交且被 CI 使用 | `Cargo.lock`、`pnpm-lock.yaml` 在版本控制中；CI 用 `--frozen-lockfile` | 通过 |
| C-3 | 依赖漏洞扫描 | `cargo audit`、`pnpm audit` | 无未处理的高危项 |
| C-4 | CI 最小权限 | 每个工作流有顶层 `permissions` | 校验器无相关警告 |

---

## 2. 转公开时执行的动作（一次性）

按顺序执行，逐步验证。**实际执行记录（2026-10-07）**：

| # | 原定动作 | 结果 |
| --- | --- | --- |
| 1 | 完成 §1 全部门禁并记录到 §4 | 🟡 自动部分全通过；人工项（B-3/B-4 盲测、B-10）按 ADR-005 与本 ADR 的"不阻塞公开"判定延后，见 §4 |
| 2 | 在 GitHub 上把 visibility 改为 public | ✅ 已执行（匿名 API：`private=false`、`visibility=public`） |
| 3 | 立即验证 Actions 额度，并用一次 `workflow_dispatch` 确认三平台构建 | ⏳ 推送后由 `push main` 的 ci.yml 自动验证（公开仓库 standard runner 不计费） |
| 4 | 修改 `ci.yml`：删除 build job 上的 `if` | ✅ 已删除。**更正**：原文记的条件 `if: github.event_name != 'pull_request'` 是更早期形态；实际被删的是 `startsWith(github.ref,'refs/tags/') \|\| (workflow_dispatch && inputs.run_build)` |
| 5 | "启用 GitHub Pages" | ⚠️ **本项目不需要**——ADR-003 已把官网与更新清单托管改为 Cloudflare Pages（私有/公开均可用） |
| 6 | 启用 Discussions 并创建分类 | ⏳ 需人类在仓库设置里做（模板已在 `.github/DISCUSSION_TEMPLATE/`） |
| 7 | 恢复/启用 `e2e.yml` 与 `nightly.yml` | 🟡 `nightly.yml` 已加 `schedule`；`e2e.yml` 已创建但**只挂手动触发**（整套 spec 此前仅在 Windows+Edge 跑过，首跑通过后再接 PR 门禁） |
| 8 | 仓库设置：Issues 开、Wiki 关、Security Advisories 开 | ⏳ 需人类在设置里做 |
| 9 | 更新 README 措辞与安装说明 | ✅ 安装段已改为指向下载页与 Releases，并保留"尚无发布"的如实表述 |
| 10 | 发布公告与欢迎贴 | ⏳ 待首次发布（§1.3 凭据 + 打 tag）之后进行 |

> 第 2 步的执行顺序上，代码侧存在一个**先后关系**：把 M7 的提交推送到 main 之前，公开仓库仍停留在
> M6 的内容（缺 `PRIVACY.md`/`SECURITY.md`/`RELEASE.md` 等，且站点仍是占位页）。因此"转公开"与
> "推送 M7 成果"必须当作同一件事完成，否则公开仓库会短暂处于"文件缺失"的状态。

---

## 3. 转公开后立即做的检查

| # | 检查 | 期望 |
| --- | --- | --- |
| D-1 | 匿名访问仓库首页 | 可看到 README、LICENSE、免责声明 |
| D-2 | 匿名访问 Actions | 构建记录与产物可见（公共仓库的 artifact 对未登录用户仍不可下载，属正常） |
| D-3 | 匿名访问 Issues 模板 | 可看到 bug/feature/RFC 模板 |
| D-4 | 搜索仓库内容中的敏感关键词 | `token`、`password`、`secret`、本机路径 → 0 命中 |
| D-5 | 用未登录浏览器打开官网（Pages） | 可访问（本项目用 Cloudflare Pages，见 ADR-003） |
| D-6 | 对照 PLAN §9.5 的红线复检 | 无 Git / GitHub / Tauri Logo，无 Octocat 变体，产品名不含相关字样 |

**2026-10-07 实测**：

- **D-1 ✅** 匿名 `GET https://api.github.com/repos/Ember1414/forgedesk` → 200，`private=false`、`visibility=public`、`default_branch=main`；
- **D-3 ✅** 匿名访问仓库首页 → 200（353 KB，含 README 与免责声明）；
- **D-5 ✅** 匿名访问 `https://forgedesk.pages.dev/` → 200（当时仍是 M0 占位页；站点随下一次 `site/**` 变更自动部署）；
- **D-2 / D-4 / D-6 ⏳** 待 M7 成果推送后复检（D-4 的内容侧已在 A 组扫描过：无凭据、无本机路径）。

---

## 4. 审计记录

> 每次执行本清单时在此留痕（日期、执行人、结果、未通过项与处理）。

| 日期 | 执行人 | 门禁结果 | 备注 |
| --- | --- | --- | --- |
| 2026-09-23 | 编码代理 | §0 决策完成（P-1/P-2 待人类执行 git config） | M0 阶段；B/C 组门禁待 T0.12 与 M7 完成后执行 |
| 2026-10-07 | 编码代理 | B-1/B-2/B-5/B-6/B-7 通过（compliance 全绿）；B-4 见 BRAND 记录 | M7 文档阶段：README 重写、PRIVACY.md 产出并断言免责声明；B-8（隐私政策）✅、B-9（社区文件：CoC/SECURITY/模板）✅ |
| 2026-10-07 | 编码代理 | B-8 完整闭合 | 应用内「设置 → 隐私」页落地（内容与 `docs/PRIVACY.md` 一致），B-8 的"与应用内页面一致"要求满足 |
| 2026-10-07 | 人类（确认"按建议"） | 决策记录 | **不引入遥测**（保持零遥测承诺；PLAN 的 PF-11 属 V1.1）；**暂不引入 `deny.toml`**（`pnpm compliance` 为唯一许可门禁，避免两套 allow 列表漂移）；**接受**传递依赖 `r-efi` 的 LGPL（仅 UEFI target，不进入产品链接）。C-1 的 action 固定 SHA 仍待发布流水线阶段执行 |
| 2026-10-07 | 编码代理 | **C-1 通过**（`pnpm check:workflows` 0 警告，此前 33） | `node scripts/ci/pin-actions.mjs` 固定 32 处 action 引用（保留 `# v7` 标签注释）；给 `compliance.yml` 补 `concurrency`。运维提示：本机存在 TLS 检查代理时需 `node --use-system-ca scripts/ci/pin-actions.mjs`，否则 fetch 报 `UNABLE_TO_VERIFY_LEAF_SIGNATURE` |
| 2026-10-07 | 编码代理 | **M7 逐条验收自检完成** | 见 `docs/acceptance/M7.md`：10 条验收标准 5 通过 / 2 部分通过 / 3 未执行；未执行项全部卡在发布凭据（更新签名密钥 + 更新清单托管 + `release.yml`）。B-9（社区文件）、B-8（隐私政策）已闭合 |
| 2026-10-07 | 编码代理 | **P-1 / P-2 实为已完成**（文档状态过期） | 只读核对：`git config user.name` = `EMBER`、`git config user.email` = `237394828+Ember1414@users.noreply.github.com`；`git log --all --format='%ae'` 去重 = **1**（222 个提交全部使用隐私身份）。未改动任何 git 配置 |
| 2026-10-07 | 编码代理 | **A-2 / A-3 / A-5 / A-6 通过；A-1 以特征式替代完成** | **A-3**：作者与提交者身份唯一且为隐私地址（见上）。**A-5**：`git ls-files` 对 `/.env` / `*.key` / `*.pem` / `*.pfx` / `*.p12` / `id_rsa` / `id_ed25519` / `.npmrc` **0 命中**。**A-6**：全部 4566 个 git 对象中**无 >1MB 的 blob**（最大者远低于阈值）。**A-2**：全历史 2924 行路径命中**全部为占位**（`/home/u`、`/home/octocat`、`/home/runner/work`，以及 `C:\Users\…\` 带省略号的示例）；真实标记 `E:\Projects` **0 命中**、`Users\TD` **0 命中**、中国大陆手机号 0 命中。**A-1**（gitleaks / trufflehog 均未安装，改用高信号特征式全历史检索）：唯一凭据串共 **8 个，全部为显式假夹具**（`ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789`、`AKIAIOSFODNN7EXAMPLE`（AWS 官方文档示例键）、`ghp_AAAA…/BBBB…`、`glpat-Zx9Yy8Ww7Vv6Uu5Tt4Ss`、`github_pat_11ABCDEFG0…`），且它们是**脱敏器自身的测试夹具**（`crates/diagnostics/src/sanitize.rs` 等）；工作区已跟踪文件与 34 个未跟踪文件同样 0 真凭据。**建议在能安装 `gitleaks` 的环境补跑一次本项**（本机无该工具，且网络受限） |
| 2026-10-07 | 编码代理 | **A 组发现 1 处真实个人信息（需人类决策）** | `docs/adr/ADR-004-…md:88` 曾**回显个人 QQ 邮箱**——正是该段文字警告不要暴露的东西。**HEAD 已就地脱敏**（改为"个人邮箱"描述，并记录本次处置）。**历史侧**：`git log --all -S` 显示该串仅由 **1 个提交**引入（`08be90e` M0 引导提交），因此清洗历史的成本很低（只需重写一个提交的 diff）。**待人类在「新建仓库并只推送清洗后的历史」与「接受该地址公开」之间决定**——处置路径见 §1.1 的说明 |
| 2026-10-07 | 编码代理 | **C-2 / C-4 通过** | **C-2**：`Cargo.lock` 与 `pnpm-lock.yaml` 均被跟踪；CI 用 `pnpm install --frozen-lockfile`（3 处），本轮又给 cargo 的构建/测试步骤补上 `--locked`（`ci.yml` 的 clippy / tests / safety / example-plugins 与 `nightly.yml` 的 probe；加参数前先本地跑 `cargo check --workspace --all-targets --locked` 确认锁文件与工作区一致）。**C-4**：5 个工作流均有顶层 `permissions`，`pnpm check:workflows` 0 警告 |
| 2026-10-07 | 编码代理 | **C-3 通过：2 处 high 已修，扫描归零** | 本机默认 registry（`registry.npmmirror.com`）不实现 audit 端点 → 改用 `pnpm audit --registry=https://registry.npmjs.org/`。修复前 2 处：`sharp` <0.35.5（CVE-2026-96889，librsvg）与 `source-map-js` <1.2.2（GHSA-68fv-2mgg-jv7q，事件循环 DoS）——**两者都在 dev 链路、不进产物**。处置：`sharp` 提到 `^0.35.5`；`source-map-js` 用 `pnpm-workspace.yaml` 的 `overrides` 钉到 `1.2.2`（**pnpm 11 不读 `package.json#pnpm`**，该字段会告警并被忽略——本项目配置本就以 `pnpm-workspace.yaml` 为准，与既有 `ansi-regex` 条目同一处）。复核：`pnpm audit` → **No known vulnerabilities found**；前端 891 测试、`compliance`（含依赖许可）、`check:workflows`、`typecheck`、`lint`、`format:check`、`i18n:check` 全绿。**Rust 侧 `cargo audit` 未跑**（本机无该工具且需联网拉 RustSec 库）→ 建议在可安装的环境补跑 |
| 2026-10-07 | 编码代理 | **B-3 / B-4 的证据链就位；人工盲测仍待执行（ADR-005）** | **真源自检**：`docs/brand/icon-source.svg` 全部图元为基础几何（`rect`×5 + `path`×3 + 渐变×2），无 `<image>`、无外部引用、注释外无品牌词。**产物可复现**：重跑 `node scripts/brand/render-icon.mjs` 后 `icon-1024.png` 的 SHA256 **完全不变**（`def8097a509dc22a…`，且在升级后的 `sharp` 0.35.5 下复现）——证明图标确由真源渲染。**产物台账**（真源 + 1024 PNG + 17 个平台图标）写入 `docs/BRAND.md` §4.1。自动化侧：`pnpm compliance` 的「图标检查（R2）」通过。**未过项**：`known-logos.json` 的 `knownSha256` 为空（有意为之，避免虚假安全感），故"与官方 Logo 无相似"仍须**人工并排目视**确认；3 人盲测按 ADR-005 延后，方法 + 记录表见 `docs/BRAND.md` §4.2（不得由代理代填） |
| 2026-10-07 | **人类** | **仓库转为公开（`visibility=public`）** | 编码代理不能执行账号级操作（本机亦无 `gh`），由人类在 GitHub 设置里完成；代理随后匿名核实：`private=false`、`visibility=public`、首页 200（见 §3） |
| 2026-10-07 | **人类（决策）+ 编码代理（记录）** | **A-2 残留处置：接受公开** | 历史中 `docs/adr/ADR-004` 曾回显个人邮箱（HEAD 已于 `c707a54` 脱敏，历史侧因该串存在于 M0 之后的每个树而无法在不重写历史的前提下收回）。人类选择**选项①接受**——重写历史会使文档中数十处提交哈希失效，代价高于该地址（QQ 邮箱）已在多处公开的事实。代理已把该决策写入本表，并据此关闭 A-2 残留项 |
| 2026-10-07 | 编码代理 | **转公开后的收口变更** | `ci.yml`：删除 build job 的私有阶段 `if`，恢复"每个 PR 跑三平台矩阵"；`nightly.yml`：加 `schedule: cron '0 2 * * *'` 真正启用；新增 `.github/workflows/e2e.yml`（**仅手动触发**，Linux+Chromium 首跑待观察）；`playwright.config.ts` 的浏览器改为 `PW_CHANNEL` 可覆盖（CI 用 chromium）；README 安装段指向下载页与 Releases；ADR-002 补记"公开阶段已生效"。E-2 旁证：`https://forgedesk.pages.dev` 返回 200，说明 Pages 项目与部署凭据此前已就位 |
| 2026-10-07 | 编码代理 | **发布链路代码侧就位（承接 E-1 ~ E-4）** | `release.yml`（preflight / build-windows / publish）+ 4 个脚本落地，`pnpm check:workflows` 6 个工作流 0 警告。**E-3 的口径已明确**：endpoints 由发布构建注入而非写进仓库（见上表注），与工作流写出的 `<渠道>/<target>.json` 路径逐字一致。E-2 / E-4 仍待人类配置凭据后复核（`docs/RELEASE.md` §4.1 的凭据表） |
| 2026-10-07 | 编码代理 | **E-2 复核为通过：Cloudflare 凭据已配置并经真实部署验证** | 当天一次真实推送触发 Deploy site：`Setup pnpm` / `Deploy` 等步骤由 skip 变为 **success**，线上落地页（含下载区与校验和说明）匿名可访问——满足"清单 URL 能匿名访问"的核对口径（RELEASE.md §4.1） |
| 2026-10-07 | 编码代理 | **发布前置的真实环境缺陷修复（转公开后首轮实跑发现）** | ① `pnpm dev` 启动即崩：`tauri-plugin-updater` 要求 conf 必须有合法 `plugins.updater`（`pubkey` 必填，键缺失按 null 反序列化）→ 补占位 `pubkey:""` + `endpoints:[]`，发布时仍由 overlay 注入真实值；该崩溃自 T7.1 落地起就存在，只是全部测试都不启动 Tauri 运行时——"能启动"从此列入待补的冒烟项。② CI 长期红灯三根因（`8c539ae`）：safety 作业缺 `libdbus-1-dev`（9-30 后全红的根因，10-01 keyring 落地引入）、example-plugins 跨机器 wasm 字节差异（构建机路径进了 panic 字符串 → `--remap-path-prefix` + 钉 rustc 版本）、失败详情提升为匿名可读注解。③ safety 在 Linux 暴露真实差异：`chmod 0o555` 不递归，git 能先动 HEAD——`ReadOnlyGuard` 改为递归只读（与 Windows icacls 继承语义对齐）。④ quality 在 Linux 的 `forgedesk-platform` 编译错误已由注解管道锁定到具体 crate，复现与修复进行中 |
| 2026-10-08 | 编码代理 | **v1.1.0 实机反馈批次 + 两条"从未生效"的门禁修复** | ① 19 处 IPC 调用点参数平铺（结构体参数未包同名键）→ 编辑器/GitHub 议题/PR/仪表盘整块失效，而 922 个单测全绿（都 mock 掉 invoke）——新增 `pnpm check:ipc` 门禁。② 命令注册漂移：`git_commit_detail` 只在 release、`workspace_diff*` 只在 dev → 对应功能在另一构建下必失败，同门禁纳入两份清单比对。③ Nightly 性能门禁**从落地起从未跑通**：工作流给脚本多传了一个参数 → `readdirSync` 抛异常；修正接口并让回归指标/失败原因进匿名可读注解。④ 黑框（GUI 子进程 `CREATE_NO_WINDOW`）、主线程卡顿（45 个命令移出事件循环）、编辑器与主题增强、命令词典 36→69 条、插件面板可见性——详见 CHANGELOG [1.1.0] |
| — | — | §1 其余门禁（B-3 / B-4 盲测、B-10、C-3 的 Rust 侧）与 §1.3 托管凭据（E-4：更新签名密钥） | 已公开，按"能改的改、不能改的记录"处理 |

---

## 附：与里程碑的衔接

| 清单项 | 归属里程碑 | 说明 |
| --- | --- | --- |
| P-1 ~ P-5 | **M0（现在）** | 首次提交前处理，成本为零 |
| A-1 ~ A-6 | 转公开前 | 随时可执行；每次提交后复跑 A-1 成本很低 |
| B-1 ~ B-7 | M0 / M7 | 部分由 `scripts/compliance/check.mjs`（T0.12）自动保障 |
| B-8 ~ B-10 | M7 | T7.6 / T7.7 / T7.8 产出 |
| C-1 ~ C-4 | 转公开前（M7） | C-1 用 `scripts/ci/pin-actions.mjs` |
| 第 2、3 节 | 转公开时 | 一次性动作 |
