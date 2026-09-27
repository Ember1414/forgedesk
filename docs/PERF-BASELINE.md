# M1 性能基线（T1.12 第 4 条）

本文件记录 M1 收官时的一组**可复现**测量：怎么量的、量到了什么、哪些结论不能从中得出。

- 测量日期：2026-09-27
- 机器：Windows（x86_64-pc-windows-msvc），release 构建（`cargo build --release`）
- git：系统 CLI（与用户终端同一个）
- 被测代码：`forgedesk-services` 的真实服务栈（`RepositoryService` / `WorkspaceService` /
  `StagingService` + 真实 `GitEngines`），通过 `crates/services/examples/perf_probe.rs` 调用

## 1. 怎么量

```powershell
# 1) 生成夹具（可复现：同一 N 每次生成同样的提交 oid）
node scripts/perf/gen-repo.mjs target/perf/commits-100000 --commits 100000
node scripts/perf/gen-repo.mjs target/perf/modified-10000 --modified 10000
node scripts/perf/gen-repo.mjs target/perf/untracked-10000 --untracked 10000

# 2) 构建探针并跑基线（内存与 CPU 由脚本从进程外部采样）
cargo build --release -p forgedesk-services --example perf_probe
powershell -NoProfile -File scripts/perf/run-baseline.ps1 -WatchSeconds 5
```

探针量四个动作，全部在**同一个进程**内完成（因此 `openMs` 包含了引擎初始化后的
首次仓库读取）：

| 指标 | 含义 |
| --- | --- |
| `openMs` | `RepositoryService::open`：发现仓库 + 配置审计 + git 版本检查 + 登记 |
| `statusMs` | `WorkspaceService::status`：状态面板一屏所需的数据 |
| `diffMs` | 单文件完整 diff（打开查看器那一次请求） |
| `stagingMs` | 整文件暂存 + 取消暂存各一次（写路径 + 补丁通道） |
| `watchCpuMs` | 启动文件监听后**空闲** 5 秒所消耗的 CPU（外部采样 `TotalProcessorTime`） |
| 峰值内存 | 外部采样 `WorkingSet64` 的最大值（含 git 子进程之外的一切） |

## 2. 结果（`target/perf/baseline.json` 的汇总）

| 仓库 | 形状 | openMs | statusMs | diffMs | stagingMs | 峰值内存 | 监听空闲 CPU / 5s |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| commits-100 | 100 提交 | 313 | 12.8 | – | – | 8.2 MB | 62 ms |
| commits-1000 | 1k 提交 | 299 | 11.0 | – | – | 8.1 MB | 0 ms |
| commits-10000 | 10k 提交 | 300 | 11.5 | – | – | 8.0 MB | 62 ms |
| commits-50000 | 50k 提交 | 312 | 9.6 | – | – | 8.1 MB | 78 ms |
| commits-100000 | 100k 提交 | 301 | 11.6 | – | – | 8.1 MB | 31 ms |
| modified-10000 | 10k 已跟踪修改 | 299 | **6168** | 112 | 510 | 17.0 MB | 47 ms（见下） |
| untracked-10000 | 10k 未跟踪 | 302 | 376 | – | – | 14.8 MB | 78 ms |
| forgedesk | 本仓库（真实，16 条变更） | 286 | 72 | 101 | 74 | 8.4 MB | 109 ms |

读法（这几条是结论，不是数字游戏）：

1. **`openMs` 与仓库规模无关**（286–317 ms，抖动在测量噪声内）：它是固定成本
   （git 版本探测 + 配置审计各起一次 git 子进程），不是"读了多少历史"。
   M1 验收的"打开仓库 ≤ 2s 显示状态"因此有大量余量。
2. **状态刷新的成本由"变更文件数"决定，而不是"提交数"**：10 万提交的仓库
   状态刷新 11.6 ms，与 100 提交的仓库没有区别——状态只读索引与工作区。
3. **监听空闲时 CPU ≈ 0**：5 秒窗口内 0–109 ms（约 0–2% 单核），
   与仓库规模无关（10k 文件与 100k 提交都一样）。这是 T1.10 的"大仓库空闲占用接近 0"
   验收项，此前只能靠推理，现在有实测。
4. **内存峰值 8–17 MB**：10k 变更条目时约 17 MB（每个条目一条 DTO + 路径），
   仍然远低于任何值得担心的量级。
5. **`statusMs = 6168 ms` 是本基线的唯一红灯**：见下一节。

## 3. 已知问题与结论边界

### 3.1 10k 已跟踪修改时，状态刷新 6.2 秒（P1，待决策）

同一仓库 `git status --porcelain` 只用 **0.61 秒**。差异来自**读路径走 libgit2**
（T1.2 的设计：读用 libgit2、写用 CLI）：libgit2 在"索引 1 万条、工作区全改"
这一形状上比 git CLI 慢约 10 倍。

排查过程中的两次对照（同一台机器、release 构建、热身后）：

| 配置 | statusMs | 说明 |
| --- | ---: | --- |
| 当前实现 | 6168 ms | 富化 + libgit2 状态 |
| 临时禁用富化 | 5813 ms | 说明瓶颈在 libgit2，不在我们的富化 |
| 富化中**去掉二进制嗅探**（已修） | – | 修复前该动作每次状态刷新要**打开并读取每个文件**，在 1 万文件上量到 39–47 秒 |

**已修的部分**（本任务内）：`crates/git-engine/src/engine/enrich.rs` 不再对每个条目做
二进制嗅探（`File::open` + 读 8KB）。git 自己也不在 `status` 里判二进制；
真正需要它的地方（diff、`git apply`）本来就有 git 的判定。

**未修的部分**（需要产品/架构决策，不在 T1.12 单方面改动）：

- 方案 A：把 `status` 也切到 CLI 引擎（0.61 s）。代价：改变 T1.2 "读用 libgit2"
  的划分，且 CLI 引擎的状态路径没有富化（文件大小、操作状态），界面会少两个字段；
- 方案 B：按规模分流（索引条目超过阈值时用 CLI，小仓库继续 libgit2）。
  代价：引入了运行时两条路径，差分测试之外的组合变多；
- 方案 C：接受 6.2 s，界面用骨架屏 + 明确进度提示覆盖这段时间。

在决策之前，界面侧的验收（10k 行渲染不卡顿）**是过的**：
`e2e/workspace.spec.ts` 的 "10000 个变更文件的首屏渲染（性能基准）" 用例通过。

### 3.2 未跟踪目录的表示与 git 不同（P1，待决策）

libgit2 的 `recurse_untracked_dirs(true)` 会**逐个列出**未跟踪目录里的文件
（1 万个文件 = 1 万条），而 `git status` 默认把它们折叠成一条 `scratch/`。
因此同一仓库、同一时刻，"未跟踪"分组的条目数可能比用户在终端里看到的多。
数字上我们的路径更快（376 ms），但**语义与 git 不一致**。
修复选项：`recurse_untracked_dirs(false)`（与 git 对齐，但会改变 T1.4 的未跟踪列表形状）。

### 3.3 这组数字不能证明什么

- 它是**单机、单次**测量（每个仓库一次，冷启动一次已被 3.1 表剔除）。
  可信的是量级与相对关系，不是小数点后一位；
- 磁盘是本地 SSD。网络盘、加密盘上的 `status`/`diff` 会明显更差；
- `openMs` 里的 git 子进程启动成本在 Windows 上偏高（约 150–250 ms）。
  Linux/macOS 上会低得多，因此**不要**把这里的绝对值当作跨平台指标；
- 没有量：终端（M5）、历史图（M2）、大文件 diff 的渲染（M5）。
