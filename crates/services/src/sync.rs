//! 远端同步（T2.6）：fetch / pull / push 的编排与远端管理。
//!
//! # 编排，而不是新的 git 语义
//!
//! 引擎（T1.9）已有 fetch/pull/push 的完整实现（进度解析、refspec 更新、
//! 冲突与拒绝的结构化结果）。本层的职责是**安全与体验编排**：
//!
//! - **pull 前快照**（`PreSync`）：合并/变基会移动 HEAD，与提交路径同一个
//!   快照机制；快照失败不阻断但记日志（同 [`crate::commit`] 的取舍）。
//! - **PUSH_REJECTED 包装**（任务书要求 4）：push 结果里出现 non-fast-forward
//!   拒绝时，把结构化 outcome 转成带 **actions** 的 `AppError`（先拉取 /
//!   force-with-lease / 取消），让前端统一错误通道给出可执行的下一步；
//!   非 non-ff 的拒绝（权限、hook）保持 outcome 返回——那些不该引导用户
//!   去强推。
//! - **remote 管理**：名称校验（ref-format 的宽松子集：单段、无空格）与
//!   URL 校验（HTTPS/SSH/file 形状 + `git remote -v` 无法校验可达性，
//!   可达性由 fetch 验证）。
//!
//! # 取消
//!
//! fetch/pull/push 把 `CancellationToken` 透传到进程层：取消 = kill 子进程
//! （`kill_on_drop`），返回 `ErrorCode::Cancelled`。

use std::path::{Path, PathBuf};

use tokio_util::sync::CancellationToken;

use forgedesk_domain::git::{
    FetchOutcome, FetchSpec, PullOutcome, PullSpec, PushOutcome, PushSpec, Remote, RepoId,
};
use forgedesk_domain::{AppError, AppResult, ErrorCode};
use forgedesk_git_engine::engine::progress::ProgressSink;
use forgedesk_git_engine::engine::GitEngine;
use forgedesk_git_engine::engines::GitEngines;

use forgedesk_snapshot::{SnapshotKind, SnapshotManager, SnapshotRequest};

use forgedesk_storage::RepositoryStore;

use crate::credentials::{resolve_remote_url, CredentialContext, CredentialGate};

/// 远端同步服务。
pub struct SyncService<'a> {
    engines: &'a GitEngines,
    store: RepositoryStore<'a>,
    snapshots: &'a dyn SnapshotManager,
    /// 凭据门（T2.7）。为 `None` 时不做注入：网络操作只有匿名与 SSH 两条路。
    credentials: Option<&'a CredentialGate>,
    /// 每仓库绑定的账号登录名（T4.5）：同 host 多账号时，凭据门按它挑人。
    login_hint: Option<String>,
}

impl<'a> SyncService<'a> {
    /// 组装服务（快照管理器与提交/分支路径共享同一实例）。
    pub fn new(
        engines: &'a GitEngines,
        store: RepositoryStore<'a>,
        snapshots: &'a dyn SnapshotManager,
    ) -> Self {
        Self {
            engines,
            store,
            snapshots,
            credentials: None,
            login_hint: None,
        }
    }

    /// 接上"每仓库绑定的账号"（T4.5）：同 host 多账号时凭据门按它挑人。
    ///
    /// 值来自仓库级设置 `accounts.preferredAccount` 的解析结果，由命令层
    /// 传入；`None` 保持旧行为（URL 里的用户名优先，否则取第一条凭据）。
    #[must_use]
    pub fn with_login_hint(mut self, login_hint: Option<String>) -> Self {
        self.login_hint = login_hint;
        self
    }

    /// 接上凭据门（T2.7）：fetch/pull/push 会为远端解析并注入凭据。
    #[must_use]
    pub fn with_credentials(mut self, credentials: &'a CredentialGate) -> Self {
        self.credentials = Some(credentials);
        self
    }

    /// 接上凭据门；`None`（宿主拿不到自身可执行文件路径）时退化为匿名/SSH。
    ///
    /// 存在的理由：调用方持有的往往是 `Option<Arc<CredentialGate>>`（任务闭包里
    /// 只能克隆 Arc），用这个形态可以一行接上，不必在每处写一遍 `match`——
    /// 而"漏接一次"正是 T2.7 已经踩过的坑（凭据门没接上，网络操作悄悄变成匿名）。
    #[must_use]
    pub fn with_credential_gate(mut self, credentials: Option<&'a CredentialGate>) -> Self {
        self.credentials = credentials;
        self
    }

    fn resolve_workdir(&self, repo_id: i64) -> AppResult<PathBuf> {
        let record = self.store.find_by_id(repo_id)?.ok_or(
            AppError::new(ErrorCode::NotFound, "the repository record does not exist")
                .with_detail(format!("repo_id: {repo_id}")),
        )?;
        Ok(PathBuf::from(record.path))
    }

    /// 解析这次操作要用的凭据上下文（含"连续失败上限"的闸门判断）。
    ///
    /// 远端名不存在时返回**匿名**上下文：交给 git 报"远端不存在"，
    /// 比我们编一个错误更准确；那种情况下也没有 host 可以记账。
    fn auth_context(
        &self,
        workdir: &Path,
        remote: Option<&str>,
    ) -> AppResult<CredentialContext<'a>> {
        let Some(url) = resolve_remote_url(self.engines, workdir, remote)? else {
            return Ok(CredentialContext::anonymous());
        };
        CredentialContext::resolve(self.credentials, &url, self.login_hint.as_deref())
    }

    // ------------------------------------------------------------ 同步

    /// 拉取远端引用。不移动任何分支，安全。
    pub fn fetch(
        &self,
        repo_id: i64,
        spec: FetchSpec,
        progress: &ProgressSink,
        cancel: &CancellationToken,
    ) -> AppResult<FetchOutcome> {
        let workdir = self.resolve_workdir(repo_id)?;
        let context = self.auth_context(&workdir, spec.remote.as_deref())?;
        let repo = RepoId::new(workdir);
        let outcome =
            match self
                .engines
                .write()
                .fetch(&repo, spec, progress, cancel, context.auth())
            {
                Ok(outcome) => outcome,
                Err(error) => return Err(context.describe_failure(error)),
            };
        context.note_success();
        Ok(outcome)
    }

    /// 拉取并合并/变基。**先打 `PreSync` 快照**（会移动 HEAD）。
    pub fn pull(
        &self,
        repo_id: i64,
        spec: PullSpec,
        progress: &ProgressSink,
        cancel: &CancellationToken,
    ) -> AppResult<PullOutcome> {
        let workdir = self.resolve_workdir(repo_id)?;
        let context = self.auth_context(&workdir, spec.remote.as_deref())?;
        let snapshot_id = self.snapshot_before(repo_id, &workdir, "pull");
        let repo = RepoId::new(workdir);
        let mut outcome =
            match self
                .engines
                .write()
                .pull(&repo, spec, progress, cancel, context.auth())
            {
                Ok(outcome) => outcome,
                Err(error) => return Err(context.describe_failure(error)),
            };
        outcome.snapshot_id = snapshot_id;
        context.note_success();
        Ok(outcome)
    }

    /// 推送。non-fast-forward 拒绝时转成带 actions 的 `PUSH_REJECTED`。
    pub fn push(
        &self,
        repo_id: i64,
        spec: PushSpec,
        progress: &ProgressSink,
        cancel: &CancellationToken,
    ) -> AppResult<PushOutcome> {
        let workdir = self.resolve_workdir(repo_id)?;
        let context = self.auth_context(&workdir, spec.remote.as_deref())?;
        let repo = RepoId::new(workdir);
        let outcome = match self
            .engines
            .write()
            .push(&repo, spec, progress, cancel, context.auth())
        {
            Ok(outcome) => outcome,
            Err(error) => return Err(context.describe_failure(error)),
        };
        context.note_success();
        if let Some(rejection) = outcome.rejections.iter().find(|r| r.non_fast_forward) {
            // 任务书要求 4：non-ff 拒绝转成带 actions 的结构化错误。
            // label_key 走 errors 命名空间的既有键（"重试"类文案由前端 i18n 渲染），
            // id 与 command 让前端能直接接按钮（fetch → git_fetch；重试 → 原样重发）。
            use forgedesk_domain::error::FixAction;
            return Err(AppError::new(
                ErrorCode::PushRejected,
                "the push was rejected because the remote has commits you do not have",
            )
            .with_detail(rejection.reason.clone())
            .with_action(FixAction::new(
                "fetch-first",
                "errors:actions.pushFetchFirst",
                "git_fetch",
            ))
            .with_action(FixAction::new(
                "force-with-lease",
                "errors:actions.pushForceWithLease",
                "noop",
            ))
            .with_action(FixAction::new("cancel", "errors:actions.cancel", "noop")));
        }
        Ok(outcome)
    }

    /// 探活远端（"测试连接"）：返回远端引用条数。
    ///
    /// 入参二选一（设置页两种用法都要支持）：
    /// - `url`：直接测一个手填的地址（还没保存的远端）；
    /// - `repo_id`（+ 可选 `remote`）：测某个仓库的远端（缺省当前分支的上游远端 → `origin`）。
    ///
    /// 这是**只读**操作：`ls-remote` 不写引用、不动工作区，因此它也是
    /// "改完凭据后确认一下"的安全动作。失败走与同步操作同一套凭据门逻辑
    /// （认证类失败计数 + 达到上限换文案）。
    pub fn probe_remote(
        &self,
        repo_id: Option<i64>,
        remote: Option<&str>,
        url: Option<&str>,
    ) -> AppResult<usize> {
        let (target_url, cwd) = match url {
            Some(url) => {
                if url.trim().is_empty() {
                    return Err(AppError::new(
                        ErrorCode::Validation,
                        "the remote URL must not be empty",
                    ));
                }
                // 只给了 URL 时没有仓库上下文：`ls-remote` 不需要仓库，
                // 但需要一个工作目录（git 会从这里读配置，例如 insteadOf 改写）
                let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
                (url.to_owned(), cwd)
            }
            None => {
                let repo_id = repo_id.ok_or_else(|| {
                    AppError::new(
                        ErrorCode::Validation,
                        "either a remote URL or a repository id is required",
                    )
                })?;
                let workdir = self.resolve_workdir(repo_id)?;
                let resolved = resolve_remote_url(self.engines, &workdir, remote)?;
                let url = resolved.ok_or_else(|| {
                    AppError::new(ErrorCode::NotFound, "the remote does not exist")
                        .with_hint(remote.unwrap_or("origin").to_owned())
                })?;
                (url, workdir)
            }
        };

        // 探活也是一次真实的凭据尝试：同样计数、同样受"连续失败上限"约束。
        // 否则用户可以在被锁之后继续用"测试连接"刷账号——而那正是上限要防的事。
        let context = CredentialContext::resolve(self.credentials, &target_url, None)?;
        match self
            .engines
            .write()
            .probe_remote(&cwd, &target_url, context.auth())
        {
            Ok(refs) => {
                context.note_success();
                Ok(refs)
            }
            Err(error) => Err(context.describe_failure(error)),
        }
    }

    // ------------------------------------------------------------ 远端管理

    /// 远端列表。
    pub fn remote_list(&self, repo_id: i64) -> AppResult<Vec<Remote>> {
        let workdir = self.resolve_workdir(repo_id)?;
        self.engines.read().remote_list(&RepoId::new(workdir))
    }

    /// 添加远端（名称与 URL 先校验）。
    pub fn remote_add(&self, repo_id: i64, name: &str, url: &str) -> AppResult<()> {
        validate_remote_name(name).map_err(|reason| invalid_remote("remote name", name, reason))?;
        validate_remote_url(url)?;
        let workdir = self.resolve_workdir(repo_id)?;
        self.engines
            .write()
            .remote_add(&RepoId::new(workdir), name, url)
    }

    /// 删除远端。
    pub fn remote_remove(&self, repo_id: i64, name: &str) -> AppResult<()> {
        let workdir = self.resolve_workdir(repo_id)?;
        self.engines
            .write()
            .remote_remove(&RepoId::new(workdir), name)
    }

    /// 重命名远端。
    pub fn remote_rename(&self, repo_id: i64, old: &str, new: &str) -> AppResult<()> {
        validate_remote_name(new).map_err(|reason| invalid_remote("remote name", new, reason))?;
        let workdir = self.resolve_workdir(repo_id)?;
        self.engines
            .write()
            .remote_rename(&RepoId::new(workdir), old, new)
    }

    /// 改远端 URL。
    pub fn remote_set_url(&self, repo_id: i64, name: &str, url: &str) -> AppResult<()> {
        validate_remote_url(url)?;
        let workdir = self.resolve_workdir(repo_id)?;
        self.engines
            .write()
            .remote_set_url(&RepoId::new(workdir), name, url)
    }

    /// 危险操作前打快照（pull 会移动 HEAD）；失败**不阻断**，返回 `None`。
    fn snapshot_before(&self, repo_id: i64, workdir: &std::path::Path, label: &str) -> Option<i64> {
        let request = SnapshotRequest {
            repo_id,
            workdir,
            label: SnapshotKind::PreSync.key(),
            kind: SnapshotKind::PreSync,
        };
        match self.snapshots.create(&request) {
            Ok(outcome) => Some(outcome.id),
            Err(error) => {
                tracing::warn!(error = %error.message(), label, "同步前未能创建快照");
                None
            }
        }
    }
}

/// 校验远端名：单段、无空白、不含 ref-format 禁用字符（远端名不进引用，
/// 但会拼进 `refs/remotes/<name>/*`，因此按 ref 分量的规则校验）。
fn validate_remote_name(name: &str) -> Result<(), &'static str> {
    if name.is_empty() {
        return Err("远端名不能为空");
    }
    if name.contains('/') || name.contains(char::is_whitespace) {
        return Err("远端名不能包含斜杠或空白");
    }
    if name.starts_with('.') || name.starts_with('-') || name.starts_with('/') {
        return Err("远端名不能以点、斜杠或连字符开头");
    }
    if name
        .chars()
        .any(|ch| matches!(ch, '~' | '^' | ':' | '?' | '*' | '[' | '\\' | '\u{7f}'))
    {
        return Err("远端名包含 git 不允许的字符");
    }
    Ok(())
}

/// 校验远端 URL 的形状（**不**校验可达性——那只有网络操作本身能验证）。
///
/// 接受 HTTPS / SSH（`git@host:path` 与 `ssh://`）/ `file://` / 本地路径；
/// 其余前缀（ftp:// 等 git 不支持的）拒绝并给协议说明——这正是"建议"
/// 的一部分：HTTPS 免配钥、SSH 免输密码，两者都行时推荐 HTTPS。
fn validate_remote_url(url: &str) -> AppResult<()> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Err(AppError::new(
            ErrorCode::Validation,
            "the remote URL is empty",
        ));
    }
    let acceptable = trimmed.starts_with("https://")
        || trimmed.starts_with("http://")
        || trimmed.starts_with("ssh://")
        || trimmed.starts_with("file://")
        || trimmed.starts_with("git@")
        // 本地路径（本地 bare 模拟与网络盘仓库）
        || (!trimmed.contains("://") && !trimmed.starts_with('-'));
    if acceptable {
        Ok(())
    } else {
        Err(AppError::new(
            ErrorCode::Validation,
            "the remote URL scheme is not supported",
        )
        .with_detail(format!("url: {trimmed}"))
        .with_hint("use HTTPS (https://host/owner/repo.git) or SSH (git@host:owner/repo.git)"))
    }
}

fn invalid_remote(kind: &str, value: &str, reason: &str) -> AppError {
    AppError::new(ErrorCode::Validation, format!("the {kind} is invalid"))
        .with_detail(format!("{value}: {reason}"))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{validate_remote_name, validate_remote_url};

    #[test]
    fn remote_names_follow_ref_component_rules() {
        assert!(validate_remote_name("origin").is_ok());
        assert!(validate_remote_name("upstream-mirror").is_ok());
        assert!(validate_remote_name("").is_err());
        assert!(validate_remote_name("a/b").is_err(), "远端名是单段");
        assert!(validate_remote_name("a b").is_err());
        assert!(validate_remote_name("-flag").is_err());
    }

    #[test]
    fn remote_urls_accept_https_ssh_file_and_local_paths_only() {
        assert!(validate_remote_url("https://github.com/o/r.git").is_ok());
        assert!(validate_remote_url("git@github.com:o/r.git").is_ok());
        assert!(validate_remote_url("ssh://git@host/o/r.git").is_ok());
        assert!(validate_remote_url("file:///tmp/repo").is_ok());
        assert!(validate_remote_url("/tmp/repo").is_ok());
        assert!(validate_remote_url("ftp://host/repo").is_err());
        assert!(validate_remote_url("").is_err());
    }
}
