//! `CliGitEngine`：系统 git CLI 实现。
//!
//! 模块划分：
//!
//! - [`args`]：参数构造（纯函数，安全边界，见该模块头）；
//! - [`read`] / [`write`]：读操作与写操作的实现与输出解析；
//! - 本文件：引擎结构体、`GitEngine` 实现、以及统一的"跑一条命令"的收口。
//!
//! # 为什么所有调用都要经过本文件的三个 `run_*`
//!
//! 超时、可选锁、错误分类、stderr 脱敏、非零退出的判定，这五件事每个调用点都要做对。
//! 让它们各自 `process.run(...)` 意味着五处复制粘贴，而复制出来的代码里
//! "少判一个退出码"或"忘了脱敏"都不会有测试发现。

pub mod args;
pub mod conflict;
pub mod read;
pub mod write;

use std::path::{Path, PathBuf};
use std::time::Duration;

use forgedesk_diagnostics::sanitize_log;
use forgedesk_domain::git::{
    ApplyPatchSpec, Branch, BranchCreateSpec, BranchDeleteSpec, BranchRenameSpec,
    BranchSetUpstreamSpec, CheckoutSpec, CherryPickSpec, CloneSpec, Commit, CommitSpec,
    ConflictAbortOutcome, ConflictContinueOutcome, ConflictFileDetail, ConflictOpKind,
    ConflictState, DiffReport, DiffSpec, DiscardSpec, FetchOutcome, FetchSpec, InitSpec,
    LineEnding, LogQuery, MergeOutcome, MergeSpec, Page, PullOutcome, PullSpec, PushOutcome,
    PushSpec, ReflogEntry, Remote, ReorderSpec, RepoId, RepoPath, RepositoryInfo, ResetSpec,
    RevertSpec, StageSpec, StashEntry, StashOutcome, StashSpec, StatusQuery, StatusReport,
    SwitchStrategy, Tag, TagCreateSpec, TagDeleteSpec, TakeSide,
};
use forgedesk_domain::{AppError, AppResult, ErrorCode};

use super::progress::ProgressSink;
use super::{not_implemented, EngineId, GitEngine, ProbeOutput};
use crate::process::{GitOutput, GitProcess, GitRunOpts, NetworkAuth};

/// 本地操作的超时。
///
/// 与 [`crate::process::DEFAULT_TIMEOUT`] 一致，单独命名是为了让"网络操作有更长的
/// 超时"这件事在调用点一眼可见。
const LOCAL_TIMEOUT: Duration = crate::process::DEFAULT_TIMEOUT;

/// 网络操作（clone/fetch/pull/push）的超时。
///
/// 15 分钟不是"觉得够了"，而是"超过它基本可以确定是卡死而不是慢"：
/// 一个正常的仓库推送在慢网络下也就几分钟。卡死的连接必须被杀掉，
/// 否则用户只能强杀应用。
const NETWORK_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// 远端探活（`git ls-remote`）的超时。
///
/// 5 秒是"测试连接"这个动作的合理等待窗口：用户点按钮是为了**立刻**知道
/// 通不通，而不是等一个可能永远不来的回答。慢网络下会误报失败——这是刻意的
/// 取舍：探活是排错工具，不是传输通道（真正的 fetch/push 有 15 分钟）。
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// 错误详情里保留的 stderr 长度上限。
const ERROR_DETAIL_LIMIT: usize = 8192;

/// 命令类别，决定是否允许 git 获取可选锁。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RunKind {
    /// 读操作：`GIT_OPTIONAL_LOCKS=0`，避免为了刷新索引与用户终端里的 git 争锁。
    Read,
    /// 写操作：允许获取锁。
    Write,
}

impl RunKind {
    const fn optional_locks(self) -> bool {
        matches!(self, Self::Write)
    }
}

/// 系统 git 命令行实现。
pub struct CliGitEngine {
    process: GitProcess,
    bridge: super::BlockingBridge,
}

impl std::fmt::Debug for CliGitEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CliGitEngine")
            .field("program", &self.process.program())
            .finish_non_exhaustive()
    }
}

impl CliGitEngine {
    /// 使用 `PATH` 里的 `git`。
    pub fn new() -> AppResult<Self> {
        Self::with_process(GitProcess::new())
    }

    /// 使用指定的 git 可执行文件（测试替身与"用户自定义 git 路径"）。
    pub fn with_program(program: impl Into<PathBuf>) -> AppResult<Self> {
        Self::with_process(GitProcess::with_program(program))
    }

    /// 用给定的进程执行器创建。
    pub fn with_process(process: GitProcess) -> AppResult<Self> {
        Ok(Self {
            process,
            bridge: super::BlockingBridge::new()?,
        })
    }

    /// 底层进程执行器（诊断与测试用）。
    pub fn process(&self) -> &GitProcess {
        &self.process
    }

    /// 系统 git 的版本（`git --version`）。
    ///
    /// 为什么**不放在** `GitEngine` trait 上：libgit2 实现给不出"系统 git 的版本"
    /// （它自己就是另一套实现），而"用户机器上的 git 太旧"这件事只有 CLI 侧知道。
    /// 调用方（`services`）持有具体类型，因此不需要为它污染 trait。
    ///
    /// 在 `.` 下执行：`--version` 与工作目录无关，而强制要求一个 cwd 只是为了让
    /// 进程执行器的签名统一。
    pub fn version(&self) -> AppResult<forgedesk_domain::git::GitVersion> {
        let invocation = args::GitInvocation::new(vec!["--version".to_owned()]);
        let output = self.run_checked_at(Path::new("."), invocation, RunKind::Read)?;
        let stdout = output.stdout_lossy();
        forgedesk_domain::git::GitVersion::parse(&stdout).ok_or_else(|| {
            AppError::new(
                ErrorCode::Internal,
                "could not parse the git version output",
            )
            .with_detail(stdout.trim().to_owned())
            .with_retryable(false)
        })
    }

    /// 读取仓库级配置（`.git/config`，含 `include.path` 展开的内容）。
    ///
    /// 用途：打开仓库时的危险项审计（`domain::git::audit`）。三条设计取舍：
    ///
    /// - **只读 `--local`**：`global` / `system` 是用户自己机器上的选择，
    ///   不属于"陌生仓库带来的风险"（见 `ConfigScope` 的说明）；
    /// - **跟随 `include.path`**（不加 `--no-includes`）：被 include 进来的
    ///   配置同样会被 git 执行，漏掉它们等于给审计留后门；
    /// - **值先脱敏再返回**：这些值会展示在界面上、写进日志，而
    ///   `core.sshCommand` / `filter.*.clean` 里内嵌凭据是常见写法（红线 R8）。
    ///   脱敏发生在**读入时**，因此审计报告从产生那一刻起就可安全传播。
    ///
    /// 读不到配置（不是仓库、`.git/config` 缺失）返回空列表而不是错误：
    /// 审计问的是"配置里有没有危险项"，没有配置就意味着没有危险项。
    pub fn repository_config(
        &self,
        repo: &RepoId,
    ) -> AppResult<Vec<forgedesk_domain::git::ConfigEntry>> {
        let invocation = args::GitInvocation::new(vec![
            "config".to_owned(),
            "--local".to_owned(),
            "--list".to_owned(),
            "--null".to_owned(),
        ]);
        let output = self.run_at(repo.root(), invocation, RunKind::Read)?;
        if !output.success() {
            return Ok(Vec::new());
        }

        Ok(crate::parsers::parse_config_list(&output.stdout)
            .into_iter()
            .map(|entry| forgedesk_domain::git::ConfigEntry {
                value: sanitize_log(&entry.value),
                ..entry
            })
            .collect())
    }

    /// 读取一条配置值（跟随 git 默认解析链：local → global → system）。
    ///
    /// 与 [`Self::repository_config`] 的区别：那条**只读 `--local`**，用于把
    /// "仓库自带的危险配置"展示给用户（全局配置不是仓库带来的风险）；本方法
    /// 读的是"我是谁"这类**用户身份**（T2.3 的"仅显示我的提交"要用 user.email），
    /// 必须走完整解析链才找得到全局身份。`user.email` 不是凭据（红线 R8 的
    /// 范围是 token / 密码 / 私钥），因此不需要脱敏。
    ///
    /// `key` 只接受 `[A-Za-z0-9.-]`：key 会进参数数组（注入本就被防住），
    /// 这道校验防的是调用方把任意子命令拼进 key 的手滑。取不到时返回 `None`
    /// （git 对未设置的键以退出码 1 结束，是"没有"而不是"错误"）。
    pub fn config_value(&self, repo: &RepoId, key: &str) -> AppResult<Option<String>> {
        if key.is_empty()
            || !key
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'-')
        {
            return Err(forgedesk_domain::AppError::new(
                forgedesk_domain::ErrorCode::Validation,
                "the config key contains unsupported characters",
            )
            .with_detail(format!("key: {key}")));
        }
        let invocation = args::GitInvocation::new(vec![
            "config".to_owned(),
            "--get".to_owned(),
            key.to_owned(),
        ]);
        let output = self.run_at(repo.root(), invocation, RunKind::Read)?;
        if !output.success() {
            return Ok(None);
        }
        let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        Ok(if value.is_empty() { None } else { Some(value) })
    }

    /// 在指定目录执行一条命令，返回原始结果（不判退出码）。
    pub(crate) fn run_at(
        &self,
        cwd: &Path,
        invocation: args::GitInvocation,
        kind: RunKind,
    ) -> AppResult<GitOutput> {
        self.run_at_with_timeout(cwd, invocation, kind, LOCAL_TIMEOUT, None)
    }

    /// 执行一条命令并断言退出码为 0。
    pub(crate) fn run_checked_at(
        &self,
        cwd: &Path,
        invocation: args::GitInvocation,
        kind: RunKind,
    ) -> AppResult<GitOutput> {
        let output = self.run_at(cwd, invocation, kind)?;
        ensure_success(&output)?;
        Ok(output)
    }

    /// 在仓库里执行读命令并断言成功。
    pub(crate) fn run_read(
        &self,
        repo: &RepoId,
        invocation: args::GitInvocation,
    ) -> AppResult<GitOutput> {
        self.run_checked_at(repo.root(), invocation, RunKind::Read)
    }

    /// 在仓库里执行写命令并断言成功。
    pub(crate) fn run_write(
        &self,
        repo: &RepoId,
        invocation: args::GitInvocation,
    ) -> AppResult<GitOutput> {
        self.run_checked_at(repo.root(), invocation, RunKind::Write)
    }

    /// 网络操作：更长的超时 + 进度回调 + 退出码断言。
    pub(crate) fn run_network(
        &self,
        cwd: &Path,
        invocation: args::GitInvocation,
        progress: &ProgressSink,
        cancel: &tokio_util::sync::CancellationToken,
        auth: &NetworkAuth,
    ) -> AppResult<GitOutput> {
        let output = self.run_network_raw(cwd, invocation, progress, cancel, auth)?;
        ensure_success(&output)?;
        Ok(output)
    }

    /// 网络操作，但**不**断言退出码（调用方按 stderr 自行判定结果）。
    ///
    /// 为什么 push 需要它：`git push` 被拒绝时以非零退出码结束，可这是**可修复的
    /// 业务结果**而不是"命令失败"——界面要按引用给出"先拉取 / force-with-lease /
    /// 取消"三条路（T2.6 验收）。断言成功会把这份信息压成一条错误，
    /// 丢掉"哪个引用被拒、是不是非快进"。
    pub(crate) fn run_network_raw(
        &self,
        cwd: &Path,
        invocation: args::GitInvocation,
        progress: &ProgressSink,
        cancel: &tokio_util::sync::CancellationToken,
        auth: &NetworkAuth,
    ) -> AppResult<GitOutput> {
        // 与 run_at_with_timeout 相同的装配，只多一个取消令牌：
        // 取消 = 进程层的 kill_on_drop 生效（含 fetch/pull/push 拉起的孙进程语义见 process.rs）
        let mut opts = GitRunOpts::new(cwd)
            .with_timeout(NETWORK_TIMEOUT)
            .with_optional_locks(RunKind::Write.optional_locks())
            .with_cancel(cancel.clone());
        if let Some(index_file) = invocation.index_file.as_ref() {
            opts = opts.with_isolated_index(index_file);
        }
        if let Some(stdin) = invocation.stdin {
            opts = opts.with_stdin(stdin);
        }
        if progress.is_active() {
            opts = opts.with_stderr_line_handler(progress.handler());
        }
        // 凭据注入（T2.7）：环境变量在前、askpass 程序在后（后者要覆盖 FIXED_ENV 的默认空值）
        for (key, value) in &auth.env {
            opts = opts.with_env(key.clone(), value.clone());
        }
        if let Some(program) = &auth.askpass_program {
            opts = opts.with_askpass(program.clone());
        }
        let output = self
            .bridge
            .block_on(self.process.run(&invocation.args, opts))??;
        Ok(output)
    }

    /// 探活远端（`git ls-remote`，只读）。
    ///
    /// 超时短（[`PROBE_TIMEOUT`]）：这是界面上"测试连接"按钮的等待窗口，
    /// 让用户等 15 分钟去确认一根线通不通是不可接受的。
    ///
    /// 名字带 `impl` 后缀是为了避开与 trait 方法同名带来的解析歧义
    /// （`fn probe_remote` 里再写 `self.probe_remote(…)` 读起来像递归）。
    pub(crate) fn probe_remote_impl(
        &self,
        cwd: &Path,
        url: &str,
        auth: &NetworkAuth,
    ) -> AppResult<usize> {
        let mut opts = GitRunOpts::new(cwd)
            .with_timeout(PROBE_TIMEOUT)
            .with_optional_locks(false);
        for (key, value) in &auth.env {
            opts = opts.with_env(key.clone(), value.clone());
        }
        if let Some(program) = &auth.askpass_program {
            opts = opts.with_askpass(program.clone());
        }

        // `--` 把 URL 与选项隔开：远端 URL 来自用户输入，不加分隔符时
        // 一个 `--upload-pack=...` 形状的"URL"会被 git 当成选项执行
        let args = vec!["ls-remote".to_owned(), "--".to_owned(), url.to_owned()];
        let output = self.bridge.block_on(self.process.run(&args, opts))??;

        if !output.success() {
            // 按 stderr 分类：SSH 主机指纹 / 公钥被拒 / 证书 / 代理各自成码（T2.7）
            let stderr = output.stderr_lossy();
            let code = ErrorCode::classify(&stderr);
            return Err(
                AppError::new(code, "git ls-remote failed").with_detail(sanitize_log(&stderr))
            );
        }

        Ok(output
            .stdout_lossy()
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count())
    }

    /// 读取 ssh-agent 的密钥清单（`ssh-add -l`）。
    ///
    /// 与 [`Self::probe_remote_impl`] 同一套超时：这也是"排错按钮"背后的动作，
    /// 不能让用户等；`ssh-add` 本身只在问一个本地进程，正常是毫秒级。
    pub(crate) fn probe_ssh_agent_impl(&self) -> AppResult<ProbeOutput> {
        // 工作目录固定用系统临时目录：`ssh-add` 与 cwd 无关，而沿用"当前目录"
        // 会在那个目录被删掉时抛出与本操作无关的错误（用户会以为 SSH 坏了）
        let cwd = std::env::temp_dir();
        let opts = GitRunOpts::new(&cwd).with_timeout(PROBE_TIMEOUT);
        let args = vec!["-l".to_owned()];

        // `Err` 只表示程序跑不起来（未安装 ssh-add / 超时）；退出码 1、2 是正常结局
        let output = self
            .bridge
            .block_on(GitProcess::with_program("ssh-add").run(&args, opts))??;

        Ok(ProbeOutput {
            // `stdout_lossy` 返回借用形态（`Cow`）：这里要带走所有权
            stdout: output.stdout_lossy().into_owned(),
            exit_code: output.exit_code,
        })
    }

    /// 在指定目录执行写命令并断言成功（用于 `init` 这类目标目录还不是仓库的场景）。
    pub(crate) fn run_write_at(
        &self,
        cwd: &Path,
        invocation: args::GitInvocation,
    ) -> AppResult<GitOutput> {
        self.run_checked_at(cwd, invocation, RunKind::Write)
    }

    /// 统一的执行入口。
    fn run_at_with_timeout(
        &self,
        cwd: &Path,
        invocation: args::GitInvocation,
        kind: RunKind,
        timeout: Duration,
        progress: Option<&ProgressSink>,
    ) -> AppResult<GitOutput> {
        let mut opts = GitRunOpts::new(cwd)
            .with_timeout(timeout)
            .with_optional_locks(kind.optional_locks());
        if let Some(index_file) = invocation.index_file.as_ref() {
            opts = opts.with_isolated_index(index_file);
        }
        if let Some(stdin) = invocation.stdin {
            opts = opts.with_stdin(stdin);
        }
        for (key, value) in &invocation.env {
            opts = opts.with_env(key.clone(), value.clone());
        }
        if let Some(progress) = progress.filter(|sink| sink.is_active()) {
            opts = opts.with_stderr_line_handler(progress.handler());
        }

        self.bridge
            .block_on(self.process.run(&invocation.args, opts))?
    }
}

/// 把非零退出码转成错误。
///
/// 分类交给 `ErrorCode::classify`（领域层纯函数，T0.6 已实现并单测），
/// 而不是在这里写一串 `if stderr.contains(...)`——那会把"哪些 stderr 对应哪类错误"
/// 这份知识复制到第二个地方，而 M5 的诊断规则引擎还要用第三遍。
pub(crate) fn ensure_success(output: &GitOutput) -> AppResult<&GitOutput> {
    if output.success() {
        return Ok(output);
    }

    let stderr = output.stderr_lossy();
    let code = ErrorCode::classify(&stderr);
    let detail = sanitize_log(&stderr);
    let detail = truncate(detail, ERROR_DETAIL_LIMIT);

    Err(
        AppError::new(code, format!("git exited with code {:?}", output.exit_code))
            .with_detail(detail)
            .with_retryable(code.default_retryable()),
    )
}

/// 按字符数截断（`detail` 可能是一整屏的 stderr）。
pub(crate) fn truncate(mut text: String, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text;
    }
    let mut truncated: String = text.chars().take(limit).collect();
    truncated.push('…');
    text = truncated;
    text
}

impl GitEngine for CliGitEngine {
    fn id(&self) -> EngineId {
        EngineId::Cli
    }

    fn discover(&self, path: &Path) -> AppResult<RepositoryInfo> {
        read::discover(self, path)
    }

    fn status(&self, repo: &RepoId, query: &StatusQuery) -> AppResult<StatusReport> {
        read::status(self, repo, query)
    }

    fn discard_worktree(&self, repo: &RepoId, spec: &DiscardSpec) -> AppResult<()> {
        write::discard_worktree(self, repo, spec)
    }

    fn diff(&self, repo: &RepoId, spec: DiffSpec) -> AppResult<DiffReport> {
        read::diff(self, repo, &spec)
    }

    fn diff_patch(&self, repo: &RepoId, spec: &DiffSpec) -> AppResult<Vec<u8>> {
        read::diff_patch(self, repo, spec)
    }

    fn log(&self, repo: &RepoId, query: LogQuery) -> AppResult<Page<Commit>> {
        read::log(self, repo, &query)
    }

    fn show(&self, repo: &RepoId, revision: &str) -> AppResult<Commit> {
        read::show(self, repo, revision)
    }

    fn count_commits(&self, repo: &RepoId, range: &str, exclude: &[String]) -> AppResult<u32> {
        read::count_commits(self, repo, range, exclude)
    }

    fn branch_list(&self, repo: &RepoId) -> AppResult<Vec<Branch>> {
        read::branch_list(self, repo)
    }

    fn tag_list(&self, repo: &RepoId) -> AppResult<Vec<Tag>> {
        read::tag_list(self, repo)
    }

    fn remote_list(&self, repo: &RepoId) -> AppResult<Vec<Remote>> {
        read::remote_list(self, repo)
    }

    fn stash_list(&self, repo: &RepoId) -> AppResult<Vec<StashEntry>> {
        read::stash_list(self, repo)
    }

    fn reflog(&self, repo: &RepoId, limit: usize) -> AppResult<Vec<ReflogEntry>> {
        read::reflog(self, repo, limit)
    }

    fn init(&self, path: &Path, spec: InitSpec) -> AppResult<RepositoryInfo> {
        write::init(self, path, &spec)
    }

    fn clone(
        &self,
        spec: CloneSpec,
        progress: &ProgressSink,
        auth: &NetworkAuth,
    ) -> AppResult<RepositoryInfo> {
        write::clone(self, &spec, progress, auth)
    }

    fn stage(&self, repo: &RepoId, spec: StageSpec) -> AppResult<()> {
        write::stage(self, repo, &spec)
    }

    fn unstage(&self, repo: &RepoId, spec: StageSpec) -> AppResult<()> {
        write::unstage(self, repo, &spec)
    }

    fn apply_patch(&self, repo: &RepoId, spec: &ApplyPatchSpec) -> AppResult<()> {
        write::apply_patch(self, repo, spec)
    }

    fn index_tree(&self, repo: &RepoId) -> AppResult<String> {
        read::index_tree(self, repo)
    }

    fn index_entry_count(&self, repo: &RepoId) -> AppResult<u64> {
        read::index_entry_count(self, repo)
    }

    fn head_tree(&self, repo: &RepoId) -> AppResult<Option<String>> {
        read::head_tree(self, repo)
    }

    fn head_oid(&self, repo: &RepoId) -> AppResult<Option<String>> {
        read::head_oid(self, repo)
    }

    fn hooks_dir(&self, repo: &RepoId) -> AppResult<PathBuf> {
        read::hooks_dir(self, repo)
    }

    fn remote_refs_containing(&self, repo: &RepoId, revision: &str) -> AppResult<Vec<String>> {
        read::remote_refs_containing(self, repo, revision)
    }

    fn authors(&self, repo: &RepoId) -> AppResult<Vec<forgedesk_domain::git::AuthorSummary>> {
        read::authors(self, repo)
    }

    fn branch_create(&self, repo: &RepoId, spec: &BranchCreateSpec) -> AppResult<()> {
        write::branch_create(self, repo, spec)
    }

    fn branch_switch(
        &self,
        repo: &RepoId,
        strategy: SwitchStrategy,
        target: &str,
    ) -> AppResult<()> {
        write::branch_switch(self, repo, strategy, target)
    }

    fn branch_rename(&self, repo: &RepoId, spec: &BranchRenameSpec) -> AppResult<()> {
        write::branch_rename(self, repo, spec)
    }

    fn branch_delete(&self, repo: &RepoId, spec: &BranchDeleteSpec) -> AppResult<Vec<String>> {
        write::branch_delete(self, repo, spec)
    }

    fn branch_set_upstream(&self, repo: &RepoId, spec: &BranchSetUpstreamSpec) -> AppResult<()> {
        write::branch_set_upstream(self, repo, spec)
    }

    fn branch_compare(&self, repo: &RepoId, a: &str, b: &str) -> AppResult<(u64, u64)> {
        read::branch_compare(self, repo, a, b)
    }

    fn branch_only_commits(
        &self,
        repo: &RepoId,
        a: &str,
        b: &str,
    ) -> AppResult<Vec<(String, String)>> {
        read::branch_only_commits(self, repo, a, b)
    }

    fn tag_create(&self, repo: &RepoId, spec: &TagCreateSpec) -> AppResult<()> {
        write::tag_create(self, repo, spec)
    }

    fn tag_delete(&self, repo: &RepoId, spec: &TagDeleteSpec) -> AppResult<()> {
        write::tag_delete(self, repo, spec)
    }

    fn update_ref(&self, repo: &RepoId, name: &str, oid: &str) -> AppResult<()> {
        write::update_ref(self, repo, name, oid)
    }

    fn delete_ref(&self, repo: &RepoId, name: &str) -> AppResult<()> {
        write::delete_ref(self, repo, name)
    }

    fn ref_exists(&self, repo: &RepoId, name: &str) -> AppResult<bool> {
        read::ref_exists(self, repo, name)
    }

    fn read_tree(&self, repo: &RepoId, treeish: &str) -> AppResult<()> {
        write::read_tree(self, repo, treeish)
    }

    fn commit(&self, repo: &RepoId, spec: CommitSpec) -> AppResult<String> {
        write::commit(self, repo, &spec)
    }

    fn reset(&self, repo: &RepoId, spec: ResetSpec) -> AppResult<()> {
        write::reset(self, repo, &spec)
    }

    fn checkout(&self, repo: &RepoId, spec: CheckoutSpec) -> AppResult<()> {
        write::checkout(self, repo, &spec)
    }

    fn merge(&self, repo: &RepoId, spec: MergeSpec) -> AppResult<MergeOutcome> {
        write::merge(self, repo, &spec)
    }

    fn cherry_pick(&self, repo: &RepoId, spec: CherryPickSpec) -> AppResult<MergeOutcome> {
        write::cherry_pick(self, repo, &spec)
    }

    fn revert(&self, repo: &RepoId, spec: RevertSpec) -> AppResult<MergeOutcome> {
        write::revert(self, repo, &spec)
    }

    fn stash(&self, repo: &RepoId, spec: StashSpec) -> AppResult<StashOutcome> {
        write::stash(self, repo, &spec)
    }

    fn conflict_state(&self, repo: &RepoId) -> AppResult<ConflictState> {
        conflict::conflict_state(self, repo)
    }

    fn conflict_mark_resolved(&self, repo: &RepoId, paths: &[RepoPath]) -> AppResult<()> {
        conflict::mark_resolved(self, repo, paths)
    }

    fn conflict_continue(
        &self,
        repo: &RepoId,
        op: ConflictOpKind,
    ) -> AppResult<ConflictContinueOutcome> {
        conflict::continue_operation(self, repo, op)
    }

    fn conflict_abort(&self, repo: &RepoId, op: ConflictOpKind) -> AppResult<ConflictAbortOutcome> {
        conflict::abort_operation(self, repo, op)
    }

    fn conflict_skip(
        &self,
        repo: &RepoId,
        op: ConflictOpKind,
    ) -> AppResult<ConflictContinueOutcome> {
        conflict::skip_operation(self, repo, op)
    }

    fn conflict_file_detail(
        &self,
        repo: &RepoId,
        path: &RepoPath,
    ) -> AppResult<ConflictFileDetail> {
        conflict::file_detail(self, repo, path)
    }

    fn conflict_take_side(&self, repo: &RepoId, path: &RepoPath, side: TakeSide) -> AppResult<()> {
        conflict::take_side(self, repo, path, side)
    }

    fn conflict_apply_resolution(
        &self,
        repo: &RepoId,
        path: &RepoPath,
        content: &str,
        eol: LineEnding,
        bom: bool,
        trailing_newline: bool,
    ) -> AppResult<()> {
        conflict::apply_resolution(self, repo, path, content, eol, bom, trailing_newline)
    }

    fn conflict_remove_file(&self, repo: &RepoId, path: &RepoPath) -> AppResult<()> {
        conflict::remove_conflict_file(self, repo, path)
    }

    fn fetch(
        &self,
        repo: &RepoId,
        spec: FetchSpec,
        progress: &ProgressSink,
        cancel: &tokio_util::sync::CancellationToken,
        auth: &NetworkAuth,
    ) -> AppResult<FetchOutcome> {
        write::fetch(self, repo, &spec, progress, cancel, auth)
    }

    fn pull(
        &self,
        repo: &RepoId,
        spec: PullSpec,
        progress: &ProgressSink,
        cancel: &tokio_util::sync::CancellationToken,
        auth: &NetworkAuth,
    ) -> AppResult<PullOutcome> {
        write::pull(self, repo, &spec, progress, cancel, auth)
    }

    fn push(
        &self,
        repo: &RepoId,
        spec: PushSpec,
        progress: &ProgressSink,
        cancel: &tokio_util::sync::CancellationToken,
        auth: &NetworkAuth,
    ) -> AppResult<PushOutcome> {
        write::push(self, repo, &spec, progress, cancel, auth)
    }

    fn probe_remote(&self, cwd: &Path, url: &str, auth: &NetworkAuth) -> AppResult<usize> {
        self.probe_remote_impl(cwd, url, auth)
    }

    fn probe_ssh_agent(&self) -> AppResult<ProbeOutput> {
        self.probe_ssh_agent_impl()
    }

    fn remote_add(&self, repo: &RepoId, name: &str, url: &str) -> AppResult<()> {
        write::remote_add(self, repo, name, url)
    }

    fn remote_remove(&self, repo: &RepoId, name: &str) -> AppResult<()> {
        write::remote_remove(self, repo, name)
    }

    fn remote_rename(&self, repo: &RepoId, old: &str, new: &str) -> AppResult<()> {
        write::remote_rename(self, repo, old, new)
    }

    fn remote_set_url(&self, repo: &RepoId, name: &str, url: &str) -> AppResult<()> {
        write::remote_set_url(self, repo, name, url)
    }

    fn rebase(
        &self,
        _repo: &RepoId,
        _plan: ReorderSpec,
        _progress: &ProgressSink,
    ) -> AppResult<MergeOutcome> {
        // 交互式 rebase 需要接管编辑器与逐条提交的重放，属于 M3（T3.6）。
        // 提前放进 trait 只是为了让签名提前稳定。
        Err(not_implemented("rebase", "T3.6"))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{truncate, RunKind};
    use crate::process::GitOutput;
    use std::time::Duration;

    #[test]
    fn only_write_commands_are_allowed_to_take_optional_locks() {
        assert!(!RunKind::Read.optional_locks());
        assert!(RunKind::Write.optional_locks());
    }

    #[test]
    fn truncate_keeps_short_text_intact() {
        assert_eq!(truncate("abc".to_owned(), 10), "abc");
    }

    #[test]
    fn truncate_counts_characters_not_bytes() {
        // 中文每字 3 字节：按字节截断会切出半个字符
        let text = "中文中文中文".to_owned();
        let truncated = truncate(text, 4);

        assert_eq!(truncated.chars().count(), 5);
        assert!(truncated.starts_with("中文中文"));
    }

    #[test]
    fn non_zero_exit_becomes_an_error_carrying_the_classified_code() {
        let output = GitOutput {
            exit_code: Some(128),
            stdout: Vec::new(),
            stderr: b"fatal: not a git repository (or any of the parent directories)".to_vec(),
            stdout_is_utf8: true,
            stderr_is_utf8: true,
            duration: Duration::from_millis(1),
        };

        let error = super::ensure_success(&output).expect_err("非零退出必须是错误");

        assert_eq!(
            error.code,
            forgedesk_domain::ErrorCode::PathNotRepo,
            "分类应复用领域层的 classify"
        );
        assert!(error.detail.unwrap().contains("fatal"));
    }

    #[test]
    fn successful_output_passes_through() {
        let output = GitOutput {
            exit_code: Some(0),
            stdout: b"ok".to_vec(),
            stderr: Vec::new(),
            stdout_is_utf8: true,
            stderr_is_utf8: true,
            duration: Duration::from_millis(1),
        };

        assert!(super::ensure_success(&output).is_ok());
    }
}
