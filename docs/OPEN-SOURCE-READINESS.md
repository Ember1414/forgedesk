# 转公开前审计清单（Open-Source Readiness）

> 背景：本项目采用**私有仓库起步**（见 `docs/adr/ADR-002-private-first-two-phase-ci.md`）。
> 关键风险：**把仓库改为 public 时，全部 git 历史会一起公开，且不可撤销**（只能靠重写历史或重建仓库补救）。
> 因此本清单必须在改动 visibility **之前**逐项完成。
>
> 状态：**进行中**——A-1 ~ A-6、B-1 / B-2 / B-4（声明与证据链部分）/ B-5 / B-6 / B-7 / B-8 / B-9、C-1 ~ C-4 均已通过（见 §4）；
> 尚未完成的全部是**人工或外部**项：**B-3 / B-4 的 3 人盲测**（ADR-005 延后至首次对外预发布前；证据链与记录表见 `docs/BRAND.md` §4.1–4.2）、
> **B-10**（干净环境照 CONTRIBUTING 复现）、§1.3 的托管凭据项、§1.4 C-3 的 Rust 侧补跑（`cargo audit`，本机不可用）。
> **A 组有一处需人类决策的残留**：历史中曾回显个人邮箱（HEAD 已脱敏），见 §4 与 §1.1 的说明。
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

按顺序执行，逐步验证：

```text
1. 完成第 1 节全部门禁，并把结果记录在本文件末尾的「审计记录」小节。
2. 在 GitHub 上把仓库 visibility 改为 public。
3. 立即验证：Actions 是否恢复不限额度；用一次 workflow_dispatch 触发三平台构建确认。
4. 修改 .github/workflows/ci.yml：删除 build job 上的
   `if: github.event_name != 'pull_request'`（见文件顶部注释的第 1 条）。
5. 启用 GitHub Pages（Settings → Pages → Source: GitHub Actions）。
6. 启用 Discussions 并创建分类（T8.7 / PLAN §14.4）。
7. 恢复/启用 e2e.yml 与 nightly.yml（T7.2）。
8. 检查仓库设置：Issues 开启、Wiki 关闭（用 docs/ 代替）、Sponsors 视需要、
   Security Advisories 开启（SECURITY.md 依赖它）。
9. 更新 README：去掉任何"开发中/私有"的措辞，补齐安装与校验说明。
10. 发布一条公告（首个公开 commit 的说明），并在 Discussions 发欢迎贴。
```

---

## 3. 转公开后立即做的检查

| # | 检查 | 期望 |
| --- | --- | --- |
| D-1 | 匿名访问仓库首页 | 可看到 README、LICENSE、免责声明 |
| D-2 | 匿名访问 Actions | 构建记录与产物可见（公共仓库的 artifact 对未登录用户仍不可下载，属正常） |
| D-3 | 匿名访问 Issues 模板 | 可看到 bug/feature/RFC 模板 |
| D-4 | 搜索仓库内容中的敏感关键词 | `token`、`password`、`secret`、本机路径 → 0 命中 |
| D-5 | 用未登录浏览器打开 GitHub Pages | 可访问（若已启用） |
| D-6 | 对照 PLAN §9.5 的红线复检 | 无 Git / GitHub / Tauri Logo，无 Octocat 变体，产品名不含相关字样 |

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
| 2026-10-07 | 编码代理 | **发布链路代码侧就位（承接 E-1 ~ E-4）** | `release.yml`（preflight / build-windows / publish）+ 4 个脚本落地，`pnpm check:workflows` 6 个工作流 0 警告。**E-3 的口径已明确**：endpoints 由发布构建注入而非写进仓库（见上表注），与工作流写出的 `<渠道>/<target>.json` 路径逐字一致。E-2 / E-4 仍待人类配置凭据后复核（`docs/RELEASE.md` §4.1 的凭据表） |
| — | — | §1 其余门禁（B-3 / B-4 盲测、B-10、C-3 的 Rust 侧）与 §1.3 托管凭据（E-2 / E-4） | 转公开前执行 |

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
