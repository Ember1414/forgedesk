# 在 Windows 上安装 ForgeDesk

> 适用版本：v1.0 及以后（M7 目标平台）。发布前本文件描述的下载地址尚未生效。

ForgeDesk 采用**零成本信任方案**：不使用付费代码签名证书。因此首次运行时
Windows SmartScreen 可能提示"未知发布者"，这是正常现象——**安装前请先用下面第 2 节
的命令校验安装包完整性**，再决定是否运行。

---

## 1. 选择安装方式

| 方式 | 适用 | 说明 |
| --- | --- | --- |
| NSIS 安装器（`.exe`） | 大多数用户 | 双击安装，支持当前用户安装，可静默安装 |
| MSI（`.msi`） | 企业/批量部署 | 支持组策略与静默部署 |
| 便携版 zip | 不想安装 / 无管理员权限 | 解压即用，**最不容易触发 SmartScreen** |
| Scoop / Winget | 习惯包管理器的用户 | 来源更可信，审核由包管理器承担（随 M8 上线） |

下载地址：GitHub Releases 页面（`Ember1414/forgedesk` 的 Releases）。

**便携版 zip 内含**：`ForgeDesk.exe`、`LICENSE`、`README-portable.txt`。
本地自行构建（Windows）：

```bash
pnpm tauri build
pnpm portable:win
# → bundles/ForgeDesk_<版本>_windows_<架构>_portable.zip（同名 .sha256 校验和文件）
```

---

## 2. 校验下载完整性（推荐）

每个 Release 都会附带 `SHA256SUMS`（以及用项目 GPG 公钥签名的 `SHA256SUMS.asc`）。

PowerShell 校验：

```powershell
# 1) 计算你下载的文件的 SHA256
Get-FileHash .\ForgeDesk_1.0.0_x64-setup.exe -Algorithm SHA256

# 2) 与 Release 中的 SHA256SUMS 对应行比对（哈希应完全一致）
```

带 GPG 的完整校验（需要已导入项目公钥）：

```powershell
gpg --verify SHA256SUMS.asc SHA256SUMS
# Windows 无 gpg 时可用 (Get-FileHash ...).Hash 手工比对
```

**签名/校验不通过时请勿安装**，并把情况报告到 issue（见 `SECURITY.md`）。

---

## 3. 处理 SmartScreen 提示

因为未使用付费证书，运行安装器时可能出现：

- "Windows 已保护你的电脑" → 点击**更多信息** → **仍要运行**；
- "来自身份不明的开发者" → 同上。

**降低摩擦的建议**：优先使用**便携版 zip**，或通过 **Scoop / Winget** 安装
（包管理器来源更可信，能绕开部分拦截）。

---

## 4. 首次启动

1. 启动后进入**仪表盘**；
2. 用顶部**仓库切换器 → 添加仓库**打开一个本地仓库目录（或克隆远端）；
3. 若系统未检测到 `git`，请先安装 Git 命令行（ForgeDesk 以系统 `git` 执行写操作）；
4. 需要登录代码托管时，到 **设置 → 代码托管账号** 使用 OAuth 设备码或 PAT。

---

## 5. 卸载

- 安装器版本：**设置 → 应用 → 已安装的应用 → ForgeDesk → 卸载**；
- 便携版：直接删除解压目录；
- 应用数据（设置、SQLite、日志、插件）默认位于
  `%APPDATA%\io.github.ember1414.forgedesk`，卸载器不会自动删除；如需彻底清理请手动删除该目录。

---

## 6. 常见问题

见 [`../TROUBLESHOOTING.md`](../TROUBLESHOOTING.md) 与 [`../FAQ.md`](../FAQ.md)。
