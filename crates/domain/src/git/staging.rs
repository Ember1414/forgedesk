//! 部分暂存的选择模型与补丁裁剪（T1.6）。
//!
//! # 为什么裁剪做在**字节层**
//!
//! T1.5 的查看器解析器（`git-engine::parsers::unified_diff`）走 `from_utf8_lossy`，
//! 对非 UTF-8 的内容行会替换成 `U+FFFD`。这对"显示"无害，但裁剪后的补丁要交回
//! `git apply` —— 一旦内容被替换，补丁必然对不上（用户看到"补丁无法应用"，
//! 而磁盘上的文件其实完好）。因此这里的视图只记**行下标**，内容原样搬运；
//! 行类别只看首字节（`+` / `-` / 空格 / `\` / `@`），那是 ASCII，不受编码影响。
//!
//! # 为什么"未选中的删除行"要改写成上下文行
//!
//! 暂存的目标是"让索引变成用户选中的样子"，不是"把未选中的东西删掉"：
//!
//! - 未选中的 `+` 行：索引里本来就没有它 → **从补丁里删掉**；
//! - 未选中的 `-` 行：这一行仍要留在索引里 → **改写成上下文行**（`-` → 空格）。
//!
//! 直接删掉未选中的 `-` 行会让 `git apply` 以为"用户也想删掉它"，从而销毁内容 ——
//! 这是部分暂存最容易出的事故，也是本模块存在的理由。
//!
//! # 边界情况的处置（都有单测钉住）
//!
//! | 情况 | 处置 |
//! | --- | --- |
//! | 新增文件（`--- /dev/null`）被部分暂存 | 头部保持"新文件"语义，行数由重写的 hunk 头表达 |
//! | 删除文件（`+++ /dev/null`）被部分暂存 | 该文件**不再被删除**：去掉 `deleted file mode`、`index` 行，改写 `+++` 为真实路径 |
//! | 重命名 / 复制（`rename from|to`） | 文件级属性整体保留（重命名不是可以"部分"的东西） |
//! | `\ No newline at end of file` | 标记**附着于前一行**：前一行保留或转上下文则标记保留，前一行是 `+` 且被丢弃则标记一并丢弃 |
//! | 裁剪后没有任何变更的 hunk | 整块丢弃（留着只会让 `git apply` 报错或产生歧义） |
//! | 裁剪后文件段没有任何 hunk | 整个文件段丢弃（否则会凭空暂存出一个空文件或一次空重命名） |
//! | 二进制文件 | 拒绝行级：只能整体暂存 |
//!
//! # 行号为什么只改行数、不改起点
//!
//! `old_start` / `new_start` 是两侧文件中的**绝对行号**，前面的 hunk 被丢弃不会让后面的
//! hunk 前移。只有一处例外：删除文件的部分暂存会把 `new_start = 0`（空文件）变成真实
//! 行号，此时必须把它抬到 1，否则 `git apply` 会认为"新文件有内容却从第 0 行开始"。
//! 行数则不重算不行 —— `--recount` 是兜底，不是借口。

use super::path::RepoPath;
use crate::{AppError, AppResult, ErrorCode};

/// 暂存 / 取消暂存的粒度。
///
/// 三种粒度对应界面上的三种操作：整文件、按块、按行。粒度进了类型，
/// 服务层就不必猜"用户到底选了什么"，命令层的参数校验也可以直接照它写。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StageScope {
    /// 整文件（走 `git add` / `git reset`，不走补丁）。
    Files(Vec<RepoPath>),
    /// 单文件内选中的 hunk 下标（0 基，与 `workspace_diff` 返回的 `hunks` 顺序一致）。
    Hunks {
        /// 目标文件（相对仓库根的目标路径，重命名时是目标路径）。
        path: RepoPath,
        /// 选中的 hunk 下标。
        indices: Vec<usize>,
    },
    /// 单文件内选中的行（按 hunk 分组）。
    Lines {
        /// 目标文件。
        path: RepoPath,
        /// 每个 hunk 内选中的行下标。
        selections: Vec<LineSelection>,
    },
}

/// 某个 hunk 内被选中的行。
///
/// `lines` 是**行在该 hunk `lines` 数组中的下标**（0 基，上下文行与标记行也参与计数），
/// 与 `workspace_diff` 返回的结构一一对应 —— 界面因此不需要再做一次映射。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineSelection {
    /// hunk 下标（0 基）。
    pub hunk_index: usize,
    /// 选中的行下标（0 基）。
    pub lines: Vec<usize>,
}

/// 暂存粒度。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StageGranularity {
    /// 整文件。
    Files,
    /// 按 hunk。
    Hunks,
    /// 按行。
    Lines,
}

impl StageScope {
    /// 选择粒度。
    pub fn granularity(&self) -> StageGranularity {
        match self {
            Self::Files(_) => StageGranularity::Files,
            Self::Hunks { .. } => StageGranularity::Hunks,
            Self::Lines { .. } => StageGranularity::Lines,
        }
    }

    /// 单文件粒度时的目标路径；整文件粒度返回 `None`。
    pub fn path(&self) -> Option<&RepoPath> {
        match self {
            Self::Files(_) => None,
            Self::Hunks { path, .. } | Self::Lines { path, .. } => Some(path),
        }
    }

    /// 是否没有任何选择（界面上的"什么都没选"）。
    ///
    /// 空选择**不是错误**：它与"用户点了按钮但一行都没选"是同一件事，
    /// 服务层据此幂等返回，而不是调用 `git apply` 一份空补丁。
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Files(paths) => paths.is_empty(),
            Self::Hunks { indices, .. } => indices.is_empty(),
            Self::Lines { selections, .. } => selections.iter().all(|item| item.lines.is_empty()),
        }
    }

    /// 按 hunk 选择。
    pub fn hunks(path: RepoPath, indices: Vec<usize>) -> Self {
        Self::Hunks { path, indices }
    }

    /// 按行选择。
    pub fn lines(path: RepoPath, selections: Vec<LineSelection>) -> Self {
        Self::Lines { path, selections }
    }
}

/// 补丁将被使用的方向。
///
/// 裁剪规则只有一处依赖它 —— "未选中的行怎么处理"，而这一处恰恰决定补丁能否应用：
///
/// - [`PatchDirection::Forward`]（暂存）：补丁将**正向**应用，目标的当前内容就是补丁的
///   **旧侧**。因此未选中的 `-` 行（旧侧有它）要转成上下文行，未选中的 `+` 行
///   （旧侧没有它）要从补丁里删掉。
/// - [`PatchDirection::Reverse`]（取消暂存 / 丢弃）：补丁将**反向**应用，目标的当前内容
///   是补丁的**新侧**，规则随之镜像：未选中的 `+` 行转上下文，未选中的 `-` 行删掉。
///
/// 一句话：**未选中的行按"它在目标当前内容里是否存在"决定去留**（存在 → 上下文行，
/// 不存在 → 移除）；选中的行一律保持原类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PatchDirection {
    /// 正向：把选中的改动写进目标（暂存）。
    Forward,
    /// 反向：把选中的改动从目标里撤掉（取消暂存、丢弃）。
    Reverse,
}

/// 把一份"整个文件"的补丁裁剪成"只含选中部分"的补丁。
///
/// 入参补丁必须**只含一个文件段**（调用方在生成补丁时用 `paths` 过滤，
/// 见 `services::staging`），否则返回 `VALIDATION`：多文件段意味着调用方
/// 把选择与补丁对错了位置，继续裁剪只会产出一份比输入更危险的补丁。
///
/// 返回空字节表示"裁剪后没有任何内容需要写"（选中的 hunk 全被丢弃），
/// 调用方应视作幂等成功而不是错误。
pub fn trim_patch(
    patch: &[u8],
    scope: &StageScope,
    direction: PatchDirection,
) -> AppResult<Vec<u8>> {
    let selector = match scope {
        StageScope::Files(_) => {
            return Err(AppError::new(
                ErrorCode::Validation,
                "whole-file staging does not go through patch trimming",
            )
            .with_hint("kind=files".to_owned()));
        }
        StageScope::Hunks { indices, .. } => Selector::Hunks(indices),
        StageScope::Lines { selections, .. } => Selector::Lines(selections),
    };

    // 按 `\n` 切行：UTF-8 里 0x0A 不可能出现在多字节序列内部，
    // 因此这在字节层与 `str::split('\n')` 的划分逐行等价（见模块头）。
    let lines: Vec<&[u8]> = patch.split(|byte| *byte == b'\n').collect();
    let sections = view(&lines);

    if sections.is_empty() {
        return Err(AppError::new(
            ErrorCode::Validation,
            "the patch contains no file section",
        ));
    }
    if sections.len() > 1 {
        return Err(AppError::new(
            ErrorCode::Validation,
            "the patch must contain exactly one file section",
        )
        .with_detail(format!("sections: {}", sections.len())));
    }

    let section = &sections[0];
    if section.binary {
        return Err(AppError::new(
            ErrorCode::Validation,
            "binary files can only be staged as a whole",
        )
        .with_hint(
            scope
                .path()
                .map_or_else(String::new, |p| p.to_string_lossy().into_owned()),
        ));
    }

    validate_selector(&selector, section)?;

    // ---- 逐 hunk 裁剪
    let mut blocks: Vec<HunkBlock> = Vec::new();
    for (hunk_index, hunk) in section.hunks.iter().enumerate() {
        let Some(block) = trim_hunk(&lines, hunk, hunk_index, &selector, direction) else {
            continue;
        };
        blocks.push(block);
    }

    if blocks.is_empty() {
        // 没有任何 hunk 需要写进去：整个文件段丢弃。
        return Ok(Vec::new());
    }

    // 裁剪后的补丁在两侧各留下多少行。这两个数字决定头部要不要改写：
    // "新增文件"的成立条件是旧侧为空，"删除文件"是新侧为空 —— 一旦被部分选择破坏，
    // `/dev/null` 与 mode 行会让 git 做出**整文件级**的动作（凭空创建或整份删除），
    // 而用户以为他只动了一行。这条规则与方向无关：只看输出补丁的两侧是否为空。
    let out_old: u32 = blocks.iter().map(|block| block.old_lines).sum();
    let out_new: u32 = blocks.iter().map(|block| block.new_lines).sum();
    let rewrite_added = section.is_new_file && out_old > 0;
    let rewrite_deleted = section.is_deleted_file && out_new > 0;

    // ---- 头部
    let mut header: Vec<Vec<u8>> = Vec::with_capacity(section.header.len());
    for &index in &section.header {
        let line = lines[index];
        if rewrite_added {
            if Some(index) == section.new_mode_line || Some(index) == section.index_line {
                // `new file mode` / `index 0000000..xxx` 会让 git 认为要创建一个新文件
                continue;
            }
            if Some(index) == section.old_file_line {
                // 旧侧不再为空 → `--- /dev/null` 必须改写为对侧的真实路径
                header.push(rewrite_dev_null_line(
                    b"--- ",
                    section.new_path_raw.as_deref(),
                    b"b/",
                    b"a/",
                )?);
                continue;
            }
        }
        if rewrite_deleted {
            if Some(index) == section.deleted_mode_line || Some(index) == section.index_line {
                continue;
            }
            if Some(index) == section.new_file_line {
                header.push(rewrite_dev_null_line(
                    b"+++ ",
                    section.old_path_raw.as_deref(),
                    b"a/",
                    b"b/",
                )?);
                continue;
            }
        }
        header.push(line.to_vec());
    }

    // ---- 拼装（末尾是否补换行必须与原补丁一致：多一个或少一个都会改变补丁的结尾语义）
    let mut out: Vec<Vec<u8>> = header;
    for block in blocks {
        out.push(block.header);
        out.extend(block.body);
    }

    let mut bytes = Vec::new();
    for (position, line) in out.iter().enumerate() {
        if position > 0 {
            bytes.push(b'\n');
        }
        bytes.extend_from_slice(line);
    }
    if patch.last() == Some(&b'\n') {
        // 原补丁以换行结尾 → 输出也要有（`split` 丢掉的那个尾部空元素补回来）
        bytes.push(b'\n');
    }

    Ok(bytes)
}

/// 选择器（已从 [`StageScope`] 收敛）。
#[derive(Debug, Clone, Copy)]
enum Selector<'a> {
    Hunks(&'a [usize]),
    Lines(&'a [LineSelection]),
}

/// 一个 hunk 的裁剪结果。
#[derive(Debug, Clone, PartialEq, Eq)]
struct HunkBlock {
    /// 重写后的 `@@ … @@` 行。
    header: Vec<u8>,
    /// hunk 体（含必要的 `\ No newline` 标记）。
    body: Vec<Vec<u8>>,
    /// 重写后的旧侧行数（用于判断"还是不是新增文件"）。
    old_lines: u32,
    /// 重写后的新侧行数（用于判断"还是不是删除文件"）。
    new_lines: u32,
}

/// 裁剪单个 hunk；返回 `None` 表示该 hunk 没有留下任何变更。
///
/// 整块粒度与方向无关：选中就是"这一块被写进目标"（正向）或被撤掉（反向），
/// 两种情况下补丁字节都是原样保留 —— 只有"部分行"才需要按方向改写 body。
fn trim_hunk(
    lines: &[&[u8]],
    hunk: &HunkView,
    hunk_index: usize,
    selector: &Selector<'_>,
    direction: PatchDirection,
) -> Option<HunkBlock> {
    match selector {
        Selector::Hunks(indices) => {
            if !indices.contains(&hunk_index) {
                return None;
            }
            // 整块保留：body 与 header 都是原字节，连 `--recount` 都不需要。
            Some(HunkBlock {
                header: lines[hunk.header_index].to_vec(),
                body: hunk
                    .body
                    .iter()
                    .map(|&index| lines[index].to_vec())
                    .collect(),
                old_lines: hunk.range.old_lines,
                new_lines: hunk.range.new_lines,
            })
        }
        Selector::Lines(selections) => {
            trim_hunk_lines(lines, hunk, hunk_index, selections, direction)
        }
    }
}

/// 把一行改写成上下文行（只换首字节，内容原样）。
fn as_context(line: &[u8]) -> Vec<u8> {
    let mut rewritten = line.to_vec();
    if let Some(first) = rewritten.first_mut() {
        *first = b' ';
    }
    rewritten
}

/// 按行选择裁剪一个 hunk。
///
/// 行处置规则见 [`PatchDirection`]：选中的行保持原类别，未选中的行按"它在目标的
/// 当前内容里是否存在"决定转上下文还是被移除。搞反这一条会让补丁被 git 拒绝
/// （上下文对不上），或者在极端情况下把用户想保留的行一起撤掉。
fn trim_hunk_lines(
    lines: &[&[u8]],
    hunk: &HunkView,
    hunk_index: usize,
    selections: &[LineSelection],
    direction: PatchDirection,
) -> Option<HunkBlock> {
    // 界面的行下标是"行在该 hunk `lines` 数组里的位置"，而这里遍历的是补丁文件里的行下标。
    // 两者一一对应（`hunk.body` 与 `DiffHunk::lines` 同一个解析规则，标记行也参与计数），
    // 因此必须做一次位置 → 行下标的映射；直接比较会永远选不中任何行。
    let selected: Vec<usize> = selections
        .iter()
        .find(|selection| selection.hunk_index == hunk_index)
        .map(|selection| {
            selection
                .lines
                .iter()
                .filter_map(|position| hunk.body.get(*position).copied())
                .collect()
        })
        .unwrap_or_default();

    let mut body: Vec<Vec<u8>> = Vec::with_capacity(hunk.body.len());
    let mut old_lines = 0_u32;
    let mut new_lines = 0_u32;
    let mut has_change = false;
    // `\ No newline` 标记附着于"上一个内容行"：它的去留取决于那一行的去留，
    // 而不是它自己的下标。因此这里记住最近一次内容行的处置结果。
    let mut previous_kept = true;

    for &index in &hunk.body {
        let line = lines[index];
        let is_selected = selected.contains(&index);
        match line.first().copied() {
            Some(b' ') => {
                old_lines += 1;
                new_lines += 1;
                body.push(line.to_vec());
                previous_kept = true;
            }
            Some(b'+') => {
                if is_selected {
                    // 选中的行保持 `+`：正向 = 写进目标；反向 = 从目标里撤掉它
                    new_lines += 1;
                    has_change = true;
                    body.push(line.to_vec());
                    previous_kept = true;
                } else if direction == PatchDirection::Reverse {
                    // 反向时目标的内容就是补丁的新侧 → 这一行在目标里存在，转上下文
                    old_lines += 1;
                    new_lines += 1;
                    body.push(as_context(line));
                    previous_kept = true;
                } else {
                    // 正向时目标的内容是旧侧 → 旧侧没有这一行，从补丁里移除
                    previous_kept = false;
                }
            }
            Some(b'-') => {
                if is_selected {
                    old_lines += 1;
                    has_change = true;
                    body.push(line.to_vec());
                    previous_kept = true;
                } else if direction == PatchDirection::Forward {
                    // 正向时旧侧有这一行 → 转上下文，表示"它仍留在目标里"
                    old_lines += 1;
                    new_lines += 1;
                    body.push(as_context(line));
                    previous_kept = true;
                } else {
                    // 反向时目标（新侧）没有这一行 → 从补丁里移除
                    previous_kept = false;
                }
            }
            Some(b'\\') if previous_kept => {
                body.push(line.to_vec());
            }
            _ => {
                // 畸形行（第三方 textconv 产物）：跳过而不是让整次写操作失败。
            }
        }
    }

    if !has_change {
        // 没有一行被选中：这个 hunk 对目标没有任何影响，留着只会让 `git apply` 徒增歧义。
        return None;
    }

    let old_start = floor_start(hunk.range.old_start, old_lines);
    let new_start = floor_start(hunk.range.new_start, new_lines);
    let header = format!(
        "@@ -{old_start},{old_lines} +{new_start},{new_lines} @@{}",
        hunk.suffix
    )
    .into_bytes();

    Some(HunkBlock {
        header,
        body,
        old_lines,
        new_lines,
    })
}

/// 行数为 0 时起点必须是 0（空侧），否则至少是 1。
///
/// git 对空文件用 `-0,0`：删除文件被部分暂存后新文件有内容，原来的 `new_start = 0`
/// 就变成了非法值（"第 0 行开始却有内容"），必须抬到 1。
fn floor_start(start: u32, lines: u32) -> u32 {
    if lines == 0 {
        0
    } else {
        start.max(1)
    }
}

/// 把 `/dev/null` 那一侧改写成对侧的真实路径。
///
/// 用在两处对称的场景：删除文件被部分保留（`+++ /dev/null` → `+++ b/x`），
/// 以及新增文件被部分撤销（`--- /dev/null` → `--- a/x`）。
///
/// `source` 是对侧那行的原始字节（`a/x` 或 `"a/x y"`），`from`/`to` 是两侧的目录前缀。
/// 替换的是**第一次出现的** `from` —— 引号形式下它仍在引号内部，位置一致，
/// 因此不需要真的解析引号。
fn rewrite_dev_null_line(
    marker: &[u8],
    source: Option<&[u8]>,
    from: &[u8],
    to: &[u8],
) -> AppResult<Vec<u8>> {
    let Some(raw) = source else {
        return Err(AppError::new(
            ErrorCode::Internal,
            "cannot rewrite /dev/null: the counterpart path line is missing",
        ));
    };

    let mut rewritten: Vec<u8> = marker.to_vec();
    match find(raw, from) {
        Some(position) => {
            rewritten.extend_from_slice(&raw[..position]);
            rewritten.extend_from_slice(to);
            rewritten.extend_from_slice(&raw[position + from.len()..]);
        }
        None => rewritten.extend_from_slice(raw),
    }
    Ok(rewritten)
}

/// 校验选择是否落在解析出的结构内。
///
/// 校验发生在裁剪之前：越界下标如果被静默忽略，用户会看到"点了暂存但什么都没发生"，
/// 而真正的原因（界面持有的 diff 与仓库当前状态不一致）就此被吞掉。
fn validate_selector(selector: &Selector<'_>, section: &FileSection) -> AppResult<()> {
    match selector {
        Selector::Hunks(indices) => {
            for &index in *indices {
                if index >= section.hunks.len() {
                    return Err(out_of_range(index, section.hunks.len(), "hunk"));
                }
            }
        }
        Selector::Lines(selections) => {
            for selection in *selections {
                let Some(hunk) = section.hunks.get(selection.hunk_index) else {
                    return Err(out_of_range(
                        selection.hunk_index,
                        section.hunks.len(),
                        "hunk",
                    ));
                };
                for &index in &selection.lines {
                    if index >= hunk.body.len() {
                        return Err(out_of_range(index, hunk.body.len(), "line"));
                    }
                }
            }
        }
    }
    Ok(())
}

fn out_of_range(index: usize, total: usize, what: &str) -> AppError {
    AppError::new(
        ErrorCode::Validation,
        format!("the selected {what} index is out of range"),
    )
    .with_detail(format!("index {index}, available {total}"))
}

// ---------------------------------------------------------------- 字节视图

/// 一个文件段在补丁里的行下标视图。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct FileSection {
    /// 头部行下标（`diff --git` 到第一个 `@@` 之前）。
    header: Vec<usize>,
    /// hunk 视图。
    hunks: Vec<HunkView>,
    /// 二进制段（`Binary files … differ` / `GIT binary patch`）。
    binary: bool,
    /// `--- /dev/null`：该段表示新增文件。
    is_new_file: bool,
    /// `+++ /dev/null`：该段表示删除文件。
    is_deleted_file: bool,
    /// `--- …` 行的下标。
    old_file_line: Option<usize>,
    /// `+++ …` 行的下标。
    new_file_line: Option<usize>,
    /// `new file mode …` 行的下标。
    new_mode_line: Option<usize>,
    /// `deleted file mode …` 行的下标。
    deleted_mode_line: Option<usize>,
    /// `index …` 行的下标。
    index_line: Option<usize>,
    /// `--- …` 行前缀之后的原始字节（含 `a/` 前缀与可能的引号）。
    old_path_raw: Option<Vec<u8>>,
    /// `+++ …` 行前缀之后的原始字节（含 `b/` 前缀与可能的引号）。
    new_path_raw: Option<Vec<u8>>,
}

/// 一个 hunk 的行下标视图。
#[derive(Debug, Clone, PartialEq, Eq)]
struct HunkView {
    /// `@@ … @@` 行下标。
    header_index: usize,
    /// 头部声明的范围。
    range: HunkRange,
    /// `@@` 之后的原始字节（含前导空格）。
    suffix: String,
    /// hunk 体的行下标（含 `\ No newline` 标记）。
    body: Vec<usize>,
}

/// hunk 头声明的两侧范围。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HunkRange {
    old_start: u32,
    old_lines: u32,
    new_start: u32,
    new_lines: u32,
}

/// 按行下标解析补丁（段 → hunk → 行），与 T1.5 查看器解析器同一套消费规则。
fn view(lines: &[&[u8]]) -> Vec<FileSection> {
    let mut sections: Vec<FileSection> = Vec::new();
    let mut index = 0;

    while index < lines.len() {
        if !lines[index].starts_with(b"diff --git ") {
            index += 1;
            continue;
        }

        let mut section = FileSection::default();
        section.header.push(index);
        index += 1;

        // ---- 头部
        while index < lines.len() {
            let line = lines[index];
            if line.starts_with(b"diff --git ") || line.starts_with(b"@@ ") {
                break;
            }
            if let Some(rest) = line.strip_prefix(b"--- ".as_slice()) {
                section.old_file_line = Some(index);
                if rest == b"/dev/null" {
                    // 旧侧不存在 → 这段是"新增文件"，路径只能从对侧取
                    section.is_new_file = true;
                } else {
                    section.old_path_raw = Some(rest.to_vec());
                }
            } else if let Some(rest) = line.strip_prefix(b"+++ ".as_slice()) {
                section.new_file_line = Some(index);
                if rest == b"/dev/null" {
                    section.is_deleted_file = true;
                } else {
                    section.new_path_raw = Some(rest.to_vec());
                }
            } else if line.starts_with(b"new file mode ") {
                section.new_mode_line = Some(index);
            } else if line.starts_with(b"deleted file mode ") {
                section.deleted_mode_line = Some(index);
            } else if line.starts_with(b"index ") {
                section.index_line = Some(index);
            } else if line.starts_with(b"Binary files ") || line.starts_with(b"GIT binary patch") {
                section.binary = true;
                section.header.push(index);
                index += 1;
                // 二进制载荷（zlib / base85 行）没有结构价值，整段跳过。
                while index < lines.len() && !lines[index].starts_with(b"diff --git ") {
                    index += 1;
                }
                break;
            }
            section.header.push(index);
            index += 1;
        }

        if section.binary {
            sections.push(section);
            continue;
        }

        // ---- hunk 体
        while index < lines.len() {
            if lines[index].starts_with(b"diff --git ") {
                break;
            }
            let Some((range, suffix)) = parse_hunk_header(lines[index]) else {
                // 头部区的杂项行（`similarity index` / `new file mode`…）已经在上面收走，
                // 走到这里说明是畸形行：跳过而不是让整个解析失败。
                index += 1;
                continue;
            };
            let header_index = index;
            index += 1;

            let mut body: Vec<usize> = Vec::new();
            let mut taken_old = 0_u32;
            let mut taken_new = 0_u32;
            // 按头部声明的行数"精确消费"：hunk 体里的一行内容自己可能以 `---` 开头，
            // 靠"看到 `---` 就当新文件"会切碎 hunk（与 T1.5 解析器同一纪律）。
            while index < lines.len()
                && (taken_old < range.old_lines || taken_new < range.new_lines)
            {
                match lines[index].first().copied() {
                    Some(b' ') => {
                        taken_old += 1;
                        taken_new += 1;
                    }
                    Some(b'+') => taken_new += 1,
                    Some(b'-') => taken_old += 1,
                    Some(b'\\') => {}
                    _ => break,
                }
                body.push(index);
                index += 1;
            }
            // 计数耗尽后 git 仍会输出结尾的 `\ No newline` 标记（两侧都无末尾换行时出现两次）
            while index < lines.len() && lines[index].starts_with(b"\\") {
                body.push(index);
                index += 1;
            }

            section.hunks.push(HunkView {
                header_index,
                range,
                suffix,
                body,
            });
        }

        sections.push(section);
    }

    sections
}

/// 解析 `@@ -a[,b] +c[,d] @@ suffix`。
fn parse_hunk_header(line: &[u8]) -> Option<(HunkRange, String)> {
    let rest = line.strip_prefix(b"@@ ".as_slice())?;
    let separator = find(rest, b" @@")?;
    let range_part = &rest[..separator];
    let suffix = String::from_utf8_lossy(&rest[separator + 3..]).into_owned();
    let space = range_part.iter().position(|byte| *byte == b' ')?;
    let old_range = range_part[..space].strip_prefix(b"-".as_slice())?;
    let new_range = range_part[space + 1..].strip_prefix(b"+".as_slice())?;

    let (old_start, old_lines) = parse_range(old_range)?;
    let (new_start, new_lines) = parse_range(new_range)?;
    Some((
        HunkRange {
            old_start,
            old_lines,
            new_start,
            new_lines,
        },
        suffix,
    ))
}

/// 解析 `a` 或 `a,b`（缺省计数 1，与 git 的省略规则一致）。
fn parse_range(bytes: &[u8]) -> Option<(u32, u32)> {
    let text = std::str::from_utf8(bytes).ok()?;
    match text.split_once(',') {
        Some((start, count)) => Some((start.parse().ok()?, count.parse().ok()?)),
        None => Some((text.parse().ok()?, 1)),
    }
}

/// 字节子串查找（标准库的 `find` 只对 `str` 有单模式版本）。
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{trim_patch, LineSelection, PatchDirection, StageScope};
    use crate::git::path::RepoPath;
    use crate::{AppResult, ErrorCode};

    /// 暂存方向（正向：把选中的改动写进目标）。
    fn trim_forward(patch: &[u8], scope: &StageScope) -> AppResult<Vec<u8>> {
        trim_patch(patch, scope, PatchDirection::Forward)
    }

    /// 取消暂存 / 丢弃方向（反向：把选中的改动从目标里撤掉）。
    fn trim_reverse(patch: &[u8], scope: &StageScope) -> AppResult<Vec<u8>> {
        trim_patch(patch, scope, PatchDirection::Reverse)
    }

    fn scope_hunks(indices: &[usize]) -> StageScope {
        StageScope::hunks(RepoPath::from("f.txt"), indices.to_vec())
    }

    fn scope_lines(selections: &[(usize, &[usize])]) -> StageScope {
        StageScope::lines(
            RepoPath::from("f.txt"),
            selections
                .iter()
                .map(|(hunk_index, lines)| LineSelection {
                    hunk_index: *hunk_index,
                    lines: lines.to_vec(),
                })
                .collect(),
        )
    }

    fn text(bytes: &[u8]) -> String {
        String::from_utf8_lossy(bytes).into_owned()
    }

    /// 两处改动的补丁（hunk 0 与 hunk 1 各改一行）。
    const TWO_HUNKS: &str = concat!(
        "diff --git a/f.txt b/f.txt\n",
        "index 1111111..2222222 100644\n",
        "--- a/f.txt\n",
        "+++ b/f.txt\n",
        "@@ -1,3 +1,3 @@ first\n",
        " a\n",
        "-b\n",
        "+B\n",
        " c\n",
        "@@ -10,3 +10,3 @@ second\n",
        " x\n",
        "-y\n",
        "+Y\n",
        " z\n",
    );

    #[test]
    fn hunk_scope_keeps_only_the_selected_hunk() {
        let trimmed = trim_forward(TWO_HUNKS.as_bytes(), &scope_hunks(&[1])).unwrap();
        let out = text(&trimmed);

        assert!(out.contains("@@ -10,3 +10,3 @@ second"), "{out}");
        assert!(out.contains("-y\n+Y\n"), "{out}");
        assert!(!out.contains("@@ -1,3 +1,3 @@ first"), "{out}");
        assert!(!out.contains("-b\n+B\n"), "{out}");
        // 头部必须保留：没有 `--- /+++` 的补丁 git 不会接受
        assert!(out.starts_with("diff --git a/f.txt b/f.txt\n"), "{out}");
        assert!(out.contains("--- a/f.txt\n+++ b/f.txt\n"), "{out}");
    }

    #[test]
    fn selecting_every_hunk_round_trips_byte_for_byte() {
        // 边界性质：全选 == 原补丁。裁剪器一旦在"全选"路径上改动字节，
        // 部分暂存就会在"用户其实想全部暂存"时产出与原文件不一致的索引。
        let trimmed = trim_forward(TWO_HUNKS.as_bytes(), &scope_hunks(&[0, 1])).unwrap();

        assert_eq!(trimmed, TWO_HUNKS.as_bytes());
    }

    #[test]
    fn selecting_only_added_lines_drops_the_unselected_removed_line() {
        let patch = concat!(
            "diff --git a/f.txt b/f.txt\n",
            "--- a/f.txt\n",
            "+++ b/f.txt\n",
            "@@ -1,2 +1,2 @@\n",
            "-old\n",
            "+new\n",
            " keep\n",
        );
        // 只选新增行 → 旧行必须变成上下文行（否则 git 会连旧行一起删掉）。
        // 新侧因此多出一行（旧行与新增行同时在索引里），hunk 头必须跟着变。
        let trimmed = trim_forward(patch.as_bytes(), &scope_lines(&[(0, &[1])])).unwrap();
        let out = text(&trimmed);

        assert!(out.contains(" old\n"), "删除行应转成上下文行：{out}");
        assert!(out.contains("+new\n"), "{out}");
        assert!(!out.contains("-old\n"), "{out}");
        assert!(
            out.contains("@@ -1,2 +1,3 @@"),
            "新侧应多出保留的旧行：{out}"
        );
    }

    #[test]
    fn unselected_added_lines_are_dropped_and_the_header_is_recounted() {
        let patch = concat!(
            "diff --git a/f.txt b/f.txt\n",
            "--- a/f.txt\n",
            "+++ b/f.txt\n",
            "@@ -1,2 +1,3 @@\n",
            " keep\n",
            "-old\n",
            "+new1\n",
            "+new2\n",
        );
        // 只选 `-old`：两条新增行都从补丁里消失，新侧行数从 3 变 1
        let trimmed = trim_forward(patch.as_bytes(), &scope_lines(&[(0, &[1])])).unwrap();
        let out = text(&trimmed);

        assert!(out.contains("@@ -1,2 +1,1 @@"), "hunk 头必须重算：{out}");
        assert!(out.contains("-old\n"), "{out}");
        assert!(!out.contains("+new1"), "{out}");
        assert!(!out.contains("new2"), "{out}");
    }

    #[test]
    fn an_unselected_removed_line_becomes_context_and_keeps_the_old_count() {
        let patch = concat!(
            "diff --git a/f.txt b/f.txt\n",
            "--- a/f.txt\n",
            "+++ b/f.txt\n",
            "@@ -1,2 +1,1 @@\n",
            "-old1\n",
            "-old2\n",
        );
        // 只删 old1：old2 转上下文，新侧因此有 1 行
        let trimmed = trim_forward(patch.as_bytes(), &scope_lines(&[(0, &[0])])).unwrap();
        let out = text(&trimmed);

        assert!(out.contains("@@ -1,2 +1,1 @@"), "{out}");
        assert!(out.contains("-old1\n"), "{out}");
        assert!(out.contains(" old2\n"), "{out}");
    }

    #[test]
    fn a_hunk_without_any_selected_change_is_dropped_entirely() {
        let trimmed = trim_forward(TWO_HUNKS.as_bytes(), &scope_lines(&[(0, &[])])).unwrap();

        assert!(
            trimmed.is_empty(),
            "选中行为空时不应产出补丁：{}",
            text(&trimmed)
        );
    }

    #[test]
    fn a_file_section_is_dropped_when_no_hunk_survives() {
        let trimmed = trim_forward(TWO_HUNKS.as_bytes(), &scope_hunks(&[])).unwrap();

        assert!(trimmed.is_empty());
    }

    #[test]
    fn selecting_only_context_lines_changes_nothing() {
        // 上下文行在两侧都存在，"选中"它没有语义；整块因此没有有效变更
        // （hunk 0 的 body 依次是：` a`(0)、`-b`(1)、`+B`(2)、` c`(3)）
        let trimmed = trim_forward(TWO_HUNKS.as_bytes(), &scope_lines(&[(0, &[0, 3])])).unwrap();

        assert!(trimmed.is_empty());
    }

    #[test]
    fn deleted_file_partially_staged_becomes_a_modification() {
        let patch = concat!(
            "diff --git a/gone.txt b/gone.txt\n",
            "deleted file mode 100644\n",
            "index 3333333..0000000\n",
            "--- a/gone.txt\n",
            "+++ /dev/null\n",
            "@@ -1,2 +0,0 @@\n",
            "-bye1\n",
            "-bye2\n",
        );
        // 只删第一行 → 文件不再被删除，头部必须跟着改
        let trimmed = trim_forward(patch.as_bytes(), &scope_lines(&[(0, &[0])])).unwrap();
        let out = text(&trimmed);

        assert!(!out.contains("deleted file mode"), "{out}");
        assert!(!out.contains("+++ /dev/null"), "{out}");
        assert!(out.contains("+++ b/gone.txt\n"), "{out}");
        assert!(
            !out.contains("index "),
            "index 行会让 git 以为整文件删除：{out}"
        );
        assert!(out.contains("@@ -1,2 +1,1 @@"), "新侧起点必须抬到 1：{out}");
        assert!(out.contains("-bye1\n"), "{out}");
        assert!(out.contains(" bye2\n"), "{out}");
    }

    #[test]
    fn deleting_every_line_keeps_the_deletion_header() {
        let patch = concat!(
            "diff --git a/gone.txt b/gone.txt\n",
            "deleted file mode 100644\n",
            "index 3333333..0000000\n",
            "--- a/gone.txt\n",
            "+++ /dev/null\n",
            "@@ -1,2 +0,0 @@\n",
            "-bye1\n",
            "-bye2\n",
        );
        let trimmed = trim_forward(patch.as_bytes(), &scope_lines(&[(0, &[0, 1])])).unwrap();

        assert_eq!(trimmed, patch.as_bytes(), "整文件删除应当逐字节相同");
    }

    #[test]
    fn added_file_partially_staged_keeps_the_new_file_header() {
        let patch = concat!(
            "diff --git a/new.txt b/new.txt\n",
            "new file mode 100644\n",
            "index 0000000..4444444\n",
            "--- /dev/null\n",
            "+++ b/new.txt\n",
            "@@ -0,0 +1,3 @@\n",
            "+one\n",
            "+two\n",
            "+three\n",
        );
        let trimmed = trim_forward(patch.as_bytes(), &scope_lines(&[(0, &[0, 2])])).unwrap();
        let out = text(&trimmed);

        assert!(out.contains("new file mode 100644"), "{out}");
        assert!(out.contains("--- /dev/null\n+++ b/new.txt\n"), "{out}");
        assert!(out.contains("@@ -0,0 +1,2 @@"), "{out}");
        assert!(out.contains("+one\n"), "{out}");
        assert!(!out.contains("+two\n"), "{out}");
        assert!(out.contains("+three\n"), "{out}");
    }

    #[test]
    fn no_newline_marker_follows_the_line_it_belongs_to() {
        let patch = concat!(
            "diff --git a/tail.txt b/tail.txt\n",
            "--- a/tail.txt\n",
            "+++ b/tail.txt\n",
            "@@ -1 +1 @@\n",
            "-old\n",
            "\\ No newline at end of file\n",
            "+new\n",
            "\\ No newline at end of file\n",
        );
        // 只选新增行（body 位置 2；位置 1 是它前面的标记行）：旧行的标记保留
        // （旧行转成了上下文行），新行的标记保留
        let trimmed = trim_forward(patch.as_bytes(), &scope_lines(&[(0, &[2])])).unwrap();
        let out = text(&trimmed);

        assert!(
            out.contains(" old\n\\ No newline at end of file\n"),
            "{out}"
        );
        assert!(
            out.contains("+new\n\\ No newline at end of file\n"),
            "{out}"
        );
        assert_eq!(out.matches("No newline").count(), 2, "{out}");

        // 只选删除行：新增行被丢弃，它后面的标记也必须一起消失
        let removed_only = trim_forward(patch.as_bytes(), &scope_lines(&[(0, &[0])])).unwrap();
        let removed_text = text(&removed_only);
        assert_eq!(
            removed_text.matches("No newline").count(),
            1,
            "被丢弃的 `+` 行不该留下标记：{removed_text}"
        );
        assert!(
            removed_text.contains("-old\n\\ No newline"),
            "{removed_text}"
        );
    }

    #[test]
    fn renamed_file_header_is_preserved() {
        let patch = concat!(
            "diff --git a/old.txt b/new.txt\n",
            "similarity index 76%\n",
            "rename from old.txt\n",
            "rename to new.txt\n",
            "index 5555555..6666666 100644\n",
            "--- a/old.txt\n",
            "+++ b/new.txt\n",
            "@@ -1,2 +1,2 @@\n",
            "-a\n",
            "+A\n",
            " b\n",
        );
        let trimmed = trim_forward(patch.as_bytes(), &scope_lines(&[(0, &[1])])).unwrap();
        let out = text(&trimmed);

        assert!(
            out.contains("rename from old.txt\nrename to new.txt\n"),
            "{out}"
        );
        assert!(out.contains("similarity index 76%"), "{out}");
        assert!(out.contains(" a\n"), "未选的删除行应转上下文：{out}");
    }

    #[test]
    fn trimmed_context_lines_keep_their_original_bytes() {
        // 非 UTF-8 的内容行必须原样搬运：经过 lossy 解码后补丁必然无法应用
        let mut patch: Vec<u8> = Vec::new();
        patch.extend_from_slice(
            b"diff --git a/bin.txt b/bin.txt\n--- a/bin.txt\n+++ b/bin.txt\n@@ -1,2 +1,2 @@\n",
        );
        patch.extend_from_slice(b"-bad \xff\xfe line\n");
        patch.extend_from_slice(b"+good \xff\xfe line\n");
        patch.extend_from_slice(" 保留中文\n".as_bytes());

        let trimmed = trim_forward(&patch, &scope_lines(&[(0, &[1])])).unwrap();

        assert!(
            trimmed.windows(2).any(|window| window == [0xFF, 0xFE]),
            "非 UTF-8 字节必须原样保留：{:?}",
            trimmed
        );
        assert!(
            trimmed.windows(2).any(|window| window == [0xE4, 0xBF]),
            "中文应原样保留"
        );
    }

    #[test]
    fn whole_file_scope_is_rejected_because_it_does_not_use_patches() {
        let error = trim_forward(
            TWO_HUNKS.as_bytes(),
            &StageScope::Files(vec!["f.txt".into()]),
        )
        .expect_err("整文件粒度不该走补丁裁剪");

        assert_eq!(error.code, ErrorCode::Validation);
    }

    #[test]
    fn binary_sections_are_rejected_with_the_path_in_the_hint() {
        let patch = concat!(
            "diff --git a/blob.bin b/blob.bin\n",
            "index 1111111..2222222 100644\n",
            "Binary files a/blob.bin and b/blob.bin differ\n",
        );
        let error =
            trim_forward(patch.as_bytes(), &scope_hunks(&[0])).expect_err("二进制不支持行级");

        assert_eq!(error.code, ErrorCode::Validation);
        assert_eq!(error.hint.as_deref(), Some("f.txt"));
    }

    #[test]
    fn out_of_range_selections_are_rejected_instead_of_silently_ignored() {
        let hunk_error = trim_forward(TWO_HUNKS.as_bytes(), &scope_hunks(&[7]))
            .expect_err("hunk 下标越界必须报错");
        assert_eq!(hunk_error.code, ErrorCode::Validation);

        let line_error = trim_forward(TWO_HUNKS.as_bytes(), &scope_lines(&[(0, &[99])]))
            .expect_err("行下标越界必须报错");
        assert_eq!(line_error.code, ErrorCode::Validation);
    }

    #[test]
    fn a_patch_with_several_sections_is_rejected() {
        let patch = concat!(
            "diff --git a/f.txt b/f.txt\n",
            "--- a/f.txt\n",
            "+++ b/f.txt\n",
            "@@ -1 +1 @@\n",
            "-a\n",
            "+A\n",
            "diff --git a/g.txt b/g.txt\n",
            "--- a/g.txt\n",
            "+++ b/g.txt\n",
            "@@ -1 +1 @@\n",
            "-b\n",
            "+B\n",
        );
        let error =
            trim_forward(patch.as_bytes(), &scope_hunks(&[0])).expect_err("多段补丁必须拒绝");

        assert_eq!(error.code, ErrorCode::Validation);
    }

    #[test]
    fn an_empty_patch_is_rejected() {
        let error = trim_forward(b"", &scope_hunks(&[0])).expect_err("空补丁没有文件段");

        assert_eq!(error.code, ErrorCode::Validation);
    }

    #[test]
    fn content_that_looks_like_a_header_does_not_split_the_hunk() {
        // 删除了一行 "--- 危险内容"：hunk 体里的 `---` 不是新文件头
        let patch = concat!(
            "diff --git a/tricky.txt b/tricky.txt\n",
            "--- a/tricky.txt\n",
            "+++ b/tricky.txt\n",
            "@@ -1,2 +1,2 @@\n",
            "--- 危险内容\n",
            "+safe now\n",
        );
        let trimmed = trim_forward(patch.as_bytes(), &scope_lines(&[(0, &[1])])).unwrap();
        let out = text(&trimmed);

        // 补丁标记是 `-`，内容本身是 `-- 危险内容`；转成上下文后是"空格 + 内容"
        assert!(
            out.contains(" -- 危险内容\n"),
            "该行应转成上下文行：{out:?}"
        );
        assert!(out.contains("+safe now\n"), "{out}");
    }

    #[test]
    fn hunk_headers_without_counts_are_recounted_in_the_explicit_form() {
        let patch = concat!(
            "diff --git a/one.txt b/one.txt\n",
            "--- a/one.txt\n",
            "+++ b/one.txt\n",
            "@@ -2 +2 @@\n",
            "-old\n",
            "+new\n",
        );
        let trimmed = trim_forward(patch.as_bytes(), &scope_lines(&[(0, &[1])])).unwrap();

        // 省略计数（`-2 +2`）的头部在裁剪后写成显式形式；
        // 未选的删除行转上下文 → 新侧两行（保留的旧行 + 新增行）
        assert!(
            text(&trimmed).contains("@@ -2,1 +2,2 @@"),
            "{}",
            text(&trimmed)
        );
    }

    #[test]
    fn trimming_keeps_the_trailing_newline_of_the_patch() {
        let trimmed = trim_forward(TWO_HUNKS.as_bytes(), &scope_hunks(&[0, 1])).unwrap();

        assert_eq!(trimmed.last(), Some(&b'\n'));
    }

    // ------------------------------------------------------------ 反向（取消暂存 / 丢弃）

    /// 一个"把 two 改成 TWO"的最小补丁。
    const REPLACEMENT: &str = concat!(
        "diff --git a/f.txt b/f.txt\n",
        "--- a/f.txt\n",
        "+++ b/f.txt\n",
        "@@ -1,3 +1,3 @@\n",
        " one\n",
        "-two\n",
        "+TWO\n",
        " three\n",
    );

    #[test]
    fn reverse_trimming_drops_unselected_removed_lines_because_the_target_lacks_them() {
        // 反向应用的目标内容是新侧（`one, TWO, three`）：未选中的 `-two` 在目标里
        // 不存在，因此必须从补丁里移除，否则 git 会因"上下文对不上"直接拒绝。
        let trimmed = trim_reverse(REPLACEMENT.as_bytes(), &scope_lines(&[(0, &[2])])).unwrap();
        let out = text(&trimmed);

        assert!(!out.contains("-two\n"), "未选中的删除行必须移除：{out}");
        assert!(out.contains("+TWO\n"), "{out}");
        assert!(out.contains("@@ -1,2 +1,3 @@"), "行数必须按输出重算：{out}");
    }

    #[test]
    fn reverse_trimming_turns_unselected_added_lines_into_context() {
        // 只撤销 `-two`（把 two 加回索引）：`+TWO` 的"添加"没被撤销，因此它在补丁的
        // 两侧都存在 → 转上下文。旧侧随之多出这一行（HEAD 里没有 TWO，但撤销后的
        // 索引里有），所以 old 侧行数是 4 而不是 3。
        let trimmed = trim_reverse(REPLACEMENT.as_bytes(), &scope_lines(&[(0, &[1])])).unwrap();
        let out = text(&trimmed);

        assert!(out.contains(" TWO\n"), "未选中的新增行应转上下文：{out}");
        assert!(out.contains("-two\n"), "{out}");
        assert!(out.contains("@@ -1,4 +1,3 @@"), "{out}");
    }

    #[test]
    fn reverse_trimming_of_an_added_file_drops_the_new_file_header_when_partially_reverted() {
        let patch = concat!(
            "diff --git a/new.txt b/new.txt\n",
            "new file mode 100644\n",
            "index 0000000..4444444\n",
            "--- /dev/null\n",
            "+++ b/new.txt\n",
            "@@ -0,0 +1,3 @@\n",
            "+one\n",
            "+two\n",
            "+three\n",
        );
        // 撤销第 1、3 行的暂存，保留第 2 行：索引里仍有内容，
        // 因此它不再是"新增文件"，`--- /dev/null` 与 `new file mode` 会误导 git
        let trimmed = trim_reverse(patch.as_bytes(), &scope_lines(&[(0, &[0, 2])])).unwrap();
        let out = text(&trimmed);

        assert!(!out.contains("new file mode"), "{out}");
        assert!(!out.contains("index "), "{out}");
        assert!(!out.contains("--- /dev/null"), "{out}");
        assert!(out.contains("--- a/new.txt\n+++ b/new.txt\n"), "{out}");
        assert!(out.contains("@@ -1,1 +1,3 @@"), "{out}");
        assert!(out.contains(" two\n"), "保留的那行应转上下文：{out}");
    }

    #[test]
    fn reverse_trimming_of_an_added_file_round_trips_when_everything_is_reverted() {
        let patch = concat!(
            "diff --git a/new.txt b/new.txt\n",
            "new file mode 100644\n",
            "index 0000000..4444444\n",
            "--- /dev/null\n",
            "+++ b/new.txt\n",
            "@@ -0,0 +1,2 @@\n",
            "+one\n",
            "+two\n",
        );
        let trimmed = trim_reverse(patch.as_bytes(), &scope_lines(&[(0, &[0, 1])])).unwrap();

        assert_eq!(
            trimmed,
            patch.as_bytes(),
            "整文件撤销应逐字节相同（此时反向应用才等于删除该索引条目）"
        );
    }

    #[test]
    fn reverse_trimming_of_a_deleted_file_keeps_the_deletion_header() {
        let patch = concat!(
            "diff --git a/gone.txt b/gone.txt\n",
            "deleted file mode 100644\n",
            "index 3333333..0000000\n",
            "--- a/gone.txt\n",
            "+++ /dev/null\n",
            "@@ -1,3 +0,0 @@\n",
            "-a\n",
            "-b\n",
            "-c\n",
        );
        // 撤销 a、b 的删除（保留 c 的删除）：新侧仍为空 → 头部不必改写，
        // 反向应用的结果是"索引里重新出现 a、b"
        let trimmed = trim_reverse(patch.as_bytes(), &scope_lines(&[(0, &[0, 1])])).unwrap();
        let out = text(&trimmed);

        assert!(out.contains("deleted file mode 100644"), "{out}");
        assert!(out.contains("+++ /dev/null"), "{out}");
        assert!(out.contains("@@ -1,2 +0,0 @@"), "{out}");
        assert!(out.contains("-a\n"), "{out}");
        assert!(!out.contains("-c\n"), "未选中的删除行必须移除：{out}");
    }

    #[test]
    fn direction_only_changes_how_unselected_lines_are_treated() {
        // 同一份补丁、同一组选择，两个方向必须产出不同的 body（这正是"镜像"的含义）。
        // 这条断言的价值是：将来有人"顺手把两个方向合并成一个"，这里会立刻失败。
        let selection = scope_lines(&[(0, &[2])]);
        let forward = trim_forward(REPLACEMENT.as_bytes(), &selection).unwrap();
        let reverse = trim_reverse(REPLACEMENT.as_bytes(), &selection).unwrap();

        assert_ne!(forward, reverse);
        assert!(text(&forward).contains(" two\n"), "{}", text(&forward));
        assert!(!text(&reverse).contains(" two\n"), "{}", text(&reverse));
    }
}
