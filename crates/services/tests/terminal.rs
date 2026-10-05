//! 真实 PTY 集成测试（T5.1）。
//!
//! 只在 Windows 启用（本仓库的日常门禁在 Windows 上跑，CI 的 windows-latest
//! 也会跑到）：断言依赖真实 shell 的行为（cmd 的 echo、PowerShell 的提示符、
//! 外部子进程对 Ctrl+C 的响应），macOS/Linux 的等价验证由三平台手工验收
//! （T5.10）与 nightly 矩阵覆盖——把"可能没有稳定 PTY 的 CI 环境"挡在门禁之外，
//! 是 spike 阶段有意的取舍（见 docs/PTY-SPIKE.md 的已知限制）。
//!
//! 注意断言口径：等待真实 shell 的输出必然有竞态，所有等待都有超时上限，
//! 断言的是"在时限内出现/不出现"，而不是字节流的精确形状。

#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use forgedesk_services::terminal::{PtyConfig, PtySession, TerminalCallbacks};
use parking_lot::Mutex;

/// 输出收集器：测试里只做"攒字节 + 等标记"。
struct Collector {
    text: Mutex<Vec<u8>>,
    exited: Mutex<Option<Option<u32>>>,
}

impl Collector {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            text: Mutex::new(Vec::new()),
            exited: Mutex::new(None),
        })
    }

    fn callbacks(self: &Arc<Self>) -> TerminalCallbacks {
        let output = Arc::clone(self);
        let exit = Arc::clone(self);
        TerminalCallbacks {
            on_output: Arc::new(move |data| {
                output.text.lock().extend_from_slice(data);
            }),
            on_exit: Arc::new(move |code| {
                *exit.exited.lock() = Some(code);
            }),
        }
    }

    fn contains(&self, marker: &str) -> bool {
        let text = self.text.lock();
        text.len() >= marker.len() && text.windows(marker.len()).any(|w| w == marker.as_bytes())
    }

    /// 等标记出现（字节级搜索，理由同 spike：输出可能含非 UTF-8 控制字节）。
    fn wait_marker(self: &Arc<Self>, marker: &str, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if self.contains(marker) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        self.contains(marker)
    }

    /// 等出现任意新输出。
    fn wait_any_output(self: &Arc<Self>, from: usize, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if self.text.lock().len() > from {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    /// 等 exit 事件，返回携带的退出码。
    fn wait_exit(self: &Arc<Self>, timeout: Duration) -> Option<Option<u32>> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let exited = self.exited.lock().clone();
            if exited.is_some() {
                return exited;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        self.exited.lock().clone()
    }
}

fn spawn(shell: Option<&str>) -> (Arc<PtySession>, Arc<Collector>) {
    let collector = Collector::new();
    let config = PtyConfig {
        cwd: std::env::temp_dir(),
        cols: 80,
        rows: 24,
        shell: shell.map(str::to_string),
        // 测试与 spike 一样是无头读取端：PSReadLine 的 DSR 等待必须被应答。
        auto_reply_dsr: true,
    };
    let session = Arc::new(PtySession::spawn(&config, collector.callbacks()).expect("spawn"));
    (session, collector)
}

/// cmd 启动快（无 PSReadLine），echo 的往返证明"写入 → shell → 输出"全链路。
#[test]
fn written_command_reaches_the_shell_and_output_comes_back() {
    let (session, collector) = spawn(Some("cmd"));
    // 等 cmd 横幅/提示符就绪再发命令（写入过早会被丢弃或排队到未知状态）。
    assert!(
        collector.wait_any_output(0, Duration::from_secs(15)),
        "shell must produce startup output within 15s"
    );
    std::thread::sleep(Duration::from_millis(500));

    session
        .write(b"echo FORGEDESK_TERM_ECHO\r\n")
        .expect("write");
    assert!(
        collector.wait_marker("FORGEDESK_TERM_ECHO", Duration::from_secs(15)),
        "echoed marker must come back through the PTY within 15s"
    );
    session.close();
}

/// `exit` 之后必须收到 exit 事件（含退出码）——退出通知是 T5.2 标签页
/// 状态与"保留最后 1000 行"功能的事件源。
#[test]
fn exiting_shell_emits_an_exit_event_with_a_code() {
    let (session, collector) = spawn(Some("cmd"));
    assert!(
        collector.wait_any_output(0, Duration::from_secs(15)),
        "startup"
    );
    std::thread::sleep(Duration::from_millis(500));

    session.write(b"exit\r\n").expect("write");
    let exited = collector.wait_exit(Duration::from_secs(15));
    assert!(exited.is_some(), "exit event must arrive within 15s");
    session.close();
}

/// ETX（0x03）必须能中断 shell 的外部子进程——这是终端 UX 的硬要求，
/// 长命令（git clone / build）跑飞时用户唯一的出路。
/// 实测口径：PowerShell + ping 可靠；PS 内建 cmdlet 在无头读取端收不到
/// 中断（已知限制，见 docs/PTY-SPIKE.md）。
#[test]
fn etx_interrupts_a_running_external_command() {
    let (session, collector) = spawn(None); // 默认候选：powershell
    assert!(
        collector.wait_any_output(0, Duration::from_secs(20)),
        "startup"
    );
    std::thread::sleep(Duration::from_secs(1));

    session
        .write(b"ping -n 30 127.0.0.1>nul\r\n")
        .expect("write");
    std::thread::sleep(Duration::from_millis(1500));

    let base = collector.text.lock().len();
    session.write(&[0x03]).expect("write");
    assert!(
        collector.wait_any_output(base, Duration::from_secs(10)),
        "prompt must return after Ctrl+C (external command aborted)"
    );
    session.close();
}
