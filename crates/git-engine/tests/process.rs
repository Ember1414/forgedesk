//! `GitProcess` 的行为测试。
//!
//! 这些测试**真的会启动子进程**（git / cmd / powershell / sleep），因为它们要验证的
//! 正是"进程边界上的行为"：环境是否被钉住、stdin 是否关闭、超时是否真的杀掉了子进程。
//! 纯函数式的替身无法覆盖这些点——替身总是"配合"的，而线上问题恰恰来自不配合的进程。
//!
//! 前提：测试机上有 `git`（本项目的硬依赖，见 docs/DEV-ENV.md §1）。
//! 不满足时测试会**失败**而不是跳过：静默跳过会制造"全绿"的假象。
//!
//! 集成测试是独立 crate，测试代码里允许 panic 式写法（失败即断言失败）。
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use forgedesk_domain::ErrorCode;
use forgedesk_git_engine::process::{GitProcess, GitRunOpts, DEFAULT_TIMEOUT};
use support::{args, TempDir};
use tokio_util::sync::CancellationToken;

/// 执行一条 git 命令并断言进程正常结束。
async fn run_git(
    process: &GitProcess,
    cwd: &Path,
    command: &[&str],
) -> forgedesk_git_engine::process::GitOutput {
    process
        .run(&args(command), GitRunOpts::new(cwd))
        .await
        .unwrap_or_else(|error| panic!("git {command:?} 启动失败: {error}"))
}

/// 在临时目录里初始化一个仓库。
async fn init_repo(process: &GitProcess, cwd: &Path) {
    let output = run_git(process, cwd, &["init", "-q", "-b", "main", "."]).await;
    assert!(output.success(), "git init 失败: {}", output.stderr_lossy());
}

/// 返回一个"会长时间运行"的命令，用于超时与取消测试。
///
/// 为什么不用 git：需要一个与 git 无关、可控的挂起进程，
/// 否则测试会随 git 版本变化而变得不稳定。
#[cfg(windows)]
fn hanging_command() -> (std::path::PathBuf, Vec<String>) {
    (
        std::path::PathBuf::from("powershell"),
        args(&[
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Start-Sleep -Seconds 30",
        ]),
    )
}

#[cfg(unix)]
fn hanging_command() -> (std::path::PathBuf, Vec<String>) {
    (std::path::PathBuf::from("sleep"), args(&["30"]))
}

/// 返回一个"打印全部环境变量"的命令。
///
/// 这里出现的 `cmd` 只是**测试替身**：产品代码从不使用 shell 包装（见 process.rs 模块头）。
#[cfg(windows)]
fn env_dump_command() -> (std::path::PathBuf, Vec<String>) {
    (std::path::PathBuf::from("cmd"), args(&["/c", "set"]))
}

#[cfg(unix)]
fn env_dump_command() -> (std::path::PathBuf, Vec<String>) {
    (std::path::PathBuf::from("env"), Vec::new())
}

/// 解析环境变量转储输出。
fn parse_env_dump(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|line| line.split_once('='))
        // Windows 的 `set` 会打印 `=C:=C:\...` 这类伪变量
        .filter(|(key, _)| !key.is_empty() && !key.starts_with('='))
        .map(|(key, value)| (key.trim().to_owned(), value.trim_end_matches('\r').to_owned()))
        .collect()
}

/// 运行"打印环境变量"的替身命令，返回解析后的环境。
async fn dump_environment(opts: GitRunOpts) -> HashMap<String, String> {
    let (program, command_args) = env_dump_command();
    let output = GitProcess::with_program(program)
        .run(&command_args, opts)
        .await
        .expect("环境转储命令启动失败");
    assert!(
        output.success(),
        "环境转储命令失败: {}",
        output.stderr_lossy()
    );

    parse_env_dump(&output.stdout_lossy())
}

#[tokio::test]
async fn stdout_and_stderr_are_captured_separately() {
    let dir = TempDir::new("process-stdout");
    let output = run_git(&GitProcess::new(), dir.path(), &["--version"]).await;

    assert!(output.success());
    assert_eq!(output.exit_code, Some(0));
    assert!(output.stdout_lossy().starts_with("git version"));
    assert!(
        output.stderr.is_empty(),
        "正常路径不应有 stderr，实际: {}",
        output.stderr_lossy()
    );
    assert!(output.stdout_is_utf8);
    assert!(output.stderr_is_utf8);
}

#[tokio::test]
async fn non_zero_exit_code_is_a_result_not_an_error() {
    let dir = TempDir::new("process-exit-code");
    let process = GitProcess::new();
    init_repo(&process, dir.path()).await;

    let output = run_git(
        &process,
        dir.path(),
        &["rev-parse", "--verify", "refs/heads/definitely-missing"],
    )
    .await;

    assert!(!output.success());
    assert_eq!(output.exit_code, Some(128));
    assert!(
        output.stderr_lossy().contains("fatal"),
        "LC_ALL=C 下 git 的致命错误应是英文 fatal，实际: {}",
        output.stderr_lossy()
    );
    assert!(output.stdout.is_empty());
}

#[tokio::test]
async fn arguments_are_passed_as_an_array_without_shell_interpretation() {
    let dir = TempDir::new("process-no-shell");
    let process = GitProcess::new();
    init_repo(&process, dir.path()).await;

    // 若参数被拼进 shell，`;` 之后的部分会被当成新命令执行
    let output = run_git(
        &process,
        dir.path(),
        &["rev-parse", "--verify", "HEAD; echo pwned"],
    )
    .await;

    assert!(!output.success());
    assert!(!output.stdout_lossy().contains("pwned"));
    assert!(!output.stderr_lossy().contains("pwned"));
}

#[tokio::test]
async fn locale_and_non_interactive_variables_are_pinned() {
    let dir = TempDir::new("process-env");
    // 故意用调用方参数覆盖固定项：固定项必须仍然生效
    let env = dump_environment(
        GitRunOpts::new(dir.path())
            .with_env("LC_ALL", "fr_FR.UTF-8")
            .with_env("GIT_TERMINAL_PROMPT", "1")
            .with_env("GIT_PAGER", "less"),
    )
    .await;

    assert_eq!(env.get("LC_ALL").map(String::as_str), Some("C"));
    assert_eq!(env.get("LANG").map(String::as_str), Some("C"));
    assert_eq!(
        env.get("GIT_TERMINAL_PROMPT").map(String::as_str),
        Some("0")
    );
    assert_eq!(env.get("GIT_PAGER").map(String::as_str), Some("cat"));
    assert_eq!(env.get("GIT_ASKPASS").map(String::as_str), Some(""));
    assert_eq!(env.get("GIT_OPTIONAL_LOCKS").map(String::as_str), Some("0"));
}

#[tokio::test]
async fn optional_locks_can_be_enabled_for_write_operations() {
    let dir = TempDir::new("process-locks");
    let env = dump_environment(GitRunOpts::new(dir.path()).with_optional_locks(true)).await;

    assert_eq!(env.get("GIT_OPTIONAL_LOCKS").map(String::as_str), Some("1"));
}

#[tokio::test]
async fn deleting_a_variable_from_the_child_environment_also_applies_to_forgedesk_askpass() {
    // 回归类断言：`GIT_TERMINAL_PROMPT` 必须始终为 0——即使启用了 askpass，
    // git 也不该在 askpass 答不上来时退回终端提示（应用没有终端，会永久挂住）
    let dir = TempDir::new("process-askpass");
    let env = dump_environment(
        GitRunOpts::new(dir.path())
            .with_askpass("/opt/forgedesk/forgedesk")
            .with_env("GIT_TERMINAL_PROMPT", "1"),
    )
    .await;

    assert_eq!(
        env.get("GIT_ASKPASS").map(String::as_str),
        Some("/opt/forgedesk/forgedesk")
    );
    assert_eq!(
        env.get("GIT_TERMINAL_PROMPT").map(String::as_str),
        Some("0")
    );
}

#[tokio::test]
async fn an_askpass_program_cannot_be_installed_through_the_generic_env_override() {
    // 这条是"默认禁止、显式开启"的另一半：普通 `with_env` 改不动 GIT_ASKPASS，
    // 否则任何调用方（含将来读配置的代码路径）都能悄悄替换凭据提示的接管者
    let dir = TempDir::new("process-askpass-guard");
    let env =
        dump_environment(GitRunOpts::new(dir.path()).with_env("GIT_ASKPASS", "/tmp/evil")).await;

    assert_eq!(env.get("GIT_ASKPASS").map(String::as_str), Some(""));
}

#[tokio::test]
async fn repository_redirect_variables_cannot_be_injected() {
    let dir = TempDir::new("process-redirect");
    let env = dump_environment(
        GitRunOpts::new(dir.path())
            .with_env("GIT_DIR", "/nonexistent/other-repo")
            .with_env("GIT_WORK_TREE", "/nonexistent/worktree")
            .with_env("GIT_INDEX_FILE", "/nonexistent/index"),
    )
    .await;

    assert!(!env.contains_key("GIT_DIR"), "GIT_DIR 必须被剔除");
    assert!(
        !env.contains_key("GIT_WORK_TREE"),
        "GIT_WORK_TREE 必须被剔除"
    );
    assert!(
        !env.contains_key("GIT_INDEX_FILE"),
        "GIT_INDEX_FILE 必须被剔除"
    );
}

#[tokio::test]
async fn stderr_lines_are_streamed_to_the_handler() {
    let dir = TempDir::new("process-stderr-lines");
    let process = GitProcess::new();
    init_repo(&process, dir.path()).await;

    let lines: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&lines);
    let output = process
        .run(
            &args(&["rev-parse", "--verify", "refs/heads/nope"]),
            GitRunOpts::new(dir.path())
                .with_stderr_line_handler(move |line| sink.lock().unwrap().push(line.to_owned())),
        )
        .await
        .unwrap();

    let collected = lines.lock().unwrap().clone();
    let stderr = output.stderr_lossy();

    assert!(!collected.is_empty(), "应当至少收到一行 stderr");
    assert!(
        collected.iter().any(|line| line.contains("fatal")),
        "收到的行: {collected:?}"
    );
    // 回调只是"预览"，每一行都必须能在原始字节里找到
    for line in &collected {
        assert!(
            stderr.contains(line.as_str()),
            "行 {line:?} 不在原始 stderr 中"
        );
    }
}

#[tokio::test]
async fn stdin_is_written_and_the_pipe_is_closed() {
    let dir = TempDir::new("process-stdin");
    let process = GitProcess::new();
    init_repo(&process, dir.path()).await;

    // `git hash-object --stdin` 读到 EOF 才会返回；管道没关就会一直挂着
    let output = process
        .run(
            &args(&["hash-object", "--stdin"]),
            GitRunOpts::new(dir.path()).with_stdin(b"hello\n".to_vec()),
        )
        .await
        .unwrap();

    assert!(output.success(), "stderr: {}", output.stderr_lossy());
    // `git hash-object` 对 "hello\n" 的固定结果是众所周知的 blob 哈希
    assert_eq!(
        output.stdout_lossy().trim(),
        "ce013625030ba8dba906f756967f9e9ca394464a"
    );
}

#[tokio::test]
async fn non_utf8_output_keeps_raw_bytes_and_is_flagged() {
    let dir = TempDir::new("process-bytes");
    let process = GitProcess::new();
    init_repo(&process, dir.path()).await;

    let raw = [0xFFu8, 0xFE, 0x00, 0x41, 0x80];
    std::fs::write(dir.path().join("bin.dat"), raw).unwrap();
    let staged = run_git(&process, dir.path(), &["add", "bin.dat"]).await;
    assert!(staged.success(), "git add 失败: {}", staged.stderr_lossy());
    let committed = run_git(
        &process,
        dir.path(),
        &[
            "-c",
            "user.name=tester",
            "-c",
            "user.email=tester@example.com",
            "commit",
            "-qm",
            "binary",
        ],
    )
    .await;
    assert!(
        committed.success(),
        "git commit 失败: {}",
        committed.stderr_lossy()
    );

    let output = run_git(&process, dir.path(), &["cat-file", "blob", "HEAD:bin.dat"]).await;

    assert!(output.success(), "stderr: {}", output.stderr_lossy());
    // 字节必须原样保留：在读取层做 lossy 会让下游拿到不存在的路径/损坏的内容
    assert_eq!(output.stdout, raw.to_vec());
    assert!(!output.stdout_is_utf8, "含 0xFF 的输出应被标记为非 UTF-8");
    // lossy 只是展示用，且必须能被识别出来
    assert!(output.stdout_lossy().contains('\u{FFFD}'));
}

#[tokio::test]
async fn timeout_kills_the_child_and_reports_an_error() {
    let dir = TempDir::new("process-timeout");
    let (program, command_args) = hanging_command();
    let process = GitProcess::with_program(program);

    let started = Instant::now();
    let result = process
        .run(
            &command_args,
            GitRunOpts::new(dir.path()).with_timeout(Duration::from_millis(700)),
        )
        .await;
    let elapsed = started.elapsed();

    let error = result.expect_err("超时必须返回 Err");
    assert_eq!(error.code, ErrorCode::Internal);
    assert!(
        error.message.contains("timed out"),
        "错误信息应说明超时，实际: {}",
        error.message
    );
    assert!(
        elapsed < Duration::from_secs(10),
        "超时后应立即返回，实际耗时 {elapsed:?}"
    );
}

#[tokio::test]
async fn cancellation_kills_the_child_and_reports_an_error() {
    let dir = TempDir::new("process-cancel");
    let (program, command_args) = hanging_command();
    let process = GitProcess::with_program(program);

    let token = CancellationToken::new();
    let trigger = token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        trigger.cancel();
    });

    let started = Instant::now();
    let result = process
        .run(
            &command_args,
            GitRunOpts::new(dir.path())
                .with_timeout(DEFAULT_TIMEOUT)
                .with_cancel(token),
        )
        .await;
    let elapsed = started.elapsed();

    let error = result.expect_err("取消必须返回 Err");
    assert_eq!(
        error.code,
        ErrorCode::Cancelled,
        "取消是用户主动的、预期内的结果，不应报成 INTERNAL"
    );
    assert!(
        error.message.contains("cancelled"),
        "错误信息应说明取消，实际: {}",
        error.message
    );
    assert!(
        elapsed < Duration::from_secs(10),
        "取消后应立即返回，实际耗时 {elapsed:?}"
    );
}

#[tokio::test]
async fn missing_executable_reports_a_spawn_error() {
    let dir = TempDir::new("process-missing");
    let process = GitProcess::with_program("definitely-not-a-real-program-xyz");

    let error = process
        .run(&args(&["--version"]), GitRunOpts::new(dir.path()))
        .await
        .expect_err("不存在的可执行文件必须返回 Err");

    assert_eq!(error.code, ErrorCode::Internal);
    assert!(
        error.message.contains("failed to start"),
        "错误信息应说明启动失败，实际: {}",
        error.message
    );
}

#[tokio::test]
async fn a_missing_working_directory_is_reported_as_not_found() {
    // 仓库可以在两次调用之间被删除/移动。此时 `Command::spawn` 的报错是
    // 平台相关的（Windows 上读起来像"git 没装"），因此我们在启动前先判断。
    let missing = std::env::temp_dir().join("forgedesk-process-cwd-gone-xyz");
    let _ = std::fs::remove_dir_all(&missing);
    let process = GitProcess::new();

    let error = process
        .run(&args(&["--version"]), GitRunOpts::new(&missing))
        .await
        .expect_err("工作目录不存在时必须返回 Err");

    assert_eq!(
        error.code,
        ErrorCode::NotFound,
        "应是用户可自救的「路径不存在」，而不是内部错误"
    );
    assert_eq!(
        error.hint.as_deref(),
        Some(missing.to_string_lossy().as_ref())
    );
}
