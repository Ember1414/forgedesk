# 故障排查（Troubleshooting）

> 通用第一步：**打开日志**（设置 → 高级 → 日志，或应用数据目录下的 `fd.log`），
> 错误详情与错误码都在里面。日志已脱敏，可放心粘贴到 issue。

---

## Windows

**1. 运行安装器时提示"Windows 已保护你的电脑 / 未知发布者"**
原因：项目采用零成本信任方案，未使用付费代码签名证书。
处理：点击**更多信息 → 仍要运行**；或改用**便携版 zip** / Scoop / Winget。
安装前建议先按 [`install/windows.md`](install/windows.md) 校验 SHA256。

**2. 应用启动后是白屏或直接闪退**
原因：多半是缺少 WebView2 运行时或图形环境异常。
处理：安装 **Microsoft Edge WebView2 Runtime**（Win10 部分版本未预装）；再查看日志中的启动错误。

**3. 终端无法输入 / 打开终端报 `PTY_UNSUPPORTED`**
原因：Windows 10 旧版本 ConPTY 不可用。
处理：升级到较新的 Windows 10/11；应用会降级为"命令流式输出"模式并给出提示。

**4. 仓库路径很长或含中文时操作失败**
原因：Windows 默认路径长度限制（260）或编码问题。
处理：启用长路径支持（组策略 `LongPathsEnabled`），或把仓库移到较短路径。

**5. 杀毒软件误报应用行为**
原因：未签名可执行文件 + 频繁调用 git 子进程。
处理：把应用目录加入白名单；优先使用 Scoop/Winget 安装（来源更可信）。

---

## macOS

**6. 打开应用提示"已损坏，无法打开"**
原因：从网络下载的未公证应用带 quarantine 隔离属性。
处理：`xattr -dr com.apple.quarantine /Applications/ForgeDesk.app`，或右键 → 打开。详见 `install/macos.md`（随 M8 提供）。

**7. 提示"无法验证开发者"**
处理：系统设置 → 隐私与安全性 → **仍要打开**。

---

## Linux

**8. 启动报缺库（WebKitGTK / GTK）**
处理：安装发行版依赖，例如 Debian/Ubuntu：
`sudo apt install libwebkit2gtk-4.1-0 libgtk-3-0 libsecret-1-0`（编译期还需 `-dev` 包）。

**9. 凭据保存报 `KEYRING_UNAVAILABLE`**
原因：无 Secret Service（gnome-keyring / KWallet 未运行）。
处理：启动一个 Secret Service 提供者；否则应用回退为**加密文件保险库**（口令进 keyring），按提示设置口令即可。

**10. 大量文件变更时状态更新缓慢 / 提示 watcher 降级**
原因：`inotify` 监视数上限过低。
处理：提高 `fs.inotify.max_user_watches`；应用会在超限时降级为轮询并提示。

---

## Git 相关

**11. 提示找不到 `git`**
原因：ForgeDesk 以系统 `git` 执行写操作。
处理：安装 Git 并确保它在 `PATH` 中；Windows 可用 Git for Windows 自带的 Git Bash。

**12. 提交报 `unable to auto-detect email address`**
原因：本机未配置提交身份。
处理：`git config --global user.name "..."` 与 `git config --global user.email "..."`。

**13. 提示 `LF will be replaced by CRLF`**
原因：`core.autocrlf` 与行尾差异。这是提示而非错误；如需统一，检查仓库的 `.gitattributes` 与 git 配置。

**14. push 被拒（`non-fast-forward`）**
处理：先 **pull** 同步；确需覆盖时应用提供 **force-with-lease**（会展示远端当前提交）。**不要**使用裸 `--force`。

**15. 处于 `detached HEAD` 状态**
原因：检出了某个提交而非分支。要在其上继续工作请**新建分支**；应用会在状态栏提示。

**16. 终端里 `git rebase -i` 打不开编辑器**
原因：交互式 rebase 需要编辑器，内嵌终端未提供交互编辑器。
处理：改用应用内的**拖拽式 rebase**（可视化、可预览、可回滚）。

---

## 应用与更新

**17. 启动时提示"上次异常退出"**
处理：可选择**安全模式**（禁用插件与终端）排查是否为插件导致；日志中有崩溃留档。

**18. 自动更新提示"签名校验失败"**
处理：**不要**安装该更新包。这通常意味着下载被篡改或清单异常；请到官方发布页手动下载并校验，并上报 issue。

**19. 代理配置后 fetch/push 仍不通**
处理：到设置 → 网络与密钥运行**连通性测试**；确认 `no_proxy` 未把目标主机排除，或代理需要认证。

**20. 插件加载失败 / 面板空白**
处理：到插件管理页查看该插件的**日志**与状态（崩溃/超时会被隔离）；确认已授予清单声明的权限。

**21. 数据库打开或迁移失败**
处理：应用在迁移前会备份数据库；查看日志中的迁移错误，必要时从备份恢复（保留最近 3 份）。

---

## 仍未解决？

请到仓库 issue 提问，并附上：操作系统与版本、ForgeDesk 版本（设置 → 高级 或状态栏）、
复现步骤、以及**已脱敏**的相关日志片段。安全相关问题请走 [`../SECURITY.md`](../SECURITY.md)。
