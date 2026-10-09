# 质量与风险审查（2026-10-09）

> 一次发布前的系统性风险审查：三路只读扫描（代码层 / 前端交互与测试缺口 / 发布与供应链）
> 加上本机真实审计（`cargo audit` / `pnpm audit` / 故障注入）。
> 本文件记录**已修复项**、**已知例外**与**待处理项**——待处理项不是"已知而不管"，
> 每条都写清了影响与建议动作，供排期。

## 1. 结论速览

| 分类 | 数量 | 说明 |
| --- | --- | --- |
| 已修复（本轮） | 14 项 | 含 2 个"会导致全量用户断更"的发布链路缺陷、4 个依赖告警（含 2 个直接依赖） |
| 已知例外（接受并跟踪） | 1 项 | `RUSTSEC-2023-0071`（rsa），理由与复查条件见 [`SECURITY-AUDIT.md`](./SECURITY-AUDIT.md) |
| 待处理（需人类决策 / 排期） | 8 项 | 见 §4；其中 3 项涉及红线的语义边界，建议人类明确表态 |

**最重要的两条**：`pages.yml` 单独部署站点会**抹掉线上全部更新清单**（所有客户端更新失败）、
跨渠道保留文件名只列了 3/7（`windows.json` / `darwin*.json` 被删）。两条都只在"改站点或发版时"触发，
因此此前从未暴露——本轮已修复并有故障注入测试。

## 1.1 第二轮执行进展（2026-10-09 晚）

按 §4 的推荐逐项落地，本轮完成：

- ✅ **P1-1（部分）**：提交与合并成功后若 `snapshotId` 为 `null`，界面明确提示「这次操作没有回滚点」——
  后端「快照失败不阻断」的取舍**保留**（快照不可用不该拦住用户提交），但不再是静默的。
  pull / stash / force 切换是同一条模式（响应同样带 `snapshotId`），待补。
- ✅ **P2-5**：wrangler 固定到精确版本 `4.148.0`（原为 `@4` 浮动大版本）。
  曾评估放入 devDependencies，实测会引入需要 postinstall 的 esbuild/workerd，并让 pnpm 往
  `pnpm-workspace.yaml` 写入无效占位符 → 采用「精确版本 + npx」（与仓库既有注释的取舍一致）。
- ✅ **P2-6**：新增 `check:packaging` 脚本；发布流程改为「写入清单后立即 `--check` 自校验」。
- ✅ **P2-11**：`e2e/graph-bench.spec.ts` 补 `window.__errs` 断言（唯一不断言的生产场景恰是数万节点）。

仍未做（按建议顺序保留）：**P1-2**（终端高危命令的「不在保护内」提示）、**P1-3**（冲突"以删除解决"预快照）、
**P1-4**（清单 / `.sig` 归档——建议与下一次真实发布一起改并验证）、**P2-7**（覆盖率阈值）、
**P2-8**（动态 i18n key 的存在性检查）、**P2-9**（radiogroup 统一）、**P2-10**（虚拟列表键盘导航）。

## 2. 已修复项（含验证证据）

### 2.1 发布链路（最高危）

| # | 问题 | 处置 | 验证 |
| --- | --- | --- | --- |
| 1 | `pages.yml` 直接部署 `site/`：Pages 是**目录快照**，而站点与更新清单同项目 → 任何 `site/**` 变更都会删掉 `updates/**`，所有客户端更新检查失败 | 新增 `scripts/ci/stage-pages.mjs`：先把线上清单拉回 staging 再部署；`pages.yml` 与 `release.yml` 共用同一脚本 | 对**真实线上站点**跑脚本：拉回 7 个文件（含旧实现会漏掉的 `windows.json` / `darwin*.json`）；故障注入（`--origin http://127.0.0.1:9`）→ 脚本以非零码**中止部署**而不是删除 |
| 2 | 跨渠道保留文件名只列 3/7：stable 发布后 beta 的 macOS/Windows 别名清单被删（beta 用户断更） | 文件清单收口到脚本的 `CHANNEL_FILES`（7 个，逐项注明来源） | 同上实测 |
| 3 | `curl` 失败与"文件不存在"被合并处理，默认动作是**删除**（fail-open） | 脚本区分 `ok / missing / abort`：网络失败 / 5xx / 超时 / 内容校验不过一律中止；`missing` 只用于确定的 404 与 HTML 兜底 | 故障注入实测（见 #1）；JSON 清单还会做可解析性校验 |
| 4 | 本地 `site/updates/<渠道>/**` 残留会随快照发布（实测有 v1.0.0 的旧文件） | 组装时排除渠道目录，只保留 `gpg-pubkey.asc` 等正经产物 | 场景 3 实测：staging 中只剩预期文件 |
| 5 | 清单 PR 基于 **tag commit** 创建 → 合并时可能把 main 的 `packaging/` 回退；重跑同一 tag 时推送失败且静默 | 改为基于 `origin/main` 重放 patch；幂等推送（存在则 `--force-with-lease`）；失败写入 step summary；PR 正文注明"GITHUB_TOKEN 创建的 PR 不触发 CI" | `pnpm check:workflows` 通过（该链路需下次真实发布验证） |
| 6 | `update-packaging.mjs` 写入模式对"规则未命中"静默跳过 → 可能产出"版本更新了但 hash 是旧的" | 两阶段执行：先扫描全部渠道，任何必须命中的规则未命中即**整体中止且不写任何文件**；`--check` 缺产物默认失败（`--allow-partial` 显式放行） | 破坏性实测：给 hash 行加尾注 → 写入模式 exit 1 且**未写任何文件**（winget 的假版本保持原样）；修复后 `--check` 全绿 |
| 7 | Scoop 的 URL 规则依赖"文件里第一次出现"（重排 JSON 就可能改错 autoupdate 段） | 锚定 `"url"` 键并排除 `$version` 占位符 | 实测：把 url 改回 v1.1.2 → 脚本只改真实 url，`autoupdate` 的 `$version` / `$baseurl` 原样保留 |
| 8 | `RELEASE.md` §6 的回滚手段（"把清单指回旧版本"）对已升级用户**无效**（客户端不接受降级），且旧清单/`.sig` 无留存 | 重写为三档（发补丁版 / 删 Release 重跑同一 tag / 公告 + 手动安装 + Pages 部署回滚），写明恢复材料来自 Pages 部署历史；§8 的 `.asc` 改为条件项 | `pnpm check:docs` 通过 |

### 2.2 依赖与供应链

| # | 问题 | 处置 | 验证 |
| --- | --- | --- | --- |
| 9 | `serde_yml` 0.0.12（直接依赖）：unsound + unmaintained（RUSTSEC-2025-0068，**无修复版本**，官方建议迁移） | 换成 `serde_norway`（serde_yaml 的维护 fork，底层 libyaml 同样维护） | `cargo test -p forgedesk-diagnostics` 全绿；审计中该警告消失 |
| 10 | `git2` 0.20.4：两个 unsound 警告（RUSTSEC-2026-0183 / 0184，官方标注 `patched >= 0.21.0`） | 升级 `git2` 0.21，并按新 API 调整 16 处（`name()` / `shorthand()` / `symbolic_target()` / `StringArray::Iter` 从 `Option` 改为 `Result`） | `cargo test -p forgedesk-git-engine -p forgedesk-diagnostics` 全绿（171 + 套件）；`clippy -D warnings` 通过 |
| 11 | `SECURITY.md` 声称"依赖漏洞扫描随 CI 执行"，实际**从未落地**（历史上那 2 个 high 是人工本地跑出来的） | 新增 `.github/workflows/audit.yml`（每日 + 依赖变更 PR/push + 手动）：`cargo audit` + `pnpm audit --audit-level high`（显式官方 registry——国内镜像没有 audit 端点） | 本机实跑：`pnpm audit` 无已知漏洞；`cargo audit` 修复后剩 1 vulnerability（已登记例外）+ 3 warnings（原 7 个） |
| 12 | 依赖审计无记录、例外无复查条件 | 新增 [`SECURITY-AUDIT.md`](./SECURITY-AUDIT.md)：审计口径、最近结果、E-1（rsa / Marvin Attack）的**不可达论证**与复查触发条件、E-2 跟踪表 | `pnpm check:docs` 通过 |

### 2.3 前端交互（"静默失败"与跨仓库状态）

| # | 问题 | 处置 | 验证 |
| --- | --- | --- | --- |
| 13 | 命令面板的「刷新状态」「拉取」失败时被 `.catch(() => {})` 吞掉 → 无 toast、无 `window.__errs`，用户看到的是"点了没反应" | 两处改为 `.catch(show)`（统一错误提示） | `pnpm lint`（修掉一处 exhaustive-deps 警告）/ `typecheck` 通过；929 前端测试全绿 |
| 14 | 快捷键录制控件：`<kbd onClick>` 键盘不可达；录制监听加在事件处理器里且只在"再按一次键"后移除 → 点录制后切走页面会**吞掉全局按键** | 录制监听搬进 `useEffect`（cleanup 保证卸载/结束都摘监听）；`kbd` 换成 `<button type="button">` 并补 `aria-label`（新增 i18n key，中英同步） | i18n 1915 key 一致性通过；快捷键页相关测试通过 |
| 15 | 切换仓库后 `graphSelectionStore` 保留旧仓库的选中集/锚点/详情/比较基准 → 旧 oid 可能被送进新仓库的 cherry-pick / rebase 入口 | 新增 `resetForRepoSwitch()`（清 oid 相关状态、保留缩放/视图偏好）；`HistoryPage` 在 `repoId` 变化时调用 | 新增 store 单测（清空 + 保留视图偏好） |

## 3. 已知例外

见 [`SECURITY-AUDIT.md`](./SECURITY-AUDIT.md) §已知例外：`RUSTSEC-2023-0071`（rsa 0.9.10，
Marvin Attack，**无修复版本**；依赖链 `octocrab → jsonwebtoken → rsa`，仅用于 JWT **签名**，
不调用解密路径 → 判定不可达；复查触发条件已写明）。

## 4. 待处理项（按优先级）

### P1（建议人类表态或尽快排期）

| # | 事项 | 影响 | 建议 |
| --- | --- | --- | --- |
| 1 | **快照/审计失败不阻断破坏性操作**（`services` 层的设计取舍：注释写明"它是安全网，不是闸门"） | 与红线 R7"破坏性操作必须可回滚"存在语义缝隙：快照写不出来时操作照做，用户失去回滚点 | 人类决策三选一：① 保持现状但把"快照失败"提升为**显式警告**（界面告知"本次操作没有安全网"）；② 快照失败要求二次确认；③ 快照失败直接中止 |
| 2 | **终端里的破坏性命令不可回滚**（只做事后审计 + 可选 3 秒延迟补偿快照） | 用户在终端里 `reset --hard` 后无法一键回滚 | 至少在终端里对高危命令显示"本次操作不在快照保护内"的提示；或把补偿快照默认打开 |
| 3 | **冲突"以删除解决"不经预快照**（文件内容删除，仅审计留痕） | 单文件内容丢失不可回滚 | 与 #1 一并决策 |
| 4 | `.sig` 不进 Release 附件；旧版本清单被同名覆盖 → **回滚材料只有 Pages 部署历史**（7 天外的 artifact 已过期） | 严重事故时取旧清单/旧签名要靠 Pages 控制台 | 把 `updates/<渠道>/v<版本>/` 作为归档一并部署（一次改动，永久解决）；`RELEASE.md` §6 已写明现状与该项 |

### P2（可排期）

| # | 事项 | 建议 |
| --- | --- | --- |
| 5 | `npx --yes wrangler@4` 在**带写权限**的发布进程里执行未固定版本的第三方代码 | 把 `wrangler` 放进 `devDependencies`（由 lockfile 固定 integrity），workflow 改 `pnpm exec wrangler` |
| 6 | `update-packaging.mjs --check` 未接入任何 CI（清单漂移在 CI 里不可见） | 加 `check:packaging` 脚本；在 release dry-run 或发布后校验步骤里调用 |
| 7 | 前端覆盖率没有阈值（`vite.config.ts` 的 coverage 只有 include/exclude，与 AGENTS 的"关键模块 ≥ 70%"不符） | 为 `src/features/commands/**`、`src/stores/**`、`src/features/settings/**` 加 per-glob thresholds |
| 8 | 约 60 处动态 i18n key（`t(\`errors.${code}.title\`)` 等）无法被 key 存在性检查覆盖 | 给检查脚本加"前缀展开"：对 `errors.*` / `plugins.*` / `diag.*` 三个由后端驱动的族逐项断言 |
| 9 | 5 处手搓 `role="radiogroup"`（`aria-pressed` 的 Button 冒充 radio） | 统一改用 `src/ui/components/radio-group.tsx`（Radix）；顺带引入 `vitest-axe` 覆盖对话框类组件 |
| 10 | 键盘导航缺口：虚拟列表（万行 status）不可键盘滚动、命令面板焦点离开输入框后 ↑↓ 失效 | 参照 `GraphCanvas` 的 `aria-activedescendant` 模式补齐 |
| 11 | `e2e/graph-bench.spec.ts` 是唯一不断言 `window.__errs` 的 e2e（恰是数万节点场景） | 补断言；顺带把"5 万节点首屏 < 1.5s"落成数值断言（先记录现状再定阈值） |

## 5. 如何复现本轮审查

```bash
# 依赖审计（Rust）：本机无法 fetch advisory-db 时用 HTTP 下载 + --no-fetch
cargo audit --ignore RUSTSEC-2023-0071            # 正常路径
# npm 侧必须显式官方 registry（国内镜像无 audit 端点）
pnpm audit --audit-level high --registry=https://registry.npmjs.org/

# 部署组装的两个关键场景
node scripts/ci/stage-pages.mjs --out .tmp-staging            # 对线上拉回（应保留 7 个文件）
node scripts/ci/stage-pages.mjs --out .tmp --origin http://127.0.0.1:9   # 故障注入（应中止）

# 清单脚本的破坏性场景
node scripts/ci/update-packaging.mjs --sums releases/SHA256SUMS        # 写入（规则未命中即整体中止）
node scripts/ci/update-packaging.mjs --check --sums releases/SHA256SUMS # 校验（缺产物默认失败）
```
