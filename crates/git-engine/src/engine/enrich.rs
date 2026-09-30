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

/// 判定进行中的多步操作（`.git` 目录的标记文件与目录的**综合**判断）。
///
/// 顺序有讲究：`CHERRY_PICK_HEAD` 与 `MERGE_HEAD` 可能同时存在（冲突的拣选），
/// 此时按用户心智"正在合并"上报 merge；`rebase-merge` / `rebase-apply`
/// 都是 rebase 的两种形态。
///
/// # T3.1 起的语义（多信号，不再依赖单一文件）
///
/// rebase 目录（`rebase-merge` / `rebase-apply`）是**强信号**，优先于一切单文件
/// 标记：目录存在说明 sequencer 正在跑，而此时任何单文件标记都只是它的影子。
/// `git am` 复用 `rebase-apply` 目录——对"操作状态检测"而言语义相同（应用补丁
/// 序列），不单独区分。
pub fn detect_operation(git_dir: &Path) -> OperationState {
    detect_operation_with_details(git_dir).state
}

/// 综合检测的完整结果：操作类型 + rebase / 序列进度。
///
/// 进度只对有序列语义的操作有意义：
/// - rebase：`rebase-merge`（或 `rebase-apply`）下的 `msgnum` / `end`；
/// - cherry-pick / revert 序列：`.git/sequencer` 的 `done` + `todo` 行数
///   （序列的**间隙**——上一个提交已完成、下一个还没开始——既没有
///   `CHERRY_PICK_HEAD` 也没有 `REVERT_HEAD`，只有 sequencer 目录）。
pub struct OperationDetection {
    /// 操作类型。
    pub state: OperationState,
    /// 当前步骤（从 1 开始计）。
    pub current_step: Option<u32>,
    /// 总步数。
    pub total_steps: Option<u32>,
    /// 被 rebase 的分支名（`head-name`；其余操作为 `None`）。
    pub head_name: Option<String>,
    /// rebase 因 `edit` 步骤暂停（`rebase-merge/amend` 标记；git 老版本是 `am`）。
    /// 用户改完内容后由应用执行 `commit --amend` + `rebase --continue`。
    pub edit_paused: bool,
}

/// 判定进行中的操作并采集进度（T3.1 的状态机数据源）。
pub fn detect_operation_with_details(git_dir: &Path) -> OperationDetection {
    let exists = |marker: &str| git_dir.join(marker).exists();

    // rebase 两种形态是强信号，最先判（见函数文档）
    let rebase_dir = if exists("rebase-merge") {
        Some(git_dir.join("rebase-merge"))
    } else if exists("rebase-apply") {
        Some(git_dir.join("rebase-apply"))
    } else {
        None
    };
    if let Some(dir) = rebase_dir {
        return OperationDetection {
            state: OperationState::Rebase,
            current_step: read_marker_number(&dir.join("msgnum")),
            total_steps: read_marker_number(&dir.join("end")),
            head_name: read_marker_text(&dir.join("head-name")),
            edit_paused: dir.join("amend").is_file() || dir.join("am").is_file(),
        };
    }

    if exists("MERGE_HEAD") {
        return OperationDetection::simple(OperationState::Merge);
    }
    if exists("CHERRY_PICK_HEAD") {
        return OperationDetection::simple(OperationState::CherryPick);
    }
    if exists("REVERT_HEAD") {
        return OperationDetection::simple(OperationState::Revert);
    }
    // sequencer 目录在而标记文件不在：多步 cherry-pick / revert 序列的间隙。
    // sequencer 是 cherry-pick 与 revert 共用的机制，todo 里无法区分二者；
    // 按 cherry-pick 上报（两者的用户语义都是"重放一组提交"）。
    if exists("sequencer") {
        let done = count_todo_lines(&git_dir.join("sequencer").join("done"));
        let todo = count_todo_lines(&git_dir.join("sequencer").join("todo"));
        if let (Some(done), Some(todo)) = (done, todo) {
            return OperationDetection {
                state: OperationState::CherryPick,
                current_step: Some(done + 1),
                total_steps: Some(done + todo),
                head_name: None,
                edit_paused: false,
            };
        }
    }
    if exists("BISECT_LOG") {
        return OperationDetection::simple(OperationState::Bisect);
    }
    OperationDetection::simple(OperationState::None)
}

impl OperationDetection {
    const fn simple(state: OperationState) -> Self {
        Self {
            state,
            current_step: None,
            total_steps: None,
            head_name: None,
            edit_paused: false,
        }
    }
}

/// 读取标记文件里的数字（`msgnum` / `end`；内容形如 `2\n`）。
///
/// 读不到或内容不是数字都返回 `None`：进度是**增强信息**，缺失时界面退化为
/// 不显示进度，绝不能因此让整个状态查询失败。
fn read_marker_number(path: &Path) -> Option<u32> {
    let text = read_marker_text(path)?;
    text.parse().ok()
}

/// 读取标记文件里的文本（`head-name`；去首尾空白与空文件）。
fn read_marker_text(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

/// 统计 sequencer todo / done 文件里的有效行数（`#` 注释与空行不算）。
fn count_todo_lines(path: &Path) -> Option<u32> {
    let text = std::fs::read_to_string(path).ok()?;
    let count = text
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty() && !trimmed.starts_with('#')
        })
        .count() as u32;
    Some(count)
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
                // **不做二进制嗅探**：它需要打开并读取每个文件，在"1 万个变更文件"
                // 这种规模下就是 1 万次 open——T1.12 的性能基线在 Windows 上量到
                // 39 秒（同一仓库 `git status --porcelain` 只要 0.05 秒，因为 git
                // 自己也不嗅探）。二进制在真正需要它的地方判定：diff 由 CLI 给出
                // `binary: true`，`git apply` 也自己认二进制。状态列表只需要"大小"，
                // 而大小一次 stat 就够。
                entry.is_binary = false;
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

/// 判断一个文件的前 8KB 里是否含 NUL（二进制嗅探）。
///
/// **当前没有调用点**，保留它是为了让"将来某个真的需要的调用方"不必重新发明——
/// 但调用前请先读上面 `enrich_filesystem` 里那段说明：状态路径上每文件一次
/// 打开，在 1 万文件的规模下是几十秒。要用它，请在**用户显式要求**的地方用
/// （例如"这个文件是二进制的吗"的单点查询），不要放进批量路径。
#[allow(dead_code)]
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

    use super::{
        detect_operation, detect_operation_with_details, lfs_paths_from_check_attr, resolve_git_dir,
    };

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

        // rebase 目录是强信号：即使 MERGE_HEAD 也在（畸形 / 残留），sequencer
        // 目录正在跑这件事优先（T3.1 起的综合判断）
        std::fs::create_dir_all(dir.join("rebase-merge")).unwrap();
        std::fs::write(dir.join("MERGE_HEAD"), b"").unwrap();
        assert_eq!(detect_operation(&dir), OperationState::Rebase);

        std::fs::remove_dir_all(dir.join("rebase-merge")).unwrap();
        assert_eq!(detect_operation(&dir), OperationState::Merge);

        std::fs::remove_file(dir.join("MERGE_HEAD")).unwrap();
        assert_eq!(detect_operation(&dir), OperationState::CherryPick);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rebase_progress_is_read_from_msgnum_and_end() {
        let dir = temp_dir("rebase-progress");
        let rebase = dir.join("rebase-merge");
        std::fs::create_dir_all(&rebase).unwrap();
        std::fs::write(rebase.join("msgnum"), b"3\n").unwrap();
        std::fs::write(rebase.join("end"), b"7\n").unwrap();
        std::fs::write(rebase.join("head-name"), b"refs/heads/main\n").unwrap();

        let detection = detect_operation_with_details(&dir);
        assert_eq!(detection.state, OperationState::Rebase);
        assert_eq!(detection.current_step, Some(3));
        assert_eq!(detection.total_steps, Some(7));
        assert_eq!(detection.head_name.as_deref(), Some("refs/heads/main"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_or_malformed_progress_degrades_to_none_instead_of_failing() {
        let dir = temp_dir("rebase-degraded");
        std::fs::create_dir_all(dir.join("rebase-merge")).unwrap();

        let detection = detect_operation_with_details(&dir);
        assert_eq!(detection.state, OperationState::Rebase);
        assert_eq!(detection.current_step, None);
        assert_eq!(detection.total_steps, None);
        assert_eq!(detection.head_name, None);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn sequencer_gap_is_reported_as_cherry_pick_with_progress() {
        // 多步拣选序列的间隙：上一步已完成、下一步未开始——没有 CHERRY_PICK_HEAD，
        // 只有 sequencer 目录（T3.1 要求的综合判断）
        let dir = temp_dir("sequencer-gap");
        let sequencer = dir.join("sequencer");
        std::fs::create_dir_all(&sequencer).unwrap();
        std::fs::write(
            sequencer.join("done"),
            b"pick aaaaaaa one\npick bbbbbbb two\n",
        )
        .unwrap();
        std::fs::write(
            sequencer.join("todo"),
            "# comment line\n\npick ccccccc three\n",
        )
        .unwrap();

        let detection = detect_operation_with_details(&dir);
        assert_eq!(detection.state, OperationState::CherryPick);
        assert_eq!(detection.current_step, Some(3));
        assert_eq!(detection.total_steps, Some(3));

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
