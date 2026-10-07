# ForgeDesk 用户手册

本手册面向使用者，覆盖 M0–M6 已实现的功能。每个页面都只描述**已经能用**的东西，
不把"计划做的"写成"已经有的"。

> 遇到问题先看 [`../FAQ.md`](../FAQ.md) 与 [`../TROUBLESHOOTING.md`](../TROUBLESHOOTING.md)；
> 安装见 [`../install/windows.md`](../install/windows.md)。

## 阅读路径

| 我想… | 看这里 |
| --- | --- |
| 打开 / 克隆 / 初始化一个仓库，查看改动并提交 | [`01-work-with-a-repository.md`](01-work-with-a-repository.md) |
| 看懂提交历史、管理分支与标签、拉取与推送 | [`02-history-branches-sync.md`](02-history-branches-sync.md) |
| 解决冲突、整理历史（rebase）、出错后回滚 | [`03-conflicts-rebase-safety-net.md`](03-conflicts-rebase-safety-net.md) |
| 使用代码托管、终端、编辑器、插件与主题 | [`04-github-and-tools.md`](04-github-and-tools.md) |

## 界面速览

- **顶栏**：仓库切换器（打开/克隆/初始化）、全局搜索入口、账号、更新提示位；
- **侧栏**：仓库（工作区 / 历史 / 分支 / 操作 / 冲突 / 终端 / 插件面板）、集成（代码托管 / 插件）、应用（命令字典 / 设置）；
- **状态栏**：当前后端/前端版本与 IPC 状态、可回滚提示等。

## 两条贯穿始终的原则

1. **破坏性操作都有安全网**：执行前给计划预览并自动打快照，之后可一键回滚。
2. **每一步都能看到等价的 git 命令**：提交等关键操作会展示将要执行的命令，便于学习与核对。
