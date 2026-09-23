//! 命令行参数构造：纯函数，安全边界所在。
//!
//! # 为什么单独一层
//!
//! 这一层承担了两条硬约束：
//!
//! - **红线 R7**：`push` 只能生成 `--force-with-lease`，不能生成裸 `--force`；
//! - **AGENTS §7**：参数以数组传递、禁止 shell 拼接。
//!
//! 把它们做成纯函数，就能直接断言"生成的参数数组里没有 `--force`"，
//! 而不必跑一次真实 git 去观察后果——后者既慢，又只能覆盖能构造出来的场景。
//!
//! # 路径为什么要分两种传法
//!
//! `GitProcess::run` 收的是 `&[String]`（T1.1 的签名），因此**非 UTF-8 路径
//! 根本没法放进 argv**。而 Git 的路径在 POSIX 上是任意字节，这正是
//! `RepoPath` 保真的原因（见 `domain::git` 模块头）。解决办法是 git 自己的
//! 机制：`--pathspec-from-file=- --pathspec-file-nul`，把路径以 NUL 分隔写进
//! **stdin**，绕开 argv 的编码问题。
//!
//! 实测（git 2.54）哪些命令支持它：
//!
//! | 命令 | `--pathspec-from-file` |
//! | --- | --- |
//! | `add` / `reset` / `commit` / `checkout` / `stash push` | 支持 |
//! | `diff` / `log` | **不支持**（`invalid option` / `unrecognized argument`） |
//!
//! 因此 [`PathSpecArgs::append_to`] 在"不支持却又有非 UTF-8 路径"时返回
//! `VALIDATION`，而不是悄悄 lossy 掉一个字节——那会让操作落到**另一个文件**上。

use forgedesk_domain::git::{
    CheckoutSpec, CloneSpec, CommitSpec, DiffSpec, DiffTarget, FetchSpec, InitSpec, LogQuery,
    MergeSpec, PullSpec, PushSpec, ReorderSpec, RepoPath, ResetSpec, StageSpec, StashAction,
    StashSpec,
};
use forgedesk_domain::{AppError, AppResult, ErrorCode};

/// 一条待执行的命令。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GitInvocation {
    /// 参数数组（不含可执行文件名）。
    pub args: Vec<String>,
    /// 写入子进程 stdin 的载荷。
    pub stdin: Option<Vec<u8>>,
}

impl GitInvocation {
    /// 只有参数、没有 stdin。
    pub fn new(args: Vec<String>) -> Self {
        Self { args, stdin: None }
    }

    /// 附加 stdin 载荷。
    #[must_use]
    pub fn with_stdin(mut self, stdin: Vec<u8>) -> Self {
        self.stdin = Some(stdin);
        self
    }
}

/// 路径参数的传法。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathSpecArgs {
    /// 没有路径限制。
    None,
    /// 全部是合法 UTF-8，直接放进 argv。
    Argv(Vec<String>),
    /// 含非 UTF-8 路径，必须走 stdin 的 pathspec 文件。
    Stdin(Vec<u8>),
}

impl PathSpecArgs {
    /// 从领域路径构造。
    pub fn from_paths(paths: &[RepoPath]) -> Self {
        if paths.is_empty() {
            return Self::None;
        }
        if paths.iter().all(RepoPath::is_utf8) {
            return Self::Argv(
                paths
                    .iter()
                    .map(|path| path.to_string_lossy().into_owned())
                    .collect(),
            );
        }
        Self::Stdin(encode_pathspec(paths))
    }

    /// 追加到参数数组。
    ///
    /// `supports_stdin` 为 `false` 且路径必须走 stdin 时返回 `VALIDATION`：
    /// 与其把非 UTF-8 路径 lossy 成另一个文件名，不如明确告诉调用方"这个组合做不到"。
    pub fn append_to(&self, args: &mut Vec<String>, supports_stdin: bool) -> AppResult<()> {
        match self {
            Self::None => Ok(()),
            Self::Argv(paths) => {
                args.push("--".to_owned());
                args.extend(paths.iter().cloned());
                Ok(())
            }
            Self::Stdin(_) => {
                if !supports_stdin {
                    return Err(AppError::new(
                        ErrorCode::Validation,
                        "this command cannot accept a non-UTF-8 path",
                    )
                    .with_hint("--pathspec-from-file".to_owned()));
                }
                args.push("--pathspec-from-file=-".to_owned());
                args.push("--pathspec-file-nul".to_owned());
                Ok(())
            }
        }
    }

    /// stdin 载荷（仅 [`PathSpecArgs::Stdin`] 有值）。
    pub fn stdin(&self) -> Option<Vec<u8>> {
        match self {
            Self::Stdin(bytes) => Some(bytes.clone()),
            _ => None,
        }
    }
}

/// 把路径编码成 `--pathspec-file-nul` 要求的 NUL 分隔格式。
///
/// 每条路径后面都必须有 NUL（**包括最后一条**）：只以 NUL 作分隔符的话，
/// 最后一条会被当成"没有终止符的残行"而报 `fatal: invalid path`。
pub fn encode_pathspec(paths: &[RepoPath]) -> Vec<u8> {
    let mut out = Vec::new();
    for path in paths {
        out.extend_from_slice(path.as_bytes());
        out.push(0);
    }
    out
}

/// 组合参数数组与可选 stdin。
fn invocation(
    mut args: Vec<String>,
    paths: &PathSpecArgs,
    supports_stdin: bool,
) -> AppResult<GitInvocation> {
    paths.append_to(&mut args, supports_stdin)?;
    let mut invocation = GitInvocation::new(args);
    if let Some(stdin) = paths.stdin() {
        invocation.stdin = Some(stdin);
    }
    Ok(invocation)
}

// ---------------------------------------------------------------- 读操作

/// `git status --porcelain=v2 -z --branch`。
///
/// 这三个开关是**契约**：解析器（T1.1）就是按它们写的，少一个 `-z`
/// 会让重命名条目解析出错误的路径（见 `parsers::status` 模块头）。
pub fn status_args() -> Vec<String> {
    vec![
        "status".to_owned(),
        "--porcelain=v2".to_owned(),
        "-z".to_owned(),
        "--branch".to_owned(),
    ]
}

/// `git diff --numstat -z` 加上目标与过滤条件。
pub fn diff_args(spec: &DiffSpec) -> AppResult<GitInvocation> {
    let mut args = vec!["diff".to_owned(), "--numstat".to_owned(), "-z".to_owned()];

    match &spec.target {
        DiffTarget::Staged => args.push("--cached".to_owned()),
        DiffTarget::Unstaged => {}
        DiffTarget::Between { from, to } => {
            args.push(from.clone());
            args.push(to.clone());
        }
        DiffTarget::Since(revision) => args.push(revision.clone()),
        DiffTarget::Commit(_) => {
            // `git diff <rev>^ <rev>` 在根提交上会失败，`show` 会自己处理"没有父提交"
            args[0] = "show".to_owned();
            args.push("--format=".to_owned());
        }
    }
    if let DiffTarget::Commit(revision) = &spec.target {
        args.push(revision.clone());
    }

    if spec.ignore_whitespace {
        args.push("-w".to_owned());
    }
    if spec.detect_renames {
        args.push("-M".to_owned());
    }
    args.push(format!("-U{}", spec.context_lines));

    // diff 不支持 --pathspec-from-file（实测 git 2.54），因此非 UTF-8 路径直接报错
    invocation(args, &PathSpecArgs::from_paths(&spec.paths), false)
}

/// `git log` 查询。
///
/// 请求 `limit + 1` 条：多出来的那条用于判断"还有下一页"，
/// 由 [`forgedesk_domain::git::Page::from_over_fetch`] 裁掉。
pub fn log_args(query: &LogQuery, format: &str) -> AppResult<GitInvocation> {
    let mut args = vec![
        "log".to_owned(),
        format!("--format={format}"),
        "-z".to_owned(),
        format!("--max-count={}", query.limit.saturating_add(1)),
    ];
    if query.skip > 0 {
        args.push(format!("--skip={}", query.skip));
    }
    if query.all_branches {
        args.push("--all".to_owned());
    }
    if let Some(author) = &query.author {
        args.push(format!("--author={author}"));
    }
    if let Some(revision) = &query.revision {
        args.push(revision.clone());
    }

    invocation(args, &PathSpecArgs::from_paths(&query.paths), false)
}

/// 单条提交查询（含正文）。
pub fn show_args(revision: &str, format: &str) -> Vec<String> {
    vec![
        "show".to_owned(),
        "--no-patch".to_owned(),
        "-z".to_owned(),
        format!("--format={format}"),
        revision.to_owned(),
    ]
}

/// 分支列表（本地 + 远程跟踪）。
pub fn branch_args(format: &str) -> Vec<String> {
    vec![
        "for-each-ref".to_owned(),
        format!("--format={format}"),
        "refs/heads".to_owned(),
        "refs/remotes".to_owned(),
    ]
}

/// 标签列表。
pub fn tag_args(format: &str) -> Vec<String> {
    vec![
        "for-each-ref".to_owned(),
        format!("--format={format}"),
        "refs/tags".to_owned(),
    ]
}

/// 远端列表。
pub fn remote_args() -> Vec<String> {
    vec!["remote".to_owned(), "-v".to_owned()]
}

/// stash 列表。
pub fn stash_list_args(format: &str) -> Vec<String> {
    vec![
        "stash".to_owned(),
        "list".to_owned(),
        format!("--format={format}"),
    ]
}

/// reflog。
pub fn reflog_args(limit: usize, format: &str) -> Vec<String> {
    vec![
        "reflog".to_owned(),
        format!("--format={format}"),
        format!("--max-count={limit}"),
    ]
}

// ---------------------------------------------------------------- 写操作

/// `git init`。
pub fn init_args(spec: &InitSpec) -> Vec<String> {
    let mut args = vec!["init".to_owned(), "--quiet".to_owned()];
    if spec.bare {
        args.push("--bare".to_owned());
    }
    if let Some(branch) = &spec.initial_branch {
        args.push("-b".to_owned());
        args.push(branch.clone());
    }
    args
}

/// `git clone`。
///
/// `--progress` 是必须的：非交互环境下 git 默认**不输出**进度，
/// 而进度是长任务唯一的反馈来源（见 `progress` 模块头）。
pub fn clone_args(spec: &CloneSpec) -> Vec<String> {
    let mut args = vec!["clone".to_owned(), "--progress".to_owned()];
    if spec.bare {
        args.push("--bare".to_owned());
    }
    if let Some(depth) = spec.depth {
        args.push("--depth".to_owned());
        args.push(depth.to_string());
    }
    if let Some(branch) = &spec.branch {
        args.push("--branch".to_owned());
        args.push(branch.clone());
    }
    if spec.recurse_submodules {
        args.push("--recurse-submodules".to_owned());
    }
    if spec.single_branch {
        args.push("--single-branch".to_owned());
    }
    args.push("--".to_owned());
    args.push(spec.url.clone());
    args.push(spec.into.to_string_lossy().into_owned());
    args
}

/// `git add`（暂存）。
pub fn stage_args(spec: &StageSpec) -> AppResult<GitInvocation> {
    match spec {
        // 注意：这里**不能**自己 push `--`，PathSpecArgs::append_to 会加
        StageSpec::Paths(paths) => invocation(
            vec!["add".to_owned()],
            &PathSpecArgs::from_paths(paths),
            true,
        ),
        StageSpec::All => Ok(GitInvocation::new(vec![
            "add".to_owned(),
            "--all".to_owned(),
        ])),
        StageSpec::Patch(patch) => Ok(GitInvocation::new(vec![
            "apply".to_owned(),
            "--cached".to_owned(),
            // --recount：补丁的行号统计可能不精确，让 git 自己重算而不是直接失败
            "--recount".to_owned(),
            // --whitespace=nowarn：用户的空白风格不该阻断"我只想暂存这几行"
            "--whitespace=nowarn".to_owned(),
            "-".to_owned(),
        ])
        .with_stdin(patch.clone())),
    }
}

/// `git reset`（取消暂存）。
pub fn unstage_args(spec: &StageSpec) -> AppResult<GitInvocation> {
    match spec {
        StageSpec::Paths(paths) => invocation(
            vec!["reset".to_owned(), "--quiet".to_owned(), "HEAD".to_owned()],
            &PathSpecArgs::from_paths(paths),
            true,
        ),
        StageSpec::All => Ok(GitInvocation::new(vec![
            "reset".to_owned(),
            "--quiet".to_owned(),
            "HEAD".to_owned(),
        ])),
        StageSpec::Patch(patch) => Ok(GitInvocation::new(vec![
            "apply".to_owned(),
            "--cached".to_owned(),
            // 反向应用 = 把补丁从索引里撤掉
            "--reverse".to_owned(),
            "--recount".to_owned(),
            "--whitespace=nowarn".to_owned(),
            "-".to_owned(),
        ])
        .with_stdin(patch.clone())),
    }
}

/// `git commit`。
///
/// # stdin 只能给一个人
///
/// 提交信息走 `--file=-`（stdin），因此**路径不能**也走
/// `--pathspec-from-file=-`：两条通道会互相抢同一个管道，结果是提交信息被当成
/// 路径。所以这里 `supports_stdin = false`，非 UTF-8 路径会得到 `VALIDATION`，
/// 调用方应先 `stage()`（那里可以走 stdin）再提交索引。
pub fn commit_args(spec: &CommitSpec) -> AppResult<GitInvocation> {
    let mut args = vec!["commit".to_owned()];

    // 用 `-F -` 从 stdin 读提交信息，而不是 `-m`：提交信息可能很长、
    // 含换行与任意 Unicode，走 argv 会撞上命令行长度限制与编码转换
    args.push("--file=-".to_owned());

    if spec.amend {
        args.push("--amend".to_owned());
    }
    if spec.allow_empty {
        args.push("--allow-empty".to_owned());
    }
    if spec.no_verify {
        args.push("--no-verify".to_owned());
    }
    match spec.sign {
        Some(true) => args.push("--gpg-sign".to_owned()),
        Some(false) => args.push("--no-gpg-sign".to_owned()),
        None => {}
    }
    if let Some(author) = &spec.author {
        args.push(format!("--author={}", author.display()));
    }

    invocation(args, &PathSpecArgs::from_paths(&spec.paths), false)
        .map(|invocation| invocation.with_stdin(spec.message.clone().into_bytes()))
}

/// `git reset`（重置）。
pub fn reset_args(spec: &ResetSpec) -> AppResult<GitInvocation> {
    let mut args = vec!["reset".to_owned(), "--quiet".to_owned()];
    if spec.is_path_scoped() {
        // 路径级 reset 只能是 mixed（git 会拒绝 --hard/--soft 与路径同时出现）
        args.push(spec.revision.clone());
        return invocation(args, &PathSpecArgs::from_paths(&spec.paths), true);
    }

    args.push(spec.mode.as_flag().to_owned());
    args.push(spec.revision.clone());
    Ok(GitInvocation::new(args))
}

/// `git checkout`。
pub fn checkout_args(spec: &CheckoutSpec) -> AppResult<GitInvocation> {
    let mut args = vec!["checkout".to_owned()];
    if spec.force {
        args.push("--force".to_owned());
    }
    if spec.detach {
        args.push("--detach".to_owned());
    }
    if let Some(branch) = &spec.create_branch {
        args.push("-b".to_owned());
        args.push(branch.clone());
    }
    args.push("--".to_owned());
    args.push(spec.target.clone());
    Ok(GitInvocation::new(args))
}

/// `git merge`。
pub fn merge_args(spec: &MergeSpec) -> Vec<String> {
    let mut args = vec!["merge".to_owned(), "--no-edit".to_owned()];
    if spec.ff_only {
        args.push("--ff-only".to_owned());
    }
    if spec.no_ff {
        args.push("--no-ff".to_owned());
    }
    if let Some(message) = &spec.message {
        args.push("-m".to_owned());
        args.push(message.clone());
    }
    args.push("--".to_owned());
    args.push(spec.revision.clone());
    args
}

/// `git cherry-pick`。
pub fn cherry_pick_args(revision: &str) -> Vec<String> {
    vec!["cherry-pick".to_owned(), revision.to_owned()]
}

/// `git revert`。
pub fn revert_args(revision: &str) -> Vec<String> {
    vec![
        "revert".to_owned(),
        // 非交互环境里没有编辑器，不传 --no-edit 会让进程挂住等输入
        "--no-edit".to_owned(),
        revision.to_owned(),
    ]
}

/// `git stash`。
pub fn stash_args(spec: &StashSpec) -> Vec<String> {
    match &spec.action {
        StashAction::Push => {
            let mut args = vec!["stash".to_owned(), "push".to_owned()];
            if spec.include_untracked {
                args.push("--include-untracked".to_owned());
            }
            if spec.keep_index {
                args.push("--keep-index".to_owned());
            }
            if let Some(message) = &spec.message {
                args.push("-m".to_owned());
                args.push(message.clone());
            }
            args
        }
        StashAction::Apply { index } => {
            vec![
                "stash".to_owned(),
                "apply".to_owned(),
                format!("stash@{{{index}}}"),
            ]
        }
        StashAction::Pop { index } => {
            vec![
                "stash".to_owned(),
                "pop".to_owned(),
                format!("stash@{{{index}}}"),
            ]
        }
        StashAction::Drop { index } => {
            vec![
                "stash".to_owned(),
                "drop".to_owned(),
                format!("stash@{{{index}}}"),
            ]
        }
    }
}

/// `git fetch`。
pub fn fetch_args(spec: &FetchSpec) -> Vec<String> {
    let mut args = vec!["fetch".to_owned(), "--progress".to_owned()];
    if spec.prune {
        args.push("--prune".to_owned());
    }
    if spec.tags {
        args.push("--tags".to_owned());
    }
    if let Some(remote) = &spec.remote {
        args.push(remote.clone());
    }
    args.extend(spec.refspecs.iter().cloned());
    args
}

/// `git pull`。
pub fn pull_args(spec: &PullSpec) -> Vec<String> {
    let mut args = vec![
        "pull".to_owned(),
        "--progress".to_owned(),
        spec.strategy.as_flag().to_owned(),
        "--no-edit".to_owned(),
    ];
    if let Some(remote) = &spec.remote {
        args.push(remote.clone());
    }
    if let Some(branch) = &spec.branch {
        args.push(branch.clone());
    }
    args
}

/// `git push`。
///
/// **红线 R7 的落点**：这里只可能产出 `--force-with-lease`。
/// 没有任何分支会产出 `--force`，测试对此有断言。
pub fn push_args(spec: &PushSpec) -> Vec<String> {
    let mut args = vec!["push".to_owned(), "--progress".to_owned()];
    if spec.set_upstream {
        args.push("--set-upstream".to_owned());
    }
    if spec.force_with_lease {
        args.push("--force-with-lease".to_owned());
    }
    if spec.tags {
        args.push("--tags".to_owned());
    }
    if let Some(remote) = &spec.remote {
        args.push(remote.clone());
    }
    if let Some(branch) = &spec.branch {
        args.push(branch.clone());
    }
    args
}

/// `git rebase -i`（M3/T3.6 实现，此处只生成计划供预览与测试）。
///
/// 生成的是 `--onto` 形式而非 `-i`：交互式 rebase 需要接管编辑器，
/// 属于 M3 的范围。这里的产物用于"计划预览"与差分测试。
pub fn rebase_args(plan: &ReorderSpec) -> Vec<String> {
    vec!["rebase".to_owned(), "--onto".to_owned(), plan.onto.clone()]
}

// 把 stderr 逐行回调包装成进度投递的工具在 `super::progress::ProgressSink::handler`：
// 那里持有 `Arc`，因此产出的闭包是 `'static`，能被移动到读取子进程输出的任务里。

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use forgedesk_domain::git::{
        DiffSpec, DiffTarget, LogQuery, PullStrategy, ResetMode, Signature,
    };

    fn joined(args: &[String]) -> String {
        args.join(" ")
    }

    #[test]
    fn status_args_always_carry_the_machine_readable_contract() {
        let args = status_args();

        assert_eq!(joined(&args), "status --porcelain=v2 -z --branch");
    }

    #[test]
    fn push_never_emits_a_bare_force_flag() {
        // 红线 R7：把四种组合都试一遍，任何一条都不允许出现裸 --force
        for force_with_lease in [false, true] {
            for tags in [false, true] {
                let spec = PushSpec::new()
                    .with_force_with_lease(force_with_lease)
                    .with_set_upstream(true);
                let spec = PushSpec { tags, ..spec };
                let args = push_args(&spec);

                assert!(
                    !args.iter().any(|arg| arg == "--force" || arg == "-f"),
                    "参数里出现了裸 force: {args:?}"
                );
                if force_with_lease {
                    assert!(args.iter().any(|arg| arg == "--force-with-lease"));
                } else {
                    assert!(!args.iter().any(|arg| arg.starts_with("--force")));
                }
            }
        }
    }

    #[test]
    fn push_can_request_upstream_tracking() {
        let args = push_args(&PushSpec::new().with_set_upstream(true));

        assert!(args.contains(&"--set-upstream".to_owned()));
        assert!(args.contains(&"--progress".to_owned()));
    }

    #[test]
    fn hard_reset_maps_to_the_hard_flag_and_path_reset_does_not() {
        let hard = reset_args(&ResetSpec::to("HEAD~1", ResetMode::Hard)).unwrap();
        assert_eq!(joined(&hard.args), "reset --quiet --hard HEAD~1");
        assert!(hard.stdin.is_none());

        let scoped = reset_args(&ResetSpec::paths(None, vec!["a.txt".into()])).unwrap();
        assert_eq!(joined(&scoped.args), "reset --quiet HEAD -- a.txt");
        assert!(
            !scoped.args.iter().any(|arg| arg.starts_with("--hard")),
            "路径级 reset 不能带 --hard"
        );
    }

    #[test]
    fn commit_reads_the_message_from_stdin_rather_than_argv() {
        let invocation = commit_args(&CommitSpec::new("subject\n\nbody")).unwrap();

        assert!(invocation.args.contains(&"--file=-".to_owned()));
        assert!(!invocation.args.iter().any(|arg| arg == "-m"));
        assert_eq!(invocation.stdin.as_deref(), Some(&b"subject\n\nbody"[..]));
    }

    #[test]
    fn commit_only_passes_signing_flags_when_explicitly_requested() {
        let default = commit_args(&CommitSpec::new("m")).unwrap();
        assert!(!default.args.iter().any(|arg| arg.contains("gpg-sign")));

        let signed = commit_args(&CommitSpec {
            sign: Some(true),
            ..CommitSpec::new("m")
        })
        .unwrap();
        assert!(signed.args.contains(&"--gpg-sign".to_owned()));

        let unsigned = commit_args(&CommitSpec {
            sign: Some(false),
            ..CommitSpec::new("m")
        })
        .unwrap();
        assert!(unsigned.args.contains(&"--no-gpg-sign".to_owned()));
    }

    #[test]
    fn amend_keeps_the_original_author_when_asked() {
        let spec = CommitSpec {
            amend: true,
            author: Some(Signature::new("Ada", "ada@example.com")),
            ..CommitSpec::new("fixed message")
        };
        let invocation = commit_args(&spec).unwrap();

        assert!(invocation.args.contains(&"--amend".to_owned()));
        assert!(invocation
            .args
            .contains(&"--author=Ada <ada@example.com>".to_owned()));
    }

    #[test]
    fn stage_all_uses_add_all_without_a_pathspec() {
        let invocation = stage_args(&StageSpec::All).unwrap();

        assert_eq!(joined(&invocation.args), "add --all");
        assert!(invocation.stdin.is_none());
    }

    #[test]
    fn stage_patch_applies_to_the_index_and_feeds_the_patch_on_stdin() {
        let invocation = stage_args(&StageSpec::Patch(b"diff --git a/x b/x\n".to_vec())).unwrap();

        assert_eq!(
            joined(&invocation.args),
            "apply --cached --recount --whitespace=nowarn -"
        );
        assert!(invocation.stdin.is_some());
    }

    #[test]
    fn unstage_patch_reverses_the_patch_instead_of_staging_it() {
        let invocation = unstage_args(&StageSpec::Patch(b"patch".to_vec())).unwrap();

        assert!(invocation.args.contains(&"--reverse".to_owned()));
        assert!(invocation.args.contains(&"--cached".to_owned()));
    }

    #[test]
    fn utf8_paths_go_into_argv_while_non_utf8_paths_go_to_stdin() {
        let utf8 = PathSpecArgs::from_paths(&[RepoPath::from("a.txt")]);
        assert_eq!(utf8, PathSpecArgs::Argv(vec!["a.txt".to_owned()]));

        let raw = RepoPath::from_bytes(vec![0xFF, b'.', b't', b'x', b't']);
        let non_utf8 = PathSpecArgs::from_paths(std::slice::from_ref(&raw));
        assert_eq!(
            non_utf8,
            PathSpecArgs::Stdin(vec![0xFF, b'.', b't', b'x', b't', 0])
        );

        let mut args = Vec::new();
        non_utf8.append_to(&mut args, true).unwrap();
        assert_eq!(args, vec!["--pathspec-from-file=-", "--pathspec-file-nul"]);
    }

    #[test]
    fn non_utf8_paths_are_rejected_when_the_command_cannot_take_them() {
        let raw = RepoPath::from_bytes(vec![0xFF, b'.', b't', b'x', b't']);
        let spec = DiffSpec::new(DiffTarget::Staged).with_paths(vec![raw]);
        let error = diff_args(&spec).expect_err("diff 不支持 pathspec 文件，必须报错");

        assert_eq!(error.code, ErrorCode::Validation);
    }

    #[test]
    fn pathspec_entries_are_all_nul_terminated_including_the_last_one() {
        let encoded = encode_pathspec(&[RepoPath::from("a"), RepoPath::from("b")]);

        assert_eq!(encoded, b"a\0b\0");
    }

    #[test]
    fn diff_targets_map_to_their_git_invocations() {
        let staged = diff_args(&DiffSpec::new(DiffTarget::Staged)).unwrap();
        assert_eq!(joined(&staged.args), "diff --numstat -z --cached -M -U3");

        let unstaged = diff_args(&DiffSpec::new(DiffTarget::Unstaged)).unwrap();
        assert_eq!(joined(&unstaged.args), "diff --numstat -z -M -U3");

        let between = diff_args(&DiffSpec::new(DiffTarget::between("main", "feature"))).unwrap();
        assert_eq!(
            joined(&between.args),
            "diff --numstat -z main feature -M -U3"
        );

        let since = diff_args(&DiffSpec::new(DiffTarget::Since("HEAD~2".to_owned()))).unwrap();
        assert_eq!(joined(&since.args), "diff --numstat -z HEAD~2 -M -U3");

        // 根提交用 show：`diff <rev>^ <rev>` 在根提交上会失败
        let commit = diff_args(&DiffSpec::new(DiffTarget::Commit("abc".to_owned()))).unwrap();
        assert_eq!(
            joined(&commit.args),
            "show --numstat -z --format= abc -M -U3"
        );
    }

    #[test]
    fn diff_honours_whitespace_and_context_options() {
        let spec = DiffSpec::new(DiffTarget::Unstaged)
            .with_ignore_whitespace(true)
            .with_context_lines(0);
        let invocation = diff_args(&spec).unwrap();

        assert!(invocation.args.contains(&"-w".to_owned()));
        assert!(invocation.args.contains(&"-U0".to_owned()));
    }

    #[test]
    fn log_over_fetches_one_extra_row_for_pagination() {
        let query = LogQuery::new().with_limit(50).with_skip(100);
        let invocation = log_args(&query, "FORMAT").unwrap();

        assert!(invocation.args.contains(&"--max-count=51".to_owned()));
        assert!(invocation.args.contains(&"--skip=100".to_owned()));
        assert!(invocation.args.contains(&"--format=FORMAT".to_owned()));
        assert!(invocation.args.contains(&"-z".to_owned()));
    }

    #[test]
    fn log_omits_filters_that_are_not_set() {
        let invocation = log_args(&LogQuery::new(), "FORMAT").unwrap();

        assert!(!invocation.args.iter().any(|arg| arg.starts_with("--skip")));
        assert!(!invocation.args.iter().any(|arg| arg == "--all"));
        assert!(!invocation
            .args
            .iter()
            .any(|arg| arg.starts_with("--author")));
    }

    #[test]
    fn log_all_branches_and_author_filters_are_forwarded() {
        let query = LogQuery::new()
            .with_all_branches(true)
            .with_author("Ada")
            .with_revision("main");
        let invocation = log_args(&query, "FORMAT").unwrap();

        assert!(invocation.args.contains(&"--all".to_owned()));
        assert!(invocation.args.contains(&"--author=Ada".to_owned()));
        assert!(invocation.args.contains(&"main".to_owned()));
    }

    #[test]
    fn clone_always_asks_for_progress() {
        let spec = CloneSpec::new("https://example.com/r.git", "dest").with_depth(1);
        let args = clone_args(&spec);

        assert!(args.contains(&"--progress".to_owned()));
        assert!(args.contains(&"--depth".to_owned()));
        assert!(args.contains(&"1".to_owned()));
        // URL 与目标目录放在 `--` 之后，避免以 `-` 开头的值被当成开关
        let separator = args.iter().position(|arg| arg == "--").unwrap();
        assert_eq!(args[separator + 1], "https://example.com/r.git");
    }

    #[test]
    fn clone_forwards_branch_submodules_and_single_branch() {
        let spec = CloneSpec::new("https://example.com/r.git", "dest")
            .with_branch("release")
            .with_submodules(true)
            .with_single_branch(true);
        let args = clone_args(&spec);

        assert!(args.contains(&"--branch".to_owned()));
        assert!(args.contains(&"release".to_owned()));
        assert!(args.contains(&"--recurse-submodules".to_owned()));
        assert!(args.contains(&"--single-branch".to_owned()));
    }

    #[test]
    fn clone_omits_the_optional_switches_by_default() {
        let args = clone_args(&CloneSpec::new("https://example.com/r.git", "dest"));

        for absent in [
            "--depth",
            "--branch",
            "--bare",
            "--recurse-submodules",
            "--single-branch",
        ] {
            assert!(
                !args.contains(&absent.to_owned()),
                "默认克隆不应带 {absent}：{args:?}"
            );
        }
    }

    #[test]
    fn pull_strategy_and_fetch_options_are_forwarded() {
        let pull = pull_args(&PullSpec::new().with_strategy(PullStrategy::Rebase));
        assert!(pull.contains(&"--rebase".to_owned()));
        assert!(pull.contains(&"--no-edit".to_owned()));

        let fetch = fetch_args(&FetchSpec::new().with_remote("origin").with_prune(true));
        assert!(fetch.contains(&"--prune".to_owned()));
        assert!(fetch.contains(&"origin".to_owned()));
        assert!(fetch.contains(&"--progress".to_owned()));
    }

    #[test]
    fn revert_never_opens_an_editor() {
        let args = revert_args("abc123");

        assert!(args.contains(&"--no-edit".to_owned()));
    }

    #[test]
    fn stash_actions_map_to_their_subcommands() {
        assert_eq!(
            joined(&stash_args(&StashSpec::push(Some("wip".to_owned())))),
            "stash push -m wip"
        );
        assert_eq!(
            joined(&stash_args(&StashSpec {
                include_untracked: true,
                ..StashSpec::pop(2)
            })),
            "stash pop stash@{2}"
        );
        assert_eq!(
            joined(&stash_args(&StashSpec {
                action: StashAction::Drop { index: 0 },
                message: None,
                include_untracked: false,
                keep_index: false,
            })),
            "stash drop stash@{0}"
        );
    }

    #[test]
    fn checkout_keeps_the_target_after_a_separator() {
        let invocation = checkout_args(&CheckoutSpec::new("feature")).unwrap();

        assert_eq!(joined(&invocation.args), "checkout -- feature");
        assert!(!invocation.args.iter().any(|arg| arg == "--force"));
    }
}
