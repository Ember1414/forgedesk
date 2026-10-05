# PTY Spike 结论（T5.1）

> 目的：在写正式终端 UI（T5.2）之前，验证 `portable-pty` 的可用性与限制。
> 本文档是 **T5.1 的验收物**，也是 T5.2 / T5.10 三平台验收的基线。
> 复跑方式：`cargo run -p forgedesk-services --example pty_spike`（全自动，约 30 秒）。

---

## 1. 总结论

| 平台 | 结论 | 依据 |
| --- | --- | --- |
| **Windows 11 (26200, x64)** | ✅ **可用**，ConPTY 全链路实测通过 | 本机实测（下表 9/9 项通过） |
| macOS | ⚠️ 依上游文档判定可用，**本机未实测** | portable-pty 使用 Unix openpty，macOS 原生支持；留待 T5.10 三平台手工验收 |
| Linux | ⚠️ 依上游文档判定可用，**本机未实测** | 同上；另需注意无 `/dev/pts` 的受限环境会失败（返回 `PTY_UNSUPPORTED`，前端给降级提示） |

**没有触发"停止并降级"条件**：任务书规定"三平台任一不可用 → 立即停下请求人类决策"，
Windows 实测可用、另两平台依上游支持判定可推进；macOS/Linux 的实测补验列在
T5.10 三平台验收清单中（`docs/acceptance/M5.md`）。若届时发现不可用，降级方案
（非交互日志式终端）再提交人类决策。

### 最低 Windows 版本

ConPTY 需要 **Windows 10 1809+**（`conhost.exe` 的伪控制台 API 在该版本引入）。
更早版本上 `openpty` 会失败，`PtySession::spawn` 返回
`AppError{ code: "PTY_UNSUPPORTED", detail: <系统错误> }`，前端据此展示
"当前系统不支持内嵌终端"与降级说明（T5.2 落地）。

---

## 2. 实测数据（Windows 11 26200 / x64 / powershell 5.1）

`cargo run -p forgedesk-services --example pty_spike` 输出（2026-10-05）：

```text
9/9 checks passed
  chinese-output     PASS -- CJK echoed back through the PTY as UTF-8
  emoji              PASS -- emoji survived the PTY as UTF-8
  ansi-sequences     PASS -- VT escape sequences reach the reader
  interactive-input  PASS -- prompt received stdin through the PTY
  resize             PASS -- resize(100x40) accepted by the master pty
  process-exit       PASS -- child exit observed after `exit`
  ctrl-c             PASS -- interrupted ping running command via ETX (shell: powershell)
  psreadline-ctrl-c  PASS -- PSReadLine cancelled the pending line on ETX
  throughput-100k    PASS -- 100002/100000 lines... 3000378 bytes in 0.83s
                     -> 120440 lines/s, 3.4 MB/s (PTY -> reader -> 16ms coalesce -> callback)
```

重复运行吞吐量在 **120k–133k lines/s（3.4–3.8 MB/s）** 之间波动。
验收门禁要求"每秒 5000 行不丢不卡"：**实测余量约 24 倍**。

---

## 3. 关键结论

### 3.1 IPC 传输方式：base64 字符串（而非 `Vec<u8>` JSON 数组）

| 方案 | 每 IPC 字节的线上体积 | 结论 |
| --- | --- | --- |
| `Vec<u8>`（JSON 数字数组） | ≈ 3.9 字节/字节（`123,` + 数组括号） | 纯开销，否决 |
| **base64 字符串** | ≈ 1.33 字节/字节（一次性 O(n) 编解码） | ✅ 采纳 |

吞吐瓶颈在 PTY 管线与渲染，base64 编解码（GB/s 量级）可忽略。
键盘输入与输出块共用同一条 base64 通道，前后端不必为"文本还是字节"各写一套。
前端配套工具在 `src/lib/ipc/ptySpike.ts`（`utf8ToBase64` / `createUtf8StreamDecoder`，
后者用 `TextDecoder` stream 模式处理"多字节字符被 16ms 合并块切开"的场景）。

### 3.2 会话线程模型（T5.2 直接沿用）

```text
reader  ：阻塞读 PTY → 合并缓冲（EOF 退出）
flusher ：每 16ms 排水 → on_output 回调（IPC 次数 ↓，单事件载荷 ↑）
waiter  ：等子进程退出 → 释放 master → join 前两条线程 → exit 事件
```

实测踩到并已修复的两个坑（T5.2 不许回退）：

1. **ConPTY 输出管道只在 master 释放时 EOF**，子进程退出不会。
   waiter 必须先 `drop(master)` 再 join reader，否则退出事件被无限推迟
   （实测 10 秒内收不到 `exit`）。
2. **DSR（`ESC[6n`）无头读取端必须应答**。PSReadLine 启动时向"终端"查询光标
   位置并阻塞等待；无头端不应答，shell 永远停在启动阶段。
   `PtyConfig.auto_reply_dsr` 因此存在：spike / 测试 / dev 页面为 `true`；
   **正式终端必须为 `false`**（xterm.js 原生应答 DSR，双份应答会混进输入流）。

### 3.3 Ctrl+C 的能力边界（Windows 实测）

| 场景 | 裸 `0x03`（ETX） |
| --- | --- |
| PowerShell + 外部子进程（ping） | ✅ 立即中断（控制事件送达进程组） |
| PowerShell 提示符处（PSReadLine 行取消） | ✅ 立即生效 |
| PowerShell 内建 cmdlet（`Start-Sleep`） | ❌ 无反应（裸 0x03、win32-input-mode 5/6 字段键序列均试过） |
| cmd.exe + ping | ❌ 本 spike 未复现中断（一次实测） |

结论：**"运行中的外部命令"与"行内取消"两条主路径可靠**；PS 内建 cmdlet 的
硬中断在无头读取端不触发，留待 T5.2 真实 xterm.js 前端复核（xterm.js 与
ConPTY 的完整交互和 spike 的最小应答器不同）。Unix 上 `0x03 → SIGINT` 是
行规，预期可靠（T5.10 验收确认）。
文档（README / 设置页说明）需要如实描述：终端里 Ctrl+C 与系统终端体验可能
存在个别差异，不宣称"与原生终端完全一致"。

### 3.4 resize

`master.resize(PtySize)` 同步生效、无错误返回值语义需要处理以外的问题；
会话退出后 resize 返回错误（`master` 槽已释放），前端按"会话已死"处理。

### 3.5 中文 / emoji / ANSI

- ConPTY 输出为 UTF-8；PowerShell 需要先设
  `[Console]::OutputEncoding=UTF8` 才能保证中文与 emoji 的正确往返
  （T5.2 的 shell 启动序列要带上这一条，cmd 用 `chcp 65001`）。
- PowerShell 的非 BMP 字符必须 `ConvertFromUtf32`（`[char]` 只装 BMP）——
  这是 spike 测试自身的坑，与产品无关，但说明"emoji 经 ConPTY 往返"本身可靠。
- ANSI 序列原样到达读端（ConPTY 重编码为标准 VT），颜色由前端渲染器决定。

---

## 4. spike 代码的去留

**保留**，两处，均有长期价值：

- `crates/services/examples/pty_spike.rs`：三平台随时可复跑的能力自检脚本
  （macOS/Linux 验收时直接跑它取证）。
- `crates/services/tests/terminal.rs`（仅 Windows 编译）：锁定 ConPTY 关键行为的
  集成测试（echo 往返 / exit 事件 / ETX 中断外部命令），防止 T5.2 重构时回退。
- `crates/commands/src/pty_spike.rs` + `src/ui/__dev__/PtySpikePanel.tsx`
  + 路由 `__dev__/pty-spike`：**仅开发构建**的 IPC 通道量测工具（人工核对
  "后端 emit → 前端收到"这一跳；后端口径 120k lines/s，见上）。

## 5. 对 T5.2 的约束汇总

1. `term_create` 的 `auto_reply_dsr` 必须为 `false`（xterm.js 自己应答）。
2. shell 启动序列按平台带编码设置（PowerShell：OutputEncoding=UTF8；cmd：chcp 65001）。
3. 会话生命周期按 §3.2 的三线程模型与 master 释放顺序实现。
4. 传输用 base64 + 16ms 合并；resize 错误按"会话已退出"处理。
5. 文档如实描述 Ctrl+C 的平台差异，不承诺"与原生终端完全一致"。

## 6. 人类审批点

- [x] 未触发"平台不可用 → 降级"条件（Windows 实测可用），按任务书继续 T5.2。
- [ ] **macOS/Linux 的 spike 复跑**（T5.10 三平台验收时执行，本机无法代跑）。
- [ ] §3.3 的 Ctrl+C 差异是否可接受（不可接受的备选：终端内提供"停止"按钮，
      后端直接 kill 进程组——不依赖 ETX）。
