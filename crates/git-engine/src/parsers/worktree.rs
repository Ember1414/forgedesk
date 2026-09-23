//! `git worktree list --porcelain` 的解析器。
//!
//! # 为什么用 porcelain 而不是默认输出
//!
//! 默认输出是给人看的（列宽会随最长路径变化、锁定/可清理状态被塞进括号里），
//! 解析它等于解析一个排版。porcelain 是一行一个属性的稳定键值格式：
//!
//! ```text
//! worktree /home/u/repo
//! HEAD 1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b
//! branch refs/heads/main
//!
//! worktree /home/u/repo-feature
//! HEAD 0f9e8d7c6b5a4938271605f4e3d2c1b0a9f8e7d6
//! branch refs/heads/feature
//! locked 正在使用的检出
//! ```
//!
//! 条目之间以**空行**分隔，`locked` / `prunable` 后面可能跟一个原因字符串。
//! 未知属性一律忽略：git 将来新增属性时，跳过它比整条记录失败要好。

use std::path::PathBuf;

use forgedesk_domain::git::Worktree;

/// 解析 `git worktree list --porcelain` 的输出。
///
/// 顺序即 git 给出的顺序：**主工作区一定在首位**（[`RepositoryInfo::worktrees`] 的约定）。
///
/// [`RepositoryInfo::worktrees`]: forgedesk_domain::git::RepositoryInfo::worktrees
pub fn parse_worktree_list(stdout: &[u8]) -> Vec<Worktree> {
    let text = String::from_utf8_lossy(stdout);
    let mut worktrees: Vec<Worktree> = Vec::new();
    let mut current: Option<Worktree> = None;

    for raw_line in text.lines() {
        // Windows 上 git 的输出仍是 LF，但经管道转手可能带上 CR
        let line = raw_line.trim_end_matches('\r');

        if line.is_empty() {
            if let Some(worktree) = current.take() {
                worktrees.push(worktree);
            }
            continue;
        }

        // `worktree` 是条目的起点：遇到它就说明上一条没有以空行结束
        if let Some(path) = line.strip_prefix("worktree ") {
            if let Some(worktree) = current.take() {
                worktrees.push(worktree);
            }
            current = Some(Worktree {
                path: PathBuf::from(path),
                head: None,
                branch: None,
                detached: false,
                is_bare: false,
                locked: false,
                prunable: false,
            });
            continue;
        }

        // 属性行出现在 `worktree` 之前说明输出畸形，跳过而不是 panic
        let Some(worktree) = current.as_mut() else {
            continue;
        };

        if let Some(oid) = line.strip_prefix("HEAD ") {
            let oid = oid.trim();
            if !oid.is_empty() {
                worktree.head = Some(oid.to_owned());
            }
        } else if let Some(reference) = line.strip_prefix("branch ") {
            worktree.branch = Some(short_branch_name(reference.trim()));
        } else if line == "detached" {
            worktree.detached = true;
        } else if line == "bare" {
            worktree.is_bare = true;
        } else if line == "locked" || line.starts_with("locked ") {
            worktree.locked = true;
        } else if line == "prunable" || line.starts_with("prunable ") {
            worktree.prunable = true;
        }
    }

    if let Some(worktree) = current.take() {
        worktrees.push(worktree);
    }

    worktrees
}

/// `refs/heads/main` → `main`。
///
/// 只剥 `refs/heads/`：工作区的 `branch` 行理论上总是本地分支，
/// 但用 `refs/remotes/` 之类的输入时原样保留，避免把远程分支显示成本地分支。
fn short_branch_name(reference: &str) -> String {
    reference
        .strip_prefix("refs/heads/")
        .unwrap_or(reference)
        .to_owned()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::parse_worktree_list;
    use std::path::Path;

    #[test]
    fn parses_a_single_main_worktree() {
        let output = b"worktree /home/u/repo\nHEAD 1a2b3c4d\nbranch refs/heads/main\n\n";
        let worktrees = parse_worktree_list(output);

        assert_eq!(worktrees.len(), 1);
        let main = &worktrees[0];
        assert_eq!(main.path, Path::new("/home/u/repo"));
        assert_eq!(main.head.as_deref(), Some("1a2b3c4d"));
        assert_eq!(main.branch.as_deref(), Some("main"));
        assert!(!main.detached);
        assert!(!main.is_bare);
        assert!(!main.locked);
        assert!(!main.prunable);
    }

    #[test]
    fn parses_linked_worktrees_and_keeps_the_main_one_first() {
        let output = b"worktree /home/u/repo\nHEAD aaa\nbranch refs/heads/main\n\n\
                       worktree /home/u/repo-feature\nHEAD bbb\nbranch refs/heads/feature\n\n";
        let worktrees = parse_worktree_list(output);

        assert_eq!(worktrees.len(), 2);
        assert_eq!(worktrees[0].path, Path::new("/home/u/repo"));
        assert_eq!(worktrees[1].path, Path::new("/home/u/repo-feature"));
        assert_eq!(worktrees[1].branch.as_deref(), Some("feature"));
    }

    #[test]
    fn detached_locked_and_prunable_are_all_read() {
        let output = b"worktree /repo\nHEAD aaa\nbranch refs/heads/main\n\n\
                       worktree /repo-detached\nHEAD bbb\ndetached\nlocked reason here\n\n\
                       worktree /repo-gone\nHEAD ccc\nbranch refs/heads/gone\nprunable gitdir file points to non-existent location\n";
        let worktrees = parse_worktree_list(output);

        assert!(worktrees[1].detached);
        assert!(worktrees[1].locked);
        assert_eq!(worktrees[1].branch, None);

        assert!(worktrees[2].prunable);
        assert!(!worktrees[2].locked);
    }

    #[test]
    fn bare_repository_worktree_is_recognised() {
        let output = b"worktree /srv/repo.git\nbare\n\n";
        let worktrees = parse_worktree_list(output);

        assert_eq!(worktrees.len(), 1);
        assert!(worktrees[0].is_bare);
        assert_eq!(worktrees[0].head, None);
        assert_eq!(worktrees[0].branch, None);
    }

    #[test]
    fn a_missing_trailing_blank_line_still_yields_the_last_entry() {
        // git 通常以空行结尾，但不该依赖它
        let output = b"worktree /repo\nHEAD aaa\nbranch refs/heads/main";
        let worktrees = parse_worktree_list(output);

        assert_eq!(worktrees.len(), 1);
        assert_eq!(worktrees[0].branch.as_deref(), Some("main"));
    }

    #[test]
    fn unknown_attributes_and_stray_lines_are_ignored() {
        let output = b"worktree /repo\nHEAD aaa\nbranch refs/heads/main\nfuture-thing x\n\n\
                       stray line before any worktree\n";
        let worktrees = parse_worktree_list(output);

        assert_eq!(worktrees.len(), 1);
        assert_eq!(worktrees[0].path, Path::new("/repo"));
    }

    #[test]
    fn empty_and_random_bytes_do_not_panic() {
        assert!(parse_worktree_list(b"").is_empty());
        assert!(parse_worktree_list(&[0xff, 0xfe, 0x00, 0x0a]).is_empty());
    }

    #[test]
    fn crlf_line_endings_are_tolerated() {
        let output = b"worktree /repo\r\nHEAD aaa\r\nbranch refs/heads/main\r\n\r\n";
        let worktrees = parse_worktree_list(output);

        assert_eq!(worktrees.len(), 1);
        assert_eq!(worktrees[0].path, Path::new("/repo"));
        assert_eq!(worktrees[0].branch.as_deref(), Some("main"));
    }
}
