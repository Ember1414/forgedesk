# Homebrew Cask（`forgedesk.rb`）

面向 **Homebrew（macOS）** 的 Cask 清单。真相源在本目录；发版后由
`scripts/ci/update-packaging.mjs` 更新 `version` 与 `sha256`。

## 1. 两条落地路径

| 路径                 | 仓库                                   | 状态       | 说明                                                             |
| -------------------- | -------------------------------------- | ---------- | ---------------------------------------------------------------- |
| 自有 tap（先做）     | `Ember1414/homebrew-tap`（需人类创建） | ⏳ 待创建  | `brew tap Ember1414/tap && brew install --cask forgedesk`        |
| 官方 `homebrew/cask` | 第三方仓库                             | ⏳ 待提 PR | 收录面最大；需人类提 PR，审核会实际安装验证（**需要 Mac 环境**） |

> 实际推送/PR 必须由人类执行；代理只维护清单并给出待提交差异。

## 2. 用户怎么装（自有 tap 上线后）

```bash
brew tap Ember1414/tap
brew install --cask forgedesk
```

## 3. 关于产物形态（与早期计划的一处偏差）

M8 计划原文写的是"sha256（arm64 与 intel 两个 dmg）"，实际发布采用
**universal dmg**（`ForgeDesk_<版本>_macos_universal.dmg`，`universal-apple-darwin`
单次构建同时含两种架构），因此本 cask 只有一个 `sha256`。
这减少了一个产物的维护面，对用户没有区别（拿到哪个都能跑）。

## 4. 安装时的 Gatekeeper 提示

应用未使用 Apple 付费签名/公证，`brew install --cask` 完成后首次打开仍会看到
Gatekeeper 提示。处理方式见 `docs/install/macos.md`（右键打开 / `xattr -dr`）。
Cask 的 `desc` 与 README 保持同一表述，不承诺"已签名"。

## 5. 常见审核拒绝原因与规避（官方 cask）

| 拒绝原因                              | 规避                                                                      |
| ------------------------------------- | ------------------------------------------------------------------------- |
| `sha256` 与下载文件不符               | hash 由 `update-packaging.mjs` 从发布产物生成，并与 `SHA256SUMS` 交叉核对 |
| `desc` 不规范（结尾句号、含商标）     | 固定 "A Git client."（与 `tauri.conf.json` 的短描述一致）                 |
| `livecheck` 不工作                    | 使用 `strategy :github_latest`，与 Release 的实际 tag 格式（`v*`）匹配    |
| `zap` 路径不完整或不存在              | 路径均以应用的 bundle id（`io.github.ember1414.forgedesk`）为准           |
| 未标注 `auto_updates`（应用自带更新） | 已标注 `auto_updates true`                                                |

## 6. 更新流程（发版后）

```powershell
node scripts/ci/update-packaging.mjs --sums releases/SHA256SUMS
node scripts/ci/update-packaging.mjs --check
```

把更新后的 `forgedesk.rb` 复制到 tap 仓库（人类操作）。若已进官方 cask，
另提"版本更新" PR：只改 `version` 与 `sha256` 两行。

## 7. 本机无法验证的部分（如实记录）

团队当前没有 Mac 真机：`brew install --cask` 与 Gatekeeper 提示的实测
**未执行**，等待真机或用户反馈（与 `docs/acceptance/M7.md` 的既有约定一致）。
可在 Linux 上先跑 `brew audit --cask packaging/homebrew/forgedesk.rb`（语法/字段级检查）
作为最低验证。
