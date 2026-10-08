# 在 macOS 上安装 ForgeDesk

> 适用：macOS 10.15（Catalina）或更高；**universal 构建**——同一份 `.dmg` 同时支持
> Apple Silicon（M 系列）与 Intel，不需要选择架构。
> 产物命名：`ForgeDesk_<版本>_macos_universal.dmg`（更新包是 `…_macos_universal.app.tar.gz`）。

## 1. 下载并校验（1 分钟）

从[下载页](https://forgedesk.pages.dev/download)或
[Releases](https://github.com/Ember1414/forgedesk/releases) 取 `.dmg`，与同一次发布的
`SHA256SUMS` 放在同一目录，然后：

```bash
shasum -a 256 ForgeDesk_<版本>_macos_universal.dmg
# 与 SHA256SUMS 里同名的那一行逐字比对（大小写不敏感）
```

有 GPG 签名（`SHA256SUMS.asc`）时可以再验一层：

```bash
# 公钥：仓库 docs/keys/forgedesk-release.pub 或站点 /updates/gpg-pubkey.asc
gpg --import forgedesk-release.pub
gpg --verify SHA256SUMS.asc SHA256SUMS
```

## 2. 安装

打开 `.dmg`，把 **ForgeDesk** 拖进「应用程序」（或直接双击运行）。

## 3. 首次打开被拦住了？（正常现象）

本项目**不购买付费的 Apple Developer 证书、不做公证**（零成本分发原则，见
[`../RELEASE.md`](../RELEASE.md) 的红线 R5）。因此首次打开会出现下面两种提示之一——它们说明的是
"没有付费签名"，不是"这个应用有问题"。上一步的校验和/签名验证才是真正的可信依据。

### 情况 A：提示"无法验证开发者"或"来自身份不明的开发者"

任选一种：

- **右键打开（推荐）**：在「应用程序」里**右键**（或按住 Control 点击）ForgeDesk → 「打开」→
  再点一次「打开」。之后双击即可正常启动。
- **系统设置**：打开应用一次让它被拦下 → 系统设置 → 隐私与安全性 → 在底部找到被拦下的
  ForgeDesk → 点「仍要打开」。

### 情况 B：提示"**已损坏**，无法打开"（Apple Silicon 上更常见）

这是 Gatekeeper 给未公证应用的**隔离标记**造成的，去掉标记即可：

```bash
xattr -dr com.apple.quarantine /Applications/ForgeDesk.app
```

然后再双击打开（若仍被拦，按情况 A 的右键流程放行一次）。

> 为什么要用户自己动手：付费证书 + 公证每年 $99，与项目的零成本原则冲突。
> 换成"把校验和与签名做扎实、把步骤写清楚"，用户仍然能验证拿到的是不是我们发布的产物。

## 4. 校验安装结果（可选但推荐）

```bash
# 确认应用带 ad-hoc 签名（Apple Silicon 上必需）
codesign -dv --verbose=2 /Applications/ForgeDesk.app
```

## 5. 自动更新

应用内会自动检查更新（设置里可关闭或切到 beta 渠道）。更新包经过签名校验，
**校验失败直接拒绝安装**，没有"跳过校验"的降级路径。

> 首版（v1.0.0）之前没有更旧的已发布版本，因此"旧版本 → 新版本"的端到端升级要等下一次发布才会被真实覆盖；
> 相关演练记录见 [`../acceptance/M7.md`](../acceptance/M7.md) 与 [`../RELEASE.md`](../RELEASE.md) §6。

## 6. 卸载

把「应用程序」里的 ForgeDesk 拖到废纸篓即可。用户数据在
`~/Library/Application Support/io.github.ember1414.forgedesk/`（数据库、快照、日志），
如需彻底清理一并删除。

## 遇到问题

- [`../TROUBLESHOOTING.md`](../TROUBLESHOOTING.md)（含"应用打不开""权限被拒"等条目）
- [`../FAQ.md`](../FAQ.md)
- [Issues](https://github.com/Ember1414/forgedesk/issues)
