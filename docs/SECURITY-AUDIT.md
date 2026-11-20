# 依赖安全审计记录（Dependency audit）

> 由 `.github/workflows/audit.yml` 持续执行：每日一次 + 依赖清单变更的 PR/push +
> 手动触发。本文件记录**已知例外**与**复查条件**——例外必须在下次审计时仍然成立，
> 否则它就不再是例外。
>
> 相关：[`SECURITY.md`](../SECURITY.md)（漏洞报告流程）、[`LICENSE-AUDIT.md`](./LICENSE-AUDIT.md)
> （许可证审计，自动生成）、`pnpm compliance`（许可证 / 名称 / AI 依赖门禁）。

## 审计口径

| 维度 | 工具 | 门禁 |
| --- | --- | --- |
| Rust 依赖 CVE | `cargo audit`（RustSec advisory-db） | 发现 vulnerability 即失败 |
| npm 依赖 CVE | `pnpm audit --registry=https://registry.npmjs.org/` | `--audit-level high`（low/moderate 只提示） |
| 许可证与名称 | `pnpm compliance` + `cargo-deny` 风格的依赖许可检查 | 见 `docs/LICENSE-AUDIT.md` |

> npm 侧必须显式指定官方 registry：国内镜像（npmmirror 等）没有实现 audit 端点，
> 会报 `ERR_PNPM_AUDIT_ENDPOINT_NOT_EXISTS`（本机实测）。

## 2026-11-20 发布前复查

GitHub Actions 首轮 npm 审计发现 7 high、11 moderate、1 low。高危来自 Vite 8.3.0、
Zod 4.6.5 与 jsdom 的传递依赖 Undici 8.11.0；另有 Vitest mocker 的中危公告。
本次采用同主版本的补丁升级：Vite 8.3.3、Zod 4.6.6、Vitest/coverage-v8 5.0.3，
并将 Undici 约束到 8.12.2。没有通过新增忽略项绕过高危门禁。

影响范围：Vite/Vitest/Undici 用于开发与测试；Zod 参与前端验证，更新后需重新运行单测、E2E 与生产构建。
Zod 的替代方案是逐处手写输入校验，既增加维护成本也不能修复原依赖；因此沿用现有库的修复版。

本机 npm 复查结果：0 high / 0 critical，剩 1 moderate（minimatch，修复需跨主版本，属于开发工具链）。
Rust 继续沿用下述唯一漏洞例外，未增加新的安全例外；GitHub Rust 审计结果以本次 PR 检查为准。

## 历史全量审计（2026-10-09，本机 Windows）

`cargo audit`：**1 个 vulnerability（已知例外，见下）+ 3 个警告**（669 个依赖）。
`pnpm audit`：**无已知漏洞**。

本轮审计同时修掉了两个**可直接修复**的问题：

| 问题 | 处置 |
| --- | --- |
| `serde_yml` 0.0.12 unsound + unmaintained（RUSTSEC-2025-0068，无修复版本，官方建议迁移） | 换成 `serde_norway`（serde_yaml 的维护 fork，底层 libyaml 同样在维护） |
| `git2` 0.20.4 两个 unsound 警告（RUSTSEC-2026-0183 `Remote::list`、RUSTSEC-2026-0184 `BlameHunk`，官方标注 `patched >= 0.21.0`） | 升级到 `git2` 0.21，并按 0.21 的 API 变化（`name()` / `shorthand()` / `symbolic_target()` 等从 `Option` 改为 `Result`）调整 16 处调用 |

## 已知例外（必须随审查复查）

### E-1：`RUSTSEC-2023-0071` — rsa 0.9.10（Marvin Attack，medium 5.9）

- **依赖链**：`octocrab → jsonwebtoken → rsa`（GitHub API 客户端）。
- **为什么不修**：RustSec 标注**无 patched 版本**（rsa crate 至今未修复）。
- **为什么可接受**：
  1. Marvin Attack 针对 **RSA 解密**（PKCS#1 v1.5 decryption）的时序侧信道；
  2. 本应用对该依赖的唯一用法是 GitHub App 风格的 JWT **签名**（sign），
     不调用解密接口；
  3. 攻击模型要求攻击者能对同一密钥反复发起解密并测量高精度时序——
     桌面应用内不存在这个入口。
- **复查触发条件**（满足任一即重新评估，必要时临时移除 `--ignore` 观察）：
  - `jsonwebtoken` / `octocrab` 升级（可能换掉 rsa 依赖）；
  - `rsa` 发布修复版本；
  - 应用新增任何使用 RSA 私钥**解密**的功能（当前没有）。
- **记录位置**：`.github/workflows/audit.yml` 的 `--ignore RUSTSEC-2023-0071`。

### E-2：其余审计警告（不阻断，仅跟踪）

| 条目 | 性质 | 现状与去向 |
| --- | --- | --- |
| `proc-macro-error` unmaintained | 构建期宏依赖 | 由生态上游（如 `clap` 旧链）引入；等上游迁移，无运行时影响 |
| `glib` 0.18.5 unsound | Tauri 的 Linux/GTK 栈 | 修复版本 `>= 0.20`，但需要 Tauri/tauri-runtime-wry 升级联动；跟随 Tauri 上游版本处理 |
| `yoke-derive` yanked | 被撤回的版本 | 依赖锁定结果；下次 `cargo update` 时自然替换 |

## 维护约定

1. **例外只减不增**：新增例外必须在 PR 里给出"为什么不可达/为什么不能修"的论证，
   并在本文件登记（含复查触发条件）。
2. **修复优先于例外**：能升级就升级（本轮 serde_yml / git2 即为例）。
3. **审计结果入档**：每次人工全量审计后更新上文"最近一次全量审计"小节（日期 + 结论）。
