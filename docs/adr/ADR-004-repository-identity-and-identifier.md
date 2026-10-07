# ADR-004: 仓库归属、提交身份与应用标识符定稿

- 状态：已接受
- 日期：2026-09-23
- 里程碑：M0（首次提交之前，此后修改成本极高）
- 关联：`docs/PLAN.md` §1.5、§15.6 D-02；`docs/OPEN-SOURCE-READINESS.md` §0

## 上下文

首次提交即将发生，以下三项一旦进入历史或发布就无法低成本变更：

1. **git 提交者身份**：会永久嵌入每一个提交对象，仓库公开后任何人可见可检索。
   变更需要重写历史（`filter-repo` / 重建仓库），对已 fork 的仓库更是不可行。
2. **仓库归属**：决定了 issue/PR/CI/发布地址，以及后续包管理器清单里的 URL。
3. **应用标识符（Tauri `identifier`）**：macOS bundle id、Windows 应用 ID、
   自动更新的身份判定、未来上架商店的凭据都基于它。发布后不可更改
   （改了等于变成另一个应用，用户的更新链路会断）。

首次提交前的原始状态：

| 项 | 原始值 | 问题 |
| --- | --- | --- |
| git user.email | 个人常用邮箱（非 GitHub 隐私邮箱） | 会被永久嵌入全部提交对象，公开后可被爬取用于骚扰 |
| git user.name | 与本机 Windows 账户一致的简写 | 过于简略，且与 GitHub 账号名不一致 |
| Cargo.toml repository | `https://github.com/forgedesk/forgedesk` | 占位符，指向不存在的仓库 |
| Tauri identifier | `org.forgedesk.desktop` | 声称持有 `forgedesk.org` 域名，实际并不持有 |

## 决策

| 项 | 定稿值 |
| --- | --- |
| 仓库 | `https://github.com/Ember1414/forgedesk`（私有起步，计划后转公开） |
| 提交身份（name） | `EMBER` |
| 提交身份（email） | `237394828+Ember1414@users.noreply.github.com` |
| Cargo.toml `repository` | `https://github.com/Ember1414/forgedesk` |
| `package.json` repository / homepage / bugs | 同上（见 `package.json`） |
| Tauri `identifier` | `io.github.ember1414.forgedesk` |
| 产品名（不变） | `ForgeDesk`（不含 Git / GitHub 字样，符合红线 R4） |

### 为什么标识符用 `io.github.ember1414.forgedesk`

- **反向 DNS 的本意是"可验证的归属"**。`io.github.<owner>.*` 是被广泛采用的约定，
  用于没有自有域名的开源项目：`github.io` 域下确实存在 `<owner>` 这个命名空间，声明是诚实的。
- 避免声称未持有的域名（`forgedesk.org` / `forgedesk.app`）——那属于商标与域名冒用风险。
- 不依赖任何需要花钱的资产，符合零成本方案。
- 不以 `.app` 结尾，规避 macOS 对该后缀的限制。

**注意**：标识符不随仓库转移或改用户名而变化。即使未来项目迁移到独立组织，
`io.github.ember1414.forgedesk` 也保持不变——这是刻意接受的代价，因为改变标识符的破坏性远大于"名字看起来过时"。

## 备选方案

| 方案 | 优点 | 缺点 | 结论 |
| --- | --- | --- | --- |
| `org.forgedesk.desktop` / `app.forgedesk.*` | 名字好看、与产品名一致 | 声称持有不存在的域名；有冒用风险 | 否 |
| `com.forgedesk.*` | 同上 | 同上，且 `.com` 更敏感 | 否 |
| **`io.github.ember1414.forgedesk`** | 归属诚实、零成本、不依赖域名 | 含个人账号名，未来迁组织时看起来"过时" | **采用** |
| 先用临时标识符，发布前再改 | 灵活 | 极易遗忘；一旦发布即不可改 | 否 |

## 后果

**正面**

- 提交历史中的邮箱为 GitHub 提供的隐私邮箱，不暴露个人真实邮箱。
- 标识符归属清晰，无商标/域名冒用风险，且与仓库地址天然对应。
- 三处（Cargo / npm / Tauri）的仓库地址一致，不存在指向失效占位符的元数据。

**负面**

- 标识符中带有个人账号名；若项目未来迁移到组织，该标识符将显得与仓库地址不一致。
  缓解：在 `docs/BRAND.md` 与本文中记录该决策的理由，避免后人误以为是疏漏。

**需要人类执行（本人按 Git 安全约定不擅自修改 git config）**

```bash
git config user.name  "EMBER"
git config user.email "237394828+Ember1414@users.noreply.github.com"
# 如需对全局生效，加 --global；仅本仓库生效则不加
```

执行后可用以下命令核对：

```bash
git config user.name
git config user.email
```

**首次提交前必须确认**：上述两项已设置；否则第一个提交会带上**个人邮箱**，
而该信息在仓库公开后将永久可见。此处**刻意不复现那个地址**——它正是本清单要挡在外面的东西
（2026-10-07 的 A 组审计发现本文曾回显完整地址，已就地脱敏；历史侧的处置见
`docs/OPEN-SOURCE-READINESS.md` §4）。
