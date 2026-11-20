# Scoop manifest（`forgedesk.json`）

面向 **Scoop（Windows 包管理器）** 的清单。这是真相源；发版后由
`scripts/ci/update-packaging.mjs` 更新 `version` 与 `hash`，再同步到实际被
`scoop` 读取的 bucket 仓库。

## 1. 两种落地方式

| 方式                                 | 仓库                                   | 状态       | 说明                                                                                                |
| ------------------------------------ | -------------------------------------- | ---------- | --------------------------------------------------------------------------------------------------- |
| 自有 bucket（推荐先做）              | `Ember1414/scoop-bucket`（需人类创建） | ⏳ 待创建  | 完全可控，无需审核；用户执行 `scoop bucket add forgedesk https://github.com/Ember1414/scoop-bucket` |
| 官方 bucket（`ScoopInstaller/Main`） | 第三方仓库                             | ⏳ 待提 PR | 收录面最大；需人类提 PR 并接受审核。本 manifest 已按官方惯例编写（见 §4）                           |

> 任何对第三方仓库（含自有 bucket 仓库）的实际推送/PR 都必须由人类执行。
> 代理只维护本目录的清单，并在发版后给出"待提交"的差异。

## 2. 用户怎么装（自有 bucket 上线后）

```powershell
scoop bucket add forgedesk https://github.com/Ember1414/scoop-bucket
scoop install forgedesk
```

## 3. 本机验证（无需 bucket 仓库）

Scoop 支持直接从清单文件安装——这条路径与真实 bucket 安装走同一套
"下载 → 校验 hash → 解压 → 建 shim/快捷方式"流程：

```powershell
scoop install .\packaging\scoop\forgedesk.json
forgedesk --help   # shim 是否可用（GUI 应用会打开窗口）
scoop uninstall forgedesk
```

验证记录见 `docs/acceptance/M8.md`。

## 4. 为什么这样写（应对外部审核）

- **用便携版 zip 而不是安装器**：Scoop 的理念是解压式安装；NSIS/MSI 安装器
  需要额外的安装参数适配，且失去"卸载即删除目录"的可预期性。
- **`hash` 是发布时生成的真实 SHA256**：与 Release 的 `SHA256SUMS` 逐字一致
  （autoupdate 也从那份清单取 hash，不另算一份）。
- **`checkver.github` + `autoupdate`**：官方 bucket 的自动更新机器人
  （Excavator）能自行发现新版本并发 PR。
- **manifest 必须纯英文**：官方 bucket 的 lint 与人工审核都要求英文文案；
  中文说明放在本文件里，不放进 manifest。
- **描述语只写 "A Git client."**：红线 R4 —— 不得把 "Git"/"GitHub" 用作产品名，
  也不得暗示与 Git/GitHub 官方有关联。

## 5. 常见审核拒绝原因与规避

| 拒绝原因                           | 规避                                                                                      |
| ---------------------------------- | ----------------------------------------------------------------------------------------- |
| hash 与下载文件不符                | 本清单的 hash 由 `update-packaging.mjs` 从发布产物生成，并与 `SHA256SUMS` 交叉核对        |
| `description` 含商标或与产品不一致 | 固定 "A Git client."；与 `tauri.conf.json` 的 `shortDescription` 保持同一表述             |
| 找不到 `bin`/`shortcuts` 目标      | 便携版 zip 是扁平结构，`ForgeDesk.exe` 在压缩包根目录（`make-portable.mjs` 保证）         |
| 依赖缺失导致启动失败               | `notes` 里提示需要系统 Git；WebView2 缺失由 NSIS 安装器场景覆盖，Scoop 场景在 README 说明 |

## 6. 更新流程（发版后）

```powershell
node scripts/ci/update-packaging.mjs --sums releases/SHA256SUMS   # 更新 version/hash
node scripts/ci/update-packaging.mjs --check                     # 校验清单与发布一致
```

更新后把 `forgedesk.json` 复制到 `scoop-bucket` 仓库根目录并提交（人类操作）。
