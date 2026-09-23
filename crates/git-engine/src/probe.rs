//! 对 `.git` 目录与工作区做少量文件系统探测（不启动任何进程）。
//!
//! 这里的每一项都是"git 的命令行没有便宜问法"的事实：问 `git config` 只能
//! 知道**配置**里有没有 LFS filter（装了 `git lfs` 的机器上人人都有），
//! 而"这个仓库是否真的用 LFS"要看它自己的 `.gitattributes` 与对象目录。
//!
//! 探测只读、不写、不执行：它永远不会因为一个陌生仓库的配置而跑起命令
//! （见 `domain::git::audit` 的威胁模型）。

use std::path::Path;

/// 判定仓库是否使用 Git LFS。
///
/// 两个依据，命中任一即为真：
///
/// 1. `workdir/.gitattributes` 里出现 `filter=lfs`——这是**声明**，
///    说明该仓库期望某些文件走 LFS；
/// 2. `<git_dir>/lfs` 目录存在——这是**事实**，说明确实取过 LFS 对象。
///
/// 只看仓库根的 `.gitattributes`：子目录里的声明同样生效，但为了一个提示性
/// 字段去遍历整棵树不划算（`is_lfs` 不参与任何数据完整性判断，见领域层文档）。
pub fn detect_lfs(git_dir: &Path, workdir: Option<&Path>) -> bool {
    if git_dir.join("lfs").is_dir() {
        return true;
    }

    let Some(workdir) = workdir else {
        // 裸仓库没有工作区，也就没有 `.gitattributes` 可看
        return false;
    };

    let Ok(contents) = std::fs::read(workdir.join(".gitattributes")) else {
        return false;
    };
    // `.gitattributes` 允许任意编码（同仓库里的其它文本文件）
    let contents = String::from_utf8_lossy(&contents);
    contents
        .lines()
        .map(|line| line.trim())
        .any(|line| !line.starts_with('#') && line.contains("filter=lfs"))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::detect_lfs;
    use std::path::Path;

    /// 建一个临时目录；测试结束由 `TempDir` 自行清理。
    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("forgedesk-probe-{label}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }

        fn write(&self, relative: &str, contents: &str) {
            let path = self.0.join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(path, contents).unwrap();
        }

        fn mkdir(&self, relative: &str) {
            std::fs::create_dir_all(self.0.join(relative)).unwrap();
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_plain_repository_is_not_using_lfs() {
        let dir = TempDir::new("plain");
        dir.write(".gitattributes", "*.txt text=auto\n");
        assert!(!detect_lfs(&dir.path().join(".git"), Some(dir.path())));
    }

    #[test]
    fn gitattributes_declaring_filter_lfs_is_detected() {
        let dir = TempDir::new("attributes");
        dir.write(
            ".gitattributes",
            "*.psd filter=lfs diff=lfs merge=lfs -text\n",
        );
        assert!(detect_lfs(&dir.path().join(".git"), Some(dir.path())));
    }

    #[test]
    fn the_lfs_object_directory_is_enough_on_its_own() {
        let dir = TempDir::new("objects");
        dir.mkdir(".git/lfs");
        assert!(detect_lfs(&dir.path().join(".git"), Some(dir.path())));
    }

    #[test]
    fn a_comment_mentioning_filter_lfs_does_not_count() {
        let dir = TempDir::new("comment");
        dir.write(".gitattributes", "# 本仓库不使用 filter=lfs\n*.txt text\n");
        assert!(!detect_lfs(&dir.path().join(".git"), Some(dir.path())));
    }

    #[test]
    fn a_bare_repository_without_workdir_is_never_lfs() {
        let dir = TempDir::new("bare");
        assert!(!detect_lfs(dir.path(), None));
    }
}
