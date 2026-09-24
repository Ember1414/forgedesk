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

/// 提交时 git 会调用的钩子（按 git 的调用顺序）。
///
/// 只有这三个：`post-commit` 在提交**之后**运行，它失败不会让提交失败，
/// 把它列进"将要执行的钩子"会让用户以为提交可能因此被拒绝。
pub const COMMIT_HOOKS: [&str; 3] = ["pre-commit", "prepare-commit-msg", "commit-msg"];

/// 钩子目录里**存在且可执行**的提交钩子（提交预览用）。
///
/// `hooks_dir` 必须由 `GitEngine::hooks_dir` 给出——`core.hooksPath`（husky 默认设置它）
/// 会让真实的钩子目录不是 `.git/hooks`。
pub fn executable_commit_hooks(hooks_dir: &Path) -> Vec<String> {
    COMMIT_HOOKS
        .iter()
        .filter(|name| is_executable(&hooks_dir.join(name)))
        .map(|name| (*name).to_owned())
        .collect()
}

/// 钩子目录里的一项（T1.8 的"hooks 状态查看"）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookEntry {
    /// 钩子名（文件名，如 `pre-commit`）。
    pub name: String,
    /// git 是否会执行它（Unix 看执行位，Windows 看文件是否存在）。
    pub executable: bool,
    /// 是否属于提交时会调用的三类之一。
    pub commit_hook: bool,
}

/// 列出钩子目录里的**全部**钩子（仅展示，不编辑）。
///
/// 为什么列全部而不是只列提交相关的三个：用户来看这个列表，
/// 想知道的多半是"为什么提交被拒/很慢"或"我装了哪些工具"，
/// `pre-push`、`post-checkout` 同样可能是答案。而"**这次**提交会执行哪些"
/// 是另一个问题，由 [`executable_commit_hooks`] 回答。
///
/// `.sample` 与点文件一律排除：`git init` 会放一批示例进去，它们永远不会被执行，
/// 列出来只会让列表看起来像"这个仓库装了一堆钩子"。
///
/// 顺序是确定的（提交相关在前，其次按名字）：`read_dir` 的顺序不保证，
/// 而不确定的顺序会让界面跳动、让测试随机失败。
pub fn list_hooks(hooks_dir: &Path) -> Vec<HookEntry> {
    let Ok(entries) = std::fs::read_dir(hooks_dir) else {
        return Vec::new();
    };

    let mut hooks: Vec<HookEntry> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.ends_with(".sample") || name.starts_with('.') {
                return None;
            }
            let path = entry.path();
            // `is_file`（而不是 file_type）会跟随符号链接：用符号链接指向
            // 别处的钩子是常见做法，不该被当成"不是文件"而漏掉
            if !path.is_file() {
                return None;
            }
            Some(HookEntry {
                executable: is_executable(&path),
                commit_hook: COMMIT_HOOKS.contains(&name.as_str()),
                name,
            })
        })
        .collect();

    hooks.sort_by(|left, right| {
        right
            .commit_hook
            .cmp(&left.commit_hook)
            .then_with(|| left.name.cmp(&right.name))
    });
    hooks
}

/// Unix：git 要求钩子文件带执行位，没有执行位的钩子会被忽略。
#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
}

/// Windows：没有执行位，git for Windows 就是"文件存在即执行"。
///
/// 这里刻意不要求 `.exe` / `.bat` 扩展名：钩子的常见形态是无扩展名的 sh 脚本，
/// 加了扩展名判定会把绝大多数真实钩子漏掉——而"预览里说没有钩子、实际执行了"
/// 比"多列一个"糟糕得多。
#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|metadata| metadata.is_file())
        .unwrap_or(false)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{detect_lfs, executable_commit_hooks, list_hooks, COMMIT_HOOKS};
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

    /// 让钩子文件带上执行位（Windows 上没有执行位，git 以文件存在为准）。
    fn make_runnable(path: &Path) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(path).unwrap().permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(path, permissions).unwrap();
        }
        #[cfg(not(unix))]
        {
            let _ = path;
        }
    }

    #[test]
    fn only_the_hooks_git_would_actually_run_are_listed() {
        let dir = TempDir::new("hooks");
        dir.write(".git/hooks/pre-commit", "#!/bin/sh\n");
        // post-commit 在提交之后运行，失败也不会让提交失败——列出来是误导
        dir.write(".git/hooks/post-commit", "#!/bin/sh\n");
        // git init 自带的示例文件名字带后缀，git 永远不执行它
        dir.write(".git/hooks/commit-msg.sample", "#!/bin/sh\n");
        make_runnable(&dir.path().join(".git/hooks/pre-commit"));

        assert_eq!(
            executable_commit_hooks(&dir.path().join(".git/hooks")),
            vec!["pre-commit".to_owned()]
        );
    }

    #[test]
    fn the_hook_list_follows_the_order_git_calls_them() {
        let dir = TempDir::new("hook-order");
        for name in COMMIT_HOOKS {
            dir.write(&format!(".git/hooks/{name}"), "#!/bin/sh\n");
            make_runnable(&dir.path().join(format!(".git/hooks/{name}")));
        }

        assert_eq!(
            executable_commit_hooks(&dir.path().join(".git/hooks")),
            vec![
                "pre-commit".to_owned(),
                "prepare-commit-msg".to_owned(),
                "commit-msg".to_owned()
            ]
        );
    }

    #[test]
    fn a_missing_hooks_directory_lists_nothing() {
        let dir = TempDir::new("no-hooks");

        assert!(executable_commit_hooks(&dir.path().join(".git/hooks")).is_empty());
    }

    #[test]
    fn hooks_are_listed_with_their_commit_role_and_samples_excluded() {
        let dir = TempDir::new("list-hooks");
        dir.write(".git/hooks/pre-commit", "#!/bin/sh\n");
        dir.write(".git/hooks/pre-push", "#!/bin/sh\n");
        dir.write(".git/hooks/post-commit", "#!/bin/sh\n");
        // git init 放的示例文件永远不会被执行，列出来只会误导
        dir.write(".git/hooks/pre-commit.sample", "#!/bin/sh\n");
        make_runnable(&dir.path().join(".git/hooks/pre-commit"));

        let hooks = list_hooks(&dir.path().join(".git/hooks"));

        assert_eq!(
            hooks
                .iter()
                .map(|hook| hook.name.as_str())
                .collect::<Vec<_>>(),
            vec!["pre-commit", "post-commit", "pre-push"],
            "提交相关的钩子排在最前，其余按名字；示例文件不列"
        );
        assert!(hooks[0].commit_hook, "pre-commit 属于提交钩子");
        assert!(!hooks[1].commit_hook, "post-commit 不在提交链路上");
        assert!(!hooks[2].commit_hook, "pre-push 不在提交链路上");

        #[cfg(unix)]
        {
            assert!(hooks[0].executable, "带执行位的钩子 git 会执行");
            assert!(!hooks[1].executable, "没有执行位的钩子 git 会忽略");
        }
    }

    #[test]
    fn a_hook_directory_with_no_hooks_lists_nothing() {
        let dir = TempDir::new("empty-hooks");
        dir.mkdir(".git/hooks");

        assert!(list_hooks(&dir.path().join(".git/hooks")).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn a_hook_without_the_execute_bit_is_not_listed_because_git_would_skip_it() {
        let dir = TempDir::new("hook-perm");
        dir.write(".git/hooks/pre-commit", "#!/bin/sh\n");

        // 默认权限是 0644：git 会忽略它，预览里也就不能说"会执行"
        assert!(executable_commit_hooks(&dir.path().join(".git/hooks")).is_empty());

        make_runnable(&dir.path().join(".git/hooks/pre-commit"));
        assert_eq!(
            executable_commit_hooks(&dir.path().join(".git/hooks")),
            vec!["pre-commit".to_owned()]
        );
    }
}
