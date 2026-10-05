//! PTY Spike（T5.1）：在写正式终端 UI 之前验证 portable-pty 的可用性与限制。
//!
//! 运行方式：`cargo run -p forgedesk-services --example pty_spike`
//!
//! 全自动完成能力检查并输出报告（结论写入 `docs/PTY-SPIKE.md`）：
//! 中文输出 / emoji / ANSI 颜色 / 交互式输入 / Ctrl+C 中断 / resize / 退出码，
//! 最后跑一次 10 万行输出的吞吐量测试。断言用子串匹配（shell 横幅与提示符
//! 因机器而异），每步都有超时——spike 卡死本身就是需要记录的结论。
//!
//! 为什么是 example 而不是测试：它要起真实 shell 并依赖其输出格式，
//! 作为保留的 example（而非 CI 门禁）既能在三平台随时手工复跑，
//! 又不给 CI 增加对宿主 shell 环境的脆弱依赖。
//!
//! Ctrl+C 的分解（T5.1 实测，Windows）：裸 0x03 在 cmd.exe 中可靠中断运行中的
//! 子命令（ping）；在 PowerShell 的 PSReadLine 提示符处可靠取消当前行；
//! 但对"运行中的 PowerShell cmdlet"三种已知传入方式（裸 0x03、win32-input-mode
//! 5/6 字段键序列）都不触发——记录为已知限制，待 T5.2 真实 xterm.js 前端复核。
//! 因此本 spike 对 Ctrl+C 的硬断言用 cmd + ping（跨平台等价物：bash + sleep）。

#![allow(clippy::print_stdout, clippy::print_stderr)]
// spike 是"手动跑的诊断脚本"，与测试同级的宽松度：失败即报错并打印现场。
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use forgedesk_services::terminal::{wait_for_exit, PtyConfig, PtySession, TerminalCallbacks};
use parking_lot::Mutex;

const CJK_MARKER: &str = "MARK_\u{4e2d}\u{6587}_OK"; // MARK_中文_OK
const EMOJI_BYTES: [u8; 4] = [0xF0, 0x9F, 0x8E, 0x89]; // 🎉

/// 输出捕获器：攒字节 + 计数（吞吐测试用）。
struct Capture {
    text: Mutex<Vec<u8>>,
    bytes: AtomicU64,
    newlines: AtomicU64,
}

impl Capture {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            text: Mutex::new(Vec::new()),
            bytes: AtomicU64::new(0),
            newlines: AtomicU64::new(0),
        })
    }

    /// 把捕获器接到会话回调上（exit 事件对捕获器无意义，仅输出需要收集）。
    fn sink(self: &Arc<Self>) -> TerminalCallbacks {
        let capture = Arc::clone(self);
        TerminalCallbacks {
            on_output: Arc::new(move |data| {
                capture
                    .bytes
                    .fetch_add(data.len() as u64, Ordering::Relaxed);
                capture.newlines.fetch_add(
                    data.iter().filter(|b| **b == b'\n').count() as u64,
                    Ordering::Relaxed,
                );
                capture.text.lock().extend_from_slice(data);
            }),
            on_exit: Arc::new(|_code| {}),
        }
    }

    fn snapshot(&self) -> Vec<u8> {
        self.text.lock().clone()
    }

    fn len(&self) -> usize {
        self.text.lock().len()
    }

    /// 等待缓冲区出现指定标记（字节级搜索：ConPTY 输出可能含非 UTF-8 控制字节，
    /// 先 from_utf8 再 contains 会因整段解码失败而永远等不到）。
    fn wait_marker(&self, marker: &str, timeout: Duration) -> bool {
        let needle = marker.as_bytes();
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if contains_seq(&self.snapshot(), needle) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    /// 等待出现任意新输出（例如 Ctrl+C 后提示符回来）。
    fn wait_any_output(&self, from: usize, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if self.len() > from {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }
}

fn contains_seq(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.len() >= needle.len() && haystack.windows(needle.len()).any(|w| w == needle)
}

/// 一项检查的结果。
struct Check {
    name: &'static str,
    passed: bool,
    note: String,
}

fn record(checks: &mut Vec<Check>, name: &'static str, passed: bool, note: impl Into<String>) {
    let note = note.into();
    println!(
        "  [{}] {} -- {}",
        if passed { "PASS" } else { "FAIL" },
        name,
        note
    );
    checks.push(Check { name, passed, note });
}

/// 启动会话 + 配套捕获器。失败直接 panic（PTY 起不来本身就是"不可用"结论）。
fn spawn_with_capture(config: &PtyConfig) -> (Arc<PtySession>, Arc<Capture>) {
    let capture = Capture::new();
    let session = Arc::new(PtySession::spawn(config, capture.sink()).expect("PTY spawn failed"));
    (session, capture)
}

fn default_config(cols: u16, rows: u16) -> PtyConfig {
    PtyConfig {
        cwd: std::env::temp_dir(),
        cols,
        rows,
        shell: None,
        // 无头读取端：PSReadLine 启动时等 DSR 应答，不开就永远等不到提示符。
        auto_reply_dsr: true,
    }
}

fn main() {
    println!("=== ForgeDesk PTY Spike (T5.1) ===");
    println!(
        "platform: {} / {}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );

    let mut checks: Vec<Check> = Vec::new();
    let (session, capture) = spawn_with_capture(&default_config(80, 24));
    let program = session.program().to_ascii_lowercase();
    println!("shell: {}", session.program());
    let is_powershell = program.starts_with("pwsh") || program.starts_with("powershell");
    let is_cmd = program == "cmd";

    // ---- 1) 中文输出（CJK 往返）----
    if is_powershell {
        let _ = session.write(b"[Console]::OutputEncoding=[System.Text.Encoding]::UTF8\r");
    } else if is_cmd {
        let _ = session.write(b"chcp 65001 >nul\r\n");
    }
    let command = if is_powershell {
        format!("Write-Output '{CJK_MARKER}'\r")
    } else if is_cmd {
        format!("echo {CJK_MARKER}\r\n")
    } else {
        format!("printf '{CJK_MARKER}\\n'\r")
    };
    let _ = session.write(command.as_bytes());
    let chinese_ok = capture.wait_marker(CJK_MARKER, Duration::from_secs(20));
    record(
        &mut checks,
        "chinese-output",
        chinese_ok,
        if chinese_ok {
            "CJK echoed back through the PTY as UTF-8"
        } else {
            "marker never arrived within 20s"
        },
    );

    // ---- 2) emoji ----
    if is_cmd {
        // cmd 的 echo 对 emoji 的表现因代码页而异：记录为"跳过"而非失败。
        record(
            &mut checks,
            "emoji",
            true,
            "skipped on cmd.exe (codepage dependent)",
        );
    } else {
        let write = if is_powershell {
            // [char] 只装得下 BMP：非 BMP 码点必须走 ConvertFromUtf32。
            b"$e=[char]::ConvertFromUtf32(0x1F389); Write-Output \"EMOJI:$e\"\r".to_vec()
        } else {
            b"printf 'EMOJI:\\U0001F389\\n'\r".to_vec()
        };
        let _ = session.write(&write);
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut ok = false;
        while Instant::now() < deadline {
            if contains_seq(&capture.snapshot(), &EMOJI_BYTES) {
                ok = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        record(
            &mut checks,
            "emoji",
            ok,
            if ok {
                "emoji survived the PTY as UTF-8"
            } else {
                "no emoji bytes within 10s"
            },
        );
    }

    // ---- 3) ANSI 颜色序列 ----
    if is_powershell {
        let _ = session
            .write(b"$e=[char]27; Write-Output (\"ANSI:\" + $e + \"[31mRED_ON\" + $e + \"[0m\")\r");
    } else if !is_cmd {
        let _ = session.write(b"printf 'ANSI:\\033[31mRED_ON\\033[0m\\n'\r");
    }
    // ConPTY 会把 ANSI 重新编码为 VT 序列；宽松断言"ESC[ 序列到达读端"。
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut ansi_ok = is_cmd; // cmd 无原生 ANSI：跳过（记录）
    while Instant::now() < deadline && !ansi_ok {
        if contains_seq(&capture.snapshot(), b"\x1b[") {
            ansi_ok = true;
        } else {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    record(
        &mut checks,
        "ansi-sequences",
        ansi_ok,
        if is_cmd {
            "skipped on cmd.exe (no native ANSI)"
        } else if ansi_ok {
            "VT escape sequences reach the reader (renderer decides colors)"
        } else {
            "no ESC[ sequences observed"
        },
    );

    // ---- 4) 交互式输入 ----
    if is_powershell {
        let _ = session.write(b"$n = Read-Host 'SPIKE_NAME'; Write-Output \"SPIKE_GOT:$n\"\r");
    } else if is_cmd {
        let _ = session.write(b"set /p SPIKE_NAME=SPIKE_NAME && echo SPIKE_GOT:%SPIKE_NAME%\r\n");
    } else {
        let _ = session.write(b"read -r -p 'SPIKE_NAME: ' n; echo \"SPIKE_GOT:$n\"\r");
    }
    std::thread::sleep(Duration::from_millis(800));
    let _ = session.write(b"forgedesk\r");
    let interactive_ok = capture.wait_marker("SPIKE_GOT:forgedesk", Duration::from_secs(15));
    record(
        &mut checks,
        "interactive-input",
        interactive_ok,
        if interactive_ok {
            "prompt received stdin through the PTY"
        } else {
            "echoed answer never arrived"
        },
    );

    // ---- 5) resize ----
    let resized = session.resize(100, 40).is_ok();
    record(
        &mut checks,
        "resize",
        resized,
        if resized {
            "resize(100x40) accepted by the master pty"
        } else {
            "master refused resize"
        },
    );

    // ---- 6) 退出码 ----
    if is_cmd {
        let _ = session.write(b"exit\r\n");
    } else {
        let _ = session.write(b"exit\r");
    }
    let exit_ok = wait_for_exit(&session, Duration::from_secs(10));
    record(
        &mut checks,
        "process-exit",
        exit_ok,
        if exit_ok {
            "child exit observed after `exit`"
        } else {
            "child did not exit within 10s"
        },
    );
    session.close();

    // ---- 7) Ctrl+C 中断（硬断言用"外部子进程"长命令）----
    // 实测（Windows）：0x03 的控制事件能可靠杀死 shell 的外部子进程
    // （PS 下的 ping / bash 下的 sleep），但杀不死 PS 内建 cmdlet（Start-Sleep），
    // 见文件头的"已知限制"。长命令必须在 shell 就绪后写入——过早写入会被
    // 尚未就绪的 PSReadLine 当成"当前输入"被随后的 ETX 整行取消。
    let (ctrl_session, ctrl_capture) = spawn_with_capture(&default_config(80, 24));
    let _ = ctrl_capture.wait_any_output(0, Duration::from_secs(15));
    std::thread::sleep(Duration::from_secs(1));
    let long_command: &[u8] = if cfg!(windows) {
        b"ping -n 30 127.0.0.1>nul\r\n"
    } else {
        b"sleep 30\r"
    };
    let _ = ctrl_session.write(long_command);
    std::thread::sleep(Duration::from_millis(1500));
    let base = ctrl_capture.len();
    let _ = ctrl_session.write(&[0x03]); // ETX：ConPTY 合成 CTRL_C_EVENT，Unix 行规发 SIGINT
    let interrupted = ctrl_capture.wait_any_output(base, Duration::from_secs(8));
    if !interrupted {
        let snap = ctrl_capture.snapshot();
        let tail = &snap[snap.len().saturating_sub(400)..];
        println!(
            "  ctrl-c failure scene (last bytes): {:?}",
            String::from_utf8_lossy(tail)
        );
    }
    record(
        &mut checks,
        "ctrl-c",
        interrupted,
        if interrupted {
            format!(
                "interrupted {} running command via ETX (shell: {})",
                if cfg!(windows) { "ping" } else { "sleep" },
                ctrl_session.program()
            )
        } else {
            "no reaction within 8s (would block the terminal UX)".to_string()
        },
    );

    // ---- 8) PowerShell 提示符处的 Ctrl+C 行取消（默认 shell 是 PS 时）----
    if is_powershell {
        let (ps_session, ps_capture) = spawn_with_capture(&default_config(80, 24));
        let _ = ps_session.write(b"echo abc");
        std::thread::sleep(Duration::from_millis(800));
        let base = ps_capture.len();
        let _ = ps_session.write(&[0x03]);
        let cancelled = ps_capture.wait_any_output(base, Duration::from_secs(6));
        record(
            &mut checks,
            "psreadline-ctrl-c",
            cancelled,
            if cancelled {
                "PSReadLine cancelled the pending line on ETX"
            } else {
                "PSReadLine did not react to ETX at the prompt"
            },
        );
        ps_session.close();
    }

    // ---- 吞吐量（10 万行，走原生文件输出）----
    println!("\n  running throughput test (100k lines)...");
    match run_throughput() {
        Ok(stats) => {
            let secs = stats.elapsed.as_secs_f64();
            record(
                &mut checks,
                "throughput-100k",
                stats.lines >= 100_000,
                format!(
                    "{}/{} lines... {} bytes in {:.2}s -> {:.0} lines/s, {:.1} MB/s \
                     (PTY -> reader -> 16ms coalesce -> callback)",
                    stats.lines,
                    100_000,
                    stats.bytes,
                    secs,
                    stats.lines as f64 / secs,
                    stats.bytes as f64 / secs / (1024.0 * 1024.0),
                ),
            );
        }
        Err(error) => record(&mut checks, "throughput-100k", false, error),
    }

    print_summary(&checks);
    if checks.iter().any(|check| !check.passed) {
        std::process::exit(1);
    }
}

struct ThroughputStats {
    lines: u64,
    bytes: u64,
    elapsed: Duration,
}

/// 吞吐量测试：预生成 10 万行文件，用 shell 的原生输出命令灌过 PTY。
///
/// 为什么用文件而不是循环命令：`for` 循环的速度测的是 shell 自身开销，
/// `type` / `cat` / ReadAllText 测的才是"输出流过 PTY 的能力"——后者才是
/// 终端组件要面对的现实（git clone / build 的输出）。
fn run_throughput() -> Result<ThroughputStats, String> {
    let file = std::env::temp_dir().join("forgedesk-pty-spike-100k.txt");
    {
        let mut handle = std::fs::File::create(&file)
            .map_err(|error| format!("could not create fixture: {error}"))?;
        for i in 0..100_000 {
            let _ = writeln!(handle, "line-{i:06} 0123456789abcdef");
        }
    }

    let (session, capture) = spawn_with_capture(&default_config(200, 50));
    // 等 shell 就绪：第一段输出（横幅/提示符）到达后再留 1s 余量。
    let _ = capture.wait_any_output(0, Duration::from_secs(15));
    std::thread::sleep(Duration::from_secs(1));

    let path = file.to_string_lossy().replace('\\', "/");
    let program = session.program().to_ascii_lowercase();
    let command = if program.starts_with("pwsh") || program.starts_with("powershell") {
        format!(
            "[Console]::OutputEncoding=[Text.Encoding]::UTF8; [IO.File]::ReadAllText('{path}')\r"
        )
    } else if program == "cmd" {
        format!("type \"{path}\"\r\n")
    } else {
        format!("cat '{path}'\r")
    };

    let started = Instant::now();
    session
        .write(command.as_bytes())
        .map_err(|error| format!("write failed: {error}"))?;

    // 等到 10 万行（回显/横幅会多出几行，只设下限）。
    let deadline = started + Duration::from_secs(90);
    loop {
        if capture.newlines.load(Ordering::Relaxed) >= 100_000 {
            break;
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "timeout: only {} lines in 90s",
                capture.newlines.load(Ordering::Relaxed)
            ));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let elapsed = started.elapsed();
    let stats = ThroughputStats {
        lines: capture.newlines.load(Ordering::Relaxed),
        bytes: capture.bytes.load(Ordering::Relaxed),
        elapsed,
    };
    session.close();
    let _ = std::fs::remove_file(&file);
    Ok(stats)
}

fn print_summary(checks: &[Check]) {
    println!("\n=== summary ===");
    let passed = checks.iter().filter(|check| check.passed).count();
    println!("{}/{} checks passed", passed, checks.len());
    for check in checks {
        println!(
            "  {:<18} {} -- {}",
            check.name,
            if check.passed { "PASS" } else { "FAIL" },
            check.note
        );
    }
    println!(
        "\n把上面的输出抄进 docs/PTY-SPIKE.md 的「实测数据」一节；\n\
         三平台各跑一次（macOS/Linux 在对应机器或 CI runner 上）。"
    );
}
