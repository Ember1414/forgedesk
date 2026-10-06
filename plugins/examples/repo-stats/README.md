# 仓库统计面板（示例插件）

sidebar 面板：最近 30 天提交数、作者分布表、按小时文本条形图、当前分支。权限 git:read + ui:panel。

## 安装

设置 → 插件 → 开发者模式：输入本目录路径（`plugins/examples/repo-stats`）安装；
首次启用时逐项授权。目录含清单 `plugin.json` 与产物 `plugin.wasm`。

## 构建（源码即文档）

源码在 `plugins/repo-stats`（本仓库 plugins workspace）。改动后：

```bash
node scripts/build-plugins.mjs   # 构建全部示例并同步 plugin.wasm
```

产物入库由 CI 校验同步（重新编译后 git diff 必须为空）。
