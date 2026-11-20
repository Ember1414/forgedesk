# packaging/ — 包管理器分发清单

本目录是**各免费分发渠道清单的真相源**。发布流水线不会向任何第三方仓库推送；
它只把清单里的 `version` / `url` / `sha256` 与发布产物对齐（`scripts/ci/update-packaging.mjs`），
真正的提交由人类按各子目录的 `PUBLISH.md` / `README.md` 执行。

## 渠道一览

| 渠道                      | 目录                                | 状态                                                            | 依赖                                                                                    |
| ------------------------- | ----------------------------------- | --------------------------------------------------------------- | --------------------------------------------------------------------------------------- |
| Scoop（Windows）          | [`scoop/`](./scoop/README.md)       | ✅ 清单就绪，**已在本机真实验证**（见 `docs/acceptance/M8.md`） | 自有 bucket 仓库待人类创建                                                              |
| Winget（Windows）         | [`winget/`](./winget/PUBLISH.md)    | 📋 清单草稿就绪                                                 | 人类账号提 PR（`microsoft/winget-pkgs`）                                                |
| Homebrew Cask（macOS）    | [`homebrew/`](./homebrew/README.md) | 📋 清单草稿就绪                                                 | 自有 tap 待创建；真机验证需 Mac                                                         |
| Flathub / Snapcraft / AUR | —                                   | ⛔ 暂不做                                                       | Linux 打包链按 [ADR-005](../docs/adr/ADR-005-defer-cross-platform-verification.md) 暂缓 |

## 更新流程（发版后）

```powershell
# 1) 从发布的校验和文件对齐三个渠道的版本与 hash
node scripts/ci/update-packaging.mjs --sums releases/SHA256SUMS

# 2) 校验（不写入）：清单与发布产物不一致时以非零码退出
node scripts/ci/update-packaging.mjs --check --sums releases/SHA256SUMS
```

`release.yml` 的 `publish` 作业在正式发布后会调用同一脚本，并把变更整理成
一个 PR（**仅对 ForgeDesk 自有仓库**；第三方仓库永远由人类操作）。

## 共同约束（红线）

- **R4**：清单一律用 "A Git client." 作为描述语；产品名不包含 "Git"/"GitHub"。
- **R5**：全部渠道零成本；不引入需要付费账号/证书的渠道。
- **R7/R8**：清单中的 hash 必须来自真实发布产物；不写占位 hash。
