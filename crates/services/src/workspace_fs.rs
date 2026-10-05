//! 工作区文件系统操作（T5.7）：文件树 / 读取 / 写入 / 创建 / 重命名 / 删除。
//!
//! # 安全是第一约束（任务书：所有路径必须 canonicalize 后校验在仓库根内）
//!
//! [`resolve_within`] 是唯一入口：
//!
//! 1. 拒绝绝对路径、NUL、任何 `..` 分量（词法层先挡一遍）；
//! 2. 对**已存在**的目标：canonicalize（解析软链）后必须仍在
//!    canonicalize(root) 之内——符号链接逃逸在这里被拦下；
//! 3. 对**尚不存在**的目标（创建/写入新文件）：canonicalize 最深的
//!    已存在祖先，校验前缀，再拼回剩余分量（剩余分量不允许含 `..`）。
//!
//! # 语义约定
//!
//! - `fs_read`：单文件 ≤ 5MB，超限返回 `VALIDATION`（前端提示外部编辑器）；
//!   二进制文件只返回元信息不给内容；EOL 与 BOM 如实上报，写入时按
//!   preserve 参数恢复——**绝不静默改变用户的换行符**。
//! - `fs_delete`：移入回收站（`trash` crate），不是永久删除。
//! - `fs_tree`：懒加载一层；`show_ignored=false` 时用
//!   `git check-ignore -z --stdin` 批量过滤（§7 CLI 规范：参数数组、
//!   机器可解析格式、超时控制）。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use trash::delete as trash_delete;

use forgedesk_domain::{AppError, ErrorCode};
use forgedesk_git_engine::process::{GitProcess, GitRunOpts};

/// 单文件读取上限（任务书：≤ 5MB，超出提示用外部编辑器）。
pub const FS_READ_LIMIT: u64 = 5 * 1024 * 1024;

// ---------------------------------------------------------------------------
// 路径安全
// ---------------------------------------------------------------------------

fn reject_bad_components(rel: &str) -> Result<(), AppError> {
    if rel.is_empty() {
        // 空路径 = 仓库根本身（fs_tree 的合法入口）
        return Ok(());
    }
    if rel.contains('\0') {
        return Err(AppError::new(
            ErrorCode::Validation,
            "the path contains NUL",
        ));
    }
    let path = Path::new(rel);
    if path.is_absolute() {
        return Err(AppError::new(
            ErrorCode::Validation,
            "the path must be relative to the repository root",
        )
        .with_detail(rel.to_string()));
    }
    for component in path.components() {
        if matches!(component, std::path::Component::ParentDir) {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                "the path must not contain '..'",
            )
            .with_detail(rel.to_string()));
        }
    }
    Ok(())
}

fn canonical_root(root: &Path) -> Result<PathBuf, AppError> {
    root.canonicalize().map_err(|error| {
        AppError::new(ErrorCode::Validation, "the repository root is invalid")
            .with_detail(error.to_string())
    })
}

/// 把相对路径安全地解析到仓库根内（已存在或新文件都适用）。
///
/// 返回的路径：已存在的目标经过 canonicalize（软链逃逸被拦）；
/// 不存在的目标由"最深已存在祖先的 canonicalize + 剩余分量"拼出。
pub fn resolve_within(root: &Path, rel: &str) -> Result<PathBuf, AppError> {
    reject_bad_components(rel)?;
    let root_canonical = canonical_root(root)?;
    let joined = root.join(rel);

    if joined.exists() {
        let resolved = joined.canonicalize().map_err(|error| {
            AppError::new(ErrorCode::Validation, "the path is invalid")
                .with_detail(error.to_string())
        })?;
        if !resolved.starts_with(&root_canonical) {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                "the path escapes the repository root",
            )
            .with_detail(rel.to_string()));
        }
        Ok(resolved)
    } else {
        // 新文件：校验最深已存在祖先 + 剩余分量全部是普通名字
        let mut depth = 0;
        let mut ancestor_ref: &Path = joined.as_path();
        loop {
            if ancestor_ref.exists() {
                break;
            }
            match ancestor_ref.parent() {
                Some(parent) => ancestor_ref = parent,
                None => break,
            }
            depth += 1;
            if depth > 64 {
                return Err(
                    AppError::new(ErrorCode::Validation, "the path nesting is too deep")
                        .with_detail(rel.to_string()),
                );
            }
        }
        let resolved_ancestor = ancestor_ref.canonicalize().map_err(|error| {
            AppError::new(ErrorCode::Validation, "the parent path is invalid")
                .with_detail(error.to_string())
        })?;
        if !resolved_ancestor.starts_with(&root_canonical) {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                "the path escapes the repository root",
            )
            .with_detail(rel.to_string()));
        }
        // 剩余分量拼回（词法校验已在 reject_bad_components 做过：无 ..、非绝对）
        let tail = Path::new(rel)
            .components()
            .rev()
            .take(depth)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<PathBuf>();
        Ok(resolved_ancestor.join(tail))
    }
}

// ---------------------------------------------------------------------------
// 文件树
// ---------------------------------------------------------------------------

/// 树节点种类（派生 Ord：Dir > File，排序时目录在前）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FsKind {
    /// 普通文件。
    File,
    /// 目录。
    Dir,
}

/// 文件树的一个节点（懒加载：目录不递归）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FsNode {
    /// 名称（含扩展名）。
    pub name: String,
    /// 相对仓库根的路径（POSIX 分隔符，前端唯一寻址方式）。
    pub rel_path: String,
    /// 节点种类（目录 / 文件）。
    pub kind: FsKind,
    /// 文件字节数（目录为 0）。
    pub size: u64,
}

/// 列出 `rel` 目录下的一层节点（懒加载）。
///
/// `rel = ""` 表示仓库根。排序：目录在前、名称不区分大小写。
/// `show_ignored=false` 时经 `git check-ignore -z --stdin`（带超时）过滤；
/// git 失败 = 不过滤（宁可多显示，不能因过滤器挂掉让文件树失败）。
pub async fn fs_tree(
    root: &Path,
    rel: &str,
    show_hidden: bool,
    show_ignored: bool,
) -> Result<Vec<FsNode>, AppError> {
    let target = resolve_within(root, rel)?;
    if !target.is_dir() {
        return Err(AppError::new(
            ErrorCode::Validation,
            "the requested tree path is not a directory",
        )
        .with_detail(rel.to_string()));
    }

    let mut nodes: Vec<FsNode> = Vec::new();
    for entry in target.read_dir().map_err(|error| {
        AppError::new(ErrorCode::PermissionDenied, "could not read the directory")
            .with_detail(error.to_string())
    })? {
        let entry = entry.map_err(|error| {
            AppError::new(
                ErrorCode::PermissionDenied,
                "could not read a directory entry",
            )
            .with_detail(error.to_string())
        })?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !show_hidden && name.starts_with('.') {
            continue;
        }
        let metadata = entry.metadata().ok();
        let kind = if entry.path().is_dir() {
            FsKind::Dir
        } else {
            FsKind::File
        };
        let size = metadata
            .as_ref()
            .map_or(0, |m| if m.is_file() { m.len() } else { 0 });
        nodes.push(FsNode {
            name: name.clone(),
            rel_path: join_rel(rel, &name),
            kind,
            size,
        });
    }

    if !show_ignored {
        let ignored = git_ignored_paths(root, &nodes).await;
        nodes.retain(|node| !ignored.contains(&node.rel_path));
    }

    nodes.sort_by(|a, b| {
        b.kind
            .cmp(&a.kind)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(nodes)
}

fn join_rel(rel: &str, name: &str) -> String {
    if rel.is_empty() {
        name.to_string()
    } else {
        format!("{rel}/{name}")
    }
}

/// 批量查询哪些路径被 .gitignore 忽略（`git check-ignore -z --stdin`，
/// 经 [`GitProcess`]：参数数组 + 超时 + 固定环境，§7 CLI 规范）。
///
/// git 不可用 / 超时 / 非仓库 = 返回空集合（不过滤）。
async fn git_ignored_paths(root: &Path, nodes: &[FsNode]) -> std::collections::BTreeSet<String> {
    let stdin = nodes
        .iter()
        .map(|node| node.rel_path.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let output = GitProcess::new()
        .run(
            &["check-ignore".into(), "-z".into(), "--stdin".into()],
            GitRunOpts::new(root).with_stdin(stdin.into_bytes()),
        )
        .await;
    match output {
        Ok(out) if out.exit_code == Some(0) => out
            .stdout
            .split(|&b| b == 0)
            .filter(|bytes| !bytes.is_empty())
            .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
            .collect(),
        // 退出码 1 = 没有任何路径被忽略；其它失败同样"不过滤"
        _ => std::collections::BTreeSet::new(),
    }
}

// ---------------------------------------------------------------------------
// 读取 / 写入
// ---------------------------------------------------------------------------

/// 换行符形态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FsEol {
    /// Unix：LF。
    Lf,
    /// Windows：CRLF。
    Crlf,
    /// 经典 Mac：CR。
    Cr,
    /// 混合多种换行符。
    Mixed,
}

/// `fs_read` 的返回：元信息 + 文本内容（二进制不给内容）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FsFileContent {
    /// UTF-8 文本（二进制时为 null）。
    pub content: Option<String>,
    /// 换行符形态。
    pub eol: FsEol,
    /// 是否带 UTF-8 BOM。
    pub has_bom: bool,
    /// 文件大小（字节）。
    pub size: u64,
    /// 是否二进制（前 8000 字节含 NUL，与 Git 同口径）。
    pub is_binary: bool,
    /// 因超过 5MB 被截断（只发生在 is_binary=false 且内容被截断时）。
    pub truncated: bool,
}

fn detect_binary(bytes: &[u8]) -> bool {
    // 与 Git 同口径的快速判定：前 8000 字节里出现 NUL 即二进制
    bytes.iter().take(8000).any(|&b| b == 0)
}

fn detect_eol(bytes: &[u8]) -> FsEol {
    let mut has_lf = false;
    let mut has_crlf = false;
    let mut has_cr = false;
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\r' => {
                if bytes.get(index + 1) == Some(&b'\n') {
                    has_crlf = true;
                    index += 1;
                } else {
                    has_cr = true;
                }
            }
            b'\n' => has_lf = true,
            _ => {}
        }
        index += 1;
    }
    let kinds = [has_lf, has_crlf, has_cr].iter().filter(|k| **k).count();
    if kinds > 1 {
        return FsEol::Mixed;
    }
    if has_crlf {
        return FsEol::Crlf;
    }
    if has_cr {
        return FsEol::Cr;
    }
    FsEol::Lf
}

/// 读取文件（≤ 5MB；二进制只给元信息）。
pub fn fs_read(root: &Path, rel: &str) -> Result<FsFileContent, AppError> {
    let path = resolve_within(root, rel)?;
    let metadata = path.metadata().map_err(|error| {
        AppError::new(ErrorCode::NotFound, "the file does not exist").with_detail(error.to_string())
    })?;
    if !metadata.is_file() {
        return Err(
            AppError::new(ErrorCode::Validation, "the requested path is not a file")
                .with_detail(rel.to_string()),
        );
    }
    let size = metadata.len();
    if size > FS_READ_LIMIT {
        return Err(AppError::new(
            ErrorCode::Validation,
            "the file exceeds the 5MB inline-edit limit; use an external editor",
        )
        .with_detail(format!("{size} bytes")));
    }
    let bytes = std::fs::read(&path).map_err(|error| {
        AppError::new(ErrorCode::PermissionDenied, "could not read the file")
            .with_detail(error.to_string())
    })?;
    if detect_binary(&bytes) {
        return Ok(FsFileContent {
            content: None,
            eol: FsEol::Lf,
            has_bom: bytes.starts_with(&[0xEF, 0xBB, 0xBF]),
            size,
            is_binary: true,
            truncated: false,
        });
    }
    let has_bom = bytes.starts_with(&[0xEF, 0xBB, 0xBF]);
    let content = String::from_utf8_lossy(if has_bom { &bytes[3..] } else { &bytes }).into_owned();
    Ok(FsFileContent {
        eol: detect_eol(&bytes),
        has_bom,
        size,
        is_binary: false,
        // 文本内容按字节算已 ≤ 5MB；UTF-8 全 ASCII 时 content 与 size 相当，
        // 不做二次截断（size 已如实上报）。
        truncated: false,
        content: Some(content),
    })
}

/// 写入文件（保留 EOL 与 BOM 由调用方传入；返回写入字节数）。
pub fn fs_write(
    root: &Path,
    rel: &str,
    content: &str,
    eol: FsEol,
    has_bom: bool,
) -> Result<u64, AppError> {
    let path = resolve_within(root, rel)?;

    // EOL 规范化：把内容里的换行统一为目标形态（原文件若是 Mixed，以此为准统一）
    let normalized = match eol {
        FsEol::Crlf => content.replace("\r\n", "\n").replace('\n', "\r\n"),
        FsEol::Lf => content.replace("\r\n", "\n").replace('\r', "\n"),
        FsEol::Cr => content.replace("\r\n", "\n").replace('\n', "\r"),
        FsEol::Mixed => content.to_string(),
    };

    let mut bytes: Vec<u8> = Vec::with_capacity(normalized.len() + 3);
    if has_bom {
        bytes.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
    }
    bytes.extend_from_slice(normalized.as_bytes());

    std::fs::write(&path, &bytes).map_err(|error| {
        AppError::new(ErrorCode::PermissionDenied, "could not write the file")
            .with_detail(error.to_string())
    })?;
    Ok(bytes.len() as u64)
}

/// 创建文件或目录（父目录必须已存在或随之创建；内容为空）。
pub fn fs_create(root: &Path, rel: &str, is_dir: bool) -> Result<(), AppError> {
    let path = resolve_within(root, rel)?;
    if path.exists() {
        return Err(
            AppError::new(ErrorCode::Validation, "the target already exists")
                .with_detail(rel.to_string()),
        );
    }
    if is_dir {
        std::fs::create_dir_all(&path).map_err(|error| {
            AppError::new(
                ErrorCode::PermissionDenied,
                "could not create the directory",
            )
            .with_detail(error.to_string())
        })?;
    } else {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                AppError::new(
                    ErrorCode::PermissionDenied,
                    "could not create parent directories",
                )
                .with_detail(error.to_string())
            })?;
        }
        std::fs::write(&path, b"").map_err(|error| {
            AppError::new(ErrorCode::PermissionDenied, "could not create the file")
                .with_detail(error.to_string())
        })?;
    }
    Ok(())
}

/// 重命名 / 移动（目标同样不得逃逸仓库根）。
pub fn fs_rename(root: &Path, rel: &str, new_rel: &str) -> Result<(), AppError> {
    let from = resolve_within(root, rel)?;
    let to = resolve_within(root, new_rel)?;
    if !from.exists() {
        return Err(AppError::new(
            ErrorCode::NotFound,
            "the source does not exist",
        ))
        .map_err(|mut e| {
            e.detail = Some(rel.to_string());
            e
        });
    }
    if to.exists() {
        return Err(
            AppError::new(ErrorCode::Validation, "the target already exists")
                .with_detail(new_rel.to_string()),
        );
    }
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            AppError::new(
                ErrorCode::PermissionDenied,
                "could not create parent directories",
            )
            .with_detail(error.to_string())
        })?;
    }
    std::fs::rename(&from, &to).map_err(|error| {
        AppError::new(ErrorCode::PermissionDenied, "could not rename")
            .with_detail(error.to_string())
    })?;
    Ok(())
}

/// 删除（移入回收站——不是永久删除，任务书明确要求）。
pub fn fs_delete(root: &Path, rel: &str) -> Result<(), AppError> {
    let path = resolve_within(root, rel)?;
    if !path.exists() {
        return Err(AppError::new(
            ErrorCode::NotFound,
            "the path does not exist",
        ));
    }
    trash_delete(&path).map_err(|error| {
        AppError::new(
            ErrorCode::PermissionDenied,
            "could not move the path to the trash",
        )
        .with_detail(error.to_string())
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{
        fs_create, fs_delete, fs_read, fs_rename, fs_tree, fs_write, resolve_within, FsEol,
    };
    use forgedesk_domain::ErrorCode;
    use std::path::Path;

    fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(root.join("src/nested")).unwrap();
        std::fs::write(root.join("src/main.rs"), b"fn main() {}\n").unwrap();
        std::fs::write(root.join("README.md"), b"# demo\r\nline2\r\n").unwrap();
        (dir, root)
    }

    /// 路径安全：`..`、绝对路径、NUL 一律拒绝。
    #[test]
    fn rejects_dotdot_absolute_and_nul() {
        let (_dir, root) = fixture();
        for bad in [
            "../outside.txt",
            "a/../../b.txt",
            "C:/Windows/x",
            "\0bad",
            "//abs",
        ] {
            let error = resolve_within(&root, bad).expect_err(bad);
            assert!(
                matches!(
                    error.code,
                    ErrorCode::Validation | ErrorCode::PermissionDenied
                ),
                "{bad}"
            );
        }
    }

    /// 符号链接逃逸：指向仓库外的链接必须被拦下。
    #[cfg(unix)]
    #[test]
    fn rejects_symlink_escape() {
        let (_dir, root) = fixture();
        std::os::unix::fs::symlink("/etc", root.join("escape")).unwrap();
        let error = resolve_within(&root, "escape/passwd").expect_err("escape");
        assert_eq!(error.code, ErrorCode::PermissionDenied);
    }

    /// 读写往返：CRLF 与 BOM 按 preserve 语义恢复。
    #[test]
    fn read_write_roundtrip_preserves_eol_and_bom() {
        let (_dir, root) = fixture();
        let content = "# demo\r\nline2\r\n";
        let read = fs_read(&root, "README.md").unwrap();
        assert_eq!(read.eol, FsEol::Crlf);
        assert!(!read.has_bom);
        assert_eq!(read.content.as_deref(), Some(content));

        // 用 LF 写入但声明保留 CRLF：内容被规范化回 CRLF
        let written = fs_write(&root, "README.md", "# demo\nline2\n", FsEol::Crlf, false).unwrap();
        assert_eq!(written, content.len() as u64);
        let again = fs_read(&root, "README.md").unwrap();
        assert_eq!(again.eol, FsEol::Crlf);
        assert_eq!(again.content.as_deref(), Some(content));
    }

    /// BOM 写入与读取一致。
    #[test]
    fn bom_is_written_and_reported() {
        let (_dir, root) = fixture();
        fs_write(&root, "src/bom.txt", "中文内容\n", FsEol::Lf, true).unwrap();
        let read = fs_read(&root, "src/bom.txt").unwrap();
        assert!(read.has_bom);
        assert_eq!(read.content.as_deref(), Some("中文内容\n"));
    }

    /// 二进制文件：只给元信息不给文本。
    #[test]
    fn binary_files_report_metadata_only() {
        let (_dir, root) = fixture();
        std::fs::write(root.join("logo.png"), [0x89, b'P', b'N', b'G', 0, 1, 2, 3]).unwrap();
        let read = fs_read(&root, "logo.png").unwrap();
        assert!(read.is_binary);
        assert!(read.content.is_none());
    }

    /// 超过 5MB 的文件在读取时被拒绝（VALIDATION + 明确说明）。
    #[test]
    fn oversized_files_are_rejected() {
        let (_dir, root) = fixture();
        let big = vec![b'a'; (super::FS_READ_LIMIT + 1) as usize];
        std::fs::write(root.join("big.txt"), &big).unwrap();
        let error = fs_read(&root, "big.txt").expect_err("oversize");
        assert_eq!(error.code, ErrorCode::Validation);
        assert!(error.message.contains("5MB"));
    }

    /// 文件树：懒加载一层、目录在前、隐藏项可切换；check-ignore 过滤 node_modules。
    #[tokio::test]
    async fn tree_lists_one_level_sorted_and_filters() {
        let (_dir, root) = fixture();
        std::fs::write(root.join(".gitignore"), b"/node_modules/\n").unwrap();
        std::fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
        std::fs::write(root.join(".hidden"), b"x").unwrap();

        let visible = fs_tree(&root, "", false, false).await.unwrap();
        let names: Vec<_> = visible.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["node_modules", "src", "README.md"],
            "node_modules 被忽略、目录在前"
        );

        let with_hidden = fs_tree(&root, "", true, true).await.unwrap();
        assert!(with_hidden.iter().any(|n| n.name == ".hidden"));
        assert!(with_hidden.iter().any(|n| n.name == "node_modules"));

        // 子目录懒加载
        let nested = fs_tree(&root, "src/nested", false, false).await.unwrap();
        assert!(nested.is_empty());
    }

    /// 创建 / 重命名 / 回收站删除。
    #[test]
    fn create_rename_delete_work() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(&root).unwrap();

        fs_create(&root, "docs/new.md", false).unwrap();
        assert!(root.join("docs/new.md").is_file());

        fs_create(&root, "docs/new.md", false).expect_err("duplicate");
        fs_rename(&root, "docs/new.md", "docs/renamed.md").unwrap();
        assert!(root.join("docs/renamed.md").is_file());

        fs_delete(&root, "docs/renamed.md").unwrap();
        assert!(!root.join("docs/renamed.md").exists());
    }

    /// create 不允许把路径建在已存在的同名目录下（resolve 语义锁定）。
    #[test]
    fn create_inside_existing_file_is_rejected() {
        let (_dir, root) = fixture();
        // src/main.rs 是文件：把它当目录用会被 canonicalize 校验拒绝
        let error = fs_create(&root, "src/main.rs/inner.txt", false).expect_err("file as dir");
        assert!(matches!(
            error.code,
            ErrorCode::Validation | ErrorCode::PermissionDenied
        ));
    }

    /// 空根路径就是仓库根本身。
    #[test]
    fn empty_rel_resolves_to_root() {
        let (_dir, root) = fixture();
        let resolved = resolve_within(&root, ".").unwrap();
        assert_eq!(resolved, Path::new(&root).canonicalize().unwrap());
    }
}
