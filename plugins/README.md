# ForgeDesk 插件 workspace（T6.5）

独立于主 workspace：这里的一切以 `wasm32-wasip1` 为目标编译（freestanding，
无 WASI 导入）。主 workspace 的测试通过 `crates/plugin-host/tests/example_plugins.rs`
直接加载 `examples/*/plugin.wasm` 做冒烟验证。

- `sdk/`：插件 SDK——ABI 打包、宿主调用封装、no_std 分配器与 panic handler。
- `commit-template/`、`repo-stats/`、`repo-audit/`：三个官方示例（见 examples/*/README.md）。

构建：`rustup target add wasm32-wasip1 && node ../scripts/build-plugins.mjs`（在主仓库根执行）。
