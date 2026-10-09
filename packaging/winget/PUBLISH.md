# Winget 提交说明（`microsoft/winget-pkgs`）

> **本文件供人类执行提交时使用**。代理不得自行向第三方仓库提交，
> 也不得声称已提交。这里的所有命令都在**你**的机器上运行。

## 1. 前提

- 一个 GitHub 账号与 `winget`（Windows 11 自带 App Installer 中的 `winget.exe`）。
- 已发布的 Release（清单里的 URL/SHA256 必须指向真实存在的资产）。
- 未提交前可先本地验证（§3），验证不通过就不要提 PR。

## 2. 要提交的四个文件

| 文件                                    | 作用                      |
| --------------------------------------- | ------------------------- |
| `ForgeDesk.ForgeDesk.yaml`              | 版本清单（version）       |
| `ForgeDesk.ForgeDesk.installer.yaml`    | 安装清单（installer）     |
| `ForgeDesk.ForgeDesk.locale.en-US.yaml` | 默认语言（defaultLocale） |
| `ForgeDesk.ForgeDesk.locale.zh-CN.yaml` | 中文语言（locale）        |

目标路径（`PackageIdentifier` 为 `ForgeDesk.ForgeDesk` → `f` / 首段 / 次段 / 版本）：

```text
manifests/f/ForgeDesk/ForgeDesk/1.1.3/
```

## 3. 本地验证（提交前必做）

```powershell
# 1) 清单结构/字段校验（一条命令校验整个目录）
winget validate .\packaging\winget

# 2) 用清单真实安装一次（验证 URL/SHA256/静默安装参数）
winget install --manifest .\packaging\winget

# 3) 升级与卸载（验证 UpgradeBehavior / ARP 条目匹配）
winget upgrade ForgeDesk.ForgeDesk
winget uninstall ForgeDesk.ForgeDesk
```

`winget validate` 通过、且安装/卸载各走通一次之后，再提交。

## 4. 提交步骤

```powershell
# 1) fork microsoft/winget-pkgs（网页操作一次即可），然后：
git clone https://github.com/<你的账号>/winget-pkgs
cd winget-pkgs
git checkout -b forgedesk-1.1.3

# 2) 放入清单
mkdir manifests\f\ForgeDesk\ForgeDesk\1.1.3
copy <ForgeDesk 仓库>\packaging\winget\*.yaml manifests\f\ForgeDesk\ForgeDesk\1.1.3\

# 3) 提交并推送到自己的 fork
git add manifests/f/ForgeDesk/ForgeDesk/1.1.3
git commit -m "New package: ForgeDesk.ForgeDesk version 1.1.3"
git push -u origin forgedesk-1.1.3

# 4) 在网页上发 PR 到 microsoft/winget-pkgs（base: master）
```

也可以直接用 `wingetcreate`（官方工具，自动开 PR）：

```powershell
wingetcreate update ForgeDesk.ForgeDesk --version 1.1.3 `
  --urls https://github.com/Ember1414/forgedesk/releases/download/v1.1.3/ForgeDesk_1.1.3_windows_x64.exe --submit
```

第一次提交建议走手工路径（能看清审核意见对应的文件）。

## 5. 首次提交前需要校准的两处

这两项在真实安装前无法 100% 确认，`winget validate` 也只能查格式：

1. **`AppsAndFeaturesEntries.DisplayName`**：安装一次 `.exe` 后，在
   「设置 → 应用 → 已安装的应用」里核对实际显示名（预期为 `ForgeDesk`），
   不一致时回填此字段，否则 `winget upgrade` 不认识已安装版本。
2. **`Publisher`**：同上位置核对发行者字段（当前 NSIS 未显式配置 `publisher`，
   若实际为空/其他值，按实况修改 locale 文件）。

## 6. 常见审核意见与处理

| 审核意见                        | 处理                                                                                                        |
| ------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| `InstallerSha256` 与资产不符    | 从 Release 的 `SHA256SUMS` 重新取（`node scripts/ci/update-packaging.mjs --check` 可本地比对）              |
| 缺少 `Publisher`/`PublisherUrl` | 补 locale 文件（本目录已提供）                                                                              |
| `Description` 过短或像广告      | 本目录的 Description 已按"事实陈述"写；不足 500 字符时可加安装/依赖说明                                     |
| 安装未被 ARP 记录               | 校准 §5 的两处                                                                                              |
| 与已有包重名                    | `ForgeDesk.ForgeDesk` 已按 `<Publisher>.<Product>` 规则选定；审核若要求改动，同步更新四个文件与 target 目录 |

## 7. 后续版本更新

官方收录后，每次发布新版本向 `microsoft/winget-pkgs` 提一个"版本更新" PR：
只新增 `manifests/f/ForgeDesk/ForgeDesk/<新版本>/` 目录（不改旧版本目录）。

```powershell
wingetcreate update ForgeDesk.ForgeDesk --version <新版本> `
  --urls https://github.com/Ember1414/forgedesk/releases/download/v<新版本>/ForgeDesk_<新版本>_windows_x64.exe --submit
```
