//! `Libgit2Engine`：libgit2（进程内库）实现，只承担读操作。
//!
//! # 为什么读走 libgit2
//!
//! 读操作会被高频调用（状态刷新、历史分页、diff 预览），每次都起一个进程
//! 既慢又浪费；libgit2 直接读 `.git` 目录，没有进程开销。写操作则相反——
//! 必须完整复刻用户环境（hooks、attributes、签名、filter），只有系统 git CLI 做得到。
//!
//! # 它与 CLI 实现的已知差异
//!
//! 这些差异是**真实的**，不是实现偷懒，全部登记在 `docs/GIT-ENGINE-DIFF.md`，
//! 并由 `tests/differential.rs` 的规范化规则显式排除（而不是假装不存在）：
//!
//! | 字段 | CLI | libgit2 | 原因 |
//! | --- | --- | --- | --- |
//! | `FileChange` 的模式与 oid | 有值 | 全为 `None` | libgit2 的状态 API 不暴露它们 |
//! | 冲突条目的 `XY` | `UU`/`AA`/`DU`… | 统一为 `UU` | libgit2 只给 `CONFLICTED` 位 |
//! | `Commit.refs` | 有值 | 空 | libgit2 没有等价的 `%D`，需要自己遍历全部 ref |
//! | `Commit.signature` | 来自 `%G?` | `Unknown` | libgit2 不做 GPG 校验 |
//! | 子模块状态细节 | 有 | 部分 | 依赖 `StatusOptions` 的开关 |
//! | `RepositoryInfo.worktrees` | 完整列表 | 只有主工作区 | `Repository::worktrees` 只返回工作区**名称**，没有路径与 HEAD |
//!
//! 界面不得依赖这些字段的"两边都一致"——`services` 层读走 libgit2，
//! 因此以 libgit2 的能力为准。**例外**是 `discover`：打开仓库是一次性的、
//! 用户发起的探测，且与仓库配置审计、git 版本检查同属一个动作，
//! 因此 `services` 用 CLI 实现做 `discover`（见 `services::repository` 的说明）。

use std::path::Path;
use std::path::PathBuf;

use forgedesk_domain::git::{
    Branch, BranchInfo, ChangeKind, Commit, DiffChangeKind, DiffReport, DiffSpec, DiffTarget,
    DiscardSpec, EntryKind, FileChange, FileDiff, LogQuery, OperationState, Page, ReflogEntry,
    Remote, RemoteKind, RepoId, RepoPath, RepositoryInfo, Signature, SignatureStatus, StashEntry,
    StatusQuery, StatusReport, SubmoduleState, Tag, Worktree,
};
use forgedesk_domain::{AppError, AppResult, ErrorCode};

use super::progress::ProgressSink;
use super::{unsupported, EngineId, GitEngine};
use crate::process::GitProcess;

/// libgit2 实现。
#[derive(Debug, Default)]
pub struct Libgit2Engine;

impl Libgit2Engine {
    /// 创建引擎。
    pub fn new() -> Self {
        Self
    }
}

/// 打开仓库（向上查找，含裸仓库）。
fn open(repo: &RepoId) -> AppResult<git2::Repository> {
    git2::Repository::discover(repo.root()).map_err(|error| map_error(&error, "open"))
}

/// 把 libgit2 的错误归一化成 `AppError`。
///
/// 分类复用领域层的 `classify`（同 CLI 实现）：两个引擎对"同一类错误"
/// 必须给出同一个错误码，否则前端会因为换了引擎而显示出不同的修复建议。
fn map_error(error: &git2::Error, operation: &str) -> AppError {
    let code = match error.code() {
        git2::ErrorCode::NotFound => ErrorCode::NotFound,
        git2::ErrorCode::Exists => ErrorCode::Validation,
        git2::ErrorCode::Auth => ErrorCode::AuthRequired,
        git2::ErrorCode::UnbornBranch => ErrorCode::NotFound,
        _ => ErrorCode::classify(error.message()),
    };
    AppError::new(
        code,
        format!("libgit2 {operation} failed: {}", error.message()),
    )
    .with_detail(error.message().to_owned())
}

/// 把 libgit2 的状态位映射成 porcelain v2 的 `XY` 对。
///
/// 为什么不是一一对应：libgit2 用位掩码表达"索引侧/工作区侧各自的变更"，
/// 而 porcelain 用两个字符。位掩码里同时置位的情况（例如索引新增 + 工作区修改）
/// 正好对应 `AM`，这是它比 porcelain 更好用的地方。
fn status_pair(status: git2::Status) -> (ChangeKind, ChangeKind) {
    let index = if status.contains(git2::Status::INDEX_NEW) {
        ChangeKind::Added
    } else if status.contains(git2::Status::INDEX_MODIFIED) {
        ChangeKind::Modified
    } else if status.contains(git2::Status::INDEX_DELETED) {
        ChangeKind::Deleted
    } else if status.contains(git2::Status::INDEX_RENAMED) {
        ChangeKind::Renamed
    } else if status.contains(git2::Status::INDEX_TYPECHANGE) {
        ChangeKind::TypeChanged
    } else if status.contains(git2::Status::CONFLICTED) {
        ChangeKind::Unmerged
    } else {
        ChangeKind::Unmodified
    };

    let worktree = if status.contains(git2::Status::WT_NEW) {
        ChangeKind::Added
    } else if status.contains(git2::Status::WT_MODIFIED) {
        ChangeKind::Modified
    } else if status.contains(git2::Status::WT_DELETED) {
        ChangeKind::Deleted
    } else if status.contains(git2::Status::WT_RENAMED) {
        ChangeKind::Renamed
    } else if status.contains(git2::Status::WT_TYPECHANGE) {
        ChangeKind::TypeChanged
    } else if status.contains(git2::Status::CONFLICTED) {
        ChangeKind::Unmerged
    } else {
        ChangeKind::Unmodified
    };

    (index, worktree)
}

/// 记录类型。
fn entry_kind(status: git2::Status) -> EntryKind {
    if status.contains(git2::Status::CONFLICTED) {
        EntryKind::Unmerged
    } else if status.contains(git2::Status::WT_NEW) {
        EntryKind::Untracked
    } else if status.contains(git2::Status::IGNORED) {
        EntryKind::Ignored
    } else {
        EntryKind::Ordinary
    }
}

/// 把 libgit2 的提交对象转成领域 [`Commit`]。
fn to_commit(commit: &git2::Commit<'_>) -> Commit {
    let signature = |who: &git2::Signature<'_>| Signature {
        name: String::from_utf8_lossy(who.name_bytes()).into_owned(),
        email: String::from_utf8_lossy(who.email_bytes()).into_owned(),
        time: Some(who.when().seconds()),
    };

    Commit {
        oid: commit.id().to_string(),
        parents: commit.parent_ids().map(|oid| oid.to_string()).collect(),
        author: signature(&commit.author()),
        committer: signature(&commit.committer()),
        // libgit2 没有 `%D` 的等价物：要拿到"哪些 ref 指向它"必须遍历全部 ref，
        // 这在每次分页查询里做一次是纯浪费（CLI 实现由 git 顺带给出）
        refs: Vec::new(),
        // 同上：GPG 校验需要调用 gpg，libgit2 不提供
        signature: SignatureStatus::Unknown,
        subject: String::from_utf8_lossy(commit.summary_bytes().unwrap_or_default()).into_owned(),
        body: commit
            .body_bytes()
            .map(|body| String::from_utf8_lossy(body).trim().to_owned())
            .filter(|body| !body.is_empty()),
    }
}

impl GitEngine for Libgit2Engine {
    fn id(&self) -> EngineId {
        EngineId::Libgit2
    }

    fn discover(&self, path: &Path) -> AppResult<RepositoryInfo> {
        let repo = git2::Repository::discover(path).map_err(|error| {
            if error.code() == git2::ErrorCode::NotFound {
                AppError::new(
                    ErrorCode::PathNotRepo,
                    "the path is not inside a git repository",
                )
                .with_hint(path.to_string_lossy().into_owned())
            } else {
                map_error(&error, "discover")
            }
        })?;

        let workdir = repo.workdir().map(Path::to_path_buf);
        let root = workdir.clone().unwrap_or_else(|| repo.path().to_path_buf());
        let git_dir = repo.path().to_path_buf();

        let is_empty = repo
            .is_empty()
            .map_err(|error| map_error(&error, "is_empty"))?;
        let detached = repo
            .head_detached()
            .map_err(|error| map_error(&error, "head_detached"))?;
        // 游离 HEAD 时 `head()` 会给一个名为 `HEAD` 的直接引用，
        // 那不是分支名——与 CLI 实现（`symbolic-ref` 失败即 `None`）保持一致
        let head = if detached {
            None
        } else {
            repo.head()
                .ok()
                .and_then(|reference| reference.shorthand().map(str::to_owned))
        };
        let head_oid = repo
            .head()
            .ok()
            .and_then(|reference| reference.target())
            .map(|oid| oid.to_string());

        let is_bare = repo.is_bare();
        Ok(RepositoryInfo {
            default_branch: origin_head_branch(&repo).or_else(|| head.clone()),
            is_shallow: repo.is_shallow(),
            is_lfs: crate::probe::detect_lfs(&git_dir, workdir.as_deref()),
            // libgit2 的 `Repository::worktrees` 只给出**名称**（`StringArray`），
            // 既没有路径也没有 HEAD，因此这里只报主工作区。完整列表由 CLI 侧提供
            // （见 docs/GIT-ENGINE-DIFF.md 的差异表）。
            worktrees: vec![Worktree {
                path: root.clone(),
                head: head_oid,
                branch: head.clone(),
                detached,
                is_bare,
                locked: false,
                prunable: false,
            }],
            id: RepoId::new(root),
            workdir,
            git_dir,
            is_bare,
            is_empty,
            head,
            detached,
            upstream: None,
        })
    }

    fn status(&self, repo: &RepoId, query: &StatusQuery) -> AppResult<StatusReport> {
        let repository = open(repo)?;
        let mut options = git2::StatusOptions::new();
        options
            .include_untracked(true)
            .recurse_untracked_dirs(true)
            .include_ignored(query.include_ignored)
            .renames_head_to_index(true)
            .renames_index_to_workdir(true);

        let statuses = repository
            .statuses(Some(&mut options))
            .map_err(|error| map_error(&error, "status"))?;

        let mut report = StatusReport {
            branch: branch_info(&repository)?,
            operation: OperationState::None,
            entries: Vec::new(),
            ignored_count: None,
        };

        for entry in statuses.iter() {
            let status = entry.status();
            let kind = entry_kind(status);
            let (index_status, worktree_status) = status_pair(status);

            // 重命名/复制条目的 delta：libgit2 的 `StatusEntry::path()` 给的是
            // **来源**路径，目标路径在 `new_file()` 上——与 porcelain 的约定相反，
            // 直接取 path() 会让界面把重命名显示成"旧文件被重命名成它自己"
            let delta = entry.head_to_index().or_else(|| entry.index_to_workdir());
            let is_rename = delta.as_ref().is_some_and(|delta| {
                matches!(delta.status(), git2::Delta::Renamed | git2::Delta::Copied)
            });

            let path = if is_rename {
                delta
                    .as_ref()
                    .and_then(|delta| delta.new_file().path_bytes().map(<[u8]>::to_vec))
                    .unwrap_or_else(|| entry.path_bytes().to_vec())
            } else {
                entry.path_bytes().to_vec()
            };
            let original_path = if is_rename {
                delta
                    .as_ref()
                    .and_then(|delta| delta.old_file().path_bytes().map(<[u8]>::to_vec))
                    .map(RepoPath::from_bytes)
            } else {
                None
            };

            report.entries.push(FileChange {
                kind,
                path: RepoPath::from_bytes(path),
                original_path,
                index_status,
                worktree_status,
                similarity: None,
                // libgit2 的状态 API 不暴露模式与 oid（见模块头的差异表）
                mode_head: None,
                mode_index: None,
                mode_worktree: None,
                oid_head: None,
                oid_index: None,
                stages: None,
                submodule: SubmoduleState::NONE,
                // 富化字段与 CLI 路径共用同一套实现，保证展示一致
                is_binary: false,
                is_lfs: false,
                size_bytes: None,
            });
        }

        // 富化：文件系统（操作状态/大小/二进制）+ LFS 属性（批量 check-attr，
        // 现场建一个 GitProcess——本引擎没有现成的进程句柄，见 bridge.rs 的成本说明）
        let workdir = repo.root().to_path_buf();
        let git_dir = PathBuf::from(repository.path());
        let candidates = super::enrich::enrich_filesystem(&mut report, &workdir, &git_dir);
        if !candidates.is_empty() {
            let process = GitProcess::new();
            let lfs = super::enrich::query_lfs_paths(&process, &workdir, &candidates)?;
            super::enrich::apply_lfs(&mut report, &lfs);
        }

        report.ignored_count = if query.include_ignored {
            super::enrich::count_ignored(&report)
        } else {
            None
        };

        Ok(report)
    }

    fn diff_patch(&self, _repo: &RepoId, _spec: &DiffSpec) -> AppResult<Vec<u8>> {
        // T1.5 明确要求：查看器的补丁文本必须来自 git CLI（与用户终端一致），
        // libgit2 的格式化输出被排除；libgit2 的 diff 统计实现保留用于差分对拍。
        Err(unsupported(EngineId::Libgit2, "diff_patch"))
    }

    fn discard_worktree(&self, _repo: &RepoId, _spec: &DiscardSpec) -> AppResult<()> {
        Err(unsupported(EngineId::Libgit2, "discard_worktree"))
    }

    fn diff(&self, repo: &RepoId, spec: DiffSpec) -> AppResult<DiffReport> {
        let repository = open(repo)?;
        let mut options = git2::DiffOptions::new();
        options
            .context_lines(spec.context_lines)
            .ignore_whitespace(spec.ignore_whitespace);
        for path in &spec.paths {
            options.pathspec(path.to_string_lossy().as_ref());
        }

        let head_tree = head_tree(&repository)?;
        let index = repository
            .index()
            .map_err(|error| map_error(&error, "index"))?;

        let diff = match &spec.target {
            DiffTarget::Staged => {
                repository.diff_tree_to_index(head_tree.as_ref(), Some(&index), Some(&mut options))
            }
            DiffTarget::Unstaged => {
                repository.diff_index_to_workdir(Some(&index), Some(&mut options))
            }
            DiffTarget::Between { from, to } => {
                let from_tree = tree_of(&repository, from)?;
                let to_tree = tree_of(&repository, to)?;
                repository.diff_tree_to_tree(Some(&from_tree), Some(&to_tree), Some(&mut options))
            }
            DiffTarget::Since(revision) => {
                let tree = tree_of(&repository, revision)?;
                repository.diff_tree_to_workdir_with_index(Some(&tree), Some(&mut options))
            }
            DiffTarget::Commit(revision) => {
                let commit = find_commit(&repository, revision)?;
                let parent_tree = commit.parent(0).ok().and_then(|parent| parent.tree().ok());
                let tree = commit.tree().map_err(|error| map_error(&error, "tree"))?;
                repository.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), Some(&mut options))
            }
        }
        .map_err(|error| map_error(&error, "diff"))?;

        // 重命名检测在 diff 建好之后再做：git2 的 `DiffFindOptions` 作用于 `Diff`，
        // 而不是 `DiffOptions`
        let mut diff = diff;
        if spec.detect_renames {
            let mut find = git2::DiffFindOptions::new();
            find.renames(true).copies(true);
            diff.find_similar(Some(&mut find))
                .map_err(|error| map_error(&error, "find_similar"))?;
        }

        let file_count = diff.deltas().len();
        let mut files = Vec::with_capacity(file_count);
        for index in 0..file_count {
            let Some(delta) = diff.get_delta(index) else {
                continue;
            };
            let patch =
                git2::Patch::from_diff(&diff, index).map_err(|error| map_error(&error, "patch"))?;
            let (_, additions, deletions) = patch
                .as_ref()
                .map(git2::Patch::line_stats)
                .unwrap_or(Ok((0, 0, 0)))
                .map_err(|error| map_error(&error, "line_stats"))?;

            files.push(FileDiff {
                path: delta
                    .new_file()
                    .path_bytes()
                    .map(RepoPath::from_bytes)
                    .or_else(|| delta.old_file().path_bytes().map(RepoPath::from_bytes))
                    .unwrap_or_else(|| RepoPath::from_bytes(Vec::new())),
                // 只有重命名/复制才有"来源路径"：libgit2 在普通修改上也会填
                // `old_file().path()`，直接取用会让每个文件都显示成重命名
                original_path: matches!(delta.status(), git2::Delta::Renamed | git2::Delta::Copied)
                    .then(|| delta.old_file().path_bytes().map(RepoPath::from_bytes))
                    .flatten(),
                change: match delta.status() {
                    git2::Delta::Added => DiffChangeKind::Added,
                    git2::Delta::Deleted => DiffChangeKind::Deleted,
                    git2::Delta::Modified => DiffChangeKind::Modified,
                    git2::Delta::Renamed => DiffChangeKind::Renamed,
                    git2::Delta::Copied => DiffChangeKind::Copied,
                    git2::Delta::Typechange => DiffChangeKind::TypeChanged,
                    _ => DiffChangeKind::Unknown,
                },
                binary: delta.flags().is_binary(),
                additions: u64::try_from(additions).unwrap_or_default(),
                deletions: u64::try_from(deletions).unwrap_or_default(),
                // libgit2 路径只做统计对拍，行级内容（与截断）由 CLI 路径提供
                truncated: false,
                hunks: Vec::new(),
            });
        }

        Ok(DiffReport {
            files,
            truncated_files: 0,
        })
    }

    fn log(&self, repo: &RepoId, query: LogQuery) -> AppResult<Page<Commit>> {
        // `--follow` 是 CLI 独有的能力（libgit2 没有等价物）：装作支持等于
        // 悄悄给出**错误结果**（漏掉重命名前的历史），宁可明确拒绝。
        if query.follow_renames {
            return Err(unsupported(EngineId::Libgit2, "log --follow"));
        }

        let repository = open(repo)?;
        let mut revwalk = repository
            .revwalk()
            .map_err(|error| map_error(&error, "revwalk"))?;
        revwalk
            .set_sorting(git2::Sort::TIME)
            .map_err(|error| map_error(&error, "sorting"))?;
        if query.first_parent_only {
            revwalk
                .simplify_first_parent()
                .map_err(|error| map_error(&error, "simplify_first_parent"))?;
        }

        if !query.revisions.is_empty() {
            // 分支多选（T2.3）：push 多个 tip 的并集。revision 可能是名字也可能是
            // oid，统一走 revparse（与 CLI 的位置参数解析一致）；名字解析失败
            // 时报错（与单 revision 路径同一原则：不掩盖调用方的拼写错误）。
            for revision in &query.revisions {
                let commit = find_commit(&repository, revision)?;
                revwalk
                    .push(commit.id())
                    .map_err(|error| map_error(&error, "push"))?;
            }
        } else if query.all_branches {
            for reference in repository
                .references()
                .map_err(|error| map_error(&error, "references"))?
            {
                let Ok(reference) = reference else { continue };
                let _ = revwalk.push_ref(reference.name().unwrap_or("HEAD"));
            }
        } else {
            // 与 CLI 实现同一条规则（见 `cli::read::log`）：HEAD 解析不出来说明仓库
            // 还没有提交，"没有提交可列"不是错误；但显式指定了不存在的修订名时
            // 仍然报错，否则会掩盖调用方的拼写错误。
            //
            // 用 `head().is_err()` 判断而不是 `is_empty()`：后者在刚 init 的仓库上
            // 并不总是给出 true，而"HEAD 指不到东西"正是我们要判断的那件事。
            if query.revision.is_none() && repository.head().is_err() {
                return Ok(Page::empty());
            }

            match query.revision.as_deref() {
                Some(revision) => {
                    // 修订可能是短名（main / v1.0）也可能是 oid：`push_ref` 只认
                    // 完整引用名（短名会报 "not valid"），统一走 revparse→push，
                    // 与 CLI 位置参数的解析能力保持一致。
                    let commit = find_commit(&repository, revision)?;
                    revwalk
                        .push(commit.id())
                        .map_err(|error| map_error(&error, "push"))?;
                }
                None => {
                    revwalk
                        .push_ref("HEAD")
                        .map_err(|error| map_error(&error, "push_ref"))?;
                }
            }
        }

        let mut commits = Vec::new();
        // 过滤与 skip 都必须发生在"凑满一页"的计数之前：CLI 的 `--author` /
        // `--grep` / `--merges` / `--skip` 全部作用在**过滤后的流**上（`--skip N`
        // 跳过的是第 N 条**匹配**，不是第 N 条提交）。这里逐条走过滤器、过滤器
        // 全通过后再决定"这条用于跳过还是计入本页"——T2.3 之前 skip 走的是
        // `revwalk.skip()`（过滤前跳行），带筛选翻第二页时会与 CLI 给出不同的
        // 窗口，这正是差分测试（作者过滤 × skip）抓到的形状。
        let mut skipped = 0_usize;
        for oid in revwalk {
            let oid = oid.map_err(|error| map_error(&error, "revwalk"))?;
            let commit = repository
                .find_commit(oid)
                .map_err(|error| map_error(&error, "find_commit"))?;

            // 作者过滤在领域层用同一个规则处理，两个引擎才不会给出不同的结果集
            if let Some(author) = &query.author {
                let hit = commit
                    .author()
                    .name_bytes()
                    .windows(author.len())
                    .any(|window| String::from_utf8_lossy(window).eq_ignore_ascii_case(author))
                    || String::from_utf8_lossy(commit.author().email_bytes())
                        .to_ascii_lowercase()
                        .contains(&author.to_ascii_lowercase());
                if !hit {
                    continue;
                }
            }

            if let Some(since) = query.since {
                if commit.time().seconds() < since {
                    continue;
                }
            }
            if let Some(until) = query.until {
                if commit.time().seconds() > until {
                    continue;
                }
            }
            if query.merges_only && commit.parent_count() < 2 {
                continue;
            }
            if let Some(term) = &query.message_contains {
                // 与 CLI 的 `--grep --fixed-strings` 同语义：全文（含正文）、字面；
                // `case_insensitive` 对应 CLI 的 `-i`（不区分大小写的字面匹配）
                let matched = std::str::from_utf8(commit.message_bytes())
                    .map(|text| {
                        if query.case_insensitive {
                            text.to_lowercase().contains(&term.to_lowercase())
                        } else {
                            text.contains(term.as_str())
                        }
                    })
                    .unwrap_or(false);
                if !matched {
                    continue;
                }
            }

            if skipped < query.skip {
                skipped += 1;
                continue;
            }

            commits.push(to_commit(&commit));
            if commits.len() >= query.limit.saturating_add(1) {
                break;
            }
        }

        Ok(Page::from_over_fetch(commits, query.limit))
    }

    fn show(&self, repo: &RepoId, revision: &str) -> AppResult<Commit> {
        let repository = open(repo)?;
        let commit = find_commit(&repository, revision)?;
        Ok(to_commit(&commit))
    }

    fn branch_list(&self, repo: &RepoId) -> AppResult<Vec<Branch>> {
        let repository = open(repo)?;
        let head = repository
            .head()
            .ok()
            .and_then(|reference| reference.shorthand().map(str::to_owned));

        let branches = repository
            .branches(None)
            .map_err(|error| map_error(&error, "branches"))?;

        let mut out = Vec::new();
        for entry in branches {
            let (branch, kind) = entry.map_err(|error| map_error(&error, "branch"))?;
            let Some(name) = branch.name().ok().flatten() else {
                continue;
            };
            let is_remote = kind == git2::BranchType::Remote;
            let Some(target) = branch.get().target() else {
                continue;
            };

            let upstream = branch.upstream().ok();
            let upstream_name = upstream
                .as_ref()
                .and_then(|upstream| upstream.name().ok().flatten())
                .map(str::to_owned);
            let (ahead, behind) = match &upstream {
                Some(upstream) => match upstream.get().target() {
                    Some(upstream_oid) => repository
                        .graph_ahead_behind(target, upstream_oid)
                        .map(|(ahead, behind)| (Some(ahead as i64), Some(behind as i64)))
                        .unwrap_or((None, None)),
                    None => (None, None),
                },
                None => (None, None),
            };

            out.push(Branch {
                name: name.to_owned(),
                is_remote,
                is_head: !is_remote && head.as_deref() == Some(name),
                target: target.to_string(),
                upstream: upstream_name,
                ahead,
                behind,
                // libgit2 不区分"没有上游"与"上游已删除"（前者 upstram() 就失败）
                upstream_gone: false,
            });
        }

        Ok(out)
    }

    fn tag_list(&self, repo: &RepoId) -> AppResult<Vec<Tag>> {
        let repository = open(repo)?;
        let names = repository
            .tag_names(None)
            .map_err(|error| map_error(&error, "tag_names"))?;

        let mut out = Vec::new();
        for name in names.iter().flatten() {
            let Ok(object) = repository.revparse_single(&format!("refs/tags/{name}")) else {
                continue;
            };
            let annotated = object.kind() == Some(git2::ObjectType::Tag);
            let target = object.id().to_string();
            let commit = object
                .peel_to_commit()
                .ok()
                .map(|commit| commit.id().to_string());

            let (message, created_at) = match object.into_tag() {
                Ok(tag) => (
                    tag.message()
                        .map(|message| message.lines().next().unwrap_or_default().to_owned())
                        .filter(|message| !message.is_empty()),
                    tag.tagger().map(|tagger| tagger.when().seconds()),
                ),
                Err(_) => (None, None),
            };

            out.push(Tag {
                name: name.to_owned(),
                target,
                commit,
                annotated,
                message,
                created_at,
            });
        }

        Ok(out)
    }

    fn remote_list(&self, repo: &RepoId) -> AppResult<Vec<Remote>> {
        let repository = open(repo)?;
        let names = repository
            .remotes()
            .map_err(|error| map_error(&error, "remotes"))?;

        let mut out = Vec::new();
        for name in names.iter().flatten() {
            let Ok(remote) = repository.find_remote(name) else {
                continue;
            };
            let fetch_url = remote.url().unwrap_or_default().to_owned();
            let push_url = remote
                .pushurl()
                .filter(|url| *url != fetch_url)
                .map(str::to_owned);

            out.push(Remote {
                name: name.to_owned(),
                kind: RemoteKind::from_url(&fetch_url),
                fetch_url,
                push_url,
            });
        }

        Ok(out)
    }

    fn stash_list(&self, repo: &RepoId) -> AppResult<Vec<StashEntry>> {
        let mut repository = open(repo)?;
        let mut collected: Vec<(usize, String, git2::Oid)> = Vec::new();
        repository
            .stash_foreach(|index, message, oid| {
                collected.push((index, message.to_owned(), *oid));
                true
            })
            .map_err(|error| map_error(&error, "stash_foreach"))?;

        let mut out = Vec::new();
        for (index, message, oid) in collected {
            let commit = repository.find_commit(oid).ok();
            let parent_count = commit
                .as_ref()
                .map(|commit| commit.parent_count())
                .unwrap_or_default();

            out.push(StashEntry {
                index,
                oid: oid.to_string(),
                base_oid: commit
                    .as_ref()
                    .and_then(|commit| commit.parent_ids().next())
                    .map(|oid| oid.to_string()),
                message,
                created_at: commit
                    .as_ref()
                    .map(|commit| commit.committer().when().seconds()),
                includes_untracked: parent_count >= 3,
            });
        }

        Ok(out)
    }

    fn reflog(&self, repo: &RepoId, limit: usize) -> AppResult<Vec<ReflogEntry>> {
        let repository = open(repo)?;
        let reflog = repository
            .reflog("HEAD")
            .map_err(|error| map_error(&error, "reflog"))?;

        let mut out = Vec::new();
        for (position, entry) in reflog.iter().enumerate().take(limit) {
            let message = entry.message().unwrap_or_default();
            let (action, detail) = match message.split_once(": ") {
                Some((action, detail)) => (action.to_owned(), detail.to_owned()),
                None => (message.to_owned(), String::new()),
            };

            out.push(ReflogEntry {
                index: position,
                oid: entry.id_new().to_string(),
                reference: "HEAD".to_owned(),
                action,
                message: detail,
                created_at: Some(entry.committer().when().seconds()),
            });
        }

        Ok(out)
    }

    // ---- 写操作：一律明确不支持（见 `engine` 模块头） ----

    fn init(
        &self,
        _path: &Path,
        _spec: forgedesk_domain::git::InitSpec,
    ) -> AppResult<RepositoryInfo> {
        Err(unsupported(EngineId::Libgit2, "init"))
    }

    fn clone(
        &self,
        _spec: forgedesk_domain::git::CloneSpec,
        _progress: &ProgressSink,
    ) -> AppResult<RepositoryInfo> {
        Err(unsupported(EngineId::Libgit2, "clone"))
    }

    fn stage(&self, _repo: &RepoId, _spec: forgedesk_domain::git::StageSpec) -> AppResult<()> {
        Err(unsupported(EngineId::Libgit2, "stage"))
    }

    fn unstage(&self, _repo: &RepoId, _spec: forgedesk_domain::git::StageSpec) -> AppResult<()> {
        Err(unsupported(EngineId::Libgit2, "unstage"))
    }

    fn apply_patch(
        &self,
        _repo: &RepoId,
        _spec: &forgedesk_domain::git::ApplyPatchSpec,
    ) -> AppResult<()> {
        // 与所有写操作同一理由：部分暂存必须复刻用户终端里 `git apply --cached` 的行为
        // （attributes / filter / 空白策略），libgit2 的索引写入不具备这条一致性。
        Err(unsupported(EngineId::Libgit2, "apply_patch"))
    }

    fn index_tree(&self, repo: &RepoId) -> AppResult<String> {
        let repository = open(repo)?;
        let mut index = repository
            .index()
            .map_err(|error| map_error(&error, "index"))?;
        // 与 CLI 的 `git write-tree` 同一语义：写树对象、不动引用与工作区。
        // 两边都由 git/libgit2 从同一份索引算出，因此结果必须一致
        // （差分测试里断言这一点）。
        let oid = index
            .write_tree()
            .map_err(|error| map_error(&error, "write-tree"))?;
        Ok(oid.to_string())
    }

    fn head_tree(&self, repo: &RepoId) -> AppResult<Option<String>> {
        let repository = open(repo)?;
        // 结果先绑定到局部变量：git2 的 `Reference` 借用 `repository`，
        // 若把这个 match 留作尾表达式，临时值会在 `repository` 之后析构，
        // 借用检查因此拒绝（E0597）。
        let tree = match repository.head() {
            Ok(head) => {
                let commit = head
                    .peel_to_commit()
                    .map_err(|error| map_error(&error, "head"))?;
                Some(commit.tree_id().to_string())
            }
            // 空仓库（HEAD 指向尚未诞生的分支）与没有 HEAD 都返回 None：
            // 它们是**正常状态**，不该让"准备提交"整体失败。
            Err(error)
                if matches!(
                    error.code(),
                    git2::ErrorCode::UnbornBranch | git2::ErrorCode::NotFound
                ) =>
            {
                None
            }
            Err(error) => return Err(map_error(&error, "head")),
        };
        Ok(tree)
    }

    fn hooks_dir(&self, repo: &RepoId) -> AppResult<std::path::PathBuf> {
        let repository = open(repo)?;
        // `core.hooksPath` 可能来自仓库配置，也可能来自全局/系统配置，
        // 因此问 config()（它按 git 的优先级合并）而不是自己读 .git/config
        let configured = repository
            .config()
            .ok()
            .and_then(|config| config.get_path("core.hooksPath").ok());

        Ok(match configured {
            Some(path) if path.is_absolute() => path,
            // 相对路径相对**仓库根**解析（与 git 的行为一致），不是 .git 目录
            Some(path) => repo.root().join(path),
            None => repository.path().join("hooks"),
        })
    }

    fn remote_refs_containing(&self, _repo: &RepoId, _revision: &str) -> AppResult<Vec<String>> {
        // 与 commit 同族：它服务的是"这次改写会不会影响远端"这个写路径判断，
        // 而 libgit2 侧要遍历 refs 自己做可达性计算。能力边界见
        // docs/GIT-ENGINE-DIFF.md §4。
        Err(unsupported(EngineId::Libgit2, "remote_refs_containing"))
    }

    fn authors(&self, _repo: &RepoId) -> AppResult<Vec<forgedesk_domain::git::AuthorSummary>> {
        Err(unsupported(EngineId::Libgit2, "authors"))
    }

    fn branch_create(
        &self,
        _repo: &RepoId,
        _spec: &forgedesk_domain::git::BranchCreateSpec,
    ) -> AppResult<()> {
        Err(unsupported(EngineId::Libgit2, "branch_create"))
    }

    fn branch_switch(
        &self,
        _repo: &RepoId,
        _strategy: forgedesk_domain::git::SwitchStrategy,
        _target: &str,
    ) -> AppResult<()> {
        Err(unsupported(EngineId::Libgit2, "branch_switch"))
    }

    fn branch_rename(
        &self,
        _repo: &RepoId,
        _spec: &forgedesk_domain::git::BranchRenameSpec,
    ) -> AppResult<()> {
        Err(unsupported(EngineId::Libgit2, "branch_rename"))
    }

    fn branch_delete(
        &self,
        _repo: &RepoId,
        _spec: &forgedesk_domain::git::BranchDeleteSpec,
    ) -> AppResult<Vec<String>> {
        Err(unsupported(EngineId::Libgit2, "branch_delete"))
    }

    fn branch_set_upstream(
        &self,
        _repo: &RepoId,
        _spec: &forgedesk_domain::git::BranchSetUpstreamSpec,
    ) -> AppResult<()> {
        Err(unsupported(EngineId::Libgit2, "branch_set_upstream"))
    }

    fn branch_compare(&self, _repo: &RepoId, _a: &str, _b: &str) -> AppResult<(u64, u64)> {
        // 比较是纯读取：libgit2 可以给出 ahead/behind（walk 两个分支找分叉点）。
        // v1 只用 CLI 一侧（与 branch_only_commits 的解析共享语义），如实拒绝以保持
        // "同一条数据一个来源"。
        Err(unsupported(EngineId::Libgit2, "branch_compare"))
    }

    fn branch_only_commits(
        &self,
        _repo: &RepoId,
        _a: &str,
        _b: &str,
    ) -> AppResult<Vec<(String, String)>> {
        Err(unsupported(EngineId::Libgit2, "branch_only_commits"))
    }

    fn tag_create(
        &self,
        _repo: &RepoId,
        _spec: &forgedesk_domain::git::TagCreateSpec,
    ) -> AppResult<()> {
        Err(unsupported(EngineId::Libgit2, "tag_create"))
    }

    fn tag_delete(
        &self,
        _repo: &RepoId,
        _spec: &forgedesk_domain::git::TagDeleteSpec,
    ) -> AppResult<()> {
        Err(unsupported(EngineId::Libgit2, "tag_delete"))
    }

    fn commit(
        &self,
        _repo: &RepoId,
        _spec: forgedesk_domain::git::CommitSpec,
    ) -> AppResult<String> {
        Err(unsupported(EngineId::Libgit2, "commit"))
    }

    fn reset(&self, _repo: &RepoId, _spec: forgedesk_domain::git::ResetSpec) -> AppResult<()> {
        Err(unsupported(EngineId::Libgit2, "reset"))
    }

    fn head_oid(&self, repo: &RepoId) -> AppResult<Option<String>> {
        let repository = open(repo)?;
        if repository
            .is_empty()
            .map_err(|error| map_error(&error, "is_empty"))?
        {
            return Ok(None);
        }
        let head = repository
            .head()
            .map_err(|error| map_error(&error, "head"))?;
        Ok(head.target().map(|oid| oid.to_string()))
    }

    fn update_ref(&self, _repo: &RepoId, _name: &str, _oid: &str) -> AppResult<()> {
        Err(unsupported(EngineId::Libgit2, "update_ref"))
    }

    fn delete_ref(&self, _repo: &RepoId, _name: &str) -> AppResult<()> {
        Err(unsupported(EngineId::Libgit2, "delete_ref"))
    }

    fn ref_exists(&self, _repo: &RepoId, _name: &str) -> AppResult<bool> {
        Err(unsupported(EngineId::Libgit2, "ref_exists"))
    }

    fn read_tree(&self, _repo: &RepoId, _treeish: &str) -> AppResult<()> {
        Err(unsupported(EngineId::Libgit2, "read_tree"))
    }

    fn checkout(
        &self,
        _repo: &RepoId,
        _spec: forgedesk_domain::git::CheckoutSpec,
    ) -> AppResult<()> {
        Err(unsupported(EngineId::Libgit2, "checkout"))
    }

    fn merge(
        &self,
        _repo: &RepoId,
        _spec: forgedesk_domain::git::MergeSpec,
    ) -> AppResult<forgedesk_domain::git::MergeOutcome> {
        Err(unsupported(EngineId::Libgit2, "merge"))
    }

    fn cherry_pick(
        &self,
        _repo: &RepoId,
        _revision: &str,
    ) -> AppResult<forgedesk_domain::git::MergeOutcome> {
        Err(unsupported(EngineId::Libgit2, "cherry_pick"))
    }

    fn revert(
        &self,
        _repo: &RepoId,
        _revision: &str,
    ) -> AppResult<forgedesk_domain::git::MergeOutcome> {
        Err(unsupported(EngineId::Libgit2, "revert"))
    }

    fn stash(&self, _repo: &RepoId, _spec: forgedesk_domain::git::StashSpec) -> AppResult<()> {
        Err(unsupported(EngineId::Libgit2, "stash"))
    }

    fn fetch(
        &self,
        _repo: &RepoId,
        _spec: forgedesk_domain::git::FetchSpec,
        _progress: &ProgressSink,
        _cancel: &tokio_util::sync::CancellationToken,
        _auth: &crate::process::NetworkAuth,
    ) -> AppResult<forgedesk_domain::git::FetchOutcome> {
        Err(unsupported(EngineId::Libgit2, "fetch"))
    }

    fn pull(
        &self,
        _repo: &RepoId,
        _spec: forgedesk_domain::git::PullSpec,
        _progress: &ProgressSink,
        _cancel: &tokio_util::sync::CancellationToken,
        _auth: &crate::process::NetworkAuth,
    ) -> AppResult<forgedesk_domain::git::PullOutcome> {
        Err(unsupported(EngineId::Libgit2, "pull"))
    }

    fn probe_remote(
        &self,
        _cwd: &std::path::Path,
        _url: &str,
        _auth: &crate::process::NetworkAuth,
    ) -> AppResult<usize> {
        Err(unsupported(EngineId::Libgit2, "probe_remote"))
    }

    fn probe_ssh_agent(&self) -> AppResult<crate::engine::ProbeOutput> {
        Err(unsupported(EngineId::Libgit2, "probe_ssh_agent"))
    }

    fn push(
        &self,
        _repo: &RepoId,
        _spec: forgedesk_domain::git::PushSpec,
        _progress: &ProgressSink,
        _cancel: &tokio_util::sync::CancellationToken,
        _auth: &crate::process::NetworkAuth,
    ) -> AppResult<forgedesk_domain::git::PushOutcome> {
        Err(unsupported(EngineId::Libgit2, "push"))
    }

    fn remote_add(&self, _repo: &RepoId, _name: &str, _url: &str) -> AppResult<()> {
        Err(unsupported(EngineId::Libgit2, "remote_add"))
    }

    fn remote_remove(&self, _repo: &RepoId, _name: &str) -> AppResult<()> {
        Err(unsupported(EngineId::Libgit2, "remote_remove"))
    }

    fn remote_rename(&self, _repo: &RepoId, _old: &str, _new: &str) -> AppResult<()> {
        Err(unsupported(EngineId::Libgit2, "remote_rename"))
    }

    fn remote_set_url(&self, _repo: &RepoId, _name: &str, _url: &str) -> AppResult<()> {
        Err(unsupported(EngineId::Libgit2, "remote_set_url"))
    }

    fn rebase(
        &self,
        _repo: &RepoId,
        _plan: forgedesk_domain::git::ReorderSpec,
        _progress: &ProgressSink,
    ) -> AppResult<forgedesk_domain::git::MergeOutcome> {
        Err(unsupported(EngineId::Libgit2, "rebase"))
    }
}

/// 远端默认分支（`refs/remotes/origin/HEAD` 指向的分支短名）。
///
/// 与 CLI 实现同一条规则（见 `cli::read::default_branch_of`）：
/// `origin/HEAD` 比"当前分支"更能代表默认分支。
fn origin_head_branch(repo: &git2::Repository) -> Option<String> {
    let reference = repo.find_reference("refs/remotes/origin/HEAD").ok()?;
    let target = reference.symbolic_target()?;
    target
        .strip_prefix("refs/remotes/origin/")
        .map(str::to_owned)
        .filter(|branch| !branch.is_empty())
}

/// 当前 HEAD 的树；空仓库返回 `None`。
fn head_tree(repository: &git2::Repository) -> AppResult<Option<git2::Tree<'_>>> {
    let Ok(head) = repository.head() else {
        return Ok(None);
    };
    let commit = head
        .peel_to_commit()
        .map_err(|error| map_error(&error, "peel_to_commit"))?;
    Ok(Some(
        commit.tree().map_err(|error| map_error(&error, "tree"))?,
    ))
}

/// 解析一个引用为树。
fn tree_of<'repo>(
    repository: &'repo git2::Repository,
    revision: &str,
) -> AppResult<git2::Tree<'repo>> {
    let object = repository
        .revparse_single(revision)
        .map_err(|error| map_error(&error, "revparse"))?;
    object
        .peel_to_tree()
        .map_err(|error| map_error(&error, "peel_to_tree"))
}

/// 解析一个引用为提交。
fn find_commit<'repo>(
    repository: &'repo git2::Repository,
    revision: &str,
) -> AppResult<git2::Commit<'repo>> {
    let object = repository
        .revparse_single(revision)
        .map_err(|error| map_error(&error, "revparse"))?;
    object
        .peel_to_commit()
        .map_err(|error| map_error(&error, "peel_to_commit"))
}

/// HEAD 所在分支的信息（供状态报告使用）。
///
/// 与 CLI 实现的差别：上游已删除（`[gone]`）在 libgit2 里表现为"取不到上游"，
/// 与"从未配置上游"不可区分——因此 `upstream_gone` 恒为 `false`（见模块头差异表）。
fn branch_info(repository: &git2::Repository) -> AppResult<BranchInfo> {
    let mut info = BranchInfo {
        detached: repository.head_detached().unwrap_or(false),
        ..BranchInfo::default()
    };

    let head = repository.head().ok();
    if let Some(reference) = head.as_ref() {
        info.head = reference.shorthand().map(str::to_owned);
        info.oid = reference.target().map(|oid| oid.to_string());
    }
    // 空仓库没有 oid：`head()` 会以 UnbornBranch 失败，但保险起见再判一次
    if repository.is_empty().unwrap_or(false) {
        info.oid = None;
        info.detached = false;
    }

    // 上游与领先/落后。`Branch::wrap` 是 unsafe 的（我们 forbid(unsafe_code)），
    // 因此走 `branch_upstream_name` + `refname_to_id` 这条等价路径
    if let Some(reference) = head.as_ref() {
        if let (Some(name), Some(local)) = (reference.name(), info.oid.clone()) {
            if let Ok(upstream_ref) = repository.branch_upstream_name(name) {
                if let Some(upstream_name) = upstream_ref.as_str() {
                    info.upstream =
                        Some(upstream_name.trim_start_matches("refs/remotes/").to_owned());
                    if let (Ok(local_oid), Ok(upstream_oid)) = (
                        git2::Oid::from_str(&local),
                        repository.refname_to_id(upstream_name),
                    ) {
                        if let Ok((ahead, behind)) =
                            repository.graph_ahead_behind(local_oid, upstream_oid)
                        {
                            info.ahead = Some(i64::try_from(ahead).unwrap_or_default());
                            info.behind = Some(i64::try_from(behind).unwrap_or_default());
                        }
                    }
                }
            }
        }
    }

    Ok(info)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{entry_kind, status_pair};
    use forgedesk_domain::git::{ChangeKind, EntryKind};

    #[test]
    fn index_and_worktree_bits_map_to_separate_status_characters() {
        let both = status_pair(git2::Status::INDEX_NEW | git2::Status::WT_MODIFIED);
        assert_eq!(both, (ChangeKind::Added, ChangeKind::Modified));

        let staged_only = status_pair(git2::Status::INDEX_MODIFIED);
        assert_eq!(staged_only, (ChangeKind::Modified, ChangeKind::Unmodified));

        let worktree_only = status_pair(git2::Status::WT_DELETED);
        assert_eq!(worktree_only, (ChangeKind::Unmodified, ChangeKind::Deleted));
    }

    #[test]
    fn conflicted_entries_are_marked_unmerged_on_both_sides() {
        assert_eq!(
            status_pair(git2::Status::CONFLICTED),
            (ChangeKind::Unmerged, ChangeKind::Unmerged)
        );
    }

    #[test]
    fn untracked_and_ignored_are_distinguished() {
        assert_eq!(entry_kind(git2::Status::WT_NEW), EntryKind::Untracked);
        assert_eq!(entry_kind(git2::Status::IGNORED), EntryKind::Ignored);
        assert_eq!(
            entry_kind(git2::Status::INDEX_MODIFIED),
            EntryKind::Ordinary
        );
        // 冲突优先于其他位：一个既被修改又冲突的文件属于冲突
        assert_eq!(
            entry_kind(git2::Status::CONFLICTED | git2::Status::WT_MODIFIED),
            EntryKind::Unmerged
        );
    }
}
