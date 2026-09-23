# ADR-003: 官网与更新清单托管改用 Cloudflare Pages（替代 GitHub Pages）

- 状态：已接受
- 日期：2026-09-23
- 里程碑：M0（决策）→ M7（实施 T7.1 / T7.9）→ M8（T8.4）
- 关联：`docs/adr/ADR-002-private-first-two-phase-ci.md`、`docs/PLAN.md` §8.4、`docs/OPEN-SOURCE-READINESS.md`

## 上下文

项目决定私有仓库起步（ADR-002）。由此暴露一个冲突：

- `docs/PLAN.md` §8.3 / §8.4 / §7(M7) 把**官网静态站**与**自动更新清单托管**放在 GitHub Pages 上。
- **GitHub Pages 在 Free 计划的私有仓库中不可用**（需要 Pro/Team/Enterprise）。
- 因此 ADR-002 的处置是"把 Pages 相关任务整体后移到转公开之后"——这会推迟 M7 的官网与 T7.1 的更新通道，
  而更新通道是"可分发给他人的桌面软件"的必要条件（没有它，用户永远停在旧版本）。

人类提出：是否可以用 Cloudflare 部署。

## 决策

**用 Cloudflare Pages 承载官网静态站；用同一个 Pages 项目（或后续的 Cloudflare Workers）承载自动更新清单。**

具体形态：

| 用途 | 载体 | 说明 |
| --- | --- | --- |
| 官网 / 下载页（T7.9、T8.4） | Cloudflare Pages | 静态站（VitePress 或 Astro），域名用 `*.pages.dev` 免费子域 |
| 更新清单（T7.1、T7.3） | 同一 Pages 项目的 `/updates/<target>/<arch>/<channel>.json` | 静态 JSON，由 release 工作流写入 |
| 后续如需动态路由（按渠道/平台返回不同清单、灰度发布） | Cloudflare Workers（免费档） | 保持在同一平台内演进，无需换供应商 |

**部署方式：Direct Upload（`wrangler pages deploy`），由 GitHub Actions 执行，不接 Git 自动构建。**

理由：Git 自动构建需要把私有仓库授权给 Cloudflare 的 GitHub App 并暴露构建配置；
Direct Upload 只需要一个 API Token 存在 GitHub Secrets 里，权限面更小，且构建产物由我们自己的 CI 产出，
与"CI 是唯一构建入口"的既有约定一致。

## 备选方案

| 方案 | 免费额度 | 支持私有源仓库 | 结论 |
| --- | --- | --- | --- |
| GitHub Pages | 无限（但私有仓库不可用） | ❌（私有需付费计划） | 否（M7 前不可用） |
| **Cloudflare Pages** | 静态请求与带宽不限；构建/部署次数有月度上限 | ✅（且 Direct Upload 不需要暴露仓库） | **采用** |
| Cloudflare Workers | 有每日请求上限但额度充足 | ✅ | 作为动态化后的演进路径 |
| Netlify / Vercel | 有月度带宽额度，超出即停 | ✅ | 否（带宽计费口径不如 Cloudflare 宽松，长期有超限风险） |
| 自建静态托管（对象存储 + CDN） | 视供应商，通常有流量费 | ✅ | 否（违反"零成本、零运维"） |
| 先不做官网与更新通道 | — | — | 否（会阻断"可分发"这一核心目标） |

> 配额数值以 Cloudflare / GitHub 官方最新定价为准；本 ADR 只固化"选谁"与"为什么"，不固化具体数字。

## 后果

**正面**

- 解除了 ADR-002 中"Pages 相关任务后移到转公开之后"的约束：**私有阶段即可完成官网与更新通道**，
  M7 的功能完成度不再被仓库可见性拖累。
- 免费额度对"静态站点 + 少量 JSON"这种负载极其宽裕，且无带宽计费风险。
- 保留了向 Workers 演进的空间（灰度发布、按渠道分发），不必更换供应商。

**负面 / 需要接受的代价**

- 引入一个新的第三方依赖（Cloudflare 账号）。虽然免费，但属于"新增外部服务"，需在隐私政策
  （`docs/PRIVACY.md`）中如实说明"应用会向 `<project>.pages.dev` 请求更新清单"。
- 需要人工完成账号与凭据设置（见下）。这类操作按 AGENTS.md §10 必须由人类执行。
- 更新清单的 URL 与 GitHub Pages 方案不同 → T7.1 的 `endpoints` 配置按本 ADR 的路径约定编写。

**需要人类执行的前置动作（一次性）**

1. 注册/登录 Cloudflare 账号（免费）。
2. 创建 Pages 项目（Direct Upload 模式），取得项目名（将成为 `<project>.pages.dev`）。
3. 生成 API Token（权限只需 `Cloudflare Pages: Edit`，并限定到该账号），
   存入 GitHub 仓库 Secrets：`CLOUDFLARE_API_TOKEN`、`CLOUDFLARE_ACCOUNT_ID`。
4. 把项目名告知编码代理，以便写入 CI 工作流与 `tauri.conf.json` 的更新端点。

**对既有文档的影响（已同步）**

- `docs/PLAN.md` §8.3、§8.4、§12.3 中"GitHub Pages"的表述改为"Cloudflare Pages（见 ADR-003）"。
- `docs/OPEN-SOURCE-READINESS.md`：删除"Pages 相关任务后移"的约束项，改为"Cloudflare 部署凭据已配置"。
- `docs/adr/ADR-002-*`：其"约束二 / 后果"部分保留原样（记录当时的判断），但结论已被本 ADR 部分解除。
