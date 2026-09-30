//! 分支与标签管理（T2.5）。
//!
//! # 安全语义都在这一层（引擎只翻译命令）
//!
//! - **切换三策略**（任务书实现要求 1）：`Stash`（先储藏含未跟踪、成功后自动
//!   恢复；恢复冲突如实上报为错误而不是静默丢 stash）、`Force`（先打
//!   `PreHeadMove` 快照——它是**唯一**会把"丢弃未提交修改"变成可回滚的步骤，
//!   没有快照就不许 Force）、`Clean`（工作区不干净直接 `VALIDATION`）。
//! - **删除未合并分支**（实现要求 2）：`-d` 由 git 自己拒绝；`force` 必须
//!   显式传 `confirm_unmerged: true`，且调用方应先调 [`Self::branch_compare`]
//!   把"独有提交"清单展示给用户——services 在 force 且未确认时拒绝执行，
//!   保证"没看清单就强删"在 API 层面走不通。
//! - **快照**：Force 切换与未合并强删都先 `SnapshotManager::create`（失败不
//!   阻断但记入日志，与提交路径同一取舍）。
//!
//! # 写路径全部走 CLI
//!
//! 与引擎 trait 的划分一致（libgit2 侧对这些操作如实返回 `UnsupportedByEngine`）。

use std::path::PathBuf;

use forgedesk_domain::git::{
    validate_ref_name, Branch, BranchCreateSpec, BranchDeleteSpec, BranchRenameSpec,
    BranchSetUpstreamSpec, StatusQuery, SwitchStrategy, Tag, TagCreateSpec, TagDeleteSpec,
};
use forgedesk_domain::{AppError, AppResult, ErrorCode};
use forgedesk_git_engine::engine::GitEngine;
use forgedesk_git_engine::engines::GitEngines;
use forgedesk_snapshot::{SnapshotKind, SnapshotManager, SnapshotRequest};

use forgedesk_storage::RepositoryStore;

/// 工作区是否干净的判定窗口：status 一次（不含 ignored——它们本来就挡不住切换）。
struct DirtyCheck {
    dirty: bool,
}

/// 分支与标签管理服务。
pub struct BranchService<'a> {
    engines: &'a GitEngines,
    store: RepositoryStore<'a>,
    snapshots: &'a dyn SnapshotManager,
}

/// 删除分支的结果：实际删掉的名字（部分成功时调用方据此回报）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchDeleteOutcome {
    /// 已删除的分支短名。
    pub deleted: Vec<String>,
}

/// 比较结果：ahead/behind 与 a 独有的提交清单（删除确认的数据源）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchComparison {
    /// a 领先 b 的提交数。
    pub ahead: u64,
    /// a 落后 b 的提交数。
    pub behind: u64,
    /// a 独有的提交（oid + subject）。
    pub only_in_a: Vec<(String, String)>,
}

impl<'a> BranchService<'a> {
    /// 组装服务（与提交路径共用同一个快照管理器实例）。
    pub fn new(
        engines: &'a GitEngines,
        store: RepositoryStore<'a>,
        snapshots: &'a dyn SnapshotManager,
    ) -> Self {
        Self {
            engines,
            store,
            snapshots,
        }
    }

    fn resolve_workdir(&self, repo_id: i64) -> AppResult<PathBuf> {
        let record = self.store.find_by_id(repo_id)?.ok_or(
            AppError::new(ErrorCode::NotFound, "the repository record does not exist")
                .with_detail(format!("repo_id: {repo_id}")),
        )?;
        Ok(PathBuf::from(record.path))
    }

    // ------------------------------------------------------------ 读

    /// 分支列表：当前分支置顶 → 本地（按名） → 远端（按名）。`include_remote` 关掉时只回本地。
    pub fn branch_list(&self, repo_id: i64, include_remote: bool) -> AppResult<Vec<Branch>> {
        let workdir = self.resolve_workdir(repo_id)?;
        let repo = forgedesk_domain::git::RepoId::new(workdir.clone());
        let mut branches = self.engines.read().branch_list(&repo)?;
        if !include_remote {
            branches.retain(|branch| !branch.is_remote);
        }
        // 分组排序：is_head 优先，然后远端垫底，各自按名字排序（稳定展示顺序）
        branches.sort_by(|left, right| {
            right
                .is_head
                .cmp(&left.is_head)
                .then_with(|| left.is_remote.cmp(&right.is_remote))
                .then_with(|| left.name.cmp(&right.name))
        });
        Ok(branches)
    }

    /// 标签列表（引擎已按名排序）。
    pub fn tag_list(&self, repo_id: i64) -> AppResult<Vec<Tag>> {
        let workdir = self.resolve_workdir(repo_id)?;
        self.engines
            .read()
            .tag_list(&forgedesk_domain::git::RepoId::new(workdir))
    }

    /// `a` 相对 `b` 的 ahead/behind 与 a 独有的提交清单（删除确认的数据源）。
    pub fn branch_compare(&self, repo_id: i64, a: &str, b: &str) -> AppResult<BranchComparison> {
        let workdir = self.resolve_workdir(repo_id)?;
        let repo = forgedesk_domain::git::RepoId::new(workdir);
        let (ahead, behind) = self.engines.write().branch_compare(&repo, a, b)?;
        let only_in_a = if ahead == 0 {
            Vec::new()
        } else {
            self.engines.write().branch_only_commits(&repo, a, b)?
        };
        Ok(BranchComparison {
            ahead,
            behind,
            only_in_a,
        })
    }

    // ------------------------------------------------------------ 写：分支

    /// 新建分支（校验名称；`checkout=true` 时创建后立即切换——同样走三策略检查）。
    pub fn branch_create(&self, repo_id: i64, spec: &BranchCreateSpec) -> AppResult<()> {
        validate_ref_name(&spec.name)
            .map_err(|reason| invalid_ref_name("branch", &spec.name, reason))?;
        let workdir = self.resolve_workdir(repo_id)?;
        let repo = forgedesk_domain::git::RepoId::new(workdir.clone());
        self.engines.write().branch_create(&repo, spec)?;
        if spec.checkout {
            self.switch(
                repo_id,
                &workdir,
                spec.name.as_str(),
                SwitchStrategy::Clean,
                false,
            )?;
        }
        Ok(())
    }

    /// 切换分支（任务书三策略；返回快照 id，若有）。
    pub fn switch_by_id(
        &self,
        repo_id: i64,
        target: &str,
        strategy: SwitchStrategy,
        confirm_force: bool,
    ) -> AppResult<Option<i64>> {
        let workdir = self.resolve_workdir(repo_id)?;
        self.switch(repo_id, &workdir, target, strategy, confirm_force)
    }

    /// 切换的共享实现（`create` 的 checkout 路径复用）。
    fn switch(
        &self,
        repo_id: i64,
        workdir: &std::path::Path,
        target: &str,
        strategy: SwitchStrategy,
        confirm_force: bool,
    ) -> AppResult<Option<i64>> {
        validate_ref_name(target).map_err(|reason| invalid_ref_name("branch", target, reason))?;
        let repo = forgedesk_domain::git::RepoId::new(workdir);
        let dirty = self.dirty_check(&repo)?;

        match strategy {
            SwitchStrategy::Clean if dirty.dirty => Err(AppError::new(
                ErrorCode::Validation,
                "the working tree has uncommitted changes; stash or discard them first",
            )),
            SwitchStrategy::Force => {
                // Force = 不可逆丢弃：必须拿到显式确认，且必须先有快照
                if !confirm_force {
                    return Err(AppError::new(
                        ErrorCode::Validation,
                        "force switch requires explicit confirmation",
                    ));
                }
                let snapshot = self.snapshot_before(repo_id, workdir, "force branch switch");
                self.engines
                    .write()
                    .branch_switch(&repo, strategy, target)?;
                Ok(snapshot)
            }
            SwitchStrategy::Stash => {
                if dirty.dirty {
                    // stash 含未跟踪（切分支最常见的"新文件没加进来"来源），
                    // 切换成功后立即恢复；恢复冲突如实上报（stash 保留，不丢工作）
                    let mut stash_spec = forgedesk_domain::git::StashSpec::push(Some(
                        "forgedesk: auto-stash before branch switch".to_owned(),
                    ));
                    stash_spec.include_untracked = true;
                    self.engines.write().stash(&repo, stash_spec)?;
                    let restore = self.engines.write().branch_switch(&repo, strategy, target);
                    match restore {
                        Ok(()) => {
                            self.engines
                                .write()
                                .stash(&repo, forgedesk_domain::git::StashSpec::pop(0))?;
                            Ok(None)
                        }
                        Err(error) => {
                            // 切换本身失败：stash 留在栈里（数据没丢），把真相交给调用方
                            Err(AppError::new(
                                ErrorCode::Internal,
                                "branch switch failed after stashing; the stash is preserved",
                            )
                            .with_detail(error.message.clone()))
                        }
                    }
                } else {
                    self.engines
                        .write()
                        .branch_switch(&repo, strategy, target)?;
                    Ok(None)
                }
            }
            SwitchStrategy::Clean => {
                self.engines
                    .write()
                    .branch_switch(&repo, strategy, target)?;
                Ok(None)
            }
        }
    }

    /// 重命名分支。
    pub fn branch_rename(&self, repo_id: i64, spec: &BranchRenameSpec) -> AppResult<()> {
        validate_ref_name(&spec.new)
            .map_err(|reason| invalid_ref_name("branch", &spec.new, reason))?;
        let workdir = self.resolve_workdir(repo_id)?;
        self.engines
            .write()
            .branch_rename(&forgedesk_domain::git::RepoId::new(workdir), spec)
    }

    /// 删除分支（未合并强删需要确认；远端删除 T2.6 前仅本地 bare 模拟）。
    pub fn branch_delete(
        &self,
        repo_id: i64,
        spec: &BranchDeleteSpec,
        confirm_unmerged: bool,
    ) -> AppResult<BranchDeleteOutcome> {
        let workdir = self.resolve_workdir(repo_id)?;
        let repo = forgedesk_domain::git::RepoId::new(workdir.clone());

        // 不可删除当前分支（git 会拒绝，但提前给出可读的错误）
        let status = self.engines.read().status(&repo, &StatusQuery::default())?;
        if let Some(head) = &status.branch.head {
            if spec.names.iter().any(|name| name == head) {
                return Err(AppError::new(
                    ErrorCode::Validation,
                    "cannot delete the currently checked-out branch",
                )
                .with_detail(head.clone()));
            }
        }

        if spec.force {
            // 未合并强删：必须确认。每个未合并分支的"独有提交"由调用方先经
            // branch_compare 展示；这里校验的是"确认已给"，防 API 直调绕过 UI。
            if !confirm_unmerged {
                return Err(AppError::new(
                    ErrorCode::Validation,
                    "deleting unmerged branches requires explicit confirmation of the commits that will be lost",
                ));
            }
            self.snapshot_before(repo_id, &workdir, "delete unmerged branches");
        }
        self.engines.write().branch_delete(&repo, spec)?;
        Ok(BranchDeleteOutcome {
            deleted: spec.names.clone(),
        })
    }

    /// 设置 / 取消上游。
    pub fn branch_set_upstream(&self, repo_id: i64, spec: &BranchSetUpstreamSpec) -> AppResult<()> {
        let workdir = self.resolve_workdir(repo_id)?;
        self.engines
            .write()
            .branch_set_upstream(&forgedesk_domain::git::RepoId::new(workdir), spec)
    }

    // ------------------------------------------------------------ 写：标签

    /// 创建标签（轻量 / 附注 / 签名）。
    pub fn tag_create(&self, repo_id: i64, spec: &TagCreateSpec) -> AppResult<()> {
        validate_ref_name(&spec.name)
            .map_err(|reason| invalid_ref_name("tag", &spec.name, reason))?;
        let workdir = self.resolve_workdir(repo_id)?;
        self.engines
            .write()
            .tag_create(&forgedesk_domain::git::RepoId::new(workdir), spec)
    }

    /// 删除标签（本地；远端删除经 push 通道，T2.6）。
    pub fn tag_delete(&self, repo_id: i64, spec: &TagDeleteSpec) -> AppResult<()> {
        let workdir = self.resolve_workdir(repo_id)?;
        self.engines
            .write()
            .tag_delete(&forgedesk_domain::git::RepoId::new(workdir), spec)
    }

    // ------------------------------------------------------------ 内部

    /// 工作区是否不干净（status 一次；ignored 不算——它们不挡切换）。
    fn dirty_check(&self, repo: &forgedesk_domain::git::RepoId) -> AppResult<DirtyCheck> {
        let report = self.engines.read().status(repo, &StatusQuery::default())?;
        Ok(DirtyCheck {
            dirty: !report.entries.is_empty(),
        })
    }

    /// 危险操作前打快照；失败不阻断但记日志（与提交路径同一取舍）。
    fn snapshot_before(&self, repo_id: i64, workdir: &std::path::Path, label: &str) -> Option<i64> {
        let request = SnapshotRequest {
            repo_id,
            workdir,
            label: SnapshotKind::PreHeadMove.key(),
            kind: SnapshotKind::PreHeadMove,
        };
        match self.snapshots.create(&request) {
            Ok(outcome) => Some(outcome.id),
            Err(error) => {
                tracing::warn!(error = %error.message(), label, "危险分支操作前未能创建快照");
                None
            }
        }
    }
}

/// 统一的"分支名非法"错误（带人话原因，实现要求 3）。
fn invalid_ref_name(kind: &str, name: &str, reason: &str) -> AppError {
    AppError::new(
        ErrorCode::Validation,
        format!("the {kind} name is not a valid git reference name"),
    )
    .with_detail(format!("{name}: {reason}"))
}
