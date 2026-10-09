# 免费代码签名计划申请材料（SignPath Foundation 等）

> **本文件供人类提交申请使用。代理不得自行提交申请，也不得声称已提交。**
> 申请需要维护者账号、项目所有权证明与对仓库的持续公开承诺——这些只能由人类完成。
>
> 相关：`docs/M8-PLAN.md` T8.6、`docs/install/windows.md` §3（SmartScreen 指引）、
> `AGENTS.md` 红线 R5（零成本：只申请**免费**计划，任何需要付费的选项直接放弃）。

## 1. 为什么值得申请

零成本方案下 Windows 用户每次安装都会遇到 SmartScreen"未知发布者"拦截。
免费 OSS 签名计划（面向开源项目的代码签名服务）能在不花钱的前提下消除大部分
拦截——这是 M8 里对"下载后打不开"问题性价比最高的一步。

已知候选（提交前再次核对当前政策与免费额度）：

| 计划 | 面向 | 备注 |
| --- | --- | --- |
| SignPath Foundation | 开源项目 | 需公开仓库 + OSI 许可；证书为组织验证（OV）级 |
| Azure Trusted Signing | 企业（含小团队） | 每月含免费额度，但注册需要企业/身份信息，个人身份是否可用需申请时确认 |
| Certum 开源开发者计划 | 开源项目 | 历史上提供免费 OV 证书，需提交项目材料审核 |

> 只申请**免费**通道。任何要求付费、购买硬件令牌或购买开发者账号的计划一律放弃
> （红线 R5），并在本文件记录放弃原因。

## 2. 申请材料清单（可直接照填）

| 项 | 内容 |
| --- | --- |
| 项目名称 | ForgeDesk（描述语：**ForgeDesk — A Git client**） |
| 项目主页 | <https://forgedesk.pages.dev/> |
| 公共仓库 | <https://github.com/Ember1414/forgedesk> |
| 许可证 | Apache-2.0（仓库根 `LICENSE`，SPDX 头见各源文件） |
| 构建与发布流程 | GitHub Actions 全自动：打 `v*` tag → 三平台构建 → `SHA256SUMS` + GPG 签名 → GitHub Release → 官网与更新清单（详见 `docs/RELEASE.md`） |
| 产物分发方式 | GitHub Releases（主渠道）+ Cloudflare Pages 官网 + Scoop/Winget（包管理器清单见 `packaging/`） |
| 维护者信息 | **占位：由申请人填写真实姓名/邮箱/所在地**（多数计划要求签署人身份可核验） |
| 是否接受源码审计 | 是。全部源码公开；CI 门禁与依赖许可审计（`pnpm compliance`、`docs/LICENSE-AUDIT.md`）可复现 |
| 为什么需要签名 | 见 §3 文案草稿 |
| 预期用户规模 | 见 §3 文案草稿（发布后 12 个月目标：全平台下载 50,000 次；来源 `docs/PLAN.md` §1 指标表） |

## 3. 英文申请文案草稿（可直接粘贴，按需微调）

> 说明：文案中的 "ForgeDesk — A Git client" 是唯一允许的产品名表述；不得出现
> "GitHub 官方""兼容 Git 官方"等暗示关联的句子（红线 R4）。

```text
Project: ForgeDesk — A Git client
Homepage: https://forgedesk.pages.dev/
Repository: https://github.com/Ember1414/forgedesk
License: Apache-2.0

ForgeDesk is a free, open-source, cross-platform desktop workbench for Git.
It turns complex Git operations into visual flows and guided steps, previews and
snapshots destructive actions before they run, and keeps all data local: no
telemetry, no accounts, no AI features. The application is built with Rust
(Tauri 2) and React, and it runs on Windows and macOS.

We distribute installers through GitHub Releases and our website. Because the
project has zero budget, it does not use a paid code-signing certificate, and
our users face "Windows protected your PC" SmartScreen warnings on every
install. We mitigate this by publishing SHA256 checksums (GPG-signed) and by
shipping portable builds, but the warning remains the single biggest source of
installation drop-off reported to us.

We would like to apply for a free code-signing certificate for open-source
projects. Our build and release pipeline is fully automated and reproducible
(GitHub Actions, pinned action SHAs, all sources public), and we are happy to
integrate any signing service that works from CI as well as to undergo source
verification.

Expected reach: we target 50,000 downloads across platforms in the first 12
months after release. The codebase is actively maintained and our release
process, checksums and signatures are documented in the repository
(docs/RELEASE.md, docs/install/).
```

## 4. 被要求提供技术细节时的现成答案

| 问题 | 回答 |
| --- | --- |
| 如何证明仓库归属？ | 维护者 GitHub 账号对仓库有 admin 权限；可在 README 临时放置申请方指定的凭据串（按对方要求操作后移除） |
| 构建是否可复现？ | 是。`release.yml` 全程公开：环境、命令、产物归一化与校验和生成脚本都在仓库内；所有 action 固定到 commit SHA |
| 是否含第三方二进制？ | 不含闭源二进制；依赖清单与许可审计见 `docs/LICENSE-AUDIT.md`（CI 强制同步） |
| 签名放哪一步？ | Release 流水线构建完成后签名（Windows：`.exe` / `.msi`；如支持 macOS 另加 `codesign` 步骤），失败即整条流水线失败（不允许未签名产物静默发布） |
| 证书到期后？ | 按 `docs/SIGNING.md` 的轮换约定处理；签名公钥与验证方法随 `docs/install/windows.md` 发布 |

## 5. 提交后的动作（代理可以接着做）

1. 拿到证书/服务凭据后：按服务商要求接入 `release.yml`（凭据一律放 Actions Secrets，绝不入仓）。
2. 更新 `docs/SIGNING.md` 与 `docs/install/windows.md`：从"未签名，请校验哈希"改为"已由 <组织> 签名"。
3. 在下载页补一行签名验证命令（`signtool verify /pa /v`）。
4. 把本节结论写进 `docs/acceptance/M8.md`。

## 6. 若申请被拒

- 记录拒绝原因与日期（本文件追加一节），继续走免费替代：Scoop/Winget 渠道（来源可信）、
  便携版（不经安装器）、SHA256+GPG 校验，以及随时间积累 SmartScreen 声誉。
- **不为签名付费**（红线 R5）；不购买证书、不使用付费商店。
