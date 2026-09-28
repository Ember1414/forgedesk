//! 提交详情（T2.4）：单提交的元数据、统计、文件清单与状态标记。
//!
//! # 编排，而不是新引擎能力
//!
//! 详情需要的每一块数据都已有引擎入口：**元数据走 CLI 的 `show`**——refs（`%D`）
//! 与签名状态（`%G?`）只有 CLI 给得出（libgit2 的 `to_commit` 有意留空这两项），
//! 详情是低频查询，一次子进程启动换 ref 胶囊与签名徽标是值得的；统计与文件清单
//! 走 **CLI 的 diff**（T1.5 起补丁/统计的唯一数据源，见
//! [`crate::workspace::WorkspaceService::diff`] 的说明）；`is_head` 走 libgit2 的
//! `head_oid`；`is_pushed` 走 `remote_refs_containing`（仅 CLI 实现）；
//! 网页 URL 由 `remote_list` 的 fetch URL 推断。本模块的职责只有三件事：
//! **校验入参**、**把父提交选择翻译成正确的 diff 目标**、**拼装 DTO**。
//!
//! # 合并提交的父选择（验收项）
//!
//! 合并提交相对第一父与相对第二父的 diff 是两个不同的问题，任务书要求都能看。
//! 实现上**必须**用 [`DiffTarget::Between`] 显式指定父提交：CLI 对 `Commit`
//! 目标（`git show`）在 merge 上输出 combined diff，它回答的是"这次合并带来了
//! 什么冲突解决"，不是"相对某个父改了什么"——两个问题都合法，但详情面板
//! 的文件清单要的是后者。只有**根提交**（没有父）才用 `Commit` 目标，
//! 让引擎自己处理"与空树比较"。

use std::path::PathBuf;

use forgedesk_domain::git::{
    DiffChangeKind, DiffSpec, DiffTarget, RepoId, Signature, SignatureStatus,
};
use forgedesk_domain::{AppError, AppResult, ErrorCode};
use forgedesk_git_engine::engine::GitEngine;
use forgedesk_git_engine::engines::GitEngines;
use serde::Serialize;

use forgedesk_storage::RepositoryStore;

/// 完整提交 oid 的长度（详情面板只接受完整 oid：前端拿到的本来就是完整的，
/// 收窄输入面可以少解释"短 oid 歧义"这类问题）。
pub const FULL_OID_LENGTH: usize = 40;

/// 提交的元数据（`show` 的结果 + 短 oid）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitMeta {
    /// 完整 oid。
    /// 完整 oid。
    pub oid: String,
    /// 短 oid（7 位；与 `git log --oneline` 的默认长度一致）。
    pub short_oid: String,
    /// 父提交 oid，顺序与 Git 一致（第一个是 first-parent）；根提交为空。
    /// 父提交 oid，顺序与 Git 一致（第一个是 first-parent）；根提交为空。
    pub parents: Vec<String>,
    /// 作者身份。
    pub author: Signature,
    /// 提交者身份（可能与作者不同：rebase / cherry-pick 会改提交者）。
    pub committer: Signature,
    /// 提交信息标题。
    pub subject: String,
    /// 提交信息正文（`show` 带 `%b`，与列表查询不同）。
    pub body: Option<String>,
    /// 签名状态（`git log --format=%G?` 的语义；不引入 GPG 库）。
    pub signature: SignatureStatus,
}

/// 变更统计汇总。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitStats {
    /// 变更文件数（相对 `parent_index` 指定的父提交）。
    pub files_changed: u64,
    /// 新增行数合计（二进制文件不计）。
    pub insertions: u64,
    /// 删除行数合计（二进制文件不计）。
    pub deletions: u64,
}

/// 文件清单里的一项（文件级统计；行级内容按需经 `workspace_diff` 拉取）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitFileChange {
    /// 路径（重命名/复制时是目标路径）。
    pub path: String,
    /// 重命名/复制的来源路径。
    pub original_path: Option<String>,
    /// 变更类别（serde 小驼峰：`"typeChanged"` 等，与 workspace diff DTO 一致）。
    pub kind: DiffChangeKind,
    /// 是否二进制（无行级统计）。
    /// 是否二进制（无行级统计）。
    pub binary: bool,
    /// 新增行数（二进制为 0）。
    pub additions: u64,
    /// 删除行数（二进制为 0）。
    pub deletions: u64,
    /// 该文件在详情查询里被截断的标记（统计口径不含行级内容，通常为 false）。
    pub truncated: bool,
}

/// 一次提交详情查询的结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitDetail {
    /// 提交元数据。
    pub meta: CommitMeta,
    /// 指向该提交的引用（`%D` 的原文，如 `HEAD -> main`、`tag: v1.0.0`）。
    /// 指向该提交的引用（`%D` 的原文，如 `HEAD -> main`、`tag: v1.0.0`）。
    pub refs: Vec<String>,
    /// 变更统计汇总。
    pub stats: CommitStats,
    /// 文件清单（相对 `parent_index` 指定的父提交）。
    pub files: Vec<CommitFileChange>,
    /// 是否为合并提交（父提交数 > 1）。
    pub is_merge: bool,
    /// 是否为当前 HEAD。
    pub is_head: bool,
    /// 是否已被某个远端分支包含（`git branch -r --contains` 非空）。
    pub is_pushed: bool,
    /// 由 origin 的 fetch URL 推断的网页 URL；无法识别时为 `None`。
    pub web_url: Option<String>,
    /// 本次文件清单相对哪个父提交（`parents` 的下标；非根提交才有意义）。
    pub parent_index: usize,
}

/// 提交详情服务。
pub struct CommitDetailService<'a> {
    engines: &'a GitEngines,
    store: RepositoryStore<'a>,
}

impl<'a> CommitDetailService<'a> {
    /// 组装服务（与其余服务共用同一批引擎与同一个数据库）。
    pub fn new(engines: &'a GitEngines, store: RepositoryStore<'a>) -> Self {
        Self { engines, store }
    }

    /// 解析记录 id 为工作区路径；记录不存在时返回 `NOT_FOUND`。
    ///
    /// 与 `WorkspaceService::resolve_workdir` / `HistoryService::resolve_workdir`
    /// 同一个实现；不合并成公共函数是为了保持各服务自包含（与 history.rs 的取舍一致）。
    fn resolve_workdir(&self, repo_id: i64) -> AppResult<PathBuf> {
        let record = self.store.find_by_id(repo_id)?.ok_or(
            AppError::new(ErrorCode::NotFound, "the repository record does not exist")
                .with_detail(format!("repo_id: {repo_id}")),
        )?;
        Ok(PathBuf::from(record.path))
    }

    /// 读取一次提交详情。能力等级：`ReadOnly`（不快照、不审计）。
    ///
    /// 错误：`VALIDATION`（oid 形状非法 / `parent_index` 越界）、
    /// `NOT_FOUND`（仓库记录不存在或提交不存在）。
    pub fn detail(
        &self,
        repo_id: i64,
        oid: &str,
        parent_index: Option<usize>,
    ) -> AppResult<CommitDetail> {
        validate_oid(oid)?;
        let workdir = self.resolve_workdir(repo_id)?;
        let repo = RepoId::new(workdir);

        // 元数据刻意走 CLI 的 `show`：refs（%D）与签名状态（%G?）只有 CLI 给得出
        // （libgit2 的 `to_commit` 有意留空，见引擎侧注释）。详情是低频查询，
        // 一次子进程启动（~15ms）换 ref 胶囊与签名徽标是值得的。
        let commit = self.engines.write().show(&repo, oid)?;
        let parents_len = commit.parents.len();
        let index = parent_index.unwrap_or(0);
        // 根提交没有父：`index == 0` 是合法的"与空树比较"（见下面的 target 选择）；
        // 其余情况按普通范围校验。
        let parent_ok = if parents_len == 0 {
            index == 0
        } else {
            index < parents_len
        };
        if !parent_ok {
            return Err(AppError::new(
                ErrorCode::Validation,
                "parent_index is out of range for this commit",
            )
            .with_detail(format!(
                "parent_index: {index}, parent count: {parents_len}"
            )));
        }

        // 非根提交一律显式 Between（见模块头：merge 上 `Commit` 目标是 combined diff）；
        // 根提交交给引擎的 Commit 目标（两侧都实现为"与空树比较"）。
        let target = if commit.parents.is_empty() {
            DiffTarget::Commit(oid.to_owned())
        } else {
            DiffTarget::Between {
                from: commit.parents[index].clone(),
                to: oid.to_owned(),
            }
        };
        let report = self.engines.write().diff(&repo, DiffSpec::new(target))?;
        let (insertions, deletions) = report.totals();
        let stats = CommitStats {
            files_changed: report.file_count() as u64,
            insertions,
            deletions,
        };
        let files = report
            .files
            .iter()
            .map(|file| CommitFileChange {
                path: file.path.to_string(),
                original_path: file.original_path.as_ref().map(|path| path.to_string()),
                kind: file.change,
                binary: file.binary,
                additions: file.additions,
                deletions: file.deletions,
                truncated: file.truncated,
            })
            .collect();

        let is_head = self
            .engines
            .read()
            .head_oid(&repo)?
            .is_some_and(|head| head == commit.oid);
        let is_pushed = !self
            .engines
            .write()
            .remote_refs_containing(&repo, &commit.oid)?
            .is_empty();
        let web_url = infer_web_url(&self.engines.read().remote_list(&repo)?);

        Ok(CommitDetail {
            meta: CommitMeta {
                short_oid: short_oid(&commit.oid),
                oid: commit.oid,
                parents: commit.parents,
                author: commit.author,
                committer: commit.committer,
                subject: commit.subject,
                body: commit.body,
                signature: commit.signature,
            },
            refs: commit.refs,
            stats,
            files,
            is_merge: parents_len > 1,
            is_head,
            is_pushed,
            web_url,
            parent_index: index,
        })
    }
}

/// 从 `show` 的完整 oid 截出短 oid（提交 oid 是 ASCII hex，按字节切安全）。
fn short_oid(oid: &str) -> String {
    oid.chars().take(7).collect()
}

/// 校验 oid 形状：完整 40 位十六进制。
///
/// 零信任（AGENTS §6）：oid 会进引擎参数。参数数组调用本身防住了注入，
/// 这里防的是"短 oid 前缀歧义"与"非 hex 字符"造成的意外解析行为。
fn validate_oid(oid: &str) -> AppResult<()> {
    let is_hex = |byte: &u8| byte.is_ascii_hexdigit();
    if oid.len() != FULL_OID_LENGTH || !oid.as_bytes().iter().all(is_hex) {
        return Err(AppError::new(
            ErrorCode::Validation,
            "the commit id must be a full 40-character hexadecimal oid",
        )
        .with_detail(format!("oid length: {}", oid.len())));
    }
    Ok(())
}

/// 从 fetch URL 推断网页 URL。
///
/// 只对**已知托管商**（GitHub / GitLab.com / Bitbucket.org）推断：
/// 自建实例的 URL 形态无法保证（GitLab 子组、Gitea 路径前缀等），
/// 猜错比不给更糟。HTTPS 与 `git@host:path` 形式的 SSH 都支持；
/// 换算规则是 strip `.git` + 把 SSH 的冒号换成斜杠。提交页锚点由前端拼
/// （不同托管商的 commit 路径不同，但 `/<owner>/<repo>` 的仓库根是通用的）。
fn infer_web_url(remotes: &[forgedesk_domain::git::Remote]) -> Option<String> {
    let origin = remotes
        .iter()
        .find(|remote| remote.name == "origin")
        .or_else(|| remotes.first())?;
    let url = origin.fetch_url.trim();

    // 取 "host/owner/repo(.git)" 形式的路径部分
    let (host, path) = if let Some(rest) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
    {
        let (host, path) = rest.split_once('/')?;
        (host, path)
    } else if let Some((host, path)) = url
        .strip_prefix("git@")
        .and_then(|rest| rest.split_once(':'))
    {
        (host, path)
    } else {
        return None;
    };
    if !matches!(host, "github.com" | "gitlab.com" | "bitbucket.org") {
        return None;
    }
    let path = path.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    if path.is_empty() {
        return None;
    }
    Some(format!("https://{host}/{path}"))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use forgedesk_domain::git::Remote;

    use super::infer_web_url;

    fn remote(name: &str, url: &str) -> Remote {
        Remote {
            name: name.to_owned(),
            fetch_url: url.to_owned(),
            push_url: None,
            kind: forgedesk_domain::git::RemoteKind::Https,
        }
    }

    #[test]
    fn https_origin_converts_to_a_web_url() {
        let remotes = [remote("origin", "https://github.com/owner/repo.git")];
        assert_eq!(
            infer_web_url(&remotes).as_deref(),
            Some("https://github.com/owner/repo")
        );
    }

    #[test]
    fn ssh_origin_converts_to_an_https_web_url() {
        let remotes = [remote("origin", "git@github.com:owner/repo.git")];
        assert_eq!(
            infer_web_url(&remotes).as_deref(),
            Some("https://github.com/owner/repo")
        );
    }

    #[test]
    fn gitlab_subgroups_are_kept_intact() {
        let remotes = [remote("origin", "https://gitlab.com/group/sub/repo.git")];
        assert_eq!(
            infer_web_url(&remotes).as_deref(),
            Some("https://gitlab.com/group/sub/repo")
        );
    }

    #[test]
    fn self_hosted_and_unknown_hosts_return_none() {
        assert_eq!(
            infer_web_url(&[remote("origin", "git@git.example.com:os/repo.git")]),
            None
        );
        assert_eq!(
            infer_web_url(&[remote("origin", "/mnt/repos/forgedesk")]),
            None
        );
        assert_eq!(infer_web_url(&[]), None);
    }

    #[test]
    fn a_remote_without_origin_falls_back_to_the_first_one() {
        let remotes = [
            remote("upstream", "https://github.com/a/b.git"),
            remote("origin", "https://gitlab.com/c/d.git"),
        ];
        assert_eq!(
            infer_web_url(&remotes).as_deref(),
            Some("https://gitlab.com/c/d")
        );
    }
}
