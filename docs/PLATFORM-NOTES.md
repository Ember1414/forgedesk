# 平台适配层说明（PLATFORM-NOTES）

> 来源：T6.9（平台适配层补齐）。记录每个能力在各平台的实现方式、已知限制与
> workaround。**改平台行为必须同步更新本文件**——这是三平台差异的唯一汇总处。

## 能力矩阵

| 能力 | 模块 | Windows | macOS | Linux |
| --- | --- | --- | --- | --- |
| 日志轮转 | `logging` | 原生（ReadFile 轮转自实现） | 同左 | 同左 |
| 文件监听 | `watcher` | ReadDirectoryChangesW（notify） | FSEvents | inotify（有上限，见下） |
| 凭据存储 | `crates/credentials` | Credential Manager | Keychain | Secret Service → 加密文件回退（T2.7） |
| 路径比较 | `path_normalizer` | 大小写折叠 + 长路径前缀剥离 | NFC 归一化 | 原样（敏感） |
| Shell 解析 | `shell_resolver` | pwsh/powershell/cmd/Git Bash | zsh/bash/pwsh | 同左 |
| 系统集成 | `system_integration` + `shell` | explorer / `start` / HKCU Run | open / LaunchAgent | xdg-open / XDG autostart |
| 子进程窗口 | `subprocess` | `CREATE_NO_WINDOW`（见下） | 不适用（无控制台窗口概念） | 不适用 |
| 通知 | `notify` | **日志兜底**（真实 Toast 随 T7.1 落地，需要安装器注册 AUMID） | 同左 | 同左 |
| 无图形检测 | `platform_checks` | 不适用 | 不适用 | DISPLAY + WAYLAND_DISPLAY 全空 → 明确报错 |

## 各平台已知限制与 workaround

### Windows

- **MAX_PATH（260 字符）**：接近上限的仓库路径在 git 子进程里可能失败。
  应用内比较走 `PathNormalizer`；超长路径在应用自身的文件操作上加 `\\?\`
  前缀（`ensure_extended_length`，UNC 用 `\\?\UNC\`）。注意加前缀会关闭
  `..` 与 `/` 的规范化，只对"接近 MAX_PATH"的路径启用（阈值 240）。
- **Git Bash 识别**：`C:\Windows\System32\bash.exe` 多半是 WSL，绝不能当
  Git Bash。判据：bash.exe 上层目录存在 `cmd\git.exe`（Git for Windows
  安装器固定生成）。`<Git>\bin` 与 `<Git>\usr\bin` 两种布局都支持。
- **开机自启**：写 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`，
  值为带引号的 exe 路径。仅当前用户，无 UAC。真实注册表行为不在单测覆盖
  （不污染开发机），T6.9 手工验证：启用 → 重启 → 应用自启 → 关闭开关 → 重启不自启。
- **默认应用打开**：`explorer <file>` 行为不可靠，走 `cmd /c start "" <file>`；
  空标题占位符不可省（含空格路径会被当成标题）。
- **Toast 通知（待 T7）**：未打包（便携版）场景没有 AUMID，系统 Toast 不可用；
  T7.1 打包器注册开始菜单快捷方式后才有 AUMID，便携版走 fallback（应用内提醒）。
- **控制台子进程会闪黑框**（2026-10-08 修复）：发布构建是
  `windows_subsystem = "windows"`（进程**没有**控制台）。此时启动一个控制台程序
  （`git.exe` / `ssh.exe` / `gpg.exe`）会让 Windows 为它**新建一个控制台窗口**并在
  屏幕上闪一下——"打开仓库"一次要跑十几条 git 命令，用户看到的就是连续的黑框闪烁，
  同时创建/销毁控制台窗口本身也拖慢整机观感。
  统一在 `forgedesk-platform::subprocess` 里用 `CREATE_NO_WINDOW` 抑制：
  子进程照样拿到标准句柄（我们的管道读写不受影响），只是没有窗口。
  调用点：`git-engine` 的 `GitProcess`（所有 git 调用）、`commands` 的 ssh / gpg。
  **不适用**：终端页的 PTY（它就是给用户的终端，弹窗即功能）、以及
  `explorer` / `open` / `xdg-open` 这类 GUI 程序（它们本来就没有控制台）。
  开发构建（debug）有控制台，子进程继承它，因此**这个问题在 dev 下看不到**。

### macOS

- **Unicode NFD**：APFS/HFS+ 以 NFD 保存文件名，程序内部通常写 NFC。
  一切路径比较必须先 NFC 归一化（`PathNormalizer::macos()` 已内置），
  否则"同一个文件"会静默比较失败。
- **Gatekeeper 隔离**：未公证构建首次打开需右键 → 打开（M8 首启引导负责检测与指引）。
- **开机自启**：LaunchAgent plist（`RunAtLoad`）。签名/沙盒变化后路径不变，
  不需要额外权限。

### Linux

- **inotify 上限**：`fs.inotify.max_user_watches`（常见默认 8192～65536）与
  `max_user_instances`（128）。大仓库（数万目录）可能超出。`watcher` 溢出时
  已自动降级为轮询（T1.10）；`platform_checks::ensure_inotify_capacity`
  供"打开仓库时预警"使用。用户侧 workaround：
  `sudo sysctl fs.inotify.max_user_watches=524288`。
- **Secret Service 缺失**（无 gnome-keyring/kwallet 的窗口管理器裸机）：
  凭据自动回退到加密文件保险库（T2.7，口令由用户设置，存 keyring 失败时
  提示设置口令）。这是回退而不是降级——功能完整，安全性由口令强度决定。
- **无图形环境**：DISPLAY 与 WAYLAND_DISPLAY 都为空时（SSH/容器），
  启动 GUI 前用 `headless_error()` 给出明确指引，不闪退。
- **xdg-open 依赖 xdg-utils**：极简环境可能缺失，失败时错误里带可手动
  打开的路径（`open_in_file_manager` / `reveal_in_file_manager` 均如此）。
- **文件管理器"选中"无统一协议**：`reveal_in_file_manager` 在 Linux 退化为
  打开所在目录（无 select）。Windows（explorer /select）与 macOS（open -R）
  支持真选中。

## 测试约定

- 纯逻辑（路径比较、shell 分类、plist/desktop 内容、/proc 解析）全部单测。
- 触碰真实系统状态的能力（注册表、LaunchAgent 目录、通知）只测内容生成
  纯函数与可注入实现；真实链路由 T6.9 手工验证 + M7 发布检查清单覆盖。
- 跨平台条件测试：`#[cfg(windows)]` 模块内测试只在 Windows 跑，其余平台
  不编译不执行（CI 矩阵会覆盖三平台）。
