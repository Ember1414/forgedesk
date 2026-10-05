# 提交信息模板（示例插件）

三套模板命令（feat/fix/chore），读取分支与变更文件生成提交信息骨架。纯字符串规则，无 AI（红线 R1）。权限 git:read + ui:command + ui:toast。

## 安装

设置 → 插件 → 开发者模式：输入本目录路径（`plugins/examples/commit-template`）安装；
首次启用时逐项授权。目录含清单 `plugin.json` 与产物 `plugin.wasm`。

## 构建（源码即文档）

源码在 `plugins/commit-template`（本仓库 plugins workspace）。改动后：

```bash
node scripts/build-plugins.mjs   # 构建全部示例并同步 plugin.wasm
```

产物入库由 CI 校验同步（重新编译后 git diff 必须为空）。
