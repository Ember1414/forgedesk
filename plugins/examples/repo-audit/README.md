# 仓库巡检（示例插件）

repo-tab 面板 + Markdown 报告命令：疑似密钥文件、未忽略构建产物、大小写冲突、提交信息规范符合率（只读）。权限 git:read + fs:read + ui:panel + ui:command。

## 安装

设置 → 插件 → 开发者模式：输入本目录路径（`plugins/examples/repo-audit`）安装；
首次启用时逐项授权。目录含清单 `plugin.json` 与产物 `plugin.wasm`。

## 构建（源码即文档）

源码在 `plugins/repo-audit`（本仓库 plugins workspace）。改动后：

```bash
node scripts/build-plugins.mjs   # 构建全部示例并同步 plugin.wasm
```

产物入库由 CI 校验同步（重新编译后 git diff 必须为空）。
