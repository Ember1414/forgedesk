# 依赖安全审计记录（Dependency audit）

> 由 `.github/workflows/audit.yml` 执行：依赖清单变更的 PR/push + 手动触发。
> 每日定时触发已于 2026-11-28 关闭；需要时可从 GitHub Actions 页面手动运行。
> 本文件记录**已知例外**与**复查条件**——例外必须在下次审计时仍然成立，
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

## 2026-11-27 定时审计失败复查

默认分支自 2026-11-22 起连续六次定时运行失败：首次失败的
[运行 45023423329](https://github.com/Ember1414/forgedesk/actions/runs/45023423329)
查出 `url` 的一条漏洞，最新的
[运行 46011571943](https://github.com/Ember1414/forgedesk/actions/runs/46011571943)
新增 `cookie` 漏洞。npm 审计均通过 high 门禁，阻断来自 Rust 审计。

| 依赖 | 公告 | 问题 | 修复 |
| --- | --- | --- | --- |
| `url` 2.5.8 | [RUSTSEC-2026-0592](https://rustsec.org/advisories/RUSTSEC-2026-0592) | 嵌套 `blob:` URL 调用 origin 时栈溢出 | 锁定 2.5.9，并按其要求更新 `form_urlencoded` 1.2.2 → 1.2.3 |
| `cookie` 0.18.2 | [RUSTSEC-2026-0594](https://rustsec.org/advisories/RUSTSEC-2026-0594) | fork 后私有 Cookie nonce 可能重复 | 锁定同一 minor 的补丁版本 0.18.3 |

以上均沿用当前依赖链与 API；不引入替代库或新的直接依赖。许可证审计随锁文件重新生成，
安装包体积变化没有单独测量。修复使用上游补丁，没有添加 vulnerability 忽略项；
定时触发的后续调整见本节后文。

原工作流曾每日运行；2026-11-28 已按维护者要求关闭定时触发，以免在没有代码变化时发送主动失败通知。
依赖清单变更的 PR/push 和手动运行仍会执行完整审计。未来若有新的依赖漏洞，相关变更的检查或手动运行仍可能失败。

本次修复的本地质量门禁已通过：Vitest 95 个测试文件、946 个测试用例全部通过；
Rust workspace 测试 1,335 个用例全部通过，`cargo fmt`、`cargo clippy -D warnings`、
文档链接、IPC 参数形状、官网自检和合规检查均通过。Rust 漏洞扫描仍以 GitHub 工作流
中的 `cargo-audit` 结果为最终确认，补丁合入默认分支后需检查一次新的定时或手动运行。

## 2026-11-20 发布前复查

GitHub Actions 首轮 npm 审计发现 7 high、11 moderate、1 low。高危来自 Vite 8.3.0、
Zod 4.6.5 与 jsdom 的传递依赖 Undici 8.11.0；另有 Vitest mocker 的中危公告。
本次采用同主版本的补丁升级：Vite 8.3.3、Zod 4.6.6、Vitest/coverage-v8 5.0.3，
并将 Undici 约束到 8.12.2。没有通过新增忽略项绕过高危门禁。

影响范围：Vite/Vitest/Undici 用于开发与测试；Zod 参与前端验证，更新后需重新运行单测、E2E 与生产构建。
Zod 的替代方案是逐处手写输入校验，既增加维护成本也不能修复原依赖；因此沿用现有库的修复版。

本机 npm 复查结果：0 high / 0 critical，剩 1 moderate（minimatch，修复需跨主版本，属于开发工具链）。

随后 Rust 审计发现 22 条 vulnerability。本次升级 `ammonia` 4.2.5、`libsqlite3-sys` 0.39.1
（`rusqlite` 0.41.1）、`libz-sys` 1.1.33、`pulldown-cmark` 0.13.6、`rustls` 0.23.46、
`rustls-webpki` 0.103.17、`webpki-root-certs` 1.0.12；同时更新 `rustix` 1.1.6 与 `tokio` 1.54.1。
SQLite 修复要求 Rust 1.89；结合已有 Tauri 与构建依赖的要求，将 workspace 最低版本声明更正为 1.91，
见 [开发环境](./DEV-ENV.md)。这些更新沿用现有库与存储格式。

两条传递依赖链通过关闭不需要的功能移除：

- `octocrab` 的 JWT 从默认 RustCrypto 改用依赖树中已有的 aws-lc 后端，移除存在时序问题的
  `crypto-bigint` / `rsa` 等依赖；没有新增密码算法或认证方式。
- Tauri updater 关闭仅用于 Tauri 1 压缩包的 `zip` 兼容功能，移除旧 `zip` 依赖。
  ForgeDesk 的 Tauri 2 Windows 原生安装器与 macOS `.app.tar.gz` 更新包继续使用原有安装路径。

相较于自行维护加密库或旧 ZIP 的补丁，这两个选项减少维护面与依赖数；新增传递依赖仅 `untrusted` 0.7.1，
来自 aws-lc 的 JWT 支持。最终安装包体积差异未单独测量。
历史 RSA 忽略项已从工作流删除，Rust 门禁执行不带 `--ignore` 的 `cargo audit`；最终扫描结果见本次发布的 GitHub 检查。

## 历史全量审计（2026-10-09，本机 Windows）

`cargo audit`：**1 个 vulnerability（已知例外，见下）+ 3 个警告**（669 个依赖）。
`pnpm audit`：**无已知漏洞**。

本轮审计同时修掉了两个**可直接修复**的问题：

| 问题 | 处置 |
| --- | --- |
| `serde_yml` 0.0.12 unsound + unmaintained（RUSTSEC-2025-0068，无修复版本，官方建议迁移） | 换成 `serde_norway`（serde_yaml 的维护 fork，底层 libyaml 同样在维护） |
| `git2` 0.20.4 两个 unsound 警告（RUSTSEC-2026-0183 `Remote::list`、RUSTSEC-2026-0184 `BlameHunk`，官方标注 `patched >= 0.21.0`） | 升级到 `git2` 0.21，并按 0.21 的 API 变化（`name()` / `shorthand()` / `symbolic_target()` 等从 `Option` 改为 `Result`）调整 16 处调用 |

## 历史例外与剩余警告

### E-1（已移除）：`RUSTSEC-2023-0071` — rsa 0.9.10（Marvin Attack，medium 5.9）

- **依赖链**：`octocrab → jsonwebtoken → rsa`（GitHub API 客户端）。
- **历史处置**：当时 RustSec 标注无 patched 版本，因此评估为可接受的例外：
  1. Marvin Attack 针对 **RSA 解密**（PKCS#1 v1.5 decryption）的时序侧信道；
  2. 本应用对该依赖的唯一用法是 GitHub App 风格的 JWT **签名**（sign），
     不调用解密接口；
  3. 攻击模型要求攻击者能对同一密钥反复发起解密并测量高精度时序——
     桌面应用内不存在这个入口。
- **2026-11-20 复查**：JWT 改用 aws-lc，锁文件不再包含 `rsa`，工作流移除对应 `--ignore`。
  当前没有 vulnerability 忽略项；将来更换 JWT 后端时仍须检查传递依赖。

### E-2：其余审计警告（不阻断，仅跟踪）

| 条目 | 性质 | 现状与去向 |
| --- | --- | --- |
| `chrono` 0.4.45 unmaintained | 由上游引入的日期库 | 当前无修复升级；业务代码沿用 `time`，跟踪上游迁移 |
| `proc-macro-error` unmaintained | 构建期宏依赖 | 由生态上游（如 `clap` 旧链）引入；等上游迁移，无运行时影响 |
| `glib` 0.18.5 unsound | Tauri 的 Linux/GTK 栈 | 修复版本 `>= 0.20`，但需要 Tauri/tauri-runtime-wry 升级联动；跟随 Tauri 上游版本处理 |
| `yoke-derive` yanked | 被撤回的版本 | 依赖锁定结果；下次 `cargo update` 时自然替换 |
| `wasmi` / `wasmi_core` 2.0.0 yanked | 被撤回的版本 | 暂保留已验证的插件运行时；升级时需重测文档中记录的 Windows dev 优化崩溃 |

## 维护约定

1. **例外只减不增**：新增例外必须在 PR 里给出"为什么不可达/为什么不能修"的论证，
   并在本文件登记（含复查触发条件）。
2. **修复优先于例外**：能升级就升级（本轮 serde_yml / git2 即为例）。
3. **审计结果入档**：每次人工全量审计后更新上文"最近一次全量审计"小节（日期 + 结论）。
