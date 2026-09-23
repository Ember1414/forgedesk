# 转公开前审计清单（Open-Source Readiness）

> 背景：本项目采用**私有仓库起步**（见 `docs/adr/ADR-002-private-first-two-phase-ci.md`）。
> 关键风险：**把仓库改为 public 时，全部 git 历史会一起公开，且不可撤销**（只能靠重写历史或重建仓库补救）。
> 因此本清单必须在改动 visibility **之前**逐项完成。
>
> 状态：**未开始**（M0 阶段建立，转公开前执行）
> 负责人：人类（涉及账号与法律判断的部分）+ 编码代理（可自动化的部分）

---

## 0. 现在（首次提交之前）就该定的事

这些事在**第一次 commit 之前**处理成本为零，之后处理就需要重写历史。

| # | 事项 | 决策（2026-09-23） | 状态 |
| --- | --- | --- | --- |
| P-1 | git 提交者邮箱 | 改用 GitHub 隐私邮箱 `237394828+Ember1414@users.noreply.github.com` | ⏳ **待人类执行 `git config`**（本人按 Git 安全约定不擅自修改） |
| P-2 | git 提交者姓名 | 改用 `EMBER` | ⏳ 同上 |
| P-3 | 是否提交 `docs/PLAN.md` / `docs/AGENT-PROMPTS.md` | **全部提交**（透明化，接受商业化讨论与技术方法公开） | ✅ 已决定 |
| P-4 | 是否提交 `docs/DEV-ENV.md` | **提交脱敏版**：已移除本机绝对路径与 IDE 内部组件路径，改用 `%USERPROFILE%` / `<repo-root>` 占位符 | ✅ 已完成 |
| P-5 | 首次提交前的敏感信息扫描 | 首次提交前执行第 1 节 A-1 ~ A-6 | ⏳ 待执行 |

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
| E-3 | 更新清单地址已写入应用配置 | `src-tauri/tauri.conf.json` 的 updater `endpoints` | 指向 `<project>.pages.dev`，且与发布工作流写入路径一致 |
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
| — | — | §1 全部门禁 | 转公开前执行 |

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
