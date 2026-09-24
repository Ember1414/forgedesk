//! 写操作的实现。
//!
//! # 三个"不能用 run_checked"的地方
//!
//! `merge` / `cherry_pick` / `revert` 在**产生冲突**时以非零退出码结束，
//! 而冲突是正常流程（M3 的冲突向导就是为它准备的）。`push` 被拒绝时同样非零，
//! 但"被拒绝"要变成界面上的可选项（`force-with-lease`）而不是一句错误。
//! 因此这四个方法读原始输出、自行判定，其余写操作一律 `run_write`（断言成功）。
//!
//! # 本层不创建快照
//!
//! 红线 R7 的"快照"由 `services` 层在执行前编排（见 `engine` 模块头）。

use std::path::{Path, PathBuf};

use forgedesk_diagnostics::sanitize_log;
use forgedesk_domain::git::{
    AmendMode, ApplyPatchSpec, CheckoutSpec, CloneSpec, CommitSpec, DiscardSpec, FetchOutcome,
    FetchSpec, InitSpec, MergeKind, MergeOutcome, MergeSpec, PullOutcome, PullSpec, PushOutcome,
    PushRejection, PushSpec, RefUpdate, RefUpdateKind, RepoId, RepositoryInfo, ResetSpec,
    StageSpec, StashSpec,
};
use forgedesk_domain::{AppError, AppResult, ErrorCode};

use super::args::{self, GitInvocation};
use super::{read, CliGitEngine, RunKind};
use crate::engine::progress::ProgressSink;
use crate::parsers::parse_ls_files_stage;
use crate::process::GitOutput;

/// `git init`。
pub(super) fn init(
    engine: &CliGitEngine,
    path: &Path,
    spec: &InitSpec,
) -> AppResult<RepositoryInfo> {
    engine.run_write_at(path, GitInvocation::new(args::init_args(spec)))?;
    read::discover(engine, path)
}

/// `git clone`。
pub(super) fn clone(
    engine: &CliGitEngine,
    spec: &CloneSpec,
    progress: &ProgressSink,
) -> AppResult<RepositoryInfo> {
    // clone 的目标目录还不存在，因此工作目录取它的父目录
    let cwd = spec
        .into
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);

    engine.run_network(&cwd, GitInvocation::new(args::clone_args(spec)), progress)?;
    read::discover(engine, &spec.into)
}

/// 暂存。
pub(super) fn stage(engine: &CliGitEngine, repo: &RepoId, spec: &StageSpec) -> AppResult<()> {
    engine.run_write(repo, args::stage_args(spec)?)?;
    Ok(())
}

/// 取消暂存。
pub(super) fn unstage(engine: &CliGitEngine, repo: &RepoId, spec: &StageSpec) -> AppResult<()> {
    engine.run_write(repo, args::unstage_args(spec)?)?;
    Ok(())
}

/// 应用一份补丁（行级 / 块级暂存与取消暂存、按块丢弃；T1.6）。
///
/// # 为什么不走 `run_write`
///
/// `run_write` 用 `ensure_success` 把非零退出分类成通用错误码，而补丁被拒绝时
/// 契约要求的是 `PATCH_APPLY_FAILED` 加上**原始 stderr**（它是唯一能解释
/// "为什么这一块应用不上"的信息）与一个"刷新状态并重试"的动作。
/// 分类不在这里猜：`ErrorCode::classify` 认得 "patch does not apply"，
/// 但补丁失败还有"上下文不匹配"这类没有固定措辞的形态，因此直接给定错误码。
///
/// 空补丁不启动进程：裁剪后没有内容要写是正常结果（用户只选了上下文行），
/// 起一个 git 进程去应用空补丁只会得到一条无意义的 stderr。
pub(super) fn apply_patch(
    engine: &CliGitEngine,
    repo: &RepoId,
    spec: &ApplyPatchSpec,
) -> AppResult<()> {
    if spec.is_empty() {
        return Ok(());
    }

    // `--check` 只读索引，不该获取可选锁（否则会与用户终端里的 git 抢锁）。
    let kind = if spec.check_only {
        RunKind::Read
    } else {
        RunKind::Write
    };
    let output = engine.run_at(repo.root(), args::apply_patch_args(spec), kind)?;
    if output.success() {
        return Ok(());
    }

    let stderr = output.stderr_lossy();
    Err(
        AppError::new(ErrorCode::PatchApplyFailed, "git apply rejected the patch")
            .with_detail(super::truncate(
                sanitize_log(&stderr),
                super::ERROR_DETAIL_LIMIT,
            ))
            .with_hint(spec.git_flags())
            .with_retryable(true),
    )
}

/// 放弃工作区修改（T1.4"放弃"操作）。
///
/// 两组路径语义不同：
/// - `tracked`：`git restore --worktree --`（工作区 ← 索引；已暂存内容保留）；
/// - `untracked`：直接删除磁盘文件（**不可恢复**，调用方必须先经确认对话框），
///   删除后顺手清理变空的父目录（直到仓库根为止），否则树视图里会留下
///   一串空目录骨架。
pub(super) fn discard_worktree(
    engine: &CliGitEngine,
    repo: &RepoId,
    spec: &DiscardSpec,
) -> AppResult<()> {
    if !spec.tracked.is_empty() {
        engine.run_write(repo, args::discard_args(&spec.tracked)?)?;
    }

    let root = repo.root();
    for path in &spec.untracked {
        let absolute = root.join(path.to_string_lossy().as_ref());
        // NotFound 视同成功：调用方的状态数据可能已过期，"目标已经不存在"
        // 恰好是用户想要的结果，报错反而让批量操作中断
        match std::fs::remove_file(&absolute) {
            Ok(()) => remove_empty_parents(root, &absolute),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(AppError::new(
                    ErrorCode::Internal,
                    "failed to delete the untracked file",
                )
                .with_detail(format!("{}: {error}", absolute.display())));
            }
        }
    }

    Ok(())
}

/// 删除文件后把变空的父目录一路清到仓库根为止。
fn remove_empty_parents(root: &Path, file: &Path) {
    let mut dir = file.parent();
    while let Some(current) = dir {
        if current == root {
            return;
        }
        // 只清"这个文件所在的分支"，重命名/斜杠路径之外的情况随 remove_dir 失败自然终止
        if std::fs::remove_dir(current).is_err() {
            return;
        }
        dir = current.parent();
    }
}

/// 提交，返回新提交的 oid。
///
/// amend 有两种语义（见 [`AmendMode`]）：把暂存区并进去，还是只换提交信息。
/// 后者走一条独立路径——它必须在一个**隔离索引**上执行，理由见
/// [`commit_amending_message_only`]。
pub(super) fn commit(engine: &CliGitEngine, repo: &RepoId, spec: &CommitSpec) -> AppResult<String> {
    if spec.amend && spec.amend_mode == AmendMode::MessageOnly {
        return commit_amending_message_only(engine, repo, spec);
    }

    engine.run_write(repo, args::commit_args(spec)?)?;
    read_head_oid(engine, repo)
}

/// 提交之后取回新提交的 oid。
fn read_head_oid(engine: &CliGitEngine, repo: &RepoId) -> AppResult<String> {
    let output = engine.run_read(
        repo,
        GitInvocation::new(vec!["rev-parse".to_owned(), "HEAD".to_owned()]),
    )?;
    let oid = output.stdout_lossy().trim().to_owned();
    if oid.is_empty() {
        return Err(AppError::new(
            ErrorCode::Internal,
            "git commit succeeded but HEAD could not be resolved",
        ));
    }
    Ok(oid)
}

/// "只改提交信息"的 amend。
///
/// # 为什么必须用隔离索引
///
/// `git commit --amend` 提交的是**当前索引**。所以"只改信息"要实现成
/// "索引内容 = HEAD 的树"，否则用户以为自己在改一个错别字，实际把暂存区里的
/// 三个文件一起提交了——这是本项目里最容易被误解、代价也最高的一类动作。
///
/// 做法是在临时索引上跑三步：
///
/// 1. `git read-tree HEAD` 把它读成 HEAD 的树；
/// 2. **校验**它确实等于 HEAD 的树。理由：read-tree 失败而 commit 继续，
///    产出的会是一棵空树的提交（内容被清空），那比报错糟糕得多。
///    多一次 git 调用换这道闸，值得；
/// 3. 在该索引上执行 `git commit --amend`。
///
/// 用户的真实索引全程没有被读写：`GIT_INDEX_FILE` 指向临时文件，git 的提交只看它。
/// 临时文件由 [`TempIndex`] 保证在提前返回时也被删掉。
fn commit_amending_message_only(
    engine: &CliGitEngine,
    repo: &RepoId,
    spec: &CommitSpec,
) -> AppResult<String> {
    if !spec.paths.is_empty() {
        // 路径限制决定"提交哪些内容"，与"内容不进提交"直接冲突：
        // 两者同时给出时，任何一个结果都会让另一方变成谎话
        return Err(AppError::new(
            ErrorCode::Validation,
            "amend with an explicit path list cannot leave the index untouched",
        )
        .with_hint("--only"));
    }

    let head_tree = read::head_tree(engine, repo)?.ok_or_else(|| {
        AppError::new(
            ErrorCode::Validation,
            "there is no commit to amend (HEAD is unborn)",
        )
    })?;

    let index = TempIndex::create();

    engine.run_write(
        repo,
        args::read_tree_args("HEAD").with_index_file(index.path()),
    )?;

    let isolated_tree = engine
        .run_read(repo, args::write_tree_args().with_index_file(index.path()))?
        .stdout_lossy()
        .trim()
        .to_owned();
    if isolated_tree != head_tree {
        return Err(AppError::new(
            ErrorCode::Internal,
            "the isolated index does not match HEAD; refusing to amend",
        )
        .with_detail(format!(
            "expected tree {head_tree}, isolated index holds {isolated_tree}"
        )));
    }

    engine.run_write(repo, args::commit_args(spec)?.with_index_file(index.path()))?;

    read_head_oid(engine, repo)
}

/// 隔离索引文件的清理守卫。
///
/// 用 `Drop` 而不是在函数末尾手动删：中间任何一步 `?` 提前返回都不该留下垃圾。
/// 刻意放系统临时目录而不是仓库里——`.git` 是用户的目录，留下一个像索引的文件
/// （`.git/index-xyz` 之类）比留下一个明显的临时文件危险得多：
/// 用户与工具都可能把它当成真实索引。
struct TempIndex(PathBuf);

impl TempIndex {
    fn create() -> Self {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let path =
            std::env::temp_dir().join(format!("forgedesk-index-{}-{unique}", std::process::id()));
        // `read-tree` 要求索引文件缺失或合法；上一次异常退出可能留下同名文件
        // （进程号与纳秒双重唯一，概率极低，但"删干净再开始"比"碰运气"便宜）
        let _ = std::fs::remove_file(&path);
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempIndex {
    fn drop(&mut self) {
        // git 失败路径上可能留下 `<index>.lock`
        let _ = std::fs::remove_file(self.0.with_extension("lock"));
        if let Err(error) = std::fs::remove_file(&self.0) {
            // 删不掉不是本次操作的失败原因（系统会清临时目录），但要留痕
            tracing::debug!(path = %self.0.display(), %error, "临时索引未能删除");
        }
    }
}

/// 重置。
pub(super) fn reset(engine: &CliGitEngine, repo: &RepoId, spec: &ResetSpec) -> AppResult<()> {
    engine.run_write(repo, args::reset_args(spec)?)?;
    Ok(())
}

/// 切换分支 / 提交。
pub(super) fn checkout(engine: &CliGitEngine, repo: &RepoId, spec: &CheckoutSpec) -> AppResult<()> {
    engine.run_write(repo, args::checkout_args(spec)?)?;
    Ok(())
}

/// 合并。
pub(super) fn merge(
    engine: &CliGitEngine,
    repo: &RepoId,
    spec: &MergeSpec,
) -> AppResult<MergeOutcome> {
    let output = engine.run_at(
        repo.root(),
        GitInvocation::new(args::merge_args(spec)),
        RunKind::Write,
    )?;
    outcome_from(engine, repo, &output, &spec.revision, MergeMessage::Merge)
}

/// 拣选提交。
pub(super) fn cherry_pick(
    engine: &CliGitEngine,
    repo: &RepoId,
    revision: &str,
) -> AppResult<MergeOutcome> {
    let output = engine.run_at(
        repo.root(),
        GitInvocation::new(args::cherry_pick_args(revision)),
        RunKind::Write,
    )?;
    outcome_from(engine, repo, &output, revision, MergeMessage::CherryPick)
}

/// 反转提交。
pub(super) fn revert(
    engine: &CliGitEngine,
    repo: &RepoId,
    revision: &str,
) -> AppResult<MergeOutcome> {
    let output = engine.run_at(
        repo.root(),
        GitInvocation::new(args::revert_args(revision)),
        RunKind::Write,
    )?;
    outcome_from(engine, repo, &output, revision, MergeMessage::Revert)
}

/// 冲突时给用户的提示前缀（写进 `detail`，不是用户可见文案）。
enum MergeMessage {
    Merge,
    CherryPick,
    Revert,
}

/// 把 merge / cherry-pick / revert 的输出归一化成 [`MergeOutcome`]。
///
/// 三者语义一致：可能无事可做、可能快进、可能产生提交、可能停在冲突状态。
fn outcome_from(
    engine: &CliGitEngine,
    repo: &RepoId,
    output: &GitOutput,
    revision: &str,
    kind: MergeMessage,
) -> AppResult<MergeOutcome> {
    let conflicts = unmerged_paths(engine, repo)?;
    if !conflicts.is_empty() {
        return Ok(MergeOutcome {
            kind: MergeKind::Conflicted,
            oid: None,
            conflicts,
        });
    }

    if !output.success() {
        // 没有冲突却失败：这是真的错误（例如 ff-only 无法快进）
        let stderr = output.stderr_lossy();
        let code = ErrorCode::classify(&stderr);
        let operation = match kind {
            MergeMessage::Merge => "merge",
            MergeMessage::CherryPick => "cherry-pick",
            MergeMessage::Revert => "revert",
        };
        return Err(AppError::new(code, format!("{operation} failed"))
            .with_detail(sanitize_log(&stderr))
            .with_hint(revision.to_owned()));
    }

    let head = head_oid(engine, repo)?;
    let target = rev_parse(engine, repo, revision)?;
    let text = output.stdout_lossy();
    let already_up_to_date =
        text.contains("Already up to date") || text.contains("Already up-to-date");

    let kind = if already_up_to_date {
        MergeKind::AlreadyUpToDate
    } else if head.is_some() && head == target {
        // HEAD 直接落到了被合并的提交上 → 没有产生新提交，是快进
        MergeKind::FastForward
    } else {
        MergeKind::MergeCommit
    };

    Ok(MergeOutcome {
        kind,
        oid: head,
        conflicts: Vec::new(),
    })
}

/// stash 操作。
pub(super) fn stash(engine: &CliGitEngine, repo: &RepoId, spec: &StashSpec) -> AppResult<()> {
    engine.run_write(repo, GitInvocation::new(args::stash_args(spec)))?;
    Ok(())
}

/// 拉取远端引用。
pub(super) fn fetch(
    engine: &CliGitEngine,
    repo: &RepoId,
    spec: &FetchSpec,
    progress: &ProgressSink,
) -> AppResult<FetchOutcome> {
    let output = engine.run_network(
        repo.root(),
        GitInvocation::new(args::fetch_args(spec)),
        progress,
    )?;
    Ok(FetchOutcome {
        remote: spec.remote.clone().unwrap_or_else(|| "origin".to_owned()),
        updates: parse_ref_updates(&output.stderr_lossy()),
    })
}

/// 拉取并合并 / 变基。
pub(super) fn pull(
    engine: &CliGitEngine,
    repo: &RepoId,
    spec: &PullSpec,
    progress: &ProgressSink,
) -> AppResult<PullOutcome> {
    let output = engine.run_network(
        repo.root(),
        GitInvocation::new(args::pull_args(spec)),
        progress,
    )?;
    let stderr = output.stderr_lossy();
    let fetch = FetchOutcome {
        remote: spec.remote.clone().unwrap_or_else(|| "origin".to_owned()),
        updates: parse_ref_updates(&stderr),
    };

    let conflicts = unmerged_paths(engine, repo)?;
    if !conflicts.is_empty() {
        return Ok(PullOutcome {
            fetch,
            strategy: spec.strategy,
            up_to_date: false,
            merge: Some(MergeOutcome {
                kind: MergeKind::Conflicted,
                oid: None,
                conflicts,
            }),
        });
    }

    if !output.success() {
        let code = ErrorCode::classify(&stderr);
        return Err(AppError::new(code, "git pull failed").with_detail(sanitize_log(&stderr)));
    }

    let up_to_date = output.stdout_lossy().contains("Already up to date")
        || output.stdout_lossy().contains("Already up-to-date");

    Ok(PullOutcome {
        fetch,
        strategy: spec.strategy,
        up_to_date,
        merge: if up_to_date {
            None
        } else {
            Some(MergeOutcome {
                kind: MergeKind::FastForward,
                oid: head_oid(engine, repo)?,
                conflicts: Vec::new(),
            })
        },
    })
}

/// 推送。
///
/// 被拒绝时**返回 `Ok`**（`rejections` 非空），而不是 `Err`：
/// 界面需要拿到"哪个引用、为什么被拒、是不是非快进"来决定给用户哪些选项
/// （M2 验收：非快进必须提供 `force-with-lease` / 先 pull / 取消三条路）。
/// 真正的错误（网络不可达、认证失败）仍然返回 `Err`。
pub(super) fn push(
    engine: &CliGitEngine,
    repo: &RepoId,
    spec: &PushSpec,
    progress: &ProgressSink,
) -> AppResult<PushOutcome> {
    let output = engine.run_network(
        repo.root(),
        GitInvocation::new(args::push_args(spec)),
        progress,
    )?;
    let stderr = output.stderr_lossy();
    let updates = parse_ref_updates(&stderr);
    let rejections = parse_rejections(&stderr);

    if !output.success() && rejections.is_empty() {
        let code = ErrorCode::classify(&stderr);
        return Err(AppError::new(code, "git push failed").with_detail(sanitize_log(&stderr)));
    }

    Ok(PushOutcome {
        remote: spec.remote.clone().unwrap_or_else(|| "origin".to_owned()),
        updates,
        rejections,
    })
}

// ---------------------------------------------------------------- 辅助

/// 未解决冲突的路径（去重）。
fn unmerged_paths(
    engine: &CliGitEngine,
    repo: &RepoId,
) -> AppResult<Vec<forgedesk_domain::git::RepoPath>> {
    let output = engine.run_read(
        repo,
        GitInvocation::new(vec![
            "ls-files".to_owned(),
            "-u".to_owned(),
            "-z".to_owned(),
        ]),
    )?;

    let mut paths: Vec<forgedesk_domain::git::RepoPath> = Vec::new();
    for entry in parse_ls_files_stage(&output.stdout) {
        if !paths.contains(&entry.path) {
            paths.push(entry.path);
        }
    }
    Ok(paths)
}

/// 当前 HEAD 的 oid；空仓库返回 `None`。
fn head_oid(engine: &CliGitEngine, repo: &RepoId) -> AppResult<Option<String>> {
    read::head_oid(engine, repo)
}

/// 解析一个引用为 oid；无法解析时返回 `None`。
fn rev_parse(engine: &CliGitEngine, repo: &RepoId, revision: &str) -> AppResult<Option<String>> {
    read::rev_parse(engine, repo, revision)
}

/// 解析 git 在 stderr 上打印的引用更新行。
///
/// 形如：
///
/// ```text
///    a1b2c3..d4e5f6  main -> origin/main
///  * [new branch]      feature -> origin/feature
///  - [deleted]         (none) -> origin/gone
///  = [up to date]      main -> origin/main
///  ! [rejected]        main -> main (non-fast-forward)
/// ```
pub(crate) fn parse_ref_updates(stderr: &str) -> Vec<RefUpdate> {
    let mut out = Vec::new();
    for line in stderr.lines() {
        let trimmed = line.trim();
        let (kind, rest) = if let Some(rest) = trimmed.strip_prefix("* [new branch]") {
            (RefUpdateKind::New, rest)
        } else if let Some(rest) = trimmed.strip_prefix("* [new tag]") {
            (RefUpdateKind::New, rest)
        } else if let Some(rest) = trimmed.strip_prefix("- [deleted]") {
            (RefUpdateKind::Deleted, rest)
        } else if let Some(rest) = trimmed.strip_prefix("= [up to date]") {
            (RefUpdateKind::UpToDate, rest)
        } else if let Some(rest) = trimmed.strip_prefix("! [rejected]") {
            (RefUpdateKind::Rejected, rest)
        } else if let Some(rest) = trimmed.strip_prefix("! [remote rejected]") {
            (RefUpdateKind::Rejected, rest)
        } else if trimmed.contains("..") && trimmed.contains("->") {
            (RefUpdateKind::Updated, trimmed)
        } else {
            continue;
        };

        let Some((oids, names)) = rest.split_once(|c: char| c.is_whitespace()) else {
            continue;
        };
        let Some((_, to)) = names.split_once("->") else {
            continue;
        };
        let (old_oid, new_oid) = oids.split_once("..").map_or((None, None), |(old, new)| {
            (Some(old.trim().to_owned()), Some(new.trim().to_owned()))
        });

        out.push(RefUpdate {
            name: to.trim().to_owned(),
            old_oid: old_oid.filter(|oid| oid != "(none)"),
            new_oid: new_oid.filter(|oid| oid != "(none)"),
            kind,
            reason: None,
        });
    }
    out
}

/// 解析被拒绝的引用。
///
/// 只把"非快进"标成 `non_fast_forward`：权限不足或 hook 拒绝时给
/// `force-with-lease` 选项只会误导用户（强推同样会被拒）。
pub(crate) fn parse_rejections(stderr: &str) -> Vec<PushRejection> {
    let mut out = Vec::new();
    for line in stderr.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed
            .strip_prefix("! [rejected]")
            .or_else(|| trimmed.strip_prefix("! [remote rejected]"))
        else {
            continue;
        };
        let Some((_, names)) = rest.split_once(|c: char| c.is_whitespace()) else {
            continue;
        };
        let Some((_, after_arrow)) = names.split_once("->") else {
            continue;
        };
        let after_arrow = after_arrow.trim();
        let (target, reason) = match after_arrow.split_once('(') {
            Some((name, rest)) => (name.trim(), rest.trim_end_matches(')').trim()),
            None => (after_arrow, ""),
        };

        out.push(PushRejection {
            name: target.to_owned(),
            reason: reason.to_owned(),
            non_fast_forward: reason.contains("non-fast-forward")
                || reason.contains("fetch first")
                || reason.contains("stale info"),
        });
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{parse_ref_updates, parse_rejections};
    use forgedesk_domain::git::RefUpdateKind;

    #[test]
    fn fetch_style_updates_are_parsed_with_their_oids() {
        let stderr = "From https://example.com/r\n   a1b2c3d..e4f5a6b  main       -> origin/main\n * [new branch]      feature    -> origin/feature\n - [deleted]         (none)     -> origin/gone\n = [up to date]      dev        -> origin/dev\n";

        let updates = parse_ref_updates(stderr);

        assert_eq!(updates.len(), 4);
        assert_eq!(updates[0].kind, RefUpdateKind::Updated);
        assert_eq!(updates[0].name, "origin/main");
        assert_eq!(updates[0].old_oid.as_deref(), Some("a1b2c3d"));
        assert_eq!(updates[0].new_oid.as_deref(), Some("e4f5a6b"));
        assert_eq!(updates[1].kind, RefUpdateKind::New);
        assert_eq!(updates[2].kind, RefUpdateKind::Deleted);
        assert_eq!(updates[2].new_oid, None, "(none) 不是 oid");
        assert_eq!(updates[3].kind, RefUpdateKind::UpToDate);
    }

    #[test]
    fn unrelated_stderr_lines_are_ignored() {
        let stderr =
            "Enumerating objects: 5, done.\nfatal: could not read from remote repository\n";

        assert!(parse_ref_updates(stderr).is_empty());
    }

    #[test]
    fn non_fast_forward_rejection_is_flagged_for_the_ui() {
        let stderr = " ! [rejected]        main -> main (non-fast-forward)\n";

        let rejections = parse_rejections(stderr);

        assert_eq!(rejections.len(), 1);
        assert_eq!(rejections[0].name, "main");
        assert!(rejections[0].non_fast_forward);
    }

    #[test]
    fn stale_info_rejection_is_also_treated_as_non_fast_forward() {
        // force-with-lease 的语义就是"远端变了就拒绝"，属于同一类可选修复
        let stderr = " ! [rejected]        main -> main (stale info)\n";

        let rejections = parse_rejections(stderr);

        assert!(rejections[0].non_fast_forward);
    }

    #[test]
    fn hook_rejection_is_not_offered_a_force_option() {
        let stderr = " ! [remote rejected] main -> main (pre-receive hook declined)\n";

        let rejections = parse_rejections(stderr);

        assert_eq!(rejections.len(), 1);
        assert!(
            !rejections[0].non_fast_forward,
            "hook 拒绝时提供强推只会误导用户"
        );
        assert_eq!(rejections[0].reason, "pre-receive hook declined");
    }

    #[test]
    fn malformed_rejection_lines_are_skipped() {
        assert!(parse_rejections(" ! [rejected]\n").is_empty());
        assert!(parse_rejections("").is_empty());
    }
}
