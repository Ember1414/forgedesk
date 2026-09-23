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
    CheckoutSpec, CloneSpec, CommitSpec, FetchOutcome, FetchSpec, InitSpec, MergeKind,
    MergeOutcome, MergeSpec, PullOutcome, PullSpec, PushOutcome, PushRejection, PushSpec,
    RefUpdate, RefUpdateKind, RepoId, RepositoryInfo, ResetSpec, StageSpec, StashSpec,
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

/// 提交，返回新提交的 oid。
pub(super) fn commit(engine: &CliGitEngine, repo: &RepoId, spec: &CommitSpec) -> AppResult<String> {
    engine.run_write(repo, args::commit_args(spec)?)?;

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
    rev_parse(engine, repo, "HEAD")
}

/// 解析一个引用为 oid；无法解析时返回 `None`。
fn rev_parse(engine: &CliGitEngine, repo: &RepoId, revision: &str) -> AppResult<Option<String>> {
    let output = engine.run_at(
        repo.root(),
        GitInvocation::new(vec![
            "rev-parse".to_owned(),
            "--verify".to_owned(),
            "--quiet".to_owned(),
            revision.to_owned(),
        ]),
        RunKind::Read,
    )?;
    if !output.success() {
        return Ok(None);
    }
    let oid = output.stdout_lossy().trim().to_owned();
    Ok(if oid.is_empty() { None } else { Some(oid) })
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
