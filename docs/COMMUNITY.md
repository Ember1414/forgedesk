# 社区运营规范（M8 / T8.8）

> 本文是**维护者视角**的操作手册：渠道分工、响应承诺、标签体系与回复模板。
> 面向贡献者的部分请转发 [`CONTRIBUTING.md`](../CONTRIBUTING.md) 与
> [`GOOD-FIRST-ISSUES.md`](./GOOD-FIRST-ISSUES.md)。

## 1. 渠道分工

| 渠道 | 用途 | 状态 |
| --- | --- | --- |
| GitHub Issues | 缺陷、功能请求、插件问题、RFC | ✅ 已启用（模板见 `.github/ISSUE_TEMPLATE/`） |
| GitHub Discussions | 提问、用法分享、想法、公告 | ⏳ **待人类创建分类**（文案见 §2） |
| 即时社区（Discord/Matrix） | — | ❌ **决定不做**：零维护优先（M8-PLAN 方案 A）。所有交流收敛到 Discussions |

## 2. Discussions 分类与欢迎贴（创建文案）

分类（按顺序创建）：

| 分类 | Emoji | 格式 | 说明 |
| --- | --- | --- | --- |
| Announcements | 📣 | Announcement | **仅维护者可发**：版本发布、重要变更 |
| Ideas | 💡 | Open discussion | 功能想法（成熟后转 RFC Issue 或进路线图） |
| Q&A | 🙋 | Question & Answer | 用法提问；答案被采纳后标记 |
| Show and tell | 🛠️ | Open discussion | 展示你的用法、主题、插件 |
| Bug triage | 🐛 | Open discussion | 从 Issue 转来的、暂不能复现或需要更多信息的讨论 |
| Docs | 📚 | Open discussion | 文档纠错与改进提案 |

**欢迎贴（置顶到 Q&A / General）**：

```markdown
# 欢迎来到 ForgeDesk 社区

ForgeDesk 是一个开源、跨平台的 Git 桌面工作台。它的目标：让"怕把仓库搞坏"的人
也能安心用 Git——破坏性操作先预览、自动快照、可一键回滚。

## 怎么提问
- 缺陷请走 Issues（选好模板，附版本号与复现步骤）；
- 用法问题、想法、展示请用对应 Discussions 分类；
- 安全相关**不要**公开发帖，见 SECURITY.md。

## 怎么贡献
- 第一次参与：看标记了 `good first issue` 的任务（有完整引导）；
- 参与前请读 CONTRIBUTING.md 与 CODE_OF_CONDUCT.md；
- 回答别人的问题同样算贡献。

## 响应预期（SLA）
- 普通 Issue / Discussion：**72 小时内首次响应**；
- 涉及数据风险（P0）：**12 小时内**首次响应；
- 维护者是志愿者：SLA 是目标而非保证，超时会说明原因。

## 行为准则
所有空间适用 CODE_OF_CONDUCT.md。请保持友善与耐心。
```

## 3. 响应 SLA 与优先级

| 级别 | 判定 | 首次响应 | 处理目标 |
| --- | --- | --- | --- |
| P0（数据风险） | 可能导致仓库数据损坏/丢失 | ≤ 12 小时 | 立即定位；必要时发补丁版本 |
| P1（功能阻断） | 核心流程不可用、崩溃 | ≤ 48 小时 | 下个补丁版本 |
| P2（一般缺陷） | 功能异常但有绕行方式 | ≤ 72 小时 | 常规排期 |
| P3（建议/提问） | 改进建议、用法问题 | ≤ 72 小时 | 进 Ideas 或直接答复 |

SLA 从"信息完整的提交"起算；缺复现步骤的会先求补充信息（同时告知缺什么）。

## 4. 标签体系

| 前缀 | 取值 | 用途 |
| --- | --- | --- |
| `type/` | `bug` `feat` `docs` `i18n` `tests` `perf` `refactor` | 改动性质 |
| `area/` | `docs` `i18n` `tests` `diagnostics` `themes` `plugins` `perf` `shortcuts` `a11y` `git-core` `site` `packaging` | 代码区域 |
| `priority/` | `p0` `p1` `p2` `p3` | 对应 §3 |
| `status/` | `needs-info` `confirmed` `in-progress` `blocked` `waiting-upstream` | 流转状态 |
| 特殊 | `good first issue` `help wanted` `security` | 聚合与路由 |

> 本仓库目前没有标签定义文件；标签需在 GitHub 上手工创建，
> 命名与 `docs/GOOD-FIRST-ISSUES.md` §3 的说明保持一致。

## 5. 不处理的情况与统一回复

| 情况 | 处理 |
| --- | --- |
| 辱骂/骚扰 | 按 CODE_OF_CONDUCT.md 处理：警告 → 锁贴 → 拉黑。不与之争论 |
| 重复提交 | 合并到已有 Issue 并关闭新开的一条（附原链接） |
| 要求实现"不做清单"内的功能（AI 推理、遥测、付费渠道…） | 引用 `docs/PLAN.md` §4.3 说明边界；不进入排期 |
| 要求我们使用付费服务/证书 | 说明红线 R5（零成本），给出免费替代（已有文档） |
| 纯广告 | 删除并拉黑 |

回复模板库见 [`templates/replies/README.md`](./templates/replies/README.md)（5 个场景）。

## 6. 反馈处理闭环（每周一次，约 30 分钟）

1. **分类**：把上周新 Issue/Discussion 打上标签与优先级（§3/§4）；
2. **升级**：有共识的功能请求 → 转 RFC Issue（模板已有）→ 进路线图讨论；
3. **回复**：所有 `needs-info` 的提交要么补信息、要么关闭（附说明）；
4. **摘要**：每月 1 日汇总上月数据（新 Issue/关闭数/合并 PR/新贡献者/发布数），
   作为 Announcements 发一条月度摘要（数据从 GitHub Insights 与 Release 页取，
   不引入任何第三方统计服务——红线 R6）。

> 月度摘要的自动化工作流（`monthly-digest.yml`）尚未实现：它需要人工确认草稿，
> 当前用手工统计。等社区量级上来再自动化（避免为不存在的数据造轮子）。

## 7. 与发布节奏的配合

- 版本发布后：在 Announcements 发发布贴（要点 + 校验指引 + 升级方式）；
- 出现 P0：先发补丁版本，再补公告与复盘（写进 `CHANGELOG.md`）；
- `good first issue` 的空缺：每季度从 `docs/GOOD-FIRST-ISSUES.md` 补一批。
