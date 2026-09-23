# ADR-001: TypeScript 固定为 6.0.3（而非 latest 的 7.0.2）

- 状态：已接受
- 日期：2026-09-23
- 里程碑：M0 / T0.1
- 关联：`docs/PLAN.md` §6.1（原计划 TS 5.x）

## 上下文

`npm view typescript dist-tags` 显示 `latest = 7.0.2`，但当前 ESLint 生态尚未跟上：

- `typescript-eslint@8.70.1` 在加载时直接抛错并终止整个 ESLint 进程：
  `typescript-eslint does not support TS 7.0.`
  （官方追踪 issue：typescript-eslint/typescript-eslint#10940，支持计划为 TS >= 7.1）
- 实测：`pnpm lint` 退出码 2，整个 lint 门禁不可用。
- 同时 TS 7 移除了 `baseUrl`（`tsc` 报 `TS5102: Option 'baseUrl' has been removed`）。

Lint 是 `AGENTS.md` §4 质量门禁的必需项，也是 CI 的必过 job。若放弃 lint，将失去
架构护栏（如"禁止绕过 IPC 层直连 Tauri"）与 `no-explicit-any` 等约束。

## 决策

**项目 TypeScript 固定为 `6.0.3`**，并在 `package.json` 中以精确版本声明。

## 备选方案

| 方案 | 优点 | 缺点 | 结论 |
| --- | --- | --- | --- |
| A. TS 7 + 并行安装 TS 6 供 eslint 使用 | 保留 TS 7 的原生编译速度 | 需维护两套 TS；7.x 生态整体不完整，后续会遇到更多兼容问题 | 否 |
| B. **TS 6.0.3** | typescript-eslint 官方支持；语言特性足够新；单一版本好维护 | 放弃 TS 7 的原生性能（当前项目规模下影响可忽略） | **采用** |
| C. TS 5.9.3 | 最保守，生态覆盖最广 | 落后两个大版本，后续升级跨度更大 | 否 |
| D. TS 7 但不使用 ESLint | 版本最新 | 失去 lint 门禁与架构护栏，不可接受 | 否 |

## 后果

- 正面：完整工具链可用；`pnpm typecheck`、`pnpm lint`、`pnpm build` 全绿（已实测）。
- 正面：因为已改用相对 `paths`（不依赖 `baseUrl`），未来升级 TS 7 的迁移成本极低。
- 负面：暂时无法享受 TS 7 的原生编译速度。
- 后续跟进：当 `typescript-eslint` 支持 TS >= 7.1 后，重新评估升级（追踪 issue #10940）。
  升级时需同时确认 Vite / Vitest / ESLint 插件链的兼容性。
