<!-- 感谢贡献！请逐条确认下面的勾选项，它们对应 AGENTS.md §2 的红线与 §4 的门禁。 -->

## 这个 PR 做了什么

<!-- 一到三句话说明改动与动机。关联 issue：Fixes #123 -->

## 为什么这样做

<!-- 关键取舍：为什么不选另一条路。评审最关心这一段。 -->

## 影响面

- [ ] 破坏性变更（如果是，下面写迁移方式）
- [ ] 新增/修改 Tauri Command（已在 `docs/API.md` 登记能力等级、参数、返回、错误码）
- [ ] 新增依赖（已在下方说明理由、体积影响、替代方案、许可证）
- [ ] 修改界面（附截图或文字线框图，便于原创性核对）
- [ ] 仅文档/杂务

## 红线自检（AGENTS.md §2）

- [ ] **未复制竞品 UI**：没有参考 GitHub Desktop / GitKraken / Sourcetree / Fork / Git-cola 的布局、配色、图标或文案
- [ ] **未使用受限商标**：没有使用 Git / GitHub / Tauri 官方 Logo、Octocat 及其变体
- [ ] **未引入 AI 依赖**：没有引入模型、推理服务或任何 AI/ML 运行时依赖（R1）
- [ ] **未引入遥测 SDK**：没有第三方分析/追踪 SDK（R6）
- [ ] **未泄露凭据**：没有把令牌/密码/私钥写入代码、测试、日志、截图或示例配置（R8）
- [ ] **破坏性操作有安全网**：涉及写仓库的改动已接入快照与审计（R7）
- [ ] 产品名、包名、窗口标题中**不含** "Git"/"GitHub" 字样（R4）

## 自检命令（必须全绿）

```bash
pnpm lint && pnpm i18n:lint && pnpm typecheck && pnpm test
pnpm format:check && pnpm check:contrast && pnpm check:workflows && pnpm check:repo && pnpm check:docs
cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

- [ ] 上述命令全部通过（CI 也会跑同一组）
- [ ] 新增/修改的用户可见文案已走 i18n，中英同步，`pnpm i18n:lint` 通过
- [ ] 新增 Rust 领域逻辑有单测；新增核心前端组件有 Vitest 用例
- [ ] 文档已按需更新（架构/契约变化 → `docs/ARCHITECTURE.md` 或 `docs/API.md`）

## 备注

<!-- 已知限制、未覆盖的边缘情况、后续计划 -->
