//! 读操作的实现与输出解析。
//!
//! 每个格式串都紧挨着它的解析器：格式串与解析器必须成对演进，
//! 分开写迟早出现"改了 `--format` 忘了改解析器"（表现是字段整体错位，
//! 而不是报错）。
//!
//! 解析统一走**字节切片**（同 T1.1）：路径可能不是合法 UTF-8。

use std::path::Path;

use forgedesk_domain::git::{
    Branch, Commit, DiffChangeKind, DiffReport, DiffSpec, FileDiff, LogQuery, Page, ReflogEntry,
    Remote, RemoteKind, RepoId, RepositoryInfo, StashEntry, StatusQuery, StatusReport, Tag,
    Worktree,
};
use forgedesk_domain::{AppError, AppResult, ErrorCode};

use super::args::{self, GitInvocation};
use super::CliGitEngine;
use crate::parsers::{
    parse_diff_numstat, parse_log_format, parse_show_format, parse_status_porcelain_v2,
    parse_worktree_list, LOG_FORMAT, SHOW_FORMAT,
};
use crate::process::GitOutput;

/// 字段分隔符（US），与 T1.1 的 `LOG_FORMAT` 保持一致。
const FIELD: char = '\u{1f}';

/// `git for-each-ref` 的分支格式。
const BRANCH_FORMAT: &str =
    "%(refname)\u{1f}%(refname:short)\u{1f}%(objectname)\u{1f}%(upstream:short)\u{1f}%(upstream:track)\u{1f}%(HEAD)";

/// `git for-each-ref` 的标签格式。
const TAG_FORMAT: &str =
    "%(refname:short)\u{1f}%(objectname)\u{1f}%(objecttype)\u{1f}%(*objectname)\u{1f}%(creatordate:unix)\u{1f}%(contents:subject)";

/// `git stash list` 的格式。
const STASH_FORMAT: &str = "%gd\u{1f}%H\u{1f}%P\u{1f}%ct\u{1f}%gs";

/// `git reflog` 的格式。
const REFLOG_FORMAT: &str = "%H\u{1f}%gd\u{1f}%gs\u{1f}%ct";

/// reflog 的默认条数上限。
pub const DEFAULT_REFLOG_LIMIT: usize = 200;

/// 执行一条读命令并返回原始输出。
fn run(engine: &CliGitEngine, repo: &RepoId, invocation: GitInvocation) -> AppResult<GitOutput> {
    engine.run_read(repo, invocation)
}

// ---------------------------------------------------------------- discover

/// 从任意目录向上查找仓库。
///
/// 为什么用 `rev-parse` 而不是 `status`：`status` 会计算整个工作区的差异，
/// 在十万文件级别的仓库上要几秒；而"打开仓库"这一步只需要知道仓库在哪、
/// HEAD 是什么。M1 的验收标准是"打开仓库 ≤2s"，这里的差别就是那 2 秒。
pub(super) fn discover(engine: &CliGitEngine, path: &Path) -> AppResult<RepositoryInfo> {
    let probe = GitInvocation::new(vec![
        "rev-parse".to_owned(),
        "--absolute-git-dir".to_owned(),
        "--is-bare-repository".to_owned(),
        "--is-inside-work-tree".to_owned(),
    ]);
    let output = engine.run_checked_at(path, probe, super::RunKind::Read)?;
    let stdout = output.stdout_lossy();
    let mut lines = stdout.lines().map(str::trim);

    let git_dir = lines.next().unwrap_or_default().to_owned();
    let is_bare = lines.next().unwrap_or_default() == "true";
    let inside_work_tree = lines.next().unwrap_or_default() == "true";
    if git_dir.is_empty() {
        return Err(
            AppError::new(ErrorCode::PathNotRepo, "git did not report a git directory")
                .with_hint(path.to_string_lossy().into_owned()),
        );
    }

    // 裸仓库没有工作区；`--show-toplevel` 在裸仓库里会失败，因此要先判断
    let workdir = if inside_work_tree {
        let invocation =
            GitInvocation::new(vec!["rev-parse".to_owned(), "--show-toplevel".to_owned()]);
        let output = engine.run_checked_at(path, invocation, super::RunKind::Read)?;
        let text = output.stdout_lossy().trim().to_owned();
        if text.is_empty() {
            None
        } else {
            Some(std::path::PathBuf::from(text))
        }
    } else {
        None
    };

    let root = workdir
        .clone()
        .unwrap_or_else(|| std::path::PathBuf::from(&git_dir));
    let repo = RepoId::new(root);

    // HEAD 是分支名还是游离：`symbolic-ref` 在游离 HEAD 时以退出码 1 失败，
    // 这是**正常状态**，不能当错误上报
    let head = head_branch(engine, &repo)?;
    // 有提交才有 oid；空仓库在这里以非零退出，同样属于正常状态
    let has_commits = engine
        .run_at(
            repo.root(),
            GitInvocation::new(vec![
                "rev-parse".to_owned(),
                "--verify".to_owned(),
                "--quiet".to_owned(),
                "HEAD".to_owned(),
            ]),
            super::RunKind::Read,
        )?
        .success();

    let upstream = upstream_of(engine, &repo);
    // 游离 HEAD 与空仓库都没有分支名，区别在"有没有提交"
    let detached = head.is_none() && has_commits;

    let git_dir = std::path::PathBuf::from(git_dir);
    // 主工作区的兜底项：`git worktree list` 不可用时至少给出仓库自身
    let fallback_worktree = Worktree {
        path: workdir.clone().unwrap_or_else(|| git_dir.clone()),
        head: None,
        branch: head.clone(),
        detached,
        is_bare,
        locked: false,
        prunable: false,
    };

    Ok(RepositoryInfo {
        default_branch: default_branch_of(engine, &repo, head.as_deref()),
        is_shallow: is_shallow_repository(engine, &repo),
        is_lfs: crate::probe::detect_lfs(&git_dir, workdir.as_deref()),
        worktrees: worktrees_of(engine, &repo, fallback_worktree),
        id: repo,
        workdir,
        git_dir,
        is_bare,
        is_empty: !has_commits,
        head,
        detached,
        upstream,
    })
}

/// 默认分支短名。
///
/// 优先 `refs/remotes/origin/HEAD`——它是"远端默认分支"的本地镜像，克隆时由 git
/// 自动建立，比"当前分支"更能代表默认分支（用户此刻可能正停在某个特性分支上）。
/// 拿不到时退回当前分支；两者都没有（游离 HEAD 且无 `origin/HEAD`）返回 `None`。
fn default_branch_of(engine: &CliGitEngine, repo: &RepoId, head: Option<&str>) -> Option<String> {
    let invocation = GitInvocation::new(vec![
        "symbolic-ref".to_owned(),
        "--short".to_owned(),
        "refs/remotes/origin/HEAD".to_owned(),
    ]);
    if let Ok(output) = engine.run_at(repo.root(), invocation, super::RunKind::Read) {
        if output.success() {
            let name = output.stdout_lossy().trim().to_owned();
            // `origin/main` → `main`：界面要的是分支名，不是远程跟踪引用
            if let Some((_, branch)) = name.split_once('/') {
                if !branch.is_empty() {
                    return Some(branch.to_owned());
                }
            }
        }
    }
    head.map(str::to_owned)
}

/// 是否为浅克隆。
///
/// `--is-shallow-repository` 在 git < 2.15 上不存在，此时命令以非零退出，
/// 我们按"不是浅仓库"处理——低版本 git 的提示已经由版本检查单独给出，
/// 在这里再报一次只会重复。
fn is_shallow_repository(engine: &CliGitEngine, repo: &RepoId) -> bool {
    let invocation = GitInvocation::new(vec![
        "rev-parse".to_owned(),
        "--is-shallow-repository".to_owned(),
    ]);
    engine
        .run_at(repo.root(), invocation, super::RunKind::Read)
        .map(|output| output.success() && output.stdout_lossy().trim() == "true")
        .unwrap_or(false)
}

/// 工作区列表；命令不可用时退化为只有一个工作区。
///
/// 失败**不上报**：工作区列表是附加信息，拿不到它不该让"打开仓库"整体失败
/// （`fallback` 已经能给出界面需要的最小事实）。
fn worktrees_of(engine: &CliGitEngine, repo: &RepoId, fallback: Worktree) -> Vec<Worktree> {
    let invocation = GitInvocation::new(vec![
        "worktree".to_owned(),
        "list".to_owned(),
        "--porcelain".to_owned(),
    ]);
    if let Ok(output) = engine.run_at(repo.root(), invocation, super::RunKind::Read) {
        if output.success() {
            let worktrees = parse_worktree_list(&output.stdout);
            if !worktrees.is_empty() {
                return worktrees;
            }
        }
    }
    vec![fallback]
}

/// 当前分支短名；游离 HEAD 或空仓库返回 `None`。
fn head_branch(engine: &CliGitEngine, repo: &RepoId) -> AppResult<Option<String>> {
    let invocation = GitInvocation::new(vec![
        "symbolic-ref".to_owned(),
        "--short".to_owned(),
        "--quiet".to_owned(),
        "HEAD".to_owned(),
    ]);
    let output = engine.run_at(repo.root(), invocation, super::RunKind::Read)?;
    if !output.success() {
        return Ok(None);
    }
    let name = output.stdout_lossy().trim().to_owned();
    Ok(if name.is_empty() { None } else { Some(name) })
}

/// 上游短名；没有上游返回 `None`。
fn upstream_of(engine: &CliGitEngine, repo: &RepoId) -> Option<String> {
    let invocation = GitInvocation::new(vec![
        "rev-parse".to_owned(),
        "--abbrev-ref".to_owned(),
        "--symbolic-full-name".to_owned(),
        "@{upstream}".to_owned(),
    ]);
    let output = engine
        .run_at(repo.root(), invocation, super::RunKind::Read)
        .ok()?;
    if !output.success() {
        return None;
    }
    let name = output.stdout_lossy().trim().to_owned();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

// ---------------------------------------------------------------- status

/// 工作区状态。
pub(super) fn status(
    engine: &CliGitEngine,
    repo: &RepoId,
    query: &StatusQuery,
) -> AppResult<StatusReport> {
    let output = run(
        engine,
        repo,
        GitInvocation::new(args::status_args(query.include_ignored)),
    )?;
    let mut report = parse_status_porcelain_v2(&output.stdout);

    // 富化第一段（文件系统）+ 第二段（LFS 属性，经同一套带 stdin 的调用约定）
    let workdir = repo.root().to_path_buf();
    let git_dir = crate::engine::enrich::resolve_git_dir(&workdir);
    let lfs_candidates = crate::engine::enrich::enrich_filesystem(&mut report, &workdir, &git_dir);

    if !lfs_candidates.is_empty() {
        let mut stdin = Vec::new();
        for path in &lfs_candidates {
            stdin.extend_from_slice(path);
            stdin.push(0);
        }
        let invocation = GitInvocation::new(vec![
            "check-attr".to_owned(),
            "-z".to_owned(),
            "--stdin".to_owned(),
            "filter".to_owned(),
        ])
        .with_stdin(stdin);
        let output = engine.run_at(repo.root(), invocation, super::RunKind::Read)?;
        if output.success() {
            crate::engine::enrich::apply_lfs(
                &mut report,
                &crate::engine::enrich::lfs_paths_from_check_attr(&output.stdout),
            );
        }
    }

    report.ignored_count = if query.include_ignored {
        crate::engine::enrich::count_ignored(&report)
    } else {
        None
    };

    Ok(report)
}

// ---------------------------------------------------------------- diff

/// 文件级 diff。
///
/// 两次调用：`--numstat` 给增删行数，`--name-status` 给变更类别
/// （新增/删除/重命名…）。`--numstat` 本身**不带**变更类别：
/// `1 1 path` 既可能是"修改"也可能是"新增后又改了"，靠行数猜会猜错。
pub(super) fn diff(engine: &CliGitEngine, repo: &RepoId, spec: &DiffSpec) -> AppResult<DiffReport> {
    let numstat = run(engine, repo, args::diff_args(spec)?)?;
    let stats = parse_diff_numstat(&numstat.stdout);

    let mut name_status_args = args::diff_args(spec)?.args;
    // 把 --numstat 换成 --name-status，其余（目标、路径过滤、重命名检测）完全一致
    if let Some(position) = name_status_args.iter().position(|arg| arg == "--numstat") {
        name_status_args[position] = "--name-status".to_owned();
    }
    let names = run(engine, repo, GitInvocation::new(name_status_args))?;
    let kinds = parse_name_status_z(&names.stdout);

    let kind_for = |path: &forgedesk_domain::git::RepoPath| -> (DiffChangeKind, Option<forgedesk_domain::git::RepoPath>) {
        kinds
            .iter()
            .find(|(_, target, _)| target == path)
            .map(|(kind, _, original)| (*kind, original.clone()))
            .unwrap_or((DiffChangeKind::Unknown, None))
    };

    // 第三次调用：统一补丁（行级内容）。T1.5 起唯一的数据源，
    // 任务要求文本 diff 必须走 git CLI（与用户终端一致），不得用 libgit2 格式化输出。
    let patch = run(engine, repo, args::patch_args(spec)?)?;
    let limits = crate::parsers::PatchLimits::defaults();
    let sections = crate::parsers::parse_unified_diff(&patch.stdout, &limits, spec.force_full);

    // 按段序号拼合：numstat 与补丁的文件顺序一致（同一次 git 调用的既定顺序）。
    // 段里尽力提取的路径只用于一致性检查；**权威路径来自 numstat（字节精确）**。
    let mut truncated_files = 0_usize;
    let files = stats
        .into_iter()
        .enumerate()
        .map(|(position, stat)| {
            let (change, original) = kind_for(&stat.path);
            let section = sections.get(position);
            if section.is_some_and(|section| section.truncated) {
                truncated_files += 1;
            }
            FileDiff {
                original_path: stat.original_path.or(original),
                path: stat.path,
                change,
                binary: stat.binary || section.is_some_and(|section| section.binary),
                additions: stat.additions.unwrap_or(0),
                deletions: stat.deletions.unwrap_or(0),
                truncated: section.is_some_and(|section| section.truncated),
                hunks: section
                    .map(|section| section.hunks.clone())
                    .unwrap_or_default(),
            }
        })
        .collect();

    Ok(DiffReport {
        files,
        truncated_files,
    })
}

/// 生成原始补丁文本（复制 / 导出 .patch 用；T1.6 的部分暂存也以它为底稿）。
///
/// 返回**原始字节**：补丁里的路径与内容都可能是非 UTF-8，
/// 转成 String 等于把字节路径换成一个不存在的路径（与解析器同一纪律）。
pub(super) fn diff_patch(
    engine: &CliGitEngine,
    repo: &RepoId,
    spec: &DiffSpec,
) -> AppResult<Vec<u8>> {
    let output = run(engine, repo, args::patch_args(spec)?)?;
    if !output.success() {
        return Err(
            AppError::new(ErrorCode::Internal, "git diff reported a failure")
                .with_detail(forgedesk_diagnostics::sanitize_log(&output.stderr_lossy())),
        );
    }
    Ok(output.stdout)
}
/// 解析 `git diff --name-status -z`。
///
/// 格式（实测 git 2.54）：普通条目是 `<状态>\0<路径>\0`；
/// 重命名/复制是 `<状态>\0<来源>\0<目标>\0`。状态首字母决定后面跟几个路径。
fn parse_name_status_z(
    input: &[u8],
) -> Vec<(
    DiffChangeKind,
    forgedesk_domain::git::RepoPath,
    Option<forgedesk_domain::git::RepoPath>,
)> {
    let mut out = Vec::new();
    let mut records = input
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty());

    while let Some(status) = records.next() {
        let kind = match status.first() {
            Some(b'A') => DiffChangeKind::Added,
            Some(b'D') => DiffChangeKind::Deleted,
            Some(b'M') => DiffChangeKind::Modified,
            Some(b'R') => DiffChangeKind::Renamed,
            Some(b'C') => DiffChangeKind::Copied,
            Some(b'T') => DiffChangeKind::TypeChanged,
            _ => DiffChangeKind::Unknown,
        };
        let is_rename = matches!(kind, DiffChangeKind::Renamed | DiffChangeKind::Copied);

        if is_rename {
            let (Some(original), Some(target)) = (records.next(), records.next()) else {
                break;
            };
            out.push((
                kind,
                forgedesk_domain::git::RepoPath::from_bytes(target.to_vec()),
                Some(forgedesk_domain::git::RepoPath::from_bytes(
                    original.to_vec(),
                )),
            ));
        } else {
            let Some(target) = records.next() else { break };
            out.push((
                kind,
                forgedesk_domain::git::RepoPath::from_bytes(target.to_vec()),
                None,
            ));
        }
    }

    out
}

// ---------------------------------------------------------------- log / show

/// 分页查询提交历史。
pub(super) fn log(
    engine: &CliGitEngine,
    repo: &RepoId,
    query: &LogQuery,
) -> AppResult<Page<Commit>> {
    // 空仓库（HEAD 尚未诞生）下 `git log` 会以非零退出结束，但"没有提交可列"
    // 不是错误——打开一个刚 init 的仓库不该弹错误框。
    // 只对默认的 HEAD 查询做这个判断：显式传了不存在的修订名时，
    // 静默返回空列表会掩盖调用方的拼写错误。
    if query.revision.is_none() && !query.all_branches && rev_parse(engine, repo, "HEAD")?.is_none()
    {
        return Ok(Page::empty());
    }

    let output = run(engine, repo, args::log_args(query, LOG_FORMAT)?)?;
    let commits = parse_log_format(&output.stdout);
    Ok(Page::from_over_fetch(commits, query.limit))
}

/// 单条提交（含正文）。
///
/// 用 [`SHOW_FORMAT`]（在列表字段之后追加 `%b`）：两个格式串共享同一套字段顺序，
/// 因此"从列表点进详情"不会出现同一提交有两种字段内容。
pub(super) fn show(engine: &CliGitEngine, repo: &RepoId, revision: &str) -> AppResult<Commit> {
    let output = run(
        engine,
        repo,
        GitInvocation::new(args::show_args(revision, SHOW_FORMAT)),
    )?;
    parse_show_format(&output.stdout)
        .into_iter()
        .next()
        .ok_or_else(|| {
            AppError::new(
                ErrorCode::NotFound,
                format!("revision `{revision}` produced no commit"),
            )
            .with_hint(revision.to_owned())
        })
}

// ---------------------------------------------------------------- refs

/// 分支列表。
pub(super) fn branch_list(engine: &CliGitEngine, repo: &RepoId) -> AppResult<Vec<Branch>> {
    let output = run(
        engine,
        repo,
        GitInvocation::new(args::branch_args(BRANCH_FORMAT)),
    )?;
    Ok(parse_branches(&output.stdout))
}

fn parse_branches(input: &[u8]) -> Vec<Branch> {
    let mut out = Vec::new();
    for line in input.split(|byte| *byte == b'\n') {
        if line.is_empty() {
            continue;
        }
        let text = String::from_utf8_lossy(line);
        let fields: Vec<&str> = text.split(FIELD).collect();
        if fields.len() < 6 {
            continue;
        }

        let full_ref = fields[0];
        let short = fields[1].to_owned();
        let target = fields[2].to_owned();
        let upstream = empty_to_none(fields[3]);
        let track = fields[4];
        let is_remote = full_ref.starts_with("refs/remotes/");

        // `[ahead 1, behind 2]` / `[gone]` / 空
        let upstream_gone = track.contains("gone");
        let (ahead, behind) = parse_track(track);

        out.push(Branch {
            name: short,
            is_remote,
            is_head: fields[5].trim() == "*",
            target,
            upstream,
            ahead,
            behind,
            upstream_gone,
        });
    }
    out
}

/// 解析 `%(upstream:track)`：`[ahead 1, behind 2]`。
fn parse_track(track: &str) -> (Option<i64>, Option<i64>) {
    let mut ahead = None;
    let mut behind = None;
    for part in track.trim_matches(['[', ']']).split(',') {
        let part = part.trim();
        if let Some(value) = part.strip_prefix("ahead ") {
            ahead = value.trim().parse::<i64>().ok();
        } else if let Some(value) = part.strip_prefix("behind ") {
            behind = value.trim().parse::<i64>().ok();
        }
    }
    (ahead, behind)
}

/// 标签列表。
pub(super) fn tag_list(engine: &CliGitEngine, repo: &RepoId) -> AppResult<Vec<Tag>> {
    let output = run(engine, repo, GitInvocation::new(args::tag_args(TAG_FORMAT)))?;
    Ok(parse_tags(&output.stdout))
}

fn parse_tags(input: &[u8]) -> Vec<Tag> {
    let mut out = Vec::new();
    for line in input.split(|byte| *byte == b'\n') {
        if line.is_empty() {
            continue;
        }
        let text = String::from_utf8_lossy(line);
        let fields: Vec<&str> = text.split(FIELD).collect();
        if fields.len() < 6 {
            continue;
        }

        let annotated = fields[2] == "tag";
        let peeled = empty_to_none(fields[3]);
        out.push(Tag {
            name: fields[0].to_owned(),
            target: fields[1].to_owned(),
            commit: peeled.clone().or_else(|| {
                // 轻量标签直接指向提交
                if annotated {
                    None
                } else {
                    Some(fields[1].to_owned())
                }
            }),
            annotated,
            // `%(contents:subject)` 对轻量标签给的是**提交**的 subject，
            // 那不是标签信息——把它当 message 会让界面显示"标签信息 = 提交标题"
            message: if annotated {
                empty_to_none(fields[5])
            } else {
                None
            },
            created_at: fields[4].trim().parse::<i64>().ok(),
        });
    }
    out
}

/// 远端列表。
pub(super) fn remote_list(engine: &CliGitEngine, repo: &RepoId) -> AppResult<Vec<Remote>> {
    let output = run(engine, repo, GitInvocation::new(args::remote_args()))?;
    Ok(parse_remotes(&output.stdout_lossy()))
}

fn parse_remotes(text: &str) -> Vec<Remote> {
    let mut out: Vec<Remote> = Vec::new();
    for line in text.lines() {
        // `origin\thttps://example.com/r.git (fetch)`
        let Some((name, rest)) = line.split_once('\t') else {
            continue;
        };
        let (url, kind) = match rest.rsplit_once(' ') {
            Some((url, kind)) => (url, kind),
            None => (rest, ""),
        };
        let is_push = kind.contains("push");

        // 不依赖"fetch 行一定在 push 行之前"：git 的顺序不保证
        if let Some(remote) = out.iter_mut().find(|remote| remote.name == name) {
            if is_push {
                if url != remote.fetch_url {
                    remote.push_url = Some(url.to_owned());
                }
            } else {
                remote.fetch_url = url.to_owned();
                remote.kind = RemoteKind::from_url(url);
            }
            continue;
        }

        out.push(if is_push {
            Remote {
                name: name.to_owned(),
                fetch_url: String::new(),
                push_url: Some(url.to_owned()),
                kind: RemoteKind::Other,
            }
        } else {
            Remote {
                name: name.to_owned(),
                fetch_url: url.to_owned(),
                push_url: None,
                kind: RemoteKind::from_url(url),
            }
        });
    }
    out
}

// ---------------------------------------------------------------- stash / reflog

/// stash 列表。
pub(super) fn stash_list(engine: &CliGitEngine, repo: &RepoId) -> AppResult<Vec<StashEntry>> {
    let output = run(
        engine,
        repo,
        GitInvocation::new(args::stash_list_args(STASH_FORMAT)),
    )?;
    Ok(parse_stash_list(&output.stdout))
}

fn parse_stash_list(input: &[u8]) -> Vec<StashEntry> {
    let mut out = Vec::new();
    for line in input.split(|byte| *byte == b'\n') {
        if line.is_empty() {
            continue;
        }
        let text = String::from_utf8_lossy(line);
        let fields: Vec<&str> = text.split(FIELD).collect();
        if fields.len() < 5 {
            continue;
        }

        let parents: Vec<&str> = fields[2]
            .split(' ')
            .filter(|part| !part.is_empty())
            .collect();
        out.push(StashEntry {
            index: parse_stash_index(fields[0]).unwrap_or(out.len()),
            oid: fields[1].to_owned(),
            base_oid: parents.first().map(|oid| (*oid).to_owned()),
            message: fields[4].to_owned(),
            created_at: fields[3].trim().parse::<i64>().ok(),
            // `-u` 创建的 stash 有第三个父提交（未跟踪文件的提交）
            includes_untracked: parents.len() >= 3,
        });
    }
    out
}

/// 从 `stash@{3}` 里取出 3。
fn parse_stash_index(selector: &str) -> Option<usize> {
    let start = selector.find('{')? + 1;
    let end = selector[start..].find('}')? + start;
    selector[start..end].trim().parse::<usize>().ok()
}

/// reflog。
pub(super) fn reflog(
    engine: &CliGitEngine,
    repo: &RepoId,
    limit: usize,
) -> AppResult<Vec<ReflogEntry>> {
    let limit = if limit == 0 {
        DEFAULT_REFLOG_LIMIT
    } else {
        limit
    };
    let output = run(
        engine,
        repo,
        GitInvocation::new(args::reflog_args(limit, REFLOG_FORMAT)),
    )?;
    Ok(parse_reflog(&output.stdout))
}

fn parse_reflog(input: &[u8]) -> Vec<ReflogEntry> {
    let mut out = Vec::new();
    for (position, line) in input.split(|byte| *byte == b'\n').enumerate() {
        if line.is_empty() {
            continue;
        }
        let text = String::from_utf8_lossy(line);
        let fields: Vec<&str> = text.split(FIELD).collect();
        if fields.len() < 4 {
            continue;
        }

        // `%gd` 形如 `HEAD@{3}`；取不出数字时按行号兜底（顺序即索引）
        let (reference, index) = match fields[1].split_once("@{") {
            Some((reference, rest)) => (
                reference.to_owned(),
                rest.trim_end_matches('}')
                    .trim()
                    .parse::<usize>()
                    .unwrap_or(position),
            ),
            None => (fields[1].to_owned(), position),
        };
        let subject = fields[2];
        let (action, message) = match subject.split_once(": ") {
            Some((action, message)) => (action.to_owned(), message.to_owned()),
            None => (subject.to_owned(), String::new()),
        };

        out.push(ReflogEntry {
            index,
            oid: fields[0].to_owned(),
            reference,
            action,
            message,
            created_at: fields[3].trim().parse::<i64>().ok(),
        });
    }
    out
}

/// 空字符串转 `None`（git 用空字段表示"没有"）。
fn empty_to_none(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

/// 解析一个引用为 oid；无法解析时返回 `None`。
///
/// 放在 `read` 里而不是 `write`：它是纯读原语，写操作（`commit` 取新 HEAD、
/// `merge` 取结果 oid）只是复用者。
pub(super) fn rev_parse(
    engine: &CliGitEngine,
    repo: &RepoId,
    revision: &str,
) -> AppResult<Option<String>> {
    let output = engine.run_at(
        repo.root(),
        GitInvocation::new(vec![
            "rev-parse".to_owned(),
            "--verify".to_owned(),
            "--quiet".to_owned(),
            revision.to_owned(),
        ]),
        super::RunKind::Read,
    )?;
    if !output.success() {
        return Ok(None);
    }
    let oid = output.stdout_lossy().trim().to_owned();
    Ok(if oid.is_empty() { None } else { Some(oid) })
}

/// 当前 HEAD 的 oid；空仓库返回 `None`。
pub(super) fn head_oid(engine: &CliGitEngine, repo: &RepoId) -> AppResult<Option<String>> {
    rev_parse(engine, repo, "HEAD")
}

/// 当前索引的树 oid（`git write-tree`；T1.7 的索引指纹）。
///
/// 走 `run_write` 而不是 `run_read`：它**确实会往对象库里写一个树对象**
/// （因此不是只读路径），只是不动引用与工作区。索引里有未合并条目时 git 会拒绝，
/// 错误按常规分类上报——冲突状态下本来就不该发起提交。
pub(super) fn index_tree(engine: &CliGitEngine, repo: &RepoId) -> AppResult<String> {
    let output = engine.run_write(repo, GitInvocation::new(vec!["write-tree".to_owned()]))?;
    let oid = output.stdout_lossy().trim().to_owned();
    if oid.is_empty() {
        return Err(AppError::new(
            ErrorCode::Internal,
            "git write-tree returned no tree id",
        ));
    }
    Ok(oid)
}

/// HEAD 的树 oid；空仓库返回 `None`。
pub(super) fn head_tree(engine: &CliGitEngine, repo: &RepoId) -> AppResult<Option<String>> {
    rev_parse(engine, repo, "HEAD^{tree}")
}

/// 实际生效的钩子目录（`git rev-parse --git-path hooks`）。
///
/// 用 git 自己解析而不是拼 `.git/hooks`：`core.hooksPath`（husky 默认设置它）
/// 会让真实目录完全不同。
///
/// 输出去 `trim` 后可能是相对路径（`--git-path` 在旧版本上给相对路径），
/// 因此非绝对路径要相对仓库根解析。
pub(super) fn hooks_dir(engine: &CliGitEngine, repo: &RepoId) -> AppResult<std::path::PathBuf> {
    let output = engine.run_read(
        repo,
        GitInvocation::new(vec![
            "rev-parse".to_owned(),
            "--git-path".to_owned(),
            "hooks".to_owned(),
        ]),
    )?;
    let text = output.stdout_lossy().trim().to_owned();
    if text.is_empty() {
        return Err(AppError::new(
            ErrorCode::Internal,
            "git rev-parse --git-path hooks returned nothing",
        ));
    }
    let path = std::path::PathBuf::from(text);
    Ok(if path.is_absolute() {
        path
    } else {
        repo.root().join(path)
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn name_status_reads_one_path_for_plain_changes() {
        let input = b"M\0b.txt\0A\0new.txt\0";

        let parsed = parse_name_status_z(input);

        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].0, DiffChangeKind::Modified);
        assert_eq!(parsed[0].1.to_string(), "b.txt");
        assert_eq!(parsed[0].2, None);
        assert_eq!(parsed[1].0, DiffChangeKind::Added);
        assert_eq!(parsed[1].1.to_string(), "new.txt");
    }

    #[test]
    fn name_status_reads_two_paths_for_renames_and_copies() {
        let input = b"R081\0a.txt\0renamed.txt\0C100\0src.txt\0copy.txt\0";

        let parsed = parse_name_status_z(input);

        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].0, DiffChangeKind::Renamed);
        assert_eq!(
            parsed[0].2.as_ref().map(|path| path.to_string()),
            Some("a.txt".to_owned())
        );
        assert_eq!(parsed[0].1.to_string(), "renamed.txt");
        assert_eq!(parsed[1].0, DiffChangeKind::Copied);
    }

    #[test]
    fn truncated_name_status_record_does_not_panic() {
        assert!(parse_name_status_z(b"R081\0a.txt\0").is_empty());
        assert!(parse_name_status_z(b"").is_empty());
    }

    #[test]
    fn branches_distinguish_local_from_remote_and_read_tracking() {
        let input = "refs/heads/main\u{1f}main\u{1f}aaa\u{1f}origin/main\u{1f}[ahead 1, behind 2]\u{1f}*\nrefs/remotes/origin/main\u{1f}origin/main\u{1f}aaa\u{1f}\u{1f}\u{1f} \n".as_bytes();

        let branches = parse_branches(input);

        assert_eq!(branches.len(), 2);
        assert!(branches[0].is_head);
        assert!(!branches[0].is_remote);
        assert_eq!(branches[0].ahead, Some(1));
        assert_eq!(branches[0].behind, Some(2));
        assert!(branches[0].has_live_upstream());
        assert!(branches[1].is_remote);
        assert_eq!(branches[1].upstream, None);
    }

    #[test]
    fn gone_upstream_is_flagged() {
        let input =
            "refs/heads/main\u{1f}main\u{1f}aaa\u{1f}origin/main\u{1f}[gone]\u{1f}*\n".as_bytes();

        let branches = parse_branches(input);

        assert!(branches[0].upstream_gone);
        assert!(!branches[0].has_live_upstream());
        assert_eq!(branches[0].ahead, None);
    }

    #[test]
    fn tags_distinguish_annotated_from_lightweight() {
        let input = "v1.0.0\u{1f}tagobj\u{1f}tag\u{1f}commitoid\u{1f}1704164645\u{1f}release\nlight\u{1f}commitoid\u{1f}commit\u{1f}\u{1f}1704164645\u{1f}light subject\n".as_bytes();

        let tags = parse_tags(input);

        assert_eq!(tags.len(), 2);
        assert!(tags[0].annotated);
        assert_eq!(tags[0].commit.as_deref(), Some("commitoid"));
        assert_eq!(tags[0].message.as_deref(), Some("release"));
        assert!(!tags[1].annotated);
        // 轻量标签直接指向提交：commit 与 target 相同，而不是 None
        assert_eq!(tags[1].commit.as_deref(), Some("commitoid"));
        assert_eq!(tags[1].message, None);
    }

    #[test]
    fn remotes_pair_fetch_and_push_urls() {
        let text = "origin\thttps://example.com/r.git (fetch)\norigin\thttps://example.com/r.git (push)\nupstream\tgit@example.com:org/r.git (fetch)\nupstream\tgit@example.com:org/r.git (push)\n";

        let remotes = parse_remotes(text);

        assert_eq!(remotes.len(), 2);
        assert_eq!(remotes[0].name, "origin");
        assert_eq!(remotes[0].kind, RemoteKind::Https);
        assert_eq!(remotes[0].push_url, None, "push 与 fetch 相同就不单独记");
        assert_eq!(remotes[1].kind, RemoteKind::Ssh);
    }

    #[test]
    fn a_distinct_push_url_is_kept() {
        let text = "origin\thttps://example.com/r.git (fetch)\norigin\thttps://push.example.com/r.git (push)\n";

        let remotes = parse_remotes(text);

        assert_eq!(
            remotes[0].push_url.as_deref(),
            Some("https://push.example.com/r.git")
        );
        assert_eq!(
            remotes[0].effective_push_url(),
            "https://push.example.com/r.git"
        );
    }

    #[test]
    fn stash_entries_read_their_index_and_base_commit() {
        let input = "stash@{1}\u{1f}oid1\u{1f}base other\u{1f}1704164645\u{1f}WIP on main: 1a2b3c4 subject\n".as_bytes();

        let entries = parse_stash_list(input);

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].index, 1);
        assert_eq!(entries[0].base_oid.as_deref(), Some("base"));
        assert!(!entries[0].includes_untracked);
        assert_eq!(entries[0].reference(), "stash@{1}");
    }

    #[test]
    fn stash_with_three_parents_includes_untracked_files() {
        let input = "stash@{0}\u{1f}oid\u{1f}base second third\u{1f}1\u{1f}WIP\n".as_bytes();

        let entries = parse_stash_list(input);

        assert!(entries[0].includes_untracked);
        assert_eq!(entries[0].base_oid.as_deref(), Some("base"));
    }

    #[test]
    fn reflog_splits_action_from_message() {
        let input = "oid0\u{1f}HEAD@{0}\u{1f}commit: initial commit\u{1f}1704164645\noid1\u{1f}HEAD@{1}\u{1f}checkout: moving from main to dev\u{1f}1704164600\n".as_bytes();

        let entries = parse_reflog(input);

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].index, 0);
        assert_eq!(entries[0].reference, "HEAD");
        assert_eq!(entries[0].action, "commit");
        assert_eq!(entries[0].message, "initial commit");
        assert_eq!(entries[0].selector(), "HEAD@{0}");
        assert_eq!(entries[1].action, "checkout");
    }

    #[test]
    fn reflog_entry_without_a_colon_keeps_the_whole_subject_as_the_action() {
        let input = "oid\u{1f}HEAD@{0}\u{1f}reset\u{1f}1704164645\n".as_bytes();

        let entries = parse_reflog(input);

        assert_eq!(entries[0].action, "reset");
        assert_eq!(entries[0].message, "");
    }

    #[test]
    fn stash_index_parser_handles_arbitrary_indices() {
        assert_eq!(parse_stash_index("stash@{12}"), Some(12));
        assert_eq!(parse_stash_index("stash@{0}"), Some(0));
        assert_eq!(parse_stash_index("nonsense"), None);
    }

    #[test]
    fn malformed_ref_lines_are_skipped_instead_of_panicking() {
        assert!(parse_branches("too\u{1f}few\n".as_bytes()).is_empty());
        assert!(parse_tags(b"only-one\n").is_empty());
        assert!(parse_stash_list("a\u{1f}b\n".as_bytes()).is_empty());
        assert!(parse_reflog("a\u{1f}b\n".as_bytes()).is_empty());
    }
}
