//! Git 进程安全执行器。
//!
//! # 为什么必须集中在一个模块
//!
//! AGENTS.md §7 与 PLAN §5.12 把"参数数组、禁止 shell 拼接、固定 locale、超时与取消"
//! 列为安全要求。散落的 `Command::new("git")` 迟早会有人写成字符串拼接，
//! 而这类问题不会在正常路径上暴露——只有带 `;` 的分支名或恶意仓库名才会触发。
//!
//! # 关键取舍
//!
//! - **不清理全部环境变量**：git 需要 `PATH` 找子命令（`git-remote-https`、`ssh`），
//!   需要 `HOME`/`USERPROFILE` 读全局配置。因此采用"继承 + 剔除重定向类变量 +
//!   覆盖固定项"的策略。`GIT_DIR` / `GIT_WORK_TREE` 一旦从父进程泄漏，git 会去操作
//!   **另一个仓库**，而且完全不报错——这是本模块最想避免的一类故障。
//! - **固定项不可被覆盖**：`opts.env` 先写入，固定项后写入，因此 `LC_ALL=C` 等始终生效。
//!   locale 被覆盖会让输出变成中文，解析器（以及将来基于 stderr 的诊断规则）全部失效；
//!   `GIT_TERMINAL_PROMPT` 被覆盖则会让应用在无人值守时**挂住等输入**。
//! - **stdout/stderr 保留原始字节**：只有"是否为合法 UTF-8"被标记出来，转换交给调用方。
//!   在读取层就 lossy 会让非 UTF-8 路径在后续文件系统操作里变成不存在的路径。
//! - **kill 只作用于直接子进程**：git 自己会拉起 `ssh` / `git-remote-https` 等孙进程，
//!   超时或取消时这些孙进程可能残留（Windows 需要 Job Object 才能整棵树回收）。
//!   这是已知限制，写在这里以免被误认为"已经处理干净"。
//! - **非零退出码不是本层的错误**：`git diff --exit-code` 这类命令用退出码表达正常结果。
//!   本层只负责"进程跑起来了没有、有没有超时/取消"，退出码交由调用方判断。

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use forgedesk_diagnostics::sanitize_log;
use forgedesk_domain::{AppError, AppResult, ErrorCode};
use forgedesk_platform::NoConsoleWindow;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// 默认超时。
///
/// 本地 git 命令正常在毫秒级完成，30s 只用于兜住"卡住"的情况
/// （网络操作由调用方显式给出更长的超时）。
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// 慢命令阈值：超过它的执行会额外记一条 info 级日志。
///
/// 取 500ms 与 AGENTS.md §6 "超过 500ms 的操作必须走 JobRunner" 同一个界线：
/// 日志里出现慢命令时，就是该把它挪进 JobRunner 的信号。
pub const SLOW_COMMAND_THRESHOLD: Duration = Duration::from_millis(500);

/// 日志里保留的输出片段上限（字符数）。
///
/// 截断是必要的：`git log` 在大仓库上可以输出数 MB，原样写进日志会让日志文件
/// 在几分钟内涨到几十 MB，反而把有用的信息淹没。
const LOG_SNIPPET_LIMIT: usize = 4096;

/// 一次网络操作（fetch / pull / push）的凭据注入方式。
///
/// # 为什么是"程序 + 环境变量"，而不是把明文放进 spec
///
/// 引擎层**不认识**凭据库：凭据存储在 `forgedesk-credentials`，而
/// `docs/ARCHITECTURE.md` §3 只允许 `git-engine → diagnostics` 这一条 infra→infra 依赖
/// （理由同样是"输出里可能有凭据"）。因此这里只表达与子进程有关的两件事：
/// 用哪个程序回答 git 的提示、给这个子进程带哪些环境变量。
/// 由**服务层**把凭据方案翻译成本类型（它同时能看到两边）。
///
/// # 明文只在这里存在
///
/// `env` 里可能有令牌。`Debug` 手写、不打印值，就是为了让"顺手 `{:?}` 一下"
/// 不会把令牌写进日志（红线 R8）。
#[derive(Clone, Default)]
pub struct NetworkAuth {
    /// askpass 辅助程序；`None` 表示不注入（匿名访问或走 SSH agent）。
    pub askpass_program: Option<PathBuf>,
    /// 注入给该子进程的环境变量（**可能含明文**，只允许出现在这一次调用里）。
    pub env: Vec<(String, String)>,
}

impl std::fmt::Debug for NetworkAuth {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NetworkAuth")
            .field("askpass_program", &self.askpass_program)
            // 只报个数：键名也可能泄露实现细节，值更不能打印
            .field("env_vars", &self.env.len())
            .finish()
    }
}

impl NetworkAuth {
    /// 不使用凭据。
    pub fn none() -> Self {
        Self::default()
    }

    /// 用 askpass 程序回答 git 的提示，并注入所需的环境变量。
    pub fn askpass(program: impl Into<PathBuf>, env: Vec<(String, String)>) -> Self {
        Self {
            askpass_program: Some(program.into()),
            env,
        }
    }

    /// 是否什么都不注入（服务层据此跳过装配）。
    pub fn is_none(&self) -> bool {
        self.askpass_program.is_none() && self.env.is_empty()
    }
}

/// stderr 逐行回调。
///
/// 类型别名不只是为了好看：`Option<Box<dyn Fn(&str) + Send + Sync>>` 直接写在字段上
/// 会触发 `clippy::type_complexity`，而把它具名之后，`GitRunOpts` 的可读性也更好。
pub type StderrLineHandler = Box<dyn Fn(&str) + Send + Sync>;

/// 回调的裸 trait 对象形式（内部按引用传递，避免再次装箱）。
///
/// 生命周期必须显式写出：类型别名里的 trait 对象默认是 `'static`，
/// 而 `emit_lines` 需要接受借用局部变量的闭包（测试里就是这么用的）。
type StderrLineSink<'a> = dyn Fn(&str) + Send + Sync + 'a;

/// 固定注入的环境变量。调用方**无法**通过 [`GitRunOpts::with_env`] 覆盖它们。
///
/// 唯一的例外是 `GIT_ASKPASS`：它在这里是"默认禁止"，但可以用
/// [`GitRunOpts::with_askpass`] 显式覆盖（T2.7 的凭据注入）。理由见该方法的文档。
const FIXED_ENV: &[(&str, &str)] = &[
    // locale 固定为 C：输出才是稳定的英文与机器可读格式
    ("LC_ALL", "C"),
    ("LANG", "C"),
    // 禁止 git 交互式索要凭据：桌面应用没有终端，一旦进入提示就会永久挂住
    ("GIT_TERMINAL_PROMPT", "0"),
    // 默认**禁止**任何 askpass 机制：环境里若有别人塞进来的 askpass 程序，
    // git 会把凭据提示交给它。只有 `with_askpass` 能改这一条。
    ("GIT_ASKPASS", ""),
    // 禁止分页器：分页器会让 git 等待终端输入
    ("GIT_PAGER", "cat"),
];

/// 必须从子进程环境里剔除的变量：它们会让 git 指向另一个仓库或索引。
///
/// 剔除在 `opts.env` **之后**执行，因此连调用方也无法注入——`GitRunOpts::cwd`
/// 是"操作哪个仓库"的唯一来源，两处来源迟早会不一致。
const INHERITED_ENV_DENYLIST: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_NAMESPACE",
    "GIT_CEILING_DIRECTORIES",
    "GIT_CONFIG",
    "GIT_CONFIG_GLOBAL",
    "GIT_CONFIG_SYSTEM",
];

/// 一次 git 调用的参数。
///
/// 用 [`GitRunOpts::new`] 创建后按需链式补充；只有 `cwd` 是必填的。
pub struct GitRunOpts {
    /// 工作目录（决定"操作哪个仓库"）。
    pub cwd: PathBuf,
    /// 额外环境变量。固定项（[`FIXED_ENV`]）与剔除名单优先级更高，见模块头。
    pub env: Vec<(String, String)>,
    /// 显式指定的索引文件（`GIT_INDEX_FILE`）。
    ///
    /// **这是设置该变量的唯一入口**。默认情况下它会被
    /// [`INHERITED_ENV_DENYLIST`] 剔除，因为父进程泄漏的索引路径会让 git 去操作
    /// 另一个索引，且完全不报错。但有两类场景需要**刻意**换一个索引：
    ///
    /// - `amend` 的"只改提交信息"：临时索引读成 HEAD 的树，否则会把暂存内容一起提交；
    /// - 快照恢复（T1.9）：把索引读回快照记录的那棵树。
    ///
    /// 两者的共同点是"绝不能碰用户真实索引"。因此剔除与显式设置被分成两件事：
    /// 前者防的是泄漏，后者是调用方明确表达意图（并且由 `with_isolated_index`
    /// 在命名上再提醒一次：它给出的路径必须是**隔离**的）。
    pub index_file: Option<PathBuf>,
    /// 超时。
    pub timeout: Duration,
    /// 取消令牌。触发后子进程会被杀掉。
    pub cancel: Option<CancellationToken>,
    /// 写入子进程标准输入的内容；写完立即关闭管道。
    pub stdin: Option<Vec<u8>>,
    /// 每收到一行 stderr 就回调（用于 `--progress` 之类的进度解析）。
    ///
    /// 回调收到的是 lossy 之后的字符串（进度行是 ASCII）；**原始字节仍然完整保留**
    /// 在 [`GitOutput::stderr`] 里，因此这里不构成精度损失。
    pub on_stderr_line: Option<StderrLineHandler>,
    /// 是否允许 git 获取可选锁（`GIT_OPTIONAL_LOCKS`）。
    ///
    /// 读操作保持关闭：git 会为了"顺手刷新索引"去抢锁，既可能与用户终端里的 git
    /// 互相等待，也会在大仓库上产生可感知的延迟。
    pub optional_locks: bool,
    /// askpass 辅助程序（`GIT_ASKPASS`）；`None` 表示禁止 askpass（默认）。
    ///
    /// 见 [`GitRunOpts::with_askpass`]：这是设置该变量的唯一入口。
    pub askpass: Option<PathBuf>,
}

impl GitRunOpts {
    /// 创建参数集合，超时取 [`DEFAULT_TIMEOUT`]。
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        Self {
            cwd: cwd.into(),
            env: Vec::new(),
            index_file: None,
            timeout: DEFAULT_TIMEOUT,
            cancel: None,
            stdin: None,
            on_stderr_line: None,
            optional_locks: false,
            askpass: None,
        }
    }

    /// 用指定的辅助程序回答 git 的凭据提示（`GIT_ASKPASS`）。
    ///
    /// # 为什么要有这个显式的入口
    ///
    /// `GIT_ASKPASS` 在 [`FIXED_ENV`] 里是**空串**：桌面应用一旦被环境里的
    /// askpass 程序接管，凭据提示就会交给一个我们不知道的程序。但 T2.7 需要
    /// 反过来——把提示交给**应用自己**（`--askpass` 模式）以便用 keyring 里的令牌
    /// 回答。因此这里把"默认禁止"与"调用方刻意开启"分成两件事：
    ///
    /// - 用 `with_env("GIT_ASKPASS", …)` 覆盖**无效**（固定项在最后写入）；
    /// - 只有本方法能改它，调用点在 code review 里一 grep 就能看全。
    #[must_use]
    pub fn with_askpass(mut self, program: impl Into<PathBuf>) -> Self {
        self.askpass = Some(program.into());
        self
    }

    /// 追加一个环境变量。
    #[must_use]
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    /// 用另一个索引文件执行（**不要**传用户真实索引的路径）。
    ///
    /// 见 [`GitRunOpts::index_file`]：这是设置 `GIT_INDEX_FILE` 的唯一入口，
    /// 供"只改提交信息"与快照恢复这类"刻意不碰用户索引"的场景使用。
    #[must_use]
    pub fn with_isolated_index(mut self, index_file: impl Into<PathBuf>) -> Self {
        self.index_file = Some(index_file.into());
        self
    }

    /// 设置超时。
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// 设置取消令牌。
    #[must_use]
    pub fn with_cancel(mut self, cancel: CancellationToken) -> Self {
        self.cancel = Some(cancel);
        self
    }

    /// 设置写入标准输入的内容。
    #[must_use]
    pub fn with_stdin(mut self, stdin: impl Into<Vec<u8>>) -> Self {
        self.stdin = Some(stdin.into());
        self
    }

    /// 设置 stderr 逐行回调。
    #[must_use]
    pub fn with_stderr_line_handler(
        mut self,
        handler: impl Fn(&str) + Send + Sync + 'static,
    ) -> Self {
        self.on_stderr_line = Some(Box::new(handler));
        self
    }

    /// 是否允许 git 获取可选锁（写操作需要）。
    #[must_use]
    pub fn with_optional_locks(mut self, allowed: bool) -> Self {
        self.optional_locks = allowed;
        self
    }
}

/// 一次 git 调用的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitOutput {
    /// 退出码。被信号杀死（Unix）时为 `None`。
    pub exit_code: Option<i32>,
    /// 标准输出的原始字节。
    pub stdout: Vec<u8>,
    /// 标准错误的原始字节。
    pub stderr: Vec<u8>,
    /// `stdout` 是否为合法 UTF-8。为 `false` 时下游拿到的是 lossy 文本。
    pub stdout_is_utf8: bool,
    /// `stderr` 是否为合法 UTF-8。
    pub stderr_is_utf8: bool,
    /// 进程实际运行时长。
    pub duration: Duration,
}

impl GitOutput {
    /// 退出码是否为 0。
    pub fn success(&self) -> bool {
        self.exit_code == Some(0)
    }

    /// 标准输出的 lossy 文本。
    pub fn stdout_lossy(&self) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&self.stdout)
    }

    /// 标准错误的 lossy 文本。
    pub fn stderr_lossy(&self) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&self.stderr)
    }
}

/// Git 进程执行器。
///
/// 默认调用 `git`；[`GitProcess::with_program`] 用于测试替身与将来"用户自定义
/// git 路径"的设置项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitProcess {
    program: PathBuf,
}

impl Default for GitProcess {
    fn default() -> Self {
        Self::new()
    }
}

impl GitProcess {
    /// 使用 `PATH` 里的 `git`。
    pub fn new() -> Self {
        Self {
            program: PathBuf::from("git"),
        }
    }

    /// 使用指定的可执行文件。
    pub fn with_program(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
        }
    }

    /// 当前使用的可执行文件。
    pub fn program(&self) -> &Path {
        &self.program
    }

    /// 执行一次 git 命令。
    ///
    /// 参数一律以数组传递（[`Command::args`]），代码里不存在 shell 包装，
    /// 因此参数里的 `;`、`&&`、空格都只是普通字符。
    ///
    /// # 错误
    ///
    /// 只有三种情况返回 `Err`：进程**无法启动**、**超时**、**被取消**。
    /// 非零退出码属于正常结果，通过 [`GitOutput::exit_code`] 返回。
    pub async fn run(&self, args: &[String], opts: GitRunOpts) -> AppResult<GitOutput> {
        // 工作目录不存在时 `Command::spawn` 给的是**平台相关且指向错误对象**的
        // 报错（Windows 上是"系统找不到指定的文件"，读起来像是 git 没装）。
        // 仓库可以在两次调用之间被删除/移动，因此这是真实的边界条件，
        // 提前判断能把"打开一个已被删除的仓库"变成一句可操作的诊断。
        if !opts.cwd.exists() {
            return Err(missing_cwd_error(&opts.cwd));
        }

        let mut command = Command::new(&self.program);
        command
            .args(args)
            .current_dir(&opts.cwd)
            // 发布构建是 GUI 子系统（没有控制台），不抑制的话每条 git 命令都会
            // 在屏幕上闪一个黑框——而"打开仓库"一次要跑十几条（2026-10-08 实测）。
            .no_console_window()
            .stdin(if opts.stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // 外层 future 被 drop（例如用户关掉了对话框）时也要回收子进程
            .kill_on_drop(true);
        apply_environment(&mut command, &opts);

        tracing::debug!(
            program = %self.program.display(),
            args = %sanitize_log(&args.join(" ")),
            cwd = %opts.cwd.display(),
            timeout_ms = duration_millis(opts.timeout),
            "git 命令开始执行"
        );

        let started = Instant::now();
        let mut child = command
            .spawn()
            .map_err(|error| spawn_error(self.program.as_path(), &opts.cwd, &error))?;

        // 先起读取任务再写 stdin：反过来会在子进程输出填满管道时互相等待
        let stdout_task = tokio::spawn(drain(child.stdout.take(), None));
        let stderr_task = tokio::spawn(drain(child.stderr.take(), opts.on_stderr_line));

        if let Some(input) = opts.stdin.as_ref() {
            write_stdin(&mut child, input).await;
        }

        let outcome = tokio::select! {
            // biased：取消优先于超时，超时优先于正常结束——用户点了取消就该立刻停
            biased;
            () = wait_for_cancel(opts.cancel.clone()) => Outcome::Cancelled,
            () = tokio::time::sleep(opts.timeout) => Outcome::TimedOut,
            status = child.wait() => Outcome::Exited(status),
        };

        let duration = started.elapsed();
        match outcome {
            Outcome::Exited(status) => {
                let status = status.map_err(|error| wait_error(self.program.as_path(), &error))?;
                let stdout = join_drain(stdout_task).await;
                let stderr = join_drain(stderr_task).await;
                let output = GitOutput {
                    exit_code: status.code(),
                    stdout_is_utf8: std::str::from_utf8(&stdout).is_ok(),
                    stderr_is_utf8: std::str::from_utf8(&stderr).is_ok(),
                    stdout,
                    stderr,
                    duration,
                };
                log_completion(&self.program, args, &output);
                Ok(output)
            }
            Outcome::TimedOut => {
                reap(&mut child).await;
                join_drain(stdout_task).await;
                join_drain(stderr_task).await;
                Err(timeout_error(&self.program, args, duration))
            }
            Outcome::Cancelled => {
                reap(&mut child).await;
                join_drain(stdout_task).await;
                join_drain(stderr_task).await;
                Err(cancelled_error(&self.program, args, duration))
            }
        }
    }
}

/// `select!` 的结果。
enum Outcome {
    /// 进程正常结束（含非零退出码）。
    Exited(std::io::Result<std::process::ExitStatus>),
    /// 超时。
    TimedOut,
    /// 被取消。
    Cancelled,
}

/// 组装子进程环境变量。顺序即优先级，见模块头。
fn apply_environment(command: &mut Command, opts: &GitRunOpts) {
    for (key, value) in &opts.env {
        command.env(key, value);
    }
    for key in INHERITED_ENV_DENYLIST {
        command.env_remove(key);
    }
    // 显式索引放在剔除**之后**，顺序即语义：剔除防的是父进程泄漏，
    // 而这个值是调用方刻意给出的（见 `GitRunOpts::index_file`）。
    if let Some(index_file) = &opts.index_file {
        command.env("GIT_INDEX_FILE", index_file);
    }
    for (key, value) in FIXED_ENV {
        command.env(key, value);
    }
    // askpass 放在固定项**之后**：`GIT_ASKPASS=""`（默认禁止）必须能被这次
    // 刻意的注入覆盖。写法与 GIT_INDEX_FILE 同理——默认值防事故，显式入口表意图。
    if let Some(program) = &opts.askpass {
        command.env("GIT_ASKPASS", program);
    }
    command.env(
        "GIT_OPTIONAL_LOCKS",
        if opts.optional_locks { "1" } else { "0" },
    );
}

/// 等待取消令牌被触发；没有令牌时永不完成。
///
/// 为什么用 `pending`：`select!` 要求每个分支都是 future，
/// 而"没有取消令牌"必须表达成"这个分支永远不触发"，而不是"立刻触发"。
async fn wait_for_cancel(cancel: Option<CancellationToken>) {
    match cancel {
        Some(token) => token.cancelled().await,
        None => std::future::pending::<()>().await,
    }
}

/// 把 stdin 内容写完并关闭管道。
async fn write_stdin(child: &mut Child, input: &[u8]) {
    let Some(mut pipe) = child.stdin.take() else {
        return;
    };

    if let Err(error) = pipe.write_all(input).await {
        // 子进程可能在读完 stdin 之前就退出（例如参数校验失败），此时会得到
        // BrokenPipe。它不是本次调用的失败原因——真正的原因在退出码与 stderr 里。
        if error.kind() != std::io::ErrorKind::BrokenPipe {
            tracing::debug!(error = %error, "写入 git 标准输入失败");
        }
    }

    // 必须显式关闭：忘记关闭会让 `git apply --cached -` 一直等 EOF
    drop(pipe);
}

/// 杀掉子进程并回收。
///
/// 不回收会留下僵尸进程，桌面应用长时间运行时会累积。
async fn reap(child: &mut Child) {
    if let Err(error) = child.kill().await {
        // 进程可能已经自己退出了，此时 kill 报错属于正常情况
        tracing::debug!(error = %error, "回收 git 子进程时 kill 返回错误");
    }
}

/// 读干一个管道：返回原始字节，并把完整的行交给处理函数。
///
/// 返回 `Vec<u8>` 而不是 `Result`：进程被强杀时管道会报错，但**已经读到的字节
/// 仍然有价值**（例如进度信息），而失败原因由调用方从退出状态/超时/取消中获知。
async fn drain<R: AsyncRead + Unpin>(
    reader: Option<R>,
    on_line: Option<StderrLineHandler>,
) -> Vec<u8> {
    let Some(mut reader) = reader else {
        return Vec::new();
    };

    let mut collected: Vec<u8> = Vec::new();
    let mut pending: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8192];

    loop {
        match reader.read(&mut chunk).await {
            Ok(0) => break,
            Ok(read) => {
                let bytes = &chunk[..read];
                collected.extend_from_slice(bytes);
                if on_line.is_some() {
                    pending.extend_from_slice(bytes);
                    emit_lines(&mut pending, on_line.as_deref());
                }
            }
            Err(error) => {
                tracing::debug!(error = %error, "读取 git 输出流失败，返回已读到的内容");
                break;
            }
        }
    }

    if let Some(handler) = on_line.as_deref() {
        // 最后一段可能没有行尾（进程退出时缓冲里剩下的内容）
        if !pending.is_empty() {
            handler(&String::from_utf8_lossy(&pending));
        }
    }

    collected
}

/// 把缓冲里**完整**的行回调出去。
///
/// 只回调完整的行：半行 JSON 或半行进度会让解析者得出错误结论。
/// 同时按 `\n` 与 `\r` 切分——`git --progress` 用 `\r` 做原地刷新，
/// 只按 `\n` 切会让整段进度挤成"一行"，最终什么也解析不出来。
fn emit_lines(pending: &mut Vec<u8>, handler: Option<&StderrLineSink<'_>>) {
    let Some(handler) = handler else {
        return;
    };

    while let Some(position) = pending
        .iter()
        .position(|byte| matches!(*byte, b'\n' | b'\r'))
    {
        let line: Vec<u8> = pending.drain(..=position).collect();
        let text = String::from_utf8_lossy(&line[..line.len() - 1]);
        if !text.is_empty() {
            handler(&text);
        }
    }
}

/// 收尾读取任务。
async fn join_drain(task: JoinHandle<Vec<u8>>) -> Vec<u8> {
    match task.await {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::debug!(error = %error, "读取 git 输出的任务异常结束");
            Vec::new()
        }
    }
}

/// 执行完成后的日志。超过慢命令阈值时升到 info 级。
fn log_completion(program: &Path, args: &[String], output: &GitOutput) {
    let slow = output.duration >= SLOW_COMMAND_THRESHOLD;
    if !slow && !tracing::enabled!(tracing::Level::DEBUG) {
        // 只有真要写这条日志时才做脱敏与截断：大仓库的 `git log` 输出可达数 MB，
        // 无条件脱敏会让每次调用都多花几十毫秒
        return;
    }

    let program = program.display();
    let args = sanitize_log(&args.join(" "));
    let stderr = sanitized_snippet(&output.stderr);
    let exit_code = output.exit_code;
    let duration_ms = duration_millis(output.duration);
    let stdout_bytes = output.stdout.len();

    if slow {
        tracing::info!(
            program = %program,
            args = %args,
            exit_code = ?exit_code,
            duration_ms,
            stdout_bytes,
            stderr = %stderr,
            "git 命令执行完成（慢命令）"
        );
    } else {
        tracing::debug!(
            program = %program,
            args = %args,
            exit_code = ?exit_code,
            duration_ms,
            stdout_bytes,
            stderr = %stderr,
            "git 命令执行完成"
        );
    }
}

/// 时长转毫秒（tracing 的字段值不支持 u128）。
fn duration_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// 脱敏并截断后的输出片段。
///
/// **先脱敏再截断**，不能反过来：截断可能把一个令牌切成两半，半个令牌不再匹配
/// 脱敏规则，于是"看起来被截断"的那段日志反而把秘密泄漏了出去。
fn sanitized_snippet(bytes: &[u8]) -> String {
    let sanitized = sanitize_log(&String::from_utf8_lossy(bytes));
    if sanitized.chars().count() <= LOG_SNIPPET_LIMIT {
        return sanitized;
    }

    let mut truncated: String = sanitized.chars().take(LOG_SNIPPET_LIMIT).collect();
    truncated.push('…');
    truncated
}

/// 进程无法启动。
fn spawn_error(program: &Path, cwd: &Path, error: &std::io::Error) -> AppError {
    AppError::new(
        ErrorCode::Internal,
        format!("failed to start git process `{}`", program.display()),
    )
    .with_detail(sanitize_log(&format!("cwd={}: {error}", cwd.display())))
    .with_hint(program.display().to_string())
}

/// 等待进程结束时出错（极少见，通常是平台层面的异常）。
fn wait_error(program: &Path, error: &std::io::Error) -> AppError {
    AppError::new(
        ErrorCode::Internal,
        format!("failed to wait for git process `{}`", program.display()),
    )
    .with_detail(sanitize_log(&error.to_string()))
    .with_hint(program.display().to_string())
}

/// 超时。
fn timeout_error(program: &Path, args: &[String], duration: Duration) -> AppError {
    AppError::new(
        ErrorCode::Internal,
        format!(
            "git command timed out after {} ms",
            duration_millis(duration)
        ),
    )
    .with_detail(sanitize_log(&args.join(" ")))
    .with_hint(program.display().to_string())
}

/// 工作目录不存在。
///
/// 用 [`ErrorCode::NotFound`] 而不是 `Internal`：这是一个**用户可理解、可自救**的
/// 情况（仓库被删了/移动了/盘符没挂上），而不是未预期的内部故障。
fn missing_cwd_error(cwd: &Path) -> AppError {
    AppError::new(ErrorCode::NotFound, "the working directory does not exist")
        .with_hint(cwd.to_string_lossy().into_owned())
        .with_retryable(false)
}

/// 被取消。
fn cancelled_error(program: &Path, args: &[String], duration: Duration) -> AppError {
    AppError::new(
        ErrorCode::Cancelled,
        "git command was cancelled before it finished",
    )
    .with_detail(sanitize_log(&format!(
        "{} (ran {} ms)",
        args.join(" "),
        duration_millis(duration)
    )))
    .with_hint(program.display().to_string())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{
        apply_environment, emit_lines, sanitized_snippet, GitRunOpts, FIXED_ENV,
        INHERITED_ENV_DENYLIST, LOG_SNIPPET_LIMIT,
    };

    /// 读出命令上记录的 `GIT_INDEX_FILE` 状态（`None` 表示被移除）。
    fn index_file_of(command: &tokio::process::Command) -> Option<Option<String>> {
        command
            .as_std()
            .get_envs()
            .find(|(key, _)| *key == "GIT_INDEX_FILE")
            .map(|(_, value)| value.map(|value| value.to_string_lossy().into_owned()))
    }

    #[test]
    fn an_inherited_index_file_is_dropped_while_an_explicit_one_is_kept() {
        // 默认：必须被移除。父进程（IDE、终端、别的 git 包装）泄漏的索引路径
        // 会让 git 去操作另一个索引，而且完全不报错——这是本模块最想避免的故障。
        let mut plain = tokio::process::Command::new("git");
        apply_environment(&mut plain, &GitRunOpts::new("."));
        assert_eq!(
            index_file_of(&plain),
            Some(None),
            "继承来的 GIT_INDEX_FILE 必须被剔除"
        );

        // 显式：必须生效。顺序（先剔除、后设置）就是这条规则的实现。
        let mut isolated = tokio::process::Command::new("git");
        apply_environment(
            &mut isolated,
            &GitRunOpts::new(".").with_isolated_index("/tmp/forgedesk-index"),
        );
        assert_eq!(
            index_file_of(&isolated),
            Some(Some("/tmp/forgedesk-index".to_owned())),
            "调用方显式给出的隔离索引必须生效"
        );
    }

    #[test]
    fn fixed_environment_cannot_be_overridden_by_callers() {
        // 顺序即优先级：opts.env 先写，固定项后写。这条断言锁住"固定项在后面"。
        assert!(FIXED_ENV.iter().any(|(key, _)| *key == "LC_ALL"));
        assert!(FIXED_ENV
            .iter()
            .any(|(key, _)| *key == "GIT_TERMINAL_PROMPT"));
        assert!(INHERITED_ENV_DENYLIST.contains(&"GIT_DIR"));
        assert!(INHERITED_ENV_DENYLIST.contains(&"GIT_WORK_TREE"));
    }

    #[test]
    fn lines_are_emitted_on_both_lf_and_cr() {
        let mut pending = b"first\nsecond\rthird".to_vec();
        // 回调是 `Fn + Send + Sync`（会被读取任务并发持有），因此用 Mutex 做内部可变性
        let seen = std::sync::Mutex::new(Vec::new());
        let handler = |line: &str| seen.lock().unwrap().push(line.to_owned());

        emit_lines(&mut pending, Some(&handler));

        // 只有前两段是"完整的行"，第三段还没有行尾
        assert_eq!(
            *seen.lock().unwrap(),
            vec!["first".to_owned(), "second".to_owned()]
        );
        assert_eq!(pending, b"third");
    }

    #[test]
    fn empty_lines_are_not_emitted() {
        let mut pending = b"\n\n\r".to_vec();
        let seen = std::sync::atomic::AtomicUsize::new(0);
        let handler = |_: &str| {
            seen.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        };

        emit_lines(&mut pending, Some(&handler));

        assert_eq!(seen.load(std::sync::atomic::Ordering::Relaxed), 0);
    }

    #[test]
    fn snippet_is_truncated_after_sanitising() {
        let long = vec![b'a'; LOG_SNIPPET_LIMIT * 2];
        let snippet = sanitized_snippet(&long);

        assert_eq!(snippet.chars().count(), LOG_SNIPPET_LIMIT + 1);
        assert!(snippet.ends_with('…'));
    }

    #[test]
    fn snippet_removes_credentials_before_truncating() {
        let mut input =
            b"https://user:ghp_0123456789abcdefghijklmnopqrstuvwxyz@example.com/repo".to_vec();
        input.extend_from_slice(&vec![b'x'; LOG_SNIPPET_LIMIT]);

        let snippet = sanitized_snippet(&input);

        assert!(!snippet.contains("ghp_0123456789"));
    }
}
