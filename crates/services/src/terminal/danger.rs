//! 终端危险命令识别（T5.3）。
//!
//! # 设计原则（任务书原文的两条硬约束）
//!
//! - **识别失败时不得阻断**：宁可漏报不可误伤。解析器把一切"看不懂"的输入
//!   原样放行；`git commit`、普通 `git push`、`git checkout <branch>` 等
//!   安全命令必须返回 `None`。
//! - **拦截不修改用户输入**：识别器是只读的——发往 PTY 的字节与用户键入
//!   完全一致（确认级的"取消"也只发 `Ctrl+C`，不改写命令）。
//!
//! # 解析的深浅
//!
//! "轻量词法分析，够用即可"：按空白分词（引号内的空格不算分隔），
//! 处理 `-C <path>` 这类"吃参数"的全局选项，识别第一个子命令与其参数。
//! 不做完整 shell 语义（管道、重定向、`&&` 连接的**第二段**命令不识别
//! ——第一段照常识别；任务书接受这个边界）。

/// 危险级别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DangerLevel {
    /// 高危：可能丢数据 / 改写历史（默认提示条 + 记录 + 补偿快照）。
    Dangerous,
    /// 注意：有影响但通常可恢复（只提示，不弹确认）。
    Caution,
}

/// 一次识别的命中结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DangerMatch {
    /// 级别。
    pub level: DangerLevel,
    /// 稳定的种类标识（前端按它走 i18n 文案）。
    pub kind: &'static str,
    /// 规范化展示（如 `git reset --hard`），提示条直接引用。
    pub canonical: String,
}

/// 对一行键入命令做危险识别；无命中或无法解析时返回 `None`。
#[must_use]
pub fn scan_terminal_command(line: &str) -> Option<DangerMatch> {
    let tokens = tokenize(line);
    let mut index = 0;
    // 跳过非 git 前缀只认 git：`sudo` / 包装脚本不做猜测（宁可漏报）
    while index < tokens.len() {
        let token = &tokens[index];
        if is_git_program(token) {
            index += 1;
            break;
        }
        if token.starts_with('-') {
            index += 1;
            continue;
        }
        return None;
    }
    if index >= tokens.len() {
        return None;
    }

    // 全局选项：-C <path> 吃一个参数；其余单 flag 跳过；`-c k=v` 吃一个参数
    let mut subcommand: Option<String> = None;
    while index < tokens.len() {
        let token = tokens[index].as_str();
        if token == "-C" || token == "--exec-path" || token == "--namespace" {
            index += 2;
            continue;
        }
        if token == "-c" {
            index += 2;
            continue;
        }
        if token.starts_with('-') && token != "-" {
            index += 1;
            continue;
        }
        subcommand = Some(token.to_string());
        index += 1;
        break;
    }
    let subcommand = subcommand?;
    let rest = &tokens[index..];

    match subcommand.as_str() {
        "reset" => match_flag(rest, "--hard").map(|_| DangerMatch {
            level: DangerLevel::Dangerous,
            kind: "reset_hard",
            canonical: "git reset --hard".into(),
        }),
        "clean" => {
            let forced = rest.iter().any(|token| {
                token.starts_with('-')
                    && !token.starts_with("--")
                    && token[1..].contains('f')
                    && !token[1..].chars().all(|c| c == 'n')
            }) || rest.iter().any(|token| token == "--force");
            forced.then(|| DangerMatch {
                level: DangerLevel::Dangerous,
                kind: "clean_force",
                canonical: "git clean -f".into(),
            })
        }
        "checkout" => (has_force(rest)).then(|| DangerMatch {
            level: DangerLevel::Dangerous,
            kind: "checkout_force",
            canonical: "git checkout --force".into(),
        }),
        "push" => {
            // 只拦裸 force：--force-with-lease 是 R7 允许的形式
            let bare_force = rest.iter().any(|token| {
                token == "--force"
                    || (token.starts_with('-')
                        && !token.starts_with("--")
                        && token[1..].contains('f')
                        && token != "-n")
            });
            bare_force.then(|| DangerMatch {
                level: DangerLevel::Dangerous,
                kind: "push_force",
                canonical: "git push --force".into(),
            })
        }
        "branch" => {
            // -D 等价于 --delete --force；单独的 -d / --delete 是安全删除
            let force_delete = rest.iter().any(|token| token == "-D")
                || (rest
                    .iter()
                    .any(|token| token == "-d" || token == "--delete")
                    && has_force(rest));
            force_delete.then(|| DangerMatch {
                level: DangerLevel::Dangerous,
                kind: "branch_delete_force",
                canonical: "git branch -D".into(),
            })
        }
        "stash" => {
            let action = rest.first().map(String::as_str);
            match action {
                Some(action @ ("drop" | "clear")) => Some(DangerMatch {
                    level: DangerLevel::Dangerous,
                    kind: "stash_discard",
                    canonical: format!("git stash {action}"),
                }),
                _ => None,
            }
        }
        "filter-branch" => Some(DangerMatch {
            level: DangerLevel::Dangerous,
            kind: "filter_branch",
            canonical: "git filter-branch".into(),
        }),
        "update-ref" => rest
            .iter()
            .any(|token| token == "-d" || token == "--delete")
            .then(|| DangerMatch {
                level: DangerLevel::Dangerous,
                kind: "update_ref_delete",
                canonical: "git update-ref -d".into(),
            }),
        "reflog" => {
            let action = rest.first().map(String::as_str);
            (action == Some("expire")).then(|| DangerMatch {
                level: DangerLevel::Dangerous,
                kind: "reflog_expire",
                canonical: "git reflog expire".into(),
            })
        }
        "gc" => rest
            .iter()
            .any(|token| token.starts_with("--prune=") && token != "--prune=never")
            .then(|| DangerMatch {
                level: DangerLevel::Dangerous,
                kind: "gc_prune_now",
                canonical: "git gc --prune=now".into(),
            }),
        "rebase" => (!rest
            .iter()
            .any(|token| token == "--abort" || token == "--quit"))
        .then(|| DangerMatch {
            level: DangerLevel::Caution,
            kind: "rebase",
            canonical: "git rebase".into(),
        }),
        _ => None,
    }
}

fn is_git_program(token: &str) -> bool {
    let lowered = token.to_ascii_lowercase();
    let file = lowered.rsplit(['/', '\\']).next().unwrap_or(&lowered);
    file == "git" || file == "git.exe"
}

/// 合并短标志（`-fd`）与长标志（`--force`）的强制的判定。
fn has_force(rest: &[String]) -> bool {
    rest.iter().any(|token| {
        token == "--force"
            || (token.starts_with('-')
                && !token.starts_with("--")
                && token != "-"
                && token[1..].contains('f'))
    })
}

fn match_flag(rest: &[String], long: &str) -> Option<()> {
    rest.iter().find(|token| token.as_str() == long).map(|_| ())
}

/// 轻量分词：空白分隔；成对的引号（单/双）内空白保留。
fn tokenize(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;
    for char in line.trim().chars() {
        match char {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            c if c.is_whitespace() && !in_single && !in_double => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{scan_terminal_command, DangerLevel};

    fn kind(line: &str) -> Option<(&'static str, DangerLevel)> {
        scan_terminal_command(line).map(|m| (m.kind, m.level))
    }

    // ---- 15 个正例：都必须命中 ----

    #[test]
    fn detects_the_dangerous_commands_from_the_task_list() {
        let dangerous = [
            "git reset --hard",
            "git reset --hard HEAD~3",
            "git clean -fd",
            "git clean -fdx",
            "git checkout -f",
            "git checkout --force main",
            "git push --force origin main",
            "git push -f",
            "git branch -D feature/x",
            "git stash drop",
            "git stash clear",
            "git filter-branch --tree-filter 'rm x' HEAD",
            "git gc --prune=now",
            "git update-ref -d refs/heads/x",
            "git reflog expire --expire=now --all",
        ];
        for line in dangerous {
            assert!(kind(line).is_some(), "必须命中：{line}");
        }
    }

    #[test]
    fn dangerous_hits_are_level_dangerous_and_carry_canonical_forms() {
        let m = scan_terminal_command("git reset --hard").expect("hit");
        assert_eq!(m.level, DangerLevel::Dangerous);
        assert_eq!(m.canonical, "git reset --hard");
        let rebase = scan_terminal_command("git rebase main").expect("hit");
        assert_eq!(rebase.level, DangerLevel::Caution);
    }

    #[test]
    fn detects_git_with_global_options_and_paths() {
        assert!(kind("git -C ../other reset --hard").is_some());
        assert!(kind("git -c core.autocrlf=false reset --hard").is_some());
        assert!(kind("\"C:\\Program Files\\Git\\bin\\git.exe\" push -f").is_some());
        assert!(kind("git stash drop stash@{2}").is_some());
    }

    // ---- 15 个负例：都必须放行（宁可漏报不可误伤）----

    #[test]
    fn passes_safe_git_commands_through() {
        let safe = [
            "git commit -m \"reset --hard in message\"",
            "git push origin main",
            "git push --force-with-lease origin main",
            "git checkout feature/x",
            "git checkout -- file.txt",
            "git branch -d merged",
            "git branch --delete merged",
            "git stash list",
            "git stash pop",
            "git stash apply",
            "git rebase --abort",
            "git gc",
            "git reflog",
            "git reset --soft HEAD~1",
            "git reset",
        ];
        for line in safe {
            assert!(kind(line).is_none(), "不得误报：{line}");
        }
    }

    #[test]
    fn passes_non_git_and_unparsable_input_through() {
        let unrelated = [
            "npm run build",
            "echo git reset --hard",
            "ls -la",
            "git",
            "",
            "   ",
        ];
        for line in unrelated {
            assert!(kind(line).is_none(), "不得误报：{line}");
        }
    }

    #[test]
    fn only_the_first_segment_of_a_pipeline_is_scanned() {
        // 管道第二段不在识别范围（边界见模块头）：第一段是 ls，安全 → None。
        // 这里锁的是"识别器不因为复杂行而 panic 或误报"。
        assert!(kind("ls | xargs git reset --hard").is_none());
    }
}
