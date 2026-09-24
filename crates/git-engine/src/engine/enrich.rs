//! 状态富化：porcelain v2 与 libgit2 都不携带、但状态面板需要的字段。
//!
//! # 为什么放这里而不是解析器
//!
//! 二进制嗅探、LFS 属性、文件大小都来自**文件系统与 git 属性机制**，
//! 而 porcelain 解析器的输入只有一段字节输出——它没法也不该知道磁盘上的文件。
//! 两个引擎（CLI / libgit2）在各自拿到 `StatusReport` 后都调用本模块富化，
//! 保证两条路径给界面完全相同的展示字段。
//!
//! # 性能约定（T1.4：10000 个变更文件）
//!
//! - 大小与二进制嗅探：每条目一次 `stat` + 最多 8KB 读取，全部本地操作；
//! - LFS 属性：**一次** `git check-attr -z --stdin filter` 批量进程，
//!   路径经 stdin 喂入（避免 10k 参数撞上 argv 上限）；
//! - 操作状态检测：`git_dir` 下的固定标记文件存在性检查。
//!
//! 失败语义：LFS 属性查询失败要让调用方看到明确错误再重试——
//! 静默降级会让"该显示 LFS 图标的文件"显示成普通文件，界面对用户说谎。

use std::path::{Path, PathBuf};

use forgedesk_domain::git::{EntryKind, OperationState, StatusReport};
use forgedesk_domain::{AppError, AppResult, ErrorCode};

use crate::engine::bridge::BlockingBridge;
use crate::process::{GitProcess, GitRunOpts};

/// 二进制嗅探读取的字节数（与 Git 的启发式一致：含 NUL 即按二进制处理）。
const BINARY_SNIFF_BYTES: usize = 8192;

/// 判定进行中的多步操作（`.git` 目录的标记文件）。
///
/// 顺序有讲究：`CHERRY_PICK_HEAD` 与 `MERGE_HEAD` 可能同时存在（冲突的拣选），
/// 此时按用户心智"正在合并"上报 merge；`rebase-merge` / `rebase-apply`
/// 都是 rebase 的两种形态。
pub fn detect_operation(git_dir: &Path) -> OperationState {
    let exists = |marker: &str| git_dir.join(marker).exists();

    if exists("MERGE_HEAD") {
        return OperationState::Merge;
    }
    if exists("rebase-merge") || exists("rebase-apply") {
        return OperationState::Rebase;
    }
    if exists("CHERRY_PICK_HEAD") {
        return OperationState::CherryPick;
    }
    if exists("REVERT_HEAD") {
        return OperationState::Revert;
    }
    if exists("BISECT_LOG") {
        return OperationState::Bisect;
    }
    OperationState::None
}

/// 解析 `.git` 指针文件（链接工作区的 `.git` 是内容为 `gitdir: <路径>` 的文件）。
pub fn resolve_git_dir(workdir: &Path) -> PathBuf {
    let dot_git = workdir.join(".git");
    if dot_git.is_dir() {
        return dot_git;
    }
    if let Ok(pointer) = std::fs::read_to_string(&dot_git) {
        if let Some(path) = pointer.trim().strip_prefix("gitdir:") {
            let trimmed = path.trim();
            if !trimmed.is_empty() {
                return workdir.join(trimmed);
            }
        }
    }
    dot_git
}

/// 从 `git check-attr -z` 的输出中提取启用了 `filter=lfs` 的路径集合。
///
/// `-z` 输出按 NUL 分隔，每条记录是 `路径 NUL 属性名 NUL 值`；
/// 不成对的尾部字节直接忽略（畸形输出只影响部分路径的判定）。
pub fn lfs_paths_from_check_attr(output: &[u8]) -> Vec<Vec<u8>> {
    let fields: Vec<&[u8]> = output.split(|byte| *byte == 0).collect();
    let mut lfs = Vec::new();

    for triple in fields.chunks_exact(3) {
        let (path, attribute, value) = (triple[0], triple[1], triple[2]);
        if attribute == b"filter" && value == b"lfs" {
            lfs.push(path.to_vec());
        }
    }
    lfs
}

/// 第一段富化：操作状态、文件大小与二进制嗅探（纯文件系统，无子进程）。
///
/// 返回需要查询 LFS 属性的路径集合（被忽略文件不参与：它们不进状态面板的操作列）。
pub fn enrich_filesystem(
    report: &mut StatusReport,
    workdir: &Path,
    git_dir: &Path,
) -> Vec<Vec<u8>> {
    report.operation = detect_operation(git_dir);

    let mut lfs_candidates: Vec<Vec<u8>> = Vec::new();
    for entry in &mut report.entries {
        let worktree_file = workdir.join(entry.path.to_string_lossy().as_ref());
        match std::fs::metadata(&worktree_file) {
            // 文件不存在（删除/纯索引侧重命名）：没有大小，也不谈二进制
            Err(_) => entry.size_bytes = None,
            // 子模块 / 目录：大小与二进制都无意义，但 LFS 属性仍然可查
            Ok(metadata) if !metadata.is_file() => entry.size_bytes = None,
            Ok(metadata) => {
                entry.size_bytes = Some(metadata.len());
                entry.is_binary = is_binary_file(&worktree_file);
            }
        }

        if entry.kind != EntryKind::Ignored {
            lfs_candidates.push(entry.path.as_bytes().to_vec());
        }
    }
    lfs_candidates
}

/// 批量查询 LFS 属性，返回启用 `filter=lfs` 的路径集合。
pub fn query_lfs_paths(
    process: &GitProcess,
    workdir: &Path,
    paths: &[Vec<u8>],
) -> AppResult<Vec<Vec<u8>>> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }

    let mut stdin = Vec::with_capacity(paths.iter().map(|path| path.len() + 1).sum::<usize>());
    for path in paths {
        stdin.extend_from_slice(path);
        stdin.push(0);
    }

    // check-attr 是一次性的辅助查询：现场建桥（每次约 50 微秒，见 bridge.rs 的说明）
    let bridge = BlockingBridge::new()?;
    let output = bridge
        .block_on(process.run(
            &[
                "check-attr".to_owned(),
                "-z".to_owned(),
                "--stdin".to_owned(),
                "filter".to_owned(),
            ],
            GitRunOpts::new(workdir.to_path_buf()).with_stdin(stdin),
        ))
        .and_then(|output| output)?;

    if !output.success() {
        return Err(
            AppError::new(ErrorCode::Internal, "git check-attr reported a failure")
                .with_detail(forgedesk_diagnostics::sanitize_log(&output.stderr_lossy())),
        );
    }
    Ok(lfs_paths_from_check_attr(&output.stdout))
}

/// 第二段富化：把查到的 LFS 路径写回条目。
pub fn apply_lfs(report: &mut StatusReport, lfs: &[Vec<u8>]) {
    if lfs.is_empty() {
        return;
    }
    for entry in &mut report.entries {
        let path = entry.path.as_bytes();
        if lfs.iter().any(|candidate| candidate == path) {
            entry.is_lfs = true;
        }
    }
}

/// 统计被忽略条目数（调用方只在请求了 `include_ignored` 时填写该字段）。
pub fn count_ignored(report: &StatusReport) -> Option<u64> {
    let count = report
        .entries
        .iter()
        .filter(|entry| entry.kind == EntryKind::Ignored)
        .count() as u64;
    Some(count)
}

fn is_binary_file(path: &Path) -> bool {
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    use std::io::Read;
    let mut buffer = vec![0_u8; BINARY_SNIFF_BYTES];
    let read = file.read(&mut buffer).unwrap_or(0);
    buffer[..read].contains(&0)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::path::PathBuf;

    use forgedesk_domain::git::OperationState;

    use super::{detect_operation, lfs_paths_from_check_attr, resolve_git_dir};

    fn temp_dir(label: &str) -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let suffix = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut path = std::env::temp_dir();
        path.push(format!("enrich-{label}-{}-{suffix}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn operation_markers_are_detected_in_priority_order() {
        let dir = temp_dir("ops");
        assert_eq!(detect_operation(&dir), OperationState::None);

        std::fs::write(dir.join("CHERRY_PICK_HEAD"), b"").unwrap();
        assert_eq!(detect_operation(&dir), OperationState::CherryPick);

        // merge 与 cherry-pick 并存（冲突的拣选）时按 merge 上报
        std::fs::write(dir.join("MERGE_HEAD"), b"").unwrap();
        std::fs::write(dir.join("rebase-merge"), b"").unwrap();
        assert_eq!(detect_operation(&dir), OperationState::Merge);

        std::fs::remove_file(dir.join("MERGE_HEAD")).unwrap();
        assert_eq!(detect_operation(&dir), OperationState::Rebase);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn git_pointer_file_resolves_to_the_real_git_dir() {
        let workdir = temp_dir("worktree");
        let real = temp_dir("gitdir");
        std::fs::write(
            workdir.join(".git"),
            format!("gitdir: {}\n", real.to_string_lossy()),
        )
        .unwrap();

        assert_eq!(resolve_git_dir(&workdir), real);
        std::fs::remove_dir_all(&workdir).ok();
        std::fs::remove_dir_all(&real).ok();
    }

    #[test]
    fn check_attr_records_are_parsed_as_triples() {
        let output = b"a.txt\0filter\0lfs\0b.bin\0filter\0-\0";
        let lfs = lfs_paths_from_check_attr(output);

        assert_eq!(lfs, vec![b"a.txt".to_vec()]);
    }

    #[test]
    fn trailing_partial_record_is_ignored() {
        let output = b"a.txt\0filter\0lfs\0b.bin\0fil";
        let lfs = lfs_paths_from_check_attr(output);

        assert_eq!(lfs.len(), 1);
    }
}
