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
pub mod read;
pub mod write;

use std::path::{Path, PathBuf};
use std::time::Duration;

use forgedesk_diagnostics::sanitize_log;
use forgedesk_domain::git::{
    ApplyPatchSpec, Branch, CheckoutSpec, CloneSpec, Commit, CommitSpec, DiffReport, DiffSpec,
    DiscardSpec, FetchOutcome, FetchSpec, InitSpec, LogQuery, MergeOutcome, MergeSpec, Page,
    PullOutcome, PullSpec, PushOutcome, PushSpec, ReflogEntry, Remote, ReorderSpec, RepoId,
    RepositoryInfo, ResetSpec, StageSpec, StashEntry, StashSpec, StatusQuery, StatusReport, Tag,
};
use forgedesk_domain::{AppError, AppResult, ErrorCode};

use super::progress::ProgressSink;
use super::{not_implemented, EngineId, GitEngine};
use crate::process::{GitOutput, GitProcess, GitRunOpts};

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
    ) -> AppResult<GitOutput> {
        let output = self.run_at_with_timeout(
            cwd,
            invocation,
            RunKind::Write,
            NETWORK_TIMEOUT,
            Some(progress),
        )?;
        ensure_success(&output)?;
        Ok(output)
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

    fn clone(&self, spec: CloneSpec, progress: &ProgressSink) -> AppResult<RepositoryInfo> {
        write::clone(self, &spec, progress)
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

    fn head_tree(&self, repo: &RepoId) -> AppResult<Option<String>> {
        read::head_tree(self, repo)
    }

    fn hooks_dir(&self, repo: &RepoId) -> AppResult<PathBuf> {
        read::hooks_dir(self, repo)
    }

    fn remote_refs_containing(&self, repo: &RepoId, revision: &str) -> AppResult<Vec<String>> {
        read::remote_refs_containing(self, repo, revision)
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

    fn cherry_pick(&self, repo: &RepoId, revision: &str) -> AppResult<MergeOutcome> {
        write::cherry_pick(self, repo, revision)
    }

    fn revert(&self, repo: &RepoId, revision: &str) -> AppResult<MergeOutcome> {
        write::revert(self, repo, revision)
    }

    fn stash(&self, repo: &RepoId, spec: StashSpec) -> AppResult<()> {
        write::stash(self, repo, &spec)
    }

    fn fetch(
        &self,
        repo: &RepoId,
        spec: FetchSpec,
        progress: &ProgressSink,
    ) -> AppResult<FetchOutcome> {
        write::fetch(self, repo, &spec, progress)
    }

    fn pull(
        &self,
        repo: &RepoId,
        spec: PullSpec,
        progress: &ProgressSink,
    ) -> AppResult<PullOutcome> {
        write::pull(self, repo, &spec, progress)
    }

    fn push(
        &self,
        repo: &RepoId,
        spec: PushSpec,
        progress: &ProgressSink,
    ) -> AppResult<PushOutcome> {
        write::push(self, repo, &spec, progress)
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
