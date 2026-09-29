//! 储藏（stash）用例（T2.8）。
//!
//! # 这一层的三件事
//!
//! 1. **编排快照**（红线 R7）：储藏会把改动从工作区搬走，应用储藏又可能冲突。
//!    两者都在动手之前打一个 `PreWorktreeChange` 快照。
//! 2. **把"什么都没做"与"失败"分开**：在一个干净的仓库里点"储藏"，git 以 0 退出
//!    且不产生任何条目——那是**正常结果**，不是错误（[`StashSaveOutcome::stashed`]）。
//! 3. **把 stash 的三父结构摊平给界面**：[`StashShowOutcome`] 同时给出"相对 base 的
//!    变更"与"未跟踪文件的变更"，因为后者**不在**前者里面（见该类型的说明）。
//!
//! # 审计在哪
//!
//! 与 T2.6 的同步路径一致：审计由 commands 层写（它知道 IPC 的参数字段），
//! 本层只负责"动手之前必须有快照"这件事。

use std::path::PathBuf;

use forgedesk_domain::git::{
    validate_ref_name, DiffReport, DiffSpec, DiffTarget, RepoId, StashEntry, StashOutcome,
    StashSpec, EMPTY_TREE_OID,
};
use forgedesk_domain::{AppError, AppResult, ErrorCode};
use forgedesk_git_engine::engine::GitEngine;
use forgedesk_git_engine::engines::GitEngines;
use forgedesk_snapshot::{SnapshotId, SnapshotKind, SnapshotManager, SnapshotRequest};
use forgedesk_storage::RepositoryStore;

/// 一次储藏的**结果**。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StashSaveOutcome {
    /// 是否真的产生了一条 stash。
    ///
    /// `false` 表示"没有任何可储藏的内容"（git 以 0 退出、什么都不做）。
    /// 把它当失败会在用户清理干净工作区之后弹一个红框。
    pub stashed: bool,
    /// 新产生的那条（`stashed` 为真时一定存在）。
    pub entry: Option<StashEntry>,
}

/// 某条 stash 相对它 base 的变更。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StashShowOutcome {
    /// 这条 stash 本身。
    pub entry: StashEntry,
    /// 相对 base 的变更（已跟踪文件的改动）。
    pub diff: DiffReport,
    /// 未跟踪文件的变更（只有 `-u` 创建的 stash 才有）。
    ///
    /// 单独一份的原因：stash 是**多父提交**，未跟踪文件在第三个父提交里，
    /// 而"相对 base 的 diff"只覆盖第一个父到 stash 之间已跟踪文件的改动。
    /// 少了这一份，界面显示的是一份**看起来完整、其实残缺**的 diff。
    pub untracked: Option<DiffReport>,
}

/// 丢弃（`drop` / `clear`）的结果。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StashDiscardOutcome {
    /// 被丢掉的条目。
    ///
    /// 为什么要把它们带出来：stash 提交在 `gc` 真正回收之前都还能按 oid 找回，
    /// 因此"刚丢掉的是哪个 oid"是审计（与用户自救）需要的唯一线索。
    pub dropped: Vec<StashEntry>,
}

/// 储藏用例。
pub struct StashService<'a> {
    engines: &'a GitEngines,
    store: RepositoryStore<'a>,
    snapshots: &'a dyn SnapshotManager,
}

impl<'a> StashService<'a> {
    /// 组装服务。
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

    /// 记录 id → 工作区路径。
    fn resolve_workdir(&self, repo_id: i64) -> AppResult<PathBuf> {
        let record = self.store.find_by_id(repo_id)?.ok_or(
            AppError::new(ErrorCode::NotFound, "the repository record does not exist")
                .with_detail(format!("repo_id: {repo_id}")),
        )?;
        Ok(PathBuf::from(record.path))
    }

    /// 动手前打一个快照；失败**不阻断**（如实降级，返回 `None`）。
    ///
    /// 与 `SyncService::snapshot_before` 同一策略：快照是安全网，不是前置条件。
    /// 快照失败时让操作失败，用户会以为"储藏坏了"。
    fn snapshot(
        &self,
        repo_id: i64,
        workdir: &std::path::Path,
        kind: SnapshotKind,
    ) -> Option<SnapshotId> {
        let request = SnapshotRequest {
            repo_id,
            workdir,
            label: kind.key(),
            kind,
        };
        match self.snapshots.create(&request) {
            Ok(id) => Some(id),
            Err(error) => {
                tracing::warn!(error = %error.message(), kind = kind.key(), "储藏操作前未能创建快照");
                None
            }
        }
    }

    // ------------------------------------------------------------ 储藏

    /// 储藏当前改动。
    pub fn save(&self, repo_id: i64, spec: &StashSpec) -> AppResult<StashSaveOutcome> {
        let workdir = self.resolve_workdir(repo_id)?;
        let repo = RepoId::new(workdir.clone());
        let before = self.engines.read().stash_list(&repo)?.len();

        self.snapshot(repo_id, &workdir, SnapshotKind::PreWorktreeChange);

        self.engines.write().stash(&repo, spec.clone())?;

        // 用"列表是否变长"判断有没有真的储藏，而不是去解析 git 的输出文案：
        // 文案随版本与语言变（`LC_ALL=C` 也只是压住一部分），长度不会。
        let entries = self.engines.read().stash_list(&repo)?;
        let stashed = entries.len() > before;
        Ok(StashSaveOutcome {
            stashed,
            entry: if stashed {
                // 新条目一定在最前面（stash 是栈）
                entries.into_iter().next()
            } else {
                None
            },
        })
    }

    /// stash 列表（新的在前）。
    pub fn list(&self, repo_id: i64) -> AppResult<Vec<StashEntry>> {
        let workdir = self.resolve_workdir(repo_id)?;
        self.engines.read().stash_list(&RepoId::new(workdir))
    }

    /// 某条 stash 相对其 base 的变更。
    ///
    /// # 这一段的 `index` 是"用户此刻看到的第几项"
    ///
    /// `stash@{n}` 只要有任何 stash 操作就会重排。界面永远先列一遍、再按用户点的那一
    /// 项的 `index` 请求，因此这里的"位置"语义是对的；但**审计与长期引用**必须用 oid
    /// （返回值里带着）。
    pub fn show(&self, repo_id: i64, index: usize) -> AppResult<StashShowOutcome> {
        let workdir = self.resolve_workdir(repo_id)?;
        let repo = RepoId::new(workdir);

        let entry = self
            .engines
            .read()
            .stash_list(&repo)?
            .into_iter()
            .find(|entry| entry.index == index)
            .ok_or_else(|| {
                AppError::new(ErrorCode::NotFound, "there is no stash entry at that index")
                    .with_detail(format!("stash@{{{index}}}"))
            })?;

        let base = entry.base_oid.clone().ok_or_else(|| {
            // 拿不到 base 就不给 diff：一份"相对错误基线"的 diff 比没有更糟，
            // 用户会据此判断这条 stash 值不值得留
            AppError::new(
                ErrorCode::Internal,
                "the stash entry has no base commit to compare against",
            )
            .with_detail(entry.oid.clone())
        })?;

        let diff = self.engines.write().diff(
            &repo,
            DiffSpec::new(DiffTarget::Between {
                from: base,
                to: entry.oid.clone(),
            }),
        )?;

        // 未跟踪文件：与**空树**比较，因此结果全是新增文件
        let untracked = match entry.untracked_oid.clone() {
            Some(oid) => Some(self.engines.write().diff(
                &repo,
                DiffSpec::new(DiffTarget::Between {
                    from: EMPTY_TREE_OID.to_owned(),
                    to: oid,
                }),
            )?),
            None => None,
        };

        Ok(StashShowOutcome {
            entry,
            diff,
            untracked,
        })
    }

    /// 应用某条 stash 并**保留**它（`apply`）。
    pub fn apply(&self, repo_id: i64, spec: &StashSpec) -> AppResult<StashOutcome> {
        self.apply_or_pop(repo_id, spec)
    }

    /// 应用某条 stash 并删除它（`pop`）。
    ///
    /// 冲突时**不会**删除：git 在 pop 冲突时会保留那条 stash（这正是用户再次尝试的
    /// 依据），我们把这个事实原样交给界面。
    pub fn pop(&self, repo_id: i64, spec: &StashSpec) -> AppResult<StashOutcome> {
        self.apply_or_pop(repo_id, spec)
    }

    fn apply_or_pop(&self, repo_id: i64, spec: &StashSpec) -> AppResult<StashOutcome> {
        let workdir = self.resolve_workdir(repo_id)?;
        let repo = RepoId::new(workdir.clone());

        self.snapshot(repo_id, &workdir, SnapshotKind::PreWorktreeChange);

        self.engines.write().stash(&repo, spec.clone())
    }

    // ------------------------------------------------------------ 丢弃

    /// 丢弃某一条（不可逆）。
    ///
    /// # 这里**不**打快照（有意为之）
    ///
    /// 快照记录的是 HEAD + 索引 + 工作区，而 stash 的内容**不在工作区里**——
    /// 打一份工作区快照并不能把丢掉的那条 stash 找回来，只会给用户一个假的安心。
    /// 真正的安全网是另外两样：返回值里带着被丢掉的 oid（写进审计），以及 git 在
    /// `gc` 回收之前都还能按 oid 找回那个悬挂提交。
    pub fn drop_one(&self, repo_id: i64, index: usize) -> AppResult<StashDiscardOutcome> {
        let workdir = self.resolve_workdir(repo_id)?;
        let repo = RepoId::new(workdir);

        // 先记下要丢的是什么：丢弃之后它就只剩审计里的 oid 了
        let target = self
            .engines
            .read()
            .stash_list(&repo)?
            .into_iter()
            .find(|entry| entry.index == index)
            .ok_or_else(|| {
                AppError::new(ErrorCode::NotFound, "there is no stash entry at that index")
                    .with_detail(format!("stash@{{{index}}}"))
            })?;

        self.engines.write().stash(&repo, StashSpec::drop(index))?;

        Ok(StashDiscardOutcome {
            dropped: vec![target],
        })
    }

    /// 丢弃**全部**（不可逆）。安全网与 [`Self::drop_one`] 相同。
    pub fn clear(&self, repo_id: i64) -> AppResult<StashDiscardOutcome> {
        let workdir = self.resolve_workdir(repo_id)?;
        let repo = RepoId::new(workdir);

        let dropped = self.engines.read().stash_list(&repo)?;
        self.engines.write().stash(&repo, StashSpec::clear())?;

        Ok(StashDiscardOutcome { dropped })
    }

    // ------------------------------------------------------------ 建分支

    /// 从某条 stash 创建分支并把它应用过去（`git stash branch`）。
    ///
    /// 这是 pop 冲突之后的正规出路：新分支从 stash 的 base 提交开始，
    /// 因此那条 stash 一定可以干净地应用上去。注意它会**切换分支**。
    pub fn branch(&self, repo_id: i64, index: usize, name: &str) -> AppResult<String> {
        validate_ref_name(name).map_err(|reason| {
            AppError::new(ErrorCode::Validation, "the branch name is not valid")
                .with_detail(reason.to_owned())
                .with_hint(name.to_owned())
        })?;

        let workdir = self.resolve_workdir(repo_id)?;
        let repo = RepoId::new(workdir.clone());

        // 切换分支会移动 HEAD
        self.snapshot(repo_id, &workdir, SnapshotKind::PreHeadMove);

        self.engines
            .write()
            .stash(&repo, StashSpec::branch(index, name))?;

        Ok(name.to_owned())
    }
}
