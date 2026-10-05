//! 诊断规则引擎（T5.5）：把 git / 网络的原始 stderr 映射为结构化诊断
//! （i18n key + 原因列表 + 修复动作），是 [`crate::sanitize`] 之上的"人话层"。
//!
//! # 数据流
//!
//! ```text
//! stderr ──┐
//! context ─┴─> diagnose() ─> DiagnosticReport { primary, alternatives, raw_summary }
//! ```
//!
//! 规则是**纯数据**（`rules/*.yaml`，编译期内嵌 + 可选的运行时覆盖目录），
//! 只含 i18n key，不含任何界面文案——中英文由前端按 key 渲染。
//!
//! # 匹配语义
//!
//! - 规则命中的充要条件：`when`（可选的上下文前提）全部满足、`match.all_of`
//!   全部命中、`match.none_of` 全不命中；
//! - 命中的规则按 `confidence` 降序排列，最高者为 `primary`，其余进
//!   `alternatives`（诊断卡片折叠展示为"可能原因"）；
//! - 识别失败**绝不报错**：任何 stderr 都能得到一个空 primary 的报告——
//!   与终端安全拦截同一条铁律：宁可漏报不可误伤。
//!
//! # 动作安全（任务书 §5）
//!
//! `fixes[].action.kind` 有三档：`command`（安全动作，前端可直接执行）、
//! `guide`（打开文档）、`dangerous`（**必须**走 DangerousActionDialog，
//! 含计划预览与快照——绝不允许诊断卡片一键直接执行危险操作）。
//! 本 crate 只传输 kind，不执行任何动作；执行约束在前端（T5.6）。

use std::collections::BTreeSet;
use std::path::Path;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// 规则数据模型（serde 直接反序列化 YAML）
// ---------------------------------------------------------------------------

/// 单条匹配条件：字面包含或正则（二选一）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleCondition {
    /// 字面子串（大小写敏感；stderr 的原文匹配）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contains: Option<String>,
    /// 正则（Rust `regex` 语法；大小写敏感）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub regex: Option<String>,
}

/// 规则的匹配块（`match` 是 Rust 关键字，serde rename）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RuleMatch {
    /// 全部命中才算命中。
    #[serde(default)]
    pub all_of: Vec<RuleCondition>,
    /// 全部不命中才算命中（排除条件）。
    #[serde(default)]
    pub none_of: Vec<RuleCondition>,
}

/// 上下文前提（消歧）：`None` = 不关心；`Some` = 必须等于。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RuleWhen {
    /// 只在指定操作类型（如 `push`）下命中。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub op_type: Option<String>,
    /// 只在"有/无上游"时命中。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream: Option<bool>,
    /// 只在分离 HEAD 状态（或非分离）下命中。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detached: Option<bool>,
    /// 只在浅克隆（或非浅克隆）下命中。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shallow: Option<bool>,
}

/// 修复动作（只传输元数据；执行在前端）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleFixAction {
    /// `command`（安全）/ `guide`（文档）/ `dangerous`（必须走危险确认对话框）。
    pub kind: String,
    /// `kind = command` 时的命令名（如 `git_fetch`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// 命令参数（前端按命令族自行装配；结构自由）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<serde_json::Value>,
    /// `kind = guide` 时的文档地址或页面标识。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

/// 一条修复建议。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleFix {
    /// 稳定 id（前端遥测/历史用）。
    pub id: String,
    /// i18n key（按钮文案）。
    pub label_key: String,
    /// 动作元数据（执行在前端）。
    pub action: RuleFixAction,
}

/// 一条诊断规则（YAML 的直接映射）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rule {
    /// 稳定 id（运行时覆盖目录里同 id 的规则会**替换**内置规则）。
    pub id: String,
    /// 影响阶段（informational：前端可用它筛选）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stage: Vec<String>,
    /// 匹配块（YAML 的 `match` 键）。
    #[serde(rename = "match")]
    pub match_rules: RuleMatch,
    /// 0.0–1.0；primary 按 confidence 降序取。
    pub confidence: f32,
    /// 上下文前提（消歧；`None` = 任何上下文）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<RuleWhen>,
    /// i18n key：标题。
    pub title_key: String,
    /// i18n key：一句话解释。
    pub explanation_key: String,
    /// i18n key 列表：可能原因。
    #[serde(default)]
    pub causes: Vec<String>,
    /// 修复建议（执行在前端；kind=dangerous 走危险确认对话框）。
    #[serde(default)]
    pub fixes: Vec<RuleFix>,
}

/// 诊断时的上下文（消歧输入）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DiagContext {
    /// 操作类型（`push` / `pull` / `commit` / `checkout` …；与审计 op_type 同源）。
    pub op_type: Option<String>,
    /// 当前分支是否有上游。
    pub upstream: bool,
    /// 是否处于分离 HEAD。
    pub detached: bool,
    /// 是否浅克隆。
    pub shallow: bool,
}

/// 一条命中的诊断。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    /// 规则 id。
    pub id: String,
    /// 置信度（0.0–1.0）。
    pub confidence: f32,
    /// i18n key：标题。
    pub title_key: String,
    /// i18n key：一句话解释。
    pub explanation_key: String,
    /// i18n key 列表：可能原因。
    pub causes: Vec<String>,
    /// 修复建议。
    pub fixes: Vec<RuleFix>,
}

/// 诊断报告。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticReport {
    /// 置信度最高的命中；`None` = 没有规则命中（原始错误照常展示）。
    pub primary: Option<Diagnostic>,
    /// 其余命中（按 confidence 降序；前端折叠为"可能原因"）。
    pub alternatives: Vec<Diagnostic>,
    /// 原始 stderr 摘要（已截断；展示层负责再脱敏）。
    pub raw_summary: String,
}

// ---------------------------------------------------------------------------
// 匹配实现
// ---------------------------------------------------------------------------

fn condition_matches(condition: &RuleCondition, stderr: &str) -> bool {
    if let Some(needle) = &condition.contains {
        return stderr.contains(needle.as_str());
    }
    if let Some(pattern) = &condition.regex {
        return regex::Regex::new(pattern)
            .map(|re| re.is_match(stderr))
            .unwrap_or(false);
    }
    // 既没有 contains 也没有 regex 的条件是规则作者的笔误：视为不命中
    // （宁可漏报不可误伤，与整个引擎的口径一致）。
    false
}

fn match_block_matches(match_rules: &RuleMatch, stderr: &str) -> bool {
    match_rules
        .all_of
        .iter()
        .all(|c| condition_matches(c, stderr))
        && !match_rules
            .none_of
            .iter()
            .any(|c| condition_matches(c, stderr))
}

fn when_matches(rule_when: &RuleWhen, ctx: &DiagContext) -> bool {
    if let Some(op) = &rule_when.op_type {
        if ctx.op_type.as_deref() != Some(op.as_str()) {
            return false;
        }
    }
    if let Some(upstream) = rule_when.upstream {
        if ctx.upstream != upstream {
            return false;
        }
    }
    if let Some(detached) = rule_when.detached {
        if ctx.detached != detached {
            return false;
        }
    }
    if let Some(shallow) = rule_when.shallow {
        if ctx.shallow != shallow {
            return false;
        }
    }
    true
}

fn to_diagnostic(rule: &Rule) -> Diagnostic {
    Diagnostic {
        id: rule.id.clone(),
        confidence: rule.confidence,
        title_key: rule.title_key.clone(),
        explanation_key: rule.explanation_key.clone(),
        causes: rule.causes.clone(),
        fixes: rule.fixes.clone(),
    }
}

/// 用给定的规则集做一次诊断（纯函数；`diagnose` 的核心，测试直接驱动）。
///
/// 返回的命中按 `confidence` 降序排列；`raw_summary` 截到 500 字符
/// （展示层的脱敏另做——这里的输入约定是"已经过脱敏的 stderr"）。
#[must_use]
pub fn diagnose_with(rules: &[Rule], stderr: &str, ctx: &DiagContext) -> DiagnosticReport {
    let mut hits: Vec<&Rule> = rules
        .iter()
        .filter(|rule| {
            rule.when
                .as_ref()
                .is_none_or(|rule_when| when_matches(rule_when, ctx))
                && match_block_matches(&rule.match_rules, stderr)
        })
        .collect();
    hits.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.id.cmp(&b.id))
    });

    let mut report = DiagnosticReport {
        primary: None,
        alternatives: Vec::new(),
        raw_summary: stderr.chars().take(500).collect(),
    };
    if let Some(first) = hits.first() {
        report.primary = Some(to_diagnostic(first));
    }
    report.alternatives = hits
        .iter()
        .skip(1)
        .map(|rule| to_diagnostic(rule))
        .collect();
    report
}

// ---------------------------------------------------------------------------
// 规则加载：编译期内嵌 + 运行时覆盖目录
// ---------------------------------------------------------------------------

/// 内嵌规则文件（`crates/diagnostics/rules/*.yaml`，每文件一个规则数组）。
const EMBEDDED_RULE_FILES: &[&str] = &[
    include_str!("../rules/push.yaml"),
    include_str!("../rules/network.yaml"),
    include_str!("../rules/worktree.yaml"),
    include_str!("../rules/commit.yaml"),
    include_str!("../rules/history.yaml"),
    include_str!("../rules/extras.yaml"),
];

/// 解析一段 YAML（规则数组）；解析失败返回错误（内嵌文件失败 = 构建期缺陷，
/// 运行时覆盖文件失败由调用方记警告并跳过）。
pub fn parse_rules(yaml: &str) -> Result<Vec<Rule>, String> {
    serde_yml::from_str::<Vec<Rule>>(yaml).map_err(|error| error.to_string())
}

/// 校验一批规则的结构约束；返回警告（重复 id / 空匹配块等，不阻塞）。
#[must_use]
pub fn validate_rules(rules: &[Rule]) -> Vec<String> {
    let mut warnings = Vec::new();
    let mut seen = BTreeSet::new();
    for rule in rules {
        if !seen.insert(rule.id.clone()) {
            warnings.push(format!("duplicate rule id: {}", rule.id));
        }
        if rule.match_rules.all_of.is_empty() && rule.match_rules.none_of.is_empty() {
            warnings.push(format!("rule {} has an empty match block", rule.id));
        }
        if !(0.0..=1.0).contains(&rule.confidence) {
            warnings.push(format!("rule {} confidence out of range", rule.id));
        }
    }
    warnings
}

/// 加载**内嵌**规则（进程内只解析一次）。
///
/// 内嵌文件解析失败会让启动失败——那是构建期缺陷，静默跳过只会让
/// "为什么所有错误都不再有诊断"变成无解之谜。
// 函数级 allow：见函数体内关于"内嵌规则解析失败 = 构建期缺陷"的说明。
#[allow(clippy::panic)]
pub fn embedded_rules() -> &'static Vec<Rule> {
    use std::sync::OnceLock;
    static EMBEDDED: OnceLock<Vec<Rule>> = OnceLock::new();
    EMBEDDED.get_or_init(|| {
        let mut rules = Vec::new();
        for (index, yaml) in EMBEDDED_RULE_FILES.iter().enumerate() {
            match parse_rules(yaml) {
                Ok(parsed) => rules.extend(parsed),
                Err(error) => {
                    panic!("embedded diagnostics rule file #{index} failed to parse: {error}")
                }
            }
        }
        rules
    })
}

/// 从运行时覆盖目录加载规则（`app_config_dir()/diagnostics/*.yaml`）。
///
/// 覆盖语义：与内嵌规则**合并**；同 id 的覆盖规则**替换**内置规则——
/// 这是"不发版修规则"的核心机制。目录不存在 = 空覆盖（正常路径）。
/// 单个文件解析失败：记警告并跳过该文件（目录是用户可写的，容错优先）。
///
/// 返回（合并后的规则， 警告列表）。
pub fn load_with_overrides(dir: &Path) -> (Vec<Rule>, Vec<String>) {
    let mut rules: Vec<Rule> = embedded_rules().clone();
    let mut warnings: Vec<String> = Vec::new();

    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return (rules, warnings),
    };

    let mut paths: Vec<std::path::PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            matches!(
                path.extension().and_then(|ext| ext.to_str()),
                Some("yaml" | "yml")
            )
        })
        .collect();
    paths.sort();

    for path in paths {
        match std::fs::read_to_string(&path) {
            Ok(content) => match parse_rules(&content) {
                Ok(overrides) => {
                    for rule in overrides {
                        if let Some(existing) = rules.iter_mut().find(|r| r.id == rule.id) {
                            warnings.push(format!("override replaced rule {}", rule.id));
                            *existing = rule;
                        } else {
                            rules.push(rule);
                        }
                    }
                }
                Err(error) => warnings.push(format!("{}: {error}", path.display())),
            },
            Err(error) => warnings.push(format!("{}: {error}", path.display())),
        }
    }

    let rules_warnings = validate_rules(&rules);
    for warning in rules_warnings {
        warnings.push(warning);
    }
    (rules, warnings)
}

/// 便捷入口：用内嵌规则（+可选覆盖）做诊断。
#[must_use]
pub fn diagnose(stderr: &str, ctx: &DiagContext, override_dir: Option<&Path>) -> DiagnosticReport {
    match override_dir {
        Some(dir) => {
            let (rules, warnings) = load_with_overrides(dir);
            for warning in &warnings {
                tracing::debug!(warning = %warning, "diagnostics rule override warning");
            }
            diagnose_with(&rules, stderr, ctx)
        }
        None => diagnose_with(embedded_rules(), stderr, ctx),
    }
}
