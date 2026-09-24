//! 提交命令（`commit_*`，M1 / T1.7）。
//!
//! 与其它命令族同一条纪律：本层只做参数校验、DTO 转换与事件投递，
//! 用例本体在 `forgedesk_services::CommitService`。
//!
//! # 三段式命令（prepare → execute → 提示）
//!
//! | 命令 | 能力等级 | 说明 |
//! | --- | --- | --- |
//! | [`commit_prepare`] | `ReadOnly` | 生成计划，**不创建提交** |
//! | [`commit_execute`] | `Mutating` | 执行计划，成功后发布 `repo:changed` |
//! | [`commit_message_hint`] | `ReadOnly` | 最近提交与分支风格（纯本地规则） |
//!
//! `commit_prepare` 标成 `ReadOnly` 是刻意的：它只读索引与配置，界面上不需要任何
//! 确认对话框。它确实会让 `git write-tree` 往对象库写一个树对象（gc 会回收），
//! 但不动索引、引用与工作区——"是否改变用户的数据"与"是否碰了磁盘"不是一回事，
//! 能力等级说的是前者。
//!
//! # 为什么 `HOOK_REJECTED` 不返回 `actions`
//!
//! 任务定义希望返回"查看输出 / 禁用 hooks 重试"两个动作。但 `FixAction` 的语义是
//! **点击后调用某个 Tauri 命令**，而这两个动作都不是单命令：
//!
//! - "查看输出" = 展开 `detail`（前端已有可折叠详情）；
//! - "禁用 hooks 重试" = 用 `noVerify: true` 重新走 prepare + execute。
//!
//! 伪造一个指向无关命令的按钮，用户点下去只会得到第二个错误——比没有按钮更糟。
//! 因此 `detail`（原始输出）与 `hint`（钩子名清单）给全数据，动作由前端实现。

use forgedesk_domain::git::{CommitPlan, PlannedFile, SignMode, Signature, COMMIT_PLAN_TTL_MS};
use forgedesk_domain::{AppError, AppResult, ErrorCode};
use forgedesk_services::{CommitOutcome, MessageHint, PrepareRequest};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use crate::state::AppState;
use crate::workspace::emit_changed;

/// 提交信息长度上限（字符）。
///
/// 这是 **IPC 边界上的防呆**，不是产品限制：正常提交信息几百字符，
/// 而没有上限就意味着一份 100MB 的字符串会穿过 IPC 进入 git 参数。
const MAX_MESSAGE_CHARS: usize = 100_000;

/// 计划 id 的长度上限（UUID 是 36 个字符）。
const MAX_PLAN_ID_LEN: usize = 64;

// ---------------------------------------------------------------- 请求

/// 作者身份（请求形状）。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityRequest {
    /// 姓名。
    pub name: String,
    /// 邮箱。
    pub email: String,
}

/// 准备提交计划的请求。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareCommitRequest {
    /// 提交信息**首行**（subject）；正文用 `description`。
    pub message: String,
    /// 正文（可选）。
    #[serde(default)]
    pub description: Option<String>,
    /// 是否 amend 上一个提交。
    #[serde(default)]
    pub amend: bool,
    /// 是否追加 `Signed-off-by`。
    #[serde(default)]
    pub sign_off: bool,
    /// 是否跳过钩子（用户显式选择才跳过）。
    #[serde(default)]
    pub no_verify: bool,
    /// GPG 签名模式：`auto`（缺省）/ `yes` / `no`。
    #[serde(default)]
    pub sign: Option<String>,
    /// 覆盖作者身份。
    #[serde(default)]
    pub author: Option<IdentityRequest>,
}

impl PrepareCommitRequest {
    /// 收敛到领域输入（所有外部输入都在这里被校验一次）。
    fn into_domain(self) -> AppResult<PrepareRequest> {
        if self.message.chars().count() > MAX_MESSAGE_CHARS {
            return Err(validation(
                "the commit message is too long for the IPC boundary",
                "message",
            ));
        }
        if let Some(description) = &self.description {
            if description.chars().count() > MAX_MESSAGE_CHARS {
                return Err(validation(
                    "the commit description is too long for the IPC boundary",
                    "description",
                ));
            }
        }

        let sign = match self.sign.as_deref() {
            None => SignMode::Auto,
            // 未知取值必须报错：静默降级成 auto 会让"我明明选了签名"变成一个谜
            Some(key) => {
                SignMode::from_key(key).ok_or_else(|| validation("unknown sign mode", "sign"))?
            }
        };

        let author = match self.author {
            None => None,
            Some(identity) => Some(identity.into_domain()?),
        };

        Ok(PrepareRequest {
            subject: self.message,
            description: self.description,
            amend: self.amend,
            sign_off: self.sign_off,
            no_verify: self.no_verify,
            sign,
            author,
        })
    }
}

impl IdentityRequest {
    fn into_domain(self) -> AppResult<Signature> {
        let name = self.name.trim();
        let email = self.email.trim();
        if name.is_empty() {
            return Err(validation("the author name is empty", "author.name"));
        }
        if !is_plausible_email(email) {
            return Err(validation(
                "the author email is not a plausible address",
                "author.email",
            ));
        }
        Ok(Signature::new(name, email))
    }
}

// ---------------------------------------------------------------- DTO

/// 计划里的一个文件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedFileDto {
    /// 仓库内路径。
    pub path: String,
    /// 索引侧状态字符（`A` / `M` / `D` / `R` / `U`），界面据此分组。
    pub index_status: String,
}

impl PlannedFileDto {
    fn from_domain(file: &PlannedFile) -> Self {
        Self {
            path: file.path.to_string_lossy().into_owned(),
            index_status: file.index_status.as_char().to_string(),
        }
    }
}

/// 作者身份（返回形状）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityDto {
    /// 姓名。
    pub name: String,
    /// 邮箱。
    pub email: String,
}

/// 提交计划（提交前预览对话框的数据源）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitPlanDto {
    /// 计划 id（执行时原样回传）。
    pub plan_id: String,
    /// 存储层记录 id。
    pub repo_id: i64,
    /// 将要被提交的文件。
    pub files: Vec<PlannedFileDto>,
    /// 完整提交信息（首行 + 空行 + 正文）。
    pub message: String,
    /// 正文。
    pub description: Option<String>,
    /// 覆盖的作者身份。
    pub author: Option<IdentityDto>,
    /// 签名模式（`auto` / `yes` / `no`）。
    pub sign: String,
    /// 是否追加 `Signed-off-by`。
    pub sign_off: bool,
    /// 是否跳过钩子。
    pub no_verify: bool,
    /// 是否 amend。
    pub amend: bool,
    /// 将要执行的钩子名（按 git 的调用顺序）。
    pub hooks: Vec<String>,
    /// 等价的 git 命令（可复制到终端）。
    pub equivalent_command: String,
    /// 准备时刻的 HEAD oid（空仓库为 `null`）。
    pub head_oid: Option<String>,
    /// 准备时刻的索引指纹（`git write-tree` 的树 oid）。
    pub index_fingerprint: String,
    /// 准备时刻（Unix 毫秒）。
    pub created_at_ms: i64,
    /// 计划失效时刻（Unix 毫秒）。
    pub expires_at_ms: i64,
    /// 提交信息首行。
    pub subject: String,
    /// 首行字符数（界面上的计数器）。
    pub subject_chars: usize,
    /// 不阻断的建议（稳定短名，前端走 i18n）。
    pub warnings: Vec<String>,
}

impl CommitPlanDto {
    fn from_domain(plan: &CommitPlan) -> Self {
        Self {
            plan_id: plan.plan_id.clone(),
            repo_id: plan.repo_id,
            files: plan.files.iter().map(PlannedFileDto::from_domain).collect(),
            message: plan.message.clone(),
            description: plan.description.clone(),
            author: plan.author.as_ref().map(|author| IdentityDto {
                name: author.name.clone(),
                email: author.email.clone(),
            }),
            sign: plan.sign.key().to_owned(),
            sign_off: plan.sign_off,
            no_verify: plan.no_verify,
            amend: plan.amend,
            hooks: plan.hooks.clone(),
            equivalent_command: plan.equivalent_command.clone(),
            head_oid: plan.head_oid.clone(),
            index_fingerprint: plan.index_fingerprint.clone(),
            created_at_ms: plan.created_at_ms,
            expires_at_ms: plan.created_at_ms + COMMIT_PLAN_TTL_MS,
            subject: plan.review.subject.clone(),
            subject_chars: plan.review.subject_chars,
            warnings: plan
                .review
                .warnings()
                .map(|issue| issue.key().to_owned())
                .collect(),
        }
    }
}

/// 提交结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitOutcomeDto {
    /// 新提交的 oid。
    pub oid: String,
    /// 提交信息首行。
    pub subject: String,
    /// 关联的快照 id（M3 之前为 `null`）。
    pub snapshot_id: Option<i64>,
    /// 本次提交涉及的路径。
    pub paths: Vec<String>,
}

impl CommitOutcomeDto {
    fn from_domain(outcome: &CommitOutcome) -> Self {
        Self {
            oid: outcome.oid.clone(),
            subject: outcome.subject.clone(),
            snapshot_id: outcome.snapshot_id,
            paths: outcome
                .paths
                .iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect(),
        }
    }
}

/// 提交信息风格提示。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageHintDto {
    /// 最近若干条提交的首行（新 → 旧）。
    pub recent_messages: Vec<String>,
    /// 建议的模板前缀（例如 `feat: `）。
    pub template: Option<String>,
    /// 从分支名推断出的风格前缀。
    pub branch_style: Option<String>,
}

impl MessageHintDto {
    fn from_domain(hint: &MessageHint) -> Self {
        Self {
            recent_messages: hint.recent_messages.clone(),
            template: hint.template.clone(),
            branch_style: hint.branch_style.clone(),
        }
    }
}

// ---------------------------------------------------------------- 命令

/// 生成提交计划（不创建提交）。能力等级：`ReadOnly`。
///
/// 会提前拦下四类"执行时才发现就太晚了"的问题：提交信息为空、没有可提交的内容
/// （`EMPTY_COMMIT`）、仓库正处于冲突中（`GIT_CONFLICT`）、仓库记录不存在
/// （`NOT_FOUND`）。
#[tauri::command]
pub fn commit_prepare(
    state: State<'_, AppState>,
    repo_id: i64,
    spec: PrepareCommitRequest,
) -> AppResult<CommitPlanDto> {
    if repo_id <= 0 {
        return Err(validation("repoId must be a positive record id", "repo_id"));
    }

    let request = spec.into_domain()?;
    state
        .commit_service()
        .prepare(repo_id, &request)
        .map(|plan| CommitPlanDto::from_domain(&plan))
}

/// 执行提交计划。能力等级：`Mutating`；成功后发布 `repo:changed`。
///
/// 计划在服务层是**一次性**的：同一 `planId` 执行两次会得到 `PLAN_STALE`
/// （第二次必然失败，因为索引已经变了，而"同一次提交被提交两次"是不该存在的用法）。
#[tauri::command]
pub fn commit_execute(
    state: State<'_, AppState>,
    app: AppHandle,
    plan_id: String,
) -> AppResult<CommitOutcomeDto> {
    let plan_id = plan_id.trim();
    if plan_id.is_empty() || plan_id.len() > MAX_PLAN_ID_LEN {
        return Err(validation("planId is not a valid plan id", "plan_id"));
    }

    let outcome = state.commit_service().execute(plan_id)?;

    // 数据变化是事实：投递失败只影响本次自动刷新（面板仍可手动刷新）
    emit_changed(
        &app,
        outcome.repo_id,
        outcome
            .paths
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect(),
    );

    Ok(CommitOutcomeDto::from_domain(&outcome))
}

/// 提交信息风格提示。能力等级：`ReadOnly`。
///
/// 纯本地规则（最近 20 条提交的首行、分支名前缀），**不含任何模型推理**（红线 R1）。
#[tauri::command]
pub fn commit_message_hint(state: State<'_, AppState>, repo_id: i64) -> AppResult<MessageHintDto> {
    if repo_id <= 0 {
        return Err(validation("repoId must be a positive record id", "repo_id"));
    }

    state
        .commit_service()
        .message_hint(repo_id)
        .map(|hint| MessageHintDto::from_domain(&hint))
}

// ---------------------------------------------------------------- 内部

/// 参数校验失败。
///
/// `hint` 只放字段名（数据），文案由前端按错误码给（CODING_STYLE §2.1）。
fn validation(message: &str, field: &str) -> AppError {
    AppError::new(ErrorCode::Validation, message).with_hint(field)
}

/// 邮箱形状的粗略检查。
///
/// 不追求 RFC 5322：`--author` 只要求 `Name <email>` 能被 git 解析，真正非法的地址
/// 会在 git 那一步被拒（错误里带着原文）。这里拦的是"把姓名填进了邮箱框"这类
/// 一眼可见的错误，避免把 `Ada <Ada>` 交给 git，让用户对着一句 git 报错发愣。
fn is_plausible_email(value: &str) -> bool {
    if value.is_empty() || value.chars().any(char::is_whitespace) {
        return false;
    }
    match value.split_once('@') {
        Some((local, domain)) => !local.is_empty() && !domain.is_empty() && !domain.contains('@'),
        None => false,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{is_plausible_email, PrepareCommitRequest};
    use forgedesk_domain::git::SignMode;
    use forgedesk_domain::ErrorCode;

    fn request() -> PrepareCommitRequest {
        PrepareCommitRequest {
            message: "feat: something".to_owned(),
            description: None,
            amend: false,
            sign_off: false,
            no_verify: false,
            sign: None,
            author: None,
        }
    }

    #[test]
    fn the_default_sign_mode_is_auto_and_unknown_modes_are_rejected() {
        let parsed = request().into_domain().unwrap();
        assert_eq!(parsed.sign, SignMode::Auto);

        let broken = PrepareCommitRequest {
            sign: Some("sometimes".to_owned()),
            ..request()
        };
        assert_eq!(
            broken
                .into_domain()
                .expect_err("未知签名模式必须被拒绝")
                .code,
            ErrorCode::Validation
        );
    }

    #[test]
    fn an_author_needs_both_a_name_and_a_plausible_email() {
        let named = PrepareCommitRequest {
            author: Some(super::IdentityRequest {
                name: "  Ada  ".to_owned(),
                email: "ada@example.com".to_owned(),
            }),
            ..request()
        };
        let domain = named.into_domain().unwrap();
        assert_eq!(
            domain.author.as_ref().map(|author| author.name.clone()),
            Some("Ada".to_owned()),
            "姓名两端的空白要去掉"
        );

        let no_name = PrepareCommitRequest {
            author: Some(super::IdentityRequest {
                name: "   ".to_owned(),
                email: "ada@example.com".to_owned(),
            }),
            ..request()
        };
        assert_eq!(
            no_name.into_domain().expect_err("空姓名必须被拒绝").code,
            ErrorCode::Validation
        );

        let name_in_email_box = PrepareCommitRequest {
            author: Some(super::IdentityRequest {
                name: "Ada".to_owned(),
                email: "Ada".to_owned(),
            }),
            ..request()
        };
        assert_eq!(
            name_in_email_box
                .into_domain()
                .expect_err("邮箱里没有 @ 必须被拒绝")
                .code,
            ErrorCode::Validation
        );
    }

    #[test]
    fn an_oversized_message_is_stopped_at_the_boundary() {
        let huge = PrepareCommitRequest {
            message: "x".repeat(super::MAX_MESSAGE_CHARS + 1),
            ..request()
        };

        assert_eq!(
            huge.into_domain().expect_err("超长信息必须被拒绝").code,
            ErrorCode::Validation
        );
    }

    #[test]
    fn email_shapes_that_git_would_reject_are_rejected_here() {
        assert!(is_plausible_email("ada@example.com"));
        assert!(is_plausible_email("a@b"), "git 接受没有点的域名");
        assert!(!is_plausible_email(""));
        assert!(!is_plausible_email("ada"));
        assert!(!is_plausible_email("@example.com"));
        assert!(!is_plausible_email("ada@"));
        assert!(!is_plausible_email("ada@@example.com"));
        assert!(!is_plausible_email("ada lovelace@example.com"));
    }
}
