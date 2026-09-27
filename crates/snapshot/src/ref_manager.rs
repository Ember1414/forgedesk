//! 基于自定义 ref 锚点的快照管理器（T1.9 的真实现）。
//!
//! # 快照的生命周期
//!
//! 1. **create**：读仓库事实（HEAD、`write-tree` 的索引树、未跟踪清单）→
//!    落库 → 用 `refs/forgedesk/snapshots/<id>` 锚住 HEAD 提交（防 gc）。
//!    落库在前、锚点在后，锚点失败就删掉记录——表里不留"没有锚点"的快照。
//! 2. **restore**：校验锚点仍在 → 给当前状态打"回滚前快照" → `reset --hard`
//!    → `read-tree` 恢复索引 → 用 git 的事实核对 HEAD 与索引树 → 对不上
//!    **自动回到回滚前快照**。
//! 3. **prune**：数量与年龄两个条件先到者生效；清理时 ref 与记录一起删
//!    （只删记录会让 ref 变成永远不被 gc 的孤儿，只删 ref 会留下指向空对象的记录）。
//!
//! # 为什么读引擎也要用
//!
//! 校验（"恢复后的 HEAD 是不是快照记录的那个"）必须走一条**与写操作不同的路径**：
//! 用写完再自己读写的同一套命令来验证，等于让考生批自己的卷子。
//! 读引擎（libgit2）与 CLI 是两条独立实现（T1.2 的差分测试保证它们对同一状态
//! 给出同一结论），这个校验才有意义。

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use forgedesk_domain::git::{EntryKind, RepoId, ResetMode, ResetSpec, StatusQuery};
use forgedesk_git_engine::engine::GitEngine;
use forgedesk_git_engine::engines::GitEngines;
use forgedesk_storage::{Database, NewSnapshot, RepositoryStore, SnapshotStore};

use crate::{
    RestoreReport, RetentionPolicy, SnapshotDiff, SnapshotError, SnapshotId, SnapshotKind,
    SnapshotManager, SnapshotMeta, SnapshotRequest,
};

/// 快照锚点 ref 的前缀（完整形如 `refs/forgedesk/snapshots/<id>`）。
pub const SNAPSHOT_REF_PREFIX: &str = "refs/forgedesk/snapshots/";

/// 默认保留条数（任务定义：每仓库最近 50 条）。
pub const DEFAULT_MAX_COUNT: u32 = 50;

/// 默认保留天数（任务定义：30 天，与条数先到者生效）。
pub const DEFAULT_MAX_AGE_DAYS: u32 = 30;

/// 可注入的时间源（测试要能推进时间来验证保留策略）。
type SnapshotClock = Arc<dyn Fn() -> i64 + Send + Sync>;

/// 基于自定义 ref 锚点的 [`SnapshotManager`] 实现。
///
/// 持有引擎组合与数据库：快照是"git 事实 + 一行记录"的组合体，
/// 只懂 git 不懂库（或者反过来）都无法保证两者一致。
pub struct RefSnapshotManager {
    engines: Arc<GitEngines>,
    database: Arc<Database>,
    clock: SnapshotClock,
    policy: RetentionPolicy,
}

impl std::fmt::Debug for RefSnapshotManager {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RefSnapshotManager")
            .field("policy", &self.policy)
            .finish_non_exhaustive()
    }
}

impl RefSnapshotManager {
    /// 创建快照管理器（默认保留策略：50 条 / 30 天，先到者生效）。
    pub fn new(engines: Arc<GitEngines>, database: Arc<Database>) -> Self {
        Self {
            engines,
            database,
            clock: Arc::new(system_now_ms),
            policy: RetentionPolicy::default(),
        }
    }

    /// 替换时间源（测试用）。
    #[must_use]
    pub fn with_clock(mut self, clock: SnapshotClock) -> Self {
        self.clock = clock;
        self
    }

    /// 替换保留策略（设置项落地后由命令层传入）。
    #[must_use]
    pub fn with_policy(mut self, policy: RetentionPolicy) -> Self {
        self.policy = policy;
        self
    }

    fn now(&self) -> i64 {
        (self.clock)()
    }

    /// 读取快照记录并核对归属仓库；顺带解析出工作区路径。
    fn snapshot_record(
        &self,
        repo_id: i64,
        snapshot_id: SnapshotId,
    ) -> Result<(PathBuf, forgedesk_storage::SnapshotRecord), SnapshotError> {
        let record = SnapshotStore::new(&self.database)
            .find(snapshot_id)
            .map_err(|error| SnapshotError::Storage(error.message.clone()))?
            .filter(|record| record.repo_id == repo_id)
            .ok_or(SnapshotError::NotFound(snapshot_id))?;

        let repository = RepositoryStore::new(&self.database)
            .find_by_id(repo_id)
            .map_err(|error| SnapshotError::Storage(error.message.clone()))?
            .ok_or(SnapshotError::NotFound(snapshot_id))?;

        Ok((PathBuf::from(repository.path), record))
    }

    /// 把仓库放回快照记录描述的状态（不打保护点、不写审计）。
    ///
    /// restore 的核心三步 + 两道校验。**公开为私有方法的原因**：
    /// "回滚失败后自动恢复到回滚前快照"要复用同一套步骤，
    /// 而那条恢复路径绝不能再嵌套打点（否则失败链会递归下去）。
    fn apply(
        &self,
        record: &forgedesk_storage::SnapshotRecord,
        repo: &RepoId,
    ) -> Result<RestoreReport, SnapshotError> {
        let git_failure = |step: &str, error: &forgedesk_domain::AppError| {
            SnapshotError::RestoreFailed(format!("{step} failed: {}", error.message))
        };

        // 1. 工作区与索引一起回到快照的 HEAD
        self.engines
            .write()
            .reset(
                repo,
                ResetSpec::to(record.head_oid.clone(), ResetMode::Hard),
            )
            .map_err(|error| git_failure("git reset --hard", &error))?;

        // 2. 索引单独回到快照的树（reset 只能带到 HEAD 的树，
        //    "已暂存未提交"的内容记录在快照自己的 index_tree_oid 里）
        self.engines
            .write()
            .read_tree(repo, &record.index_tree_oid)
            .map_err(|error| git_failure("git read-tree", &error))?;

        // 3. 校验：读引擎与写引擎是两条独立实现（T1.2 的差分测试保证一致），
        //    用它核对而不是用写完再读写的同一套命令
        let head = self
            .engines
            .read()
            .head_oid(repo)
            .map_err(|error| git_failure("verifying HEAD", &error))?;
        if head.as_deref() != Some(record.head_oid.as_str()) {
            return Err(SnapshotError::RestoreVerify(format!(
                "HEAD is {head:?} but the snapshot recorded {}",
                record.head_oid
            )));
        }

        let tree = self
            .engines
            .write()
            .index_tree(repo)
            .map_err(|error| git_failure("verifying the index", &error))?;
        if tree != record.index_tree_oid {
            return Err(SnapshotError::RestoreVerify(format!(
                "index tree is {tree} but the snapshot recorded {}",
                record.index_tree_oid
            )));
        }

        Ok(RestoreReport {
            restored_snapshot_id: record.id,
            head_oid: record.head_oid.clone(),
            index_tree_oid: record.index_tree_oid.clone(),
            pre_restore_snapshot_id: None,
            untracked_paths: serde_json::from_str(&record.untracked_paths).unwrap_or_default(),
        })
    }

    /// 创建之后顺手清理（超龄/超量的旧快照）。失败只记日志：
    /// 清理不成功不影响"快照已创建"这个事实。
    fn prune_silently(&self, repo_id: i64) {
        if let Err(error) = self.prune(repo_id, &self.policy) {
            tracing::warn!(repo_id = repo_id, error = %error.message(), "快照清理失败");
        }
    }
}

impl SnapshotManager for RefSnapshotManager {
    fn create(&self, request: &SnapshotRequest<'_>) -> Result<SnapshotId, SnapshotError> {
        let repo = RepoId::new(request.workdir.to_path_buf());
        let status = self
            .engines
            .read()
            .status(&repo, &StatusQuery::default())
            .map_err(|error| {
                SnapshotError::Failed(format!("reading the repository failed: {}", error.message))
            })?;

        // 空仓库没有可锚定的提交：快照会是空洞的，显式失败让调用方走降级提示
        let head_oid = status.branch.oid.clone().ok_or_else(|| {
            SnapshotError::Failed("the repository has no HEAD commit yet".to_owned())
        })?;

        let index_tree_oid = self.engines.write().index_tree(&repo).map_err(|error| {
            SnapshotError::Failed(format!("writing the index tree failed: {}", error.message))
        })?;

        // 未跟踪文件在状态报告里以条目形式存在（分组是前端面板的事）：
        // 快照只需要它们的路径，v1 不备份内容
        let untracked_paths: Vec<String> = status
            .entries
            .iter()
            .filter(|entry| entry.kind == EntryKind::Untracked)
            .map(|entry| entry.path.to_string_lossy().into_owned())
            .collect();
        let untracked_json = serde_json::to_string(&untracked_paths).map_err(|error| {
            SnapshotError::Failed(format!("serializing untracked paths failed: {error}"))
        })?;

        let store = SnapshotStore::new(&self.database);
        let id = store
            .insert(&NewSnapshot {
                repo_id: request.repo_id,
                label: request.label.to_owned(),
                kind: request.kind.key().to_owned(),
                head_oid: head_oid.clone(),
                index_tree_oid: index_tree_oid.clone(),
                // 先空着：锚点名里含主键，插入之后才知道 id
                snapshot_ref: String::new(),
                branch: status.branch.head.clone(),
                detached: status.branch.detached,
                operation_state: None,
                untracked_paths: untracked_json,
                created_at_ms: self.now(),
            })
            .map_err(|error| SnapshotError::Storage(error.message.clone()))?;

        let name = format!("{SNAPSHOT_REF_PREFIX}{id}");
        if let Err(error) = self.engines.write().update_ref(&repo, &name, &head_oid) {
            let _ = store.delete(id);
            return Err(SnapshotError::Failed(format!(
                "anchoring the snapshot ref failed: {}",
                error.message
            )));
        }
        store
            .update_ref_column(id, &name)
            .map_err(|error| SnapshotError::Storage(error.message.clone()))?;

        self.prune_silently(request.repo_id);
        Ok(id)
    }

    fn list(&self, repo_id: i64, limit: i64) -> Result<Vec<SnapshotMeta>, SnapshotError> {
        let records = SnapshotStore::new(&self.database)
            .list(repo_id, limit)
            .map_err(|error| SnapshotError::Storage(error.message.clone()))?;

        Ok(records
            .into_iter()
            .map(|record| SnapshotMeta {
                id: record.id,
                label: record.label,
                kind: record.kind,
                head_oid: record.head_oid,
                branch: record.branch,
                detached: record.detached,
                created_at: record.created_at,
            })
            .collect())
    }

    fn restore(
        &self,
        repo_id: i64,
        snapshot_id: SnapshotId,
    ) -> Result<RestoreReport, SnapshotError> {
        let (workdir, record) = self.snapshot_record(repo_id, snapshot_id)?;
        let repo = RepoId::new(workdir.clone());

        // 锚点必须在动手前确认：它没了就意味着"这个快照恢复不了"，
        // 而用户需要的是一句可操作的话，不是跑到一半的失败
        if !self
            .engines
            .write()
            .ref_exists(&repo, &record.snapshot_ref)
            .map_err(|error| {
                SnapshotError::RestoreFailed(format!(
                    "checking the anchor failed: {}",
                    error.message
                ))
            })?
        {
            return Err(SnapshotError::RefMissing {
                id: snapshot_id,
                name: record.snapshot_ref.clone(),
            });
        }

        // 回滚前快照是"防误回滚"的那道闸：打不出这个点就不回滚。
        // （能恢复的仓库一定有 HEAD 提交，所以这个 create 不会因空仓库失败。）
        let pre_restore = self.create(&SnapshotRequest {
            repo_id,
            workdir: &workdir,
            label: SnapshotKind::PreRestore.key(),
            kind: SnapshotKind::PreRestore,
        })?;

        // 审计不在这里写：命令层统一记录写操作（T1.11）。本层只保证
        // "回滚结果 + 回滚前保护点 id"如实返回，让上层能记全 `snapshot_id`
        // 与 `reversible`——**审计属于用例边界，不属于实现细节**。
        match self.apply(&record, &repo) {
            Ok(mut report) => {
                report.pre_restore_snapshot_id = Some(pre_restore);
                Ok(report)
            }
            Err(error) => {
                // 绝不停在中间态：回到回滚前快照（这条恢复不再嵌套打点）
                if let Ok((_, pre_record)) = self.snapshot_record(repo_id, pre_restore) {
                    let rollback = self.apply(&pre_record, &repo);
                    if let Err(rollback_error) = rollback {
                        tracing::error!(error = %rollback_error.message(), "回滚失败后恢复到回滚前快照也失败了");
                    }
                }
                Err(error)
            }
        }
    }

    fn diff(&self, repo_id: i64, snapshot_id: SnapshotId) -> Result<SnapshotDiff, SnapshotError> {
        let (workdir, record) = self.snapshot_record(repo_id, snapshot_id)?;
        let repo = RepoId::new(workdir);

        let ref_missing = !self
            .engines
            .write()
            .ref_exists(&repo, &record.snapshot_ref)
            .map_err(|error| SnapshotError::Storage(error.message.clone()))?;

        let current_head_oid = self
            .engines
            .read()
            .head_oid(&repo)
            .map_err(|error| SnapshotError::Storage(error.message.clone()))?;
        // 索引里有未合并条目时 write-tree 会失败：那本身就是"状态已经不同"的证据，
        // 记为已变化而不是把错误抛给用户
        let current_index_tree_oid = self.engines.write().index_tree(&repo).ok();

        Ok(SnapshotDiff {
            head_changed: current_head_oid.as_deref() != Some(record.head_oid.as_str()),
            index_changed: current_index_tree_oid.as_deref()
                != Some(record.index_tree_oid.as_str()),
            current_head_oid,
            current_index_tree_oid,
            ref_missing,
        })
    }

    fn prune(
        &self,
        repo_id: i64,
        policy: &RetentionPolicy,
    ) -> Result<Vec<SnapshotId>, SnapshotError> {
        let workdir = self
            .snapshot_record_workdir(repo_id)
            .ok_or(SnapshotError::NotFound(repo_id))?;
        let repo = RepoId::new(workdir);
        let store = SnapshotStore::new(&self.database);

        let candidates = store
            .prune_candidates(
                repo_id,
                i64::from(policy.max_count),
                policy.cutoff_ms(self.now()),
            )
            .map_err(|error| SnapshotError::Storage(error.message.clone()))?;

        let mut pruned = Vec::new();
        for record in candidates {
            // ref 与记录一起删：只删一头都会留下"看着还在其实没了"的假快照
            self.engines
                .write()
                .delete_ref(&repo, &record.snapshot_ref)
                .map_err(|error| SnapshotError::Storage(error.message.clone()))?;
            store
                .delete(record.id)
                .map_err(|error| SnapshotError::Storage(error.message.clone()))?;
            pruned.push(record.id);
        }
        Ok(pruned)
    }
}

impl RefSnapshotManager {
    /// 只解析工作区路径（prune 用：它不需要快照记录本身）。
    fn snapshot_record_workdir(&self, repo_id: i64) -> Option<PathBuf> {
        RepositoryStore::new(&self.database)
            .find_by_id(repo_id)
            .ok()?
            .map(|repository| PathBuf::from(repository.path))
    }
}

/// 系统时间的 Unix 毫秒（测试之外的时间源）。
fn system_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or_default()
}
