//! 冲突状态采集与 continue / abort / skip（T3.1）。
//!
//! # 数据源红线（T3.1 任务书第 1 条）
//!
//! 冲突信息必须来自 index stage：文件清单用 `git ls-files -u -z`，blob 内容按
//! **oid** 用 `git cat-file` 读取。刻意不走 `:N:<path>` 的 stage 语法：
//! `-z` 输出的是原始字节路径，可能不是合法 UTF-8，拼进 stage 语法要处理
//! 引号与编码转义；而 oid 是解析 `ls-files -u` 时就有的，用它完全绕开
//! 路径转义这一类问题。
//!
//! # 为什么本模块全部走 CLI（能力边界）
//!
//! stage 三方内容 + 2 MiB 阈值 + 二进制判定这组语义以 git CLI 为准；
//! libgit2 侧返回 [`ErrorCode::UnsupportedByEngine`]（见
//! `docs/GIT-ENGINE-DIFF.md` §4 的能力边界表）。冲突查询是低频的、
//! 用户发起的操作，"一次进程的代价"在这里不构成问题。

use std::collections::HashMap;

use forgedesk_domain::git::{
    compute_merge_blocks, ConflictAbortOutcome, ConflictContinueOutcome, ConflictFile,
    ConflictFileDetail, ConflictKind, ConflictOpKind, ConflictState, FileBlob, LineEnding,
    OperationState, RepoId, RepoPath, StageEntry, StageSpec, TakeSide, UnmergedStage,
    MAX_CONFLICT_BLOB_BYTES,
};
use forgedesk_domain::{AppError, AppResult, ErrorCode};

use super::args::GitInvocation;
use super::read::{head_branch, head_oid};
use super::write::{stage, unmerged_paths};
use super::{CliGitEngine, RunKind};
use crate::engine::enrich::{detect_operation_with_details, resolve_git_dir};
use crate::parsers::ls_files::parse_ls_files_stage;

/// 二进制嗅探读取的字节数（与 git 的启发式一致：含 NUL 即按二进制处理）。
const BINARY_SNIFF_BYTES: usize = 8192;

/// 采集冲突状态（T3.1 的 `git_conflict_state` 数据源）。
pub(super) fn conflict_state(engine: &CliGitEngine, repo: &RepoId) -> AppResult<ConflictState> {
    let workdir = repo.root();
    let git_dir = resolve_git_dir(workdir);
    let detection = detect_operation_with_details(&git_dir);

    let op_kind = match detection.state {
        OperationState::Merge => Some(ConflictOpKind::Merge),
        OperationState::Rebase => Some(ConflictOpKind::Rebase),
        OperationState::CherryPick => Some(ConflictOpKind::CherryPick),
        OperationState::Revert => Some(ConflictOpKind::Revert),
        // 二分不是本状态机的对象（任务书的 op_kind 枚举只有四种）
        OperationState::None | OperationState::Bisect => None,
    };

    let groups = grouped_stages(engine, repo)?;
    let mut files = Vec::with_capacity(groups.len());
    if !groups.is_empty() {
        // 一次 `cat-file --batch-check` 查齐全部 blob 大小，避免逐个起进程
        let oids: Vec<&str> = groups
            .iter()
            .flat_map(|(_, slots)| slots.iter().flatten().map(|stage| stage.oid.as_str()))
            .collect();
        let sizes = blob_sizes(engine, repo, &oids)?;

        for (path, slots) in groups {
            let base = stage_blob(engine, repo, slots[0].as_ref(), &sizes)?;
            let ours = stage_blob(engine, repo, slots[1].as_ref(), &sizes)?;
            let theirs = stage_blob(engine, repo, slots[2].as_ref(), &sizes)?;
            let kind = ConflictKind::classify(base.as_ref(), ours.as_ref(), theirs.as_ref());
            // 与 enrich_filesystem 同一约定：非 UTF-8 路径在 Windows 上本来
            // 就打不开，lossy join 是两条路径的一致行为
            let worktree_exists =
                std::fs::metadata(workdir.join(path.to_string_lossy().as_ref())).is_ok();
            files.push(ConflictFile {
                path,
                kind,
                base,
                ours,
                theirs,
                worktree_exists,
            });
        }
    }

    // "并入的分支"：merge / cherry-pick / revert 把变更并进当前分支；
    // rebase 的 HEAD 在重放位置上，当前分支名不是用户心智里的目标分支
    let into_branch = match op_kind {
        Some(ConflictOpKind::Merge | ConflictOpKind::CherryPick | ConflictOpKind::Revert) => {
            head_branch(engine, repo)?
        }
        _ => None,
    };

    let mut state = ConflictState {
        op_kind,
        op_in_progress: false,
        current_step: detection.current_step,
        total_steps: detection.total_steps,
        head_name: detection.head_name,
        into_branch,
        files,
        can_continue: false,
        can_abort: false,
        can_skip: false,
    };
    state.derive_flags();
    Ok(state)
}

/// 把 `ls-files -u` 的扁平 stage 记录按路径聚合成 `(路径, [stage1/2/3])`。
fn grouped_stages(
    engine: &CliGitEngine,
    repo: &RepoId,
) -> AppResult<Vec<(RepoPath, [Option<StageEntry>; 3])>> {
    let output = engine.run_read(
        repo,
        GitInvocation::new(vec![
            "ls-files".to_owned(),
            "-u".to_owned(),
            "-z".to_owned(),
        ]),
    )?;

    let mut order: Vec<RepoPath> = Vec::new();
    let mut groups: Vec<[Option<StageEntry>; 3]> = Vec::new();
    for entry in parse_ls_files_stage(&output.stdout) {
        let index = match order.iter().position(|existing| *existing == entry.path) {
            Some(index) => index,
            None => {
                order.push(entry.path.clone());
                groups.push([None, None, None]);
                groups.len() - 1
            }
        };
        let slot = match entry.stage {
            UnmergedStage::Base => 0,
            UnmergedStage::Ours => 1,
            UnmergedStage::Theirs => 2,
        };
        groups[index][slot] = Some(StageEntry {
            mode: entry.mode,
            oid: entry.oid,
        });
    }
    Ok(order.into_iter().zip(groups).collect())
}

/// 一次 `cat-file --batch-check` 查齐全部 blob 大小（oid → 字节数）。
///
/// 为什么先查大小再读内容：冲突文件可能是几百 MB 的设计稿——`git show`
/// 会把它整个读进内存，而 `cat-file --batch-check` 只回一行数字。
fn blob_sizes(
    engine: &CliGitEngine,
    repo: &RepoId,
    oids: &[&str],
) -> AppResult<HashMap<String, u64>> {
    if oids.is_empty() {
        return Ok(HashMap::new());
    }
    let mut stdin = Vec::with_capacity(oids.iter().map(|oid| oid.len() + 1).sum::<usize>());
    for oid in oids {
        stdin.extend_from_slice(oid.as_bytes());
        stdin.push(b'\n');
    }
    let output = engine.run_read(
        repo,
        GitInvocation::new(vec!["cat-file".to_owned(), "--batch-check".to_owned()])
            .with_stdin(stdin),
    )?;

    let mut sizes = HashMap::with_capacity(oids.len());
    for line in output.stdout_lossy().lines() {
        let mut fields = line.split_whitespace();
        let (oid, kind, size) = (fields.next(), fields.next(), fields.next());
        if let (Some(oid), Some("blob"), Some(size)) = (oid, kind, size) {
            if let Ok(size) = size.parse() {
                sizes.insert(oid.to_owned(), size);
            }
        }
        // `missing` 与畸形行：跳过，调用方按"大小未知"处理
    }
    Ok(sizes)
}

/// 读取一个 stage 的 blob（超过 2 MiB 或非 UTF-8 时 `content` 为 `None`）。
fn stage_blob(
    engine: &CliGitEngine,
    repo: &RepoId,
    stage: Option<&StageEntry>,
    sizes: &HashMap<String, u64>,
) -> AppResult<Option<FileBlob>> {
    let Some(stage) = stage else {
        return Ok(None);
    };
    let size = sizes.get(&stage.oid).copied().unwrap_or(0);

    // 超限：不读内容（size 仍然有效）。is_binary 未知按 false 处理——
    // 前端以 content 为 None 判断"内容不可显示"，is_binary 只影响类别标签
    if size > MAX_CONFLICT_BLOB_BYTES {
        return Ok(Some(FileBlob {
            size,
            is_binary: false,
            encoding_hint: None,
            content: None,
        }));
    }

    let output = engine.run_read(
        repo,
        GitInvocation::new(vec![
            "cat-file".to_owned(),
            "blob".to_owned(),
            stage.oid.clone(),
        ]),
    )?;
    let bytes = output.stdout;
    // size 与内容之间可能被外部改动（竞态防御）：仍超限就按超限处理
    if bytes.len() as u64 > MAX_CONFLICT_BLOB_BYTES {
        return Ok(Some(FileBlob {
            size: bytes.len() as u64,
            is_binary: false,
            encoding_hint: None,
            content: None,
        }));
    }

    let is_binary = bytes[..bytes.len().min(BINARY_SNIFF_BYTES)].contains(&0);
    // 非 UTF-8 的文本（GBK 等）：encoding_hint 不猜测，内容也不 lossy——
    // lossy 过的字符串一旦被写回文件就是数据损坏
    let (encoding_hint, content) = match String::from_utf8(bytes) {
        Ok(text) => (Some("utf-8".to_owned()), Some(text)),
        Err(_) => (None, None),
    };
    Ok(Some(FileBlob {
        size,
        is_binary,
        encoding_hint,
        content,
    }))
}

/// continue / skip 的动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SequenceAction {
    Continue,
    Skip,
}

/// 继续进行中的操作（解决完冲突后调用）。
///
/// 返回值区分两种正常结局：**完成**（`oid` = 完成后的 HEAD）与**又停在冲突**
/// （rebase / cherry-pick 序列重放下一个提交时撞新的冲突，`conflicts` 非空）。
/// 后者不是错误——调用方应该刷新冲突状态而不是弹红框。
pub(super) fn continue_operation(
    engine: &CliGitEngine,
    repo: &RepoId,
    op: ConflictOpKind,
) -> AppResult<ConflictContinueOutcome> {
    sequence_command(engine, repo, op, SequenceAction::Continue)
}

/// 跳过当前提交（只有 rebase 支持）。
pub(super) fn skip_operation(
    engine: &CliGitEngine,
    repo: &RepoId,
    op: ConflictOpKind,
) -> AppResult<ConflictContinueOutcome> {
    sequence_command(engine, repo, op, SequenceAction::Skip)
}

fn sequence_command(
    engine: &CliGitEngine,
    repo: &RepoId,
    op: ConflictOpKind,
    action: SequenceAction,
) -> AppResult<ConflictContinueOutcome> {
    let args = match (op, action) {
        // merge 的 continue 是"提交合并"：MERGE_MSG 已由 git 写好，--no-edit 直接采用
        (ConflictOpKind::Merge, SequenceAction::Continue) => {
            vec!["commit".to_owned(), "--no-edit".to_owned()]
        }
        (ConflictOpKind::Rebase, SequenceAction::Continue) => {
            vec!["rebase".to_owned(), "--continue".to_owned()]
        }
        (ConflictOpKind::Rebase, SequenceAction::Skip) => {
            vec!["rebase".to_owned(), "--skip".to_owned()]
        }
        (ConflictOpKind::CherryPick, SequenceAction::Continue) => {
            vec!["cherry-pick".to_owned(), "--continue".to_owned()]
        }
        (ConflictOpKind::Revert, SequenceAction::Continue) => {
            vec!["revert".to_owned(), "--continue".to_owned()]
        }
        (
            ConflictOpKind::Merge | ConflictOpKind::CherryPick | ConflictOpKind::Revert,
            SequenceAction::Skip,
        ) => {
            return Err(AppError::new(
                ErrorCode::Validation,
                "skip is only available during a rebase",
            ));
        }
    };

    // GIT_EDITOR=true：continue 可能触发提交信息编辑器（合并提交 / rebase 的
    // reword 步骤）。它必须覆盖用户 shell 继承的 GIT_EDITOR（环境变量的优先级
    // 高于 core.editor 配置）——true 让编辑器"立即以原内容保存退出"，这是
    // 无头执行不被挂死的关键（行为由 services 的集成测试锁定）。
    let invocation = GitInvocation::new(args).with_env("GIT_EDITOR", "true");

    // 刻意不断言退出码：再次停在冲突上时 git 以非零退出，那是**正常结局**
    // （与 push 被拒绝同一取舍）。完成与否由执行后的 index 状态判定。
    let output = engine.run_at(repo.root(), invocation, RunKind::Write)?;

    let conflicts = unmerged_paths(engine, repo)?;
    if !conflicts.is_empty() {
        return Ok(ConflictContinueOutcome {
            oid: None,
            conflicts,
        });
    }
    if !output.success() {
        let stderr = output.stderr_lossy();
        let code = ErrorCode::classify(&stderr);
        return Err(
            AppError::new(code, "git failed to continue the in-progress operation")
                .with_detail(forgedesk_diagnostics::sanitize_log(&stderr)),
        );
    }
    Ok(ConflictContinueOutcome {
        oid: head_oid(engine, repo)?,
        conflicts: Vec::new(),
    })
}

/// 中止进行中的操作（`<op> --abort`）。
///
/// 快照与"回到操作前状态"的校验在服务层编排：快照要打在 abort **之前**，
/// 而校验需要对比 abort 前后两次 HEAD / 分支名（见 `services::conflict`）。
pub(super) fn abort_operation(
    engine: &CliGitEngine,
    repo: &RepoId,
    op: ConflictOpKind,
) -> AppResult<ConflictAbortOutcome> {
    let args: &[&str] = match op {
        ConflictOpKind::Merge => &["merge", "--abort"],
        ConflictOpKind::Rebase => &["rebase", "--abort"],
        ConflictOpKind::CherryPick => &["cherry-pick", "--abort"],
        ConflictOpKind::Revert => &["revert", "--abort"],
    };
    engine.run_write(
        repo,
        GitInvocation::new(args.iter().map(|arg| (*arg).to_owned()).collect()),
    )?;
    Ok(ConflictAbortOutcome {
        head_oid: head_oid(engine, repo)?,
        head_ref: head_branch(engine, repo)?,
        snapshot_id: None,
    })
}

/// 标记文件已解决：`git add` + 校验这些路径的 stage 条目已清空（T3.1 任务书第 4 条）。
///
/// 校验是**必须**的：`git add` 成功不等于冲突已解决（例如路径拼错时 git 会
/// 静默添加别的文件、或者删除类冲突需要的是 `git add` 已删除路径——虽然本
/// 实现支持）。校验不过就报 `CONFLICT_UNRESOLVED` 并列出仍然冲突的路径，
/// 让调用方（服务层）把"哪些没解决"带还给界面。
///
/// 路径以 `RepoPath` 传入：IPC 边界上非 UTF-8 路径已是 lossy 形状（见
/// `RepoPath` 的模块说明），`git add` 对不匹配的 pathspec 会报错退出，
/// 不会静默误伤其他文件。
pub(super) fn mark_resolved(
    engine: &CliGitEngine,
    repo: &RepoId,
    paths: &[RepoPath],
) -> AppResult<()> {
    if paths.is_empty() {
        return Err(AppError::new(
            ErrorCode::Validation,
            "no paths were given to mark resolved",
        ));
    }
    stage(engine, repo, &StageSpec::Paths(paths.to_vec()))?;

    let unresolved: Vec<String> = unmerged_paths(engine, repo)?
        .iter()
        .filter(|remaining| paths.contains(remaining))
        .map(|remaining| remaining.to_string_lossy().into_owned())
        .collect();
    if unresolved.is_empty() {
        return Ok(());
    }
    Err(AppError::new(
        ErrorCode::ConflictUnresolved,
        "the paths are still conflicted after staging",
    )
    .with_hint(unresolved.join(", ")))
}

/// 读取单个冲突文件的三方内容、工作区文件形状与 diff3 合并块（T3.2）。
///
/// 路径以 pathspec 传给 `git ls-files -u`：编辑器只对用户**正在看的**文件
/// 做三方读取与块计算（state 清单页不需要），路径来自该清单。
pub(super) fn file_detail(
    engine: &CliGitEngine,
    repo: &RepoId,
    path: &RepoPath,
) -> AppResult<ConflictFileDetail> {
    let workdir = repo.root();
    let output = engine.run_read(
        repo,
        GitInvocation::new(vec![
            "ls-files".to_owned(),
            "-u".to_owned(),
            "-z".to_owned(),
            "--".to_owned(),
            path.to_string_lossy().into_owned(),
        ]),
    )?;
    let entries = parse_ls_files_stage(&output.stdout);
    if entries.is_empty() {
        return Err(AppError::new(
            ErrorCode::Validation,
            "the path has no unmerged stages (it is not conflicted)",
        )
        .with_hint(path.to_string_lossy().into_owned()));
    }

    let mut slots: [Option<StageEntry>; 3] = [None, None, None];
    for entry in &entries {
        let slot = match entry.stage {
            UnmergedStage::Base => 0,
            UnmergedStage::Ours => 1,
            UnmergedStage::Theirs => 2,
        };
        slots[slot] = Some(StageEntry {
            mode: entry.mode,
            oid: entry.oid.clone(),
        });
    }

    let oids: Vec<&str> = slots
        .iter()
        .flatten()
        .map(|stage| stage.oid.as_str())
        .collect();
    let sizes = blob_sizes(engine, repo, &oids)?;
    let base = stage_blob(engine, repo, slots[0].as_ref(), &sizes)?;
    let ours = stage_blob(engine, repo, slots[1].as_ref(), &sizes)?;
    let theirs = stage_blob(engine, repo, slots[2].as_ref(), &sizes)?;

    let worktree_file = workdir.join(path.to_string_lossy().as_ref());
    let worktree_exists = std::fs::metadata(&worktree_file).is_ok();
    // 工作区文件形状：写回时保持原样（T3.2 任务书第 5 条）；文件不在工作区
    // （删除类冲突）时给安全默认值——反正 result 没有可参照的原文件
    let (eol, bom, trailing_newline) = std::fs::read(&worktree_file)
        .map(|bytes| probe_worktree_shape(&bytes))
        .unwrap_or((LineEnding::Lf, false, false));

    // 块计算在三方都"可用文本"时进行：缺失的 stage（AddedByBoth 这类没有
    // base）按空文本参与；某个 stage 存在但内容不可用（二进制 / 超大 /
    // 非 UTF-8）则整体不算——编辑器走"采用一方"路径。BOM 从内容剥掉
    // （它在文件级由 bom 字段记录，混进第一行只会污染 diff），写回时按
    // bom 字段加回。
    let computable = [&base, &ours, &theirs].iter().all(|blob| match blob {
        None => true,
        Some(blob) => blob.content.is_some(),
    });
    // 嵌套 fn 而不是闭包：两个输入引用的生命周期省略在这里推不动
    fn blob_text(blob: &Option<FileBlob>) -> &str {
        strip_bom(
            blob.as_ref()
                .and_then(|b| b.content.as_deref())
                .unwrap_or_default(),
        )
    }
    let blocks = if computable {
        compute_merge_blocks(blob_text(&base), blob_text(&ours), blob_text(&theirs))
    } else {
        Vec::new()
    };

    Ok(ConflictFileDetail {
        path: path.clone(),
        kind: ConflictKind::classify(base.as_ref(), ours.as_ref(), theirs.as_ref()),
        base,
        ours,
        theirs,
        worktree_exists,
        eol,
        bom,
        trailing_newline,
        blocks,
    })
}

/// 探测工作区文件的换行风格、BOM 与末尾换行。
fn probe_worktree_shape(bytes: &[u8]) -> (LineEnding, bool, bool) {
    const UTF8_BOM: &[u8] = &[0xEF, 0xBB, 0xBF];
    let bom = bytes.starts_with(UTF8_BOM);
    let eol = match bytes.iter().position(|byte| *byte == b'\n') {
        Some(index) if index > 0 && bytes[index - 1] == b'\r' => LineEnding::Crlf,
        Some(_) => LineEnding::Lf,
        None if bytes.contains(&b'\r') => LineEnding::Cr,
        None => LineEnding::Lf,
    };
    let trailing_newline = matches!(bytes.last(), Some(b'\n') | Some(b'\r'));
    (eol, bom, trailing_newline)
}

/// 剥一次 UTF-8 BOM（diff 的第一行不该被它污染）。
fn strip_bom(text: &str) -> &str {
    text.strip_prefix('\u{feff}').unwrap_or(text)
}

/// 整个文件采用一方（二进制冲突与文件级快捷操作的路径）。
///
/// `git checkout --ours/--theirs` 把 stage 的 blob 恢复到工作区，
/// 随后走与 [`mark_resolved`] 相同的 add + 校验。
pub(super) fn take_side(
    engine: &CliGitEngine,
    repo: &RepoId,
    path: &RepoPath,
    side: TakeSide,
) -> AppResult<()> {
    let flag = match side {
        TakeSide::Ours => "--ours",
        TakeSide::Theirs => "--theirs",
    };
    engine.run_write(
        repo,
        GitInvocation::new(vec![
            "checkout".to_owned(),
            flag.to_owned(),
            "--".to_owned(),
            path.to_string_lossy().into_owned(),
        ]),
    )?;
    mark_resolved(engine, repo, std::slice::from_ref(path))
}

/// 把编辑器产出的结果文本写回工作区并标记已解决（T3.2 任务书第 5 条）。
///
/// 换行风格 / BOM / 末尾换行由调用方按 [`file_detail`] 探测的**原文件形状**
/// 传回，这里负责忠实重建——前端拿到的编辑文本是纯 LF 世界，
/// 文件系统才是 CRLF 的世界，转换必须只发生在一处。
pub(super) fn apply_resolution(
    engine: &CliGitEngine,
    repo: &RepoId,
    path: &RepoPath,
    content: &str,
    eol: LineEnding,
    bom: bool,
    trailing_newline: bool,
) -> AppResult<()> {
    let newline: &str = match eol {
        LineEnding::Lf => "\n",
        LineEnding::Crlf => "\r\n",
        LineEnding::Cr => "\r",
    };
    let mut bytes: Vec<u8> = Vec::with_capacity(content.len() + 8);
    if bom {
        bytes.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
    }
    let lines: Vec<&str> = content.lines().collect();
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            bytes.extend_from_slice(newline.as_bytes());
        }
        bytes.extend_from_slice(line.as_bytes());
    }
    if trailing_newline && !lines.is_empty() {
        bytes.extend_from_slice(newline.as_bytes());
    }
    let worktree_file = repo.root().join(path.to_string_lossy().as_ref());
    std::fs::write(&worktree_file, bytes).map_err(|error| {
        AppError::new(ErrorCode::Internal, "failed to write the resolved file")
            .with_hint(worktree_file.to_string_lossy().into_owned())
            .with_detail(error.to_string())
    })?;
    // 写完即 add + 校验 stage 清空（与手动标记同一收口）
    mark_resolved(engine, repo, std::slice::from_ref(path))
}

/// 以"删除该文件"解决删除类冲突（`git rm -f`：工作区与索引一起删）。
pub(super) fn remove_conflict_file(
    engine: &CliGitEngine,
    repo: &RepoId,
    path: &RepoPath,
) -> AppResult<()> {
    engine.run_write(
        repo,
        GitInvocation::new(vec![
            "rm".to_owned(),
            "-f".to_owned(),
            "--".to_owned(),
            path.to_string_lossy().into_owned(),
        ]),
    )?;
    let still_unmerged = unmerged_paths(engine, repo)?
        .iter()
        .any(|remaining| remaining == path);
    if still_unmerged {
        return Err(AppError::new(
            ErrorCode::ConflictUnresolved,
            "the path is still conflicted after removal",
        )
        .with_hint(path.to_string_lossy().into_owned()));
    }
    Ok(())
}
