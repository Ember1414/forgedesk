//! 基于自定义 ref 锚点的快照管理器（T1.9 的真实现；T3.8 补上未跟踪内容备份）。
//!
//! # 快照的生命周期
//!
//! 1. **create**：读仓库事实（HEAD、`write-tree` 的索引树、未跟踪/被忽略清单）→
//!    把未跟踪内容复制到**临时备份目录** → 落库（含清单与体积）→
//!    用 `refs/forgedesk/snapshots/<id>` 锚住 HEAD 提交（防 gc）→
//!    把临时目录改名成正式目录（原子）→ 保留策略与总占用上限各清一遍。
//!    落库在前、锚点在后，锚点失败就删掉记录——表里不留"没有锚点"的快照。
//! 2. **restore**：校验锚点仍在 → 给当前状态打"回滚前快照" → `reset --hard`
//!    → `read-tree` 恢复索引 → 从备份目录写回未跟踪内容 → 用 git 的事实核对
//!    HEAD 与索引树、逐字节核对备份内容 → HEAD/索引对不上就**自动回到回滚前快照**。
//! 3. **prune / cleanup**：数量、年龄、总占用三个条件任一触发即清理；
//!    清理时 ref、记录与**备份目录**一起删（只删一头都会留下"看着还在其实没了"
//!    的假快照，或者一个永远不被 gc 的孤儿目录）。
//!
//! # 为什么读引擎也要用
//!
//! 校验（"恢复后的 HEAD 是不是快照记录的那个"）必须走一条**与写操作不同的路径**：
//! 用写完再自己读写的同一套命令来验证，等于让考生批自己的卷子。
//! 读引擎（libgit2）与 CLI 是两条独立实现（T1.2 的差分测试保证它们对同一状态
//! 给出同一结论），这个校验才有意义。
//!
//! # 并发（T3.8）
//!
//! 每个仓库一把互斥锁，`create` / `restore` / `cleanup` 串行执行。
//! 锁是**不可重入**的，因此内部路径一律走 `*_locked` 变体：`restore` 在回滚前要
//! 打保护点（`create`），`prune` 在创建后被顺手调用——它们都不能再取一次锁。
//! 中毒的锁按"上一次持锁者 panic 了"处理并继续使用：快照的一致性由 git 与
//! 数据库保证，锁只是避免并发写入，为一次 panic 让整个仓库的快照永久不可用更糟。

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use forgedesk_domain::git::{
    hash_paths, EntryKind, RepoFingerprint, RepoId, ResetMode, ResetSpec, StatusQuery,
    TRACKED_HASH_LIMIT,
};
use forgedesk_git_engine::engine::GitEngine;
use forgedesk_git_engine::engines::GitEngines;
use forgedesk_storage::{Database, NewSnapshot, RepositoryStore, SnapshotRecord, SnapshotStore};

use crate::backup::{self, BackupPlan};
use crate::{
    BackupManifest, CleanupOutcome, EmergencyGuidance, PendingRestore, RestoreOutcomeKind,
    RestoreReport, RestoreStage, RetentionPolicy, SnapshotDiff, SnapshotError, SnapshotEstimate,
    SnapshotId, SnapshotKind, SnapshotLimits, SnapshotManager, SnapshotMeta, SnapshotOutcome,
    SnapshotRequest, SnapshotUsage, SnapshotWarning, StageResult,
};

/// 快照锚点 ref 的前缀（完整形如 `refs/forgedesk/snapshots/<id>`）。
pub const SNAPSHOT_REF_PREFIX: &str = "refs/forgedesk/snapshots/";

/// 默认保留条数（任务定义：每仓库最近 50 条）。
pub const DEFAULT_MAX_COUNT: u32 = 50;

/// 默认保留天数（任务定义：30 天，与条数先到者生效）。
pub const DEFAULT_MAX_AGE_DAYS: u32 = 30;

/// 可注入的时间源（测试要能推进时间来验证保留策略）。
type SnapshotClock = Arc<dyn Fn() -> i64 + Send + Sync>;

/// 已经复制到临时目录、等待入库后改名的备份。
#[derive(Debug)]
struct StagingBackup {
    /// 临时目录（`<root>/<repo_id>/.tmp-<token>`）。
    temporary: PathBuf,
    /// 已写入的清单。
    manifest: BackupManifest,
}

/// 基于自定义 ref 锚点的 [`SnapshotManager`] 实现。
///
/// 持有引擎组合与数据库：快照是"git 事实 + 一行记录 + 一份内容备份"的组合体，
/// 只懂 git 不懂库（或者反过来）都无法保证三者一致。
pub struct RefSnapshotManager {
    engines: Arc<GitEngines>,
    database: Arc<Database>,
    clock: SnapshotClock,
    policy: RetentionPolicy,
    limits: SnapshotLimits,
    /// 未跟踪内容备份的根目录（宿主注入 `app_cache_dir/snapshots`）。
    /// `None` = 不备份内容（快照仍然有效，只是没有内容可回滚）。
    backup_root: Option<PathBuf>,
    /// 每个仓库一把锁：同一仓库的写路径串行化。
    locks: Mutex<HashMap<i64, Arc<Mutex<()>>>>,
    /// 故障注入（**仅测试**）：`(阶段, 剩余次数)`。
    ///
    /// 为什么生产代码里留这个钩子：T3.9 的验收要求"回退也失败 → 紧急模式 →
    /// 指引命令可执行"，而那条路径只有在**回退也失败**时才会走到——用真实数据
    /// 造出这种局面几乎不可能（保护点是刚打出来的，记录与 oid 都一定合法；
    /// 而"索引被占用"这类外部原因会让保护点本身也打不出来，于是根本走不到回退）。
    /// 钩子是纯内存状态（默认 `None`，零行为影响）。
    injected_failure: Mutex<Option<(RestoreStage, usize)>>,
}

impl std::fmt::Debug for RefSnapshotManager {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RefSnapshotManager")
            .field("policy", &self.policy)
            .field("limits", &self.limits)
            .field("backup_root", &self.backup_root)
            .finish_non_exhaustive()
    }
}

impl RefSnapshotManager {
    /// 创建快照管理器（默认保留策略 50 条 / 30 天，默认磁盘上限 200 MiB / 2 GiB）。
    pub fn new(engines: Arc<GitEngines>, database: Arc<Database>) -> Self {
        Self {
            engines,
            database,
            clock: Arc::new(system_now_ms),
            policy: RetentionPolicy::default(),
            limits: SnapshotLimits::default(),
            backup_root: None,
            locks: Mutex::new(HashMap::new()),
            injected_failure: Mutex::new(None),
        }
    }

    /// 注入回滚故障（**仅测试**）：接下来 `times` 次进入 `stage` 阶段时失败。
    ///
    /// 为什么要带次数：T3.9 的两个结局需要不同的注入强度——
    /// - `times = 1`：目标失败、**回退成功** → `RolledBack`（仓库回到动手之前）；
    /// - `times = 2`：目标失败、**回退也失败** → `Emergency`（给出恢复指引）。
    ///
    /// 传 `times = 0` 取消注入。次数用尽后自动失效，不会影响后续调用。
    #[doc(hidden)]
    pub fn inject_restore_failure(&self, stage: RestoreStage, times: usize) {
        if let Ok(mut slot) = self.injected_failure.lock() {
            *slot = if times == 0 {
                None
            } else {
                Some((stage, times))
            };
        }
    }

    /// 该阶段是否应该"失败"（注入命中则消耗一次）。
    fn should_inject_failure(&self, stage: RestoreStage) -> bool {
        let Ok(mut slot) = self.injected_failure.lock() else {
            return false;
        };
        match slot.as_mut() {
            Some((injected, remaining)) if *injected == stage && *remaining > 0 => {
                *remaining -= 1;
                if *remaining == 0 {
                    *slot = None;
                }
                true
            }
            _ => false,
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

    /// 替换磁盘策略（单份/单仓库上限、是否包含被忽略文件）。
    #[must_use]
    pub fn with_limits(mut self, limits: SnapshotLimits) -> Self {
        self.limits = limits;
        self
    }

    /// 指定未跟踪内容备份的根目录（宿主的缓存目录）。
    ///
    /// 不配置时快照照常创建，只是不含内容备份，并在结果里如实告警——
    /// "没有内容备份"和"有备份"必须在界面上可区分。
    #[must_use]
    pub fn with_backup_root(mut self, root: PathBuf) -> Self {
        self.backup_root = Some(root);
        self
    }

    fn now(&self) -> i64 {
        (self.clock)()
    }

    /// 取某个仓库的锁并持有到本次调用结束。
    ///
    /// 返回的 `Arc` 必须和 guard 一起存活（guard 借用它），因此调用方的写法是
    /// `let lock = self.repo_lock(id); let _guard = ...lock.lock()...;`。
    fn repo_lock(&self, repo_id: i64) -> Arc<Mutex<()>> {
        let mut locks = match self.locks.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        Arc::clone(locks.entry(repo_id).or_default())
    }

    /// 备份根下某个快照的目录（未配置备份根时为 `None`）。
    fn backup_dir(&self, repo_id: i64, snapshot_id: SnapshotId) -> Option<PathBuf> {
        self.backup_root
            .as_ref()
            .map(|root| root.join(repo_id.to_string()).join(snapshot_id.to_string()))
    }

    /// 读取快照记录并核对归属仓库；顺带解析出工作区路径。
    fn snapshot_record(
        &self,
        repo_id: i64,
        snapshot_id: SnapshotId,
    ) -> Result<(PathBuf, SnapshotRecord), SnapshotError> {
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

    /// 只解析工作区路径（不需要快照记录的那些路径）。
    fn workdir(&self, repo_id: i64) -> Result<PathBuf, SnapshotError> {
        RepositoryStore::new(&self.database)
            .find_by_id(repo_id)
            .map_err(|error| SnapshotError::Storage(error.message.clone()))?
            .map(|repository| PathBuf::from(repository.path))
            .ok_or(SnapshotError::NotFound(repo_id))
    }

    /// 当前工作区的未跟踪路径（归一化：正斜杠、无尾斜杠）。
    fn current_untracked(&self, repo: &RepoId) -> Result<Vec<String>, SnapshotError> {
        let status = self
            .engines
            .read()
            .status(repo, &StatusQuery::default())
            .map_err(|error| SnapshotError::Storage(error.message.clone()))?;
        Ok(status
            .entries
            .iter()
            .filter(|entry| entry.kind == EntryKind::Untracked)
            .map(|entry| backup::normalize(&entry.path.to_string_lossy()))
            .collect())
    }

    /// 把计划里的文件复制进临时目录；返回（暂存备份，跳过的路径）。
    ///
    /// 失败与超限都在 `warnings` 里如实报告，**不**让快照创建失败：
    /// "没把某个文件备上"远好于"因为某个文件让这次危险操作裸奔"。
    fn prepare_backup(
        &self,
        repo_id: i64,
        workdir: &Path,
        untracked: &[String],
        ignored: &[String],
        warnings: &mut Vec<SnapshotWarning>,
    ) -> (Option<StagingBackup>, Vec<String>) {
        let Some(root) = self.backup_root.as_ref() else {
            warnings.push(SnapshotWarning::BackupDirUnavailable {
                detail: "the snapshot backup root is not configured".to_owned(),
            });
            return (None, untracked.to_vec());
        };

        let plan: BackupPlan = backup::plan(workdir, untracked, ignored, &self.limits);
        warnings.extend(plan.warnings.iter().cloned());
        if plan.files.is_empty() {
            return (None, plan.skipped);
        }

        let temporary =
            root.join(repo_id.to_string())
                .join(format!("{}{}", backup::TEMP_PREFIX, temp_token()));
        if let Err(error) = fs::create_dir_all(&temporary) {
            warnings.push(SnapshotWarning::BackupDirUnavailable {
                detail: format!("creating the staging directory failed: {error}"),
            });
            return (None, untracked.to_vec());
        }

        let (manifest, failed) = backup::copy_into(&temporary, &plan.files);
        if !failed.is_empty() {
            warnings.push(SnapshotWarning::UntrackedBackupPartial {
                detail: "copying some untracked files failed".to_owned(),
                paths: failed.clone(),
            });
        }
        if manifest.is_empty() {
            let _ = backup::remove_dir(&temporary);
            return (None, failed);
        }
        if let Err(error) = backup::write_manifest(&temporary, &manifest) {
            warnings.push(SnapshotWarning::BackupDirUnavailable {
                detail: format!("writing the backup manifest failed: {error}"),
            });
            let _ = backup::remove_dir(&temporary);
            return (None, untracked.to_vec());
        }

        (
            Some(StagingBackup {
                temporary,
                manifest,
            }),
            failed,
        )
    }

    /// 磁盘上有、数据库里没有对应记录的备份目录（含复制中途崩溃的 `.tmp-*`）。
    ///
    /// 数据库读失败时返回**空**：宁可这次不清理，也不能因为一次查询失败
    /// 把所有备份目录都当成孤儿删掉。
    fn collect_orphans(&self, repo_id: i64) -> Vec<PathBuf> {
        let Some(root) = self.backup_root.as_ref() else {
            return Vec::new();
        };
        let repo_root = root.join(repo_id.to_string());
        if !repo_root.is_dir() {
            return Vec::new();
        }
        let store = SnapshotStore::new(&self.database);
        let Ok(records) = store.list_oldest_first(repo_id) else {
            return Vec::new();
        };
        let known: HashSet<String> = records.iter().map(|record| record.id.to_string()).collect();

        backup::subdirs(&repo_root)
            .into_iter()
            .filter(|name| name.starts_with(backup::TEMP_PREFIX) || !known.contains(name))
            .map(|name| repo_root.join(name))
            .collect()
    }

    /// 清理孤儿目录，返回清理数量（失败只记日志——它是背景动作）。
    fn cleanup_orphans_locked(&self, repo_id: i64) -> usize {
        let mut removed = 0;
        for directory in self.collect_orphans(repo_id) {
            match backup::remove_dir(&directory) {
                Ok(()) => removed += 1,
                Err(error) => {
                    tracing::warn!(path = ?directory, error = %error, "清理孤儿快照目录失败");
                }
            }
        }
        removed
    }

    /// 删除一份快照：锚点 ref、备份目录、数据库记录一起走。
    fn delete_snapshot_locked(
        &self,
        repo: &RepoId,
        repo_id: i64,
        record: &SnapshotRecord,
    ) -> Result<(), SnapshotError> {
        self.engines
            .write()
            .delete_ref(repo, &record.snapshot_ref)
            .map_err(|error| SnapshotError::Storage(error.message.clone()))?;
        if let Some(directory) = self.backup_dir(repo_id, record.id) {
            if let Err(error) = backup::remove_dir(&directory) {
                tracing::warn!(path = ?directory, error = %error, "删除快照备份目录失败");
            }
        }
        SnapshotStore::new(&self.database)
            .delete(record.id)
            .map_err(|error| SnapshotError::Storage(error.message.clone()))
    }

    /// 按保留策略清理（不加锁；调用方已持有该仓库的锁）。
    fn prune_locked(
        &self,
        repo_id: i64,
        policy: &RetentionPolicy,
    ) -> Result<Vec<SnapshotId>, SnapshotError> {
        let workdir = self.workdir(repo_id)?;
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
            self.delete_snapshot_locked(&repo, repo_id, &record)?;
            pruned.push(record.id);
        }
        Ok(pruned)
    }

    /// 超过仓库总占用上限时，从**最旧**的快照开始删（LRU）。
    ///
    /// 返回（被删的 id，释放的备份字节）。
    fn enforce_repo_quota_locked(&self, repo_id: i64) -> (Vec<SnapshotId>, u64) {
        let max_bytes = self.limits.max_repo_bytes;
        if max_bytes == 0 {
            return (Vec::new(), 0);
        }
        let store = SnapshotStore::new(&self.database);
        let Ok(mut total) = store.total_backup_bytes(repo_id) else {
            return (Vec::new(), 0);
        };
        if u64::try_from(total).unwrap_or(u64::MAX) <= max_bytes {
            return (Vec::new(), 0);
        }
        let Ok(workdir) = self.workdir(repo_id) else {
            return (Vec::new(), 0);
        };
        let repo = RepoId::new(workdir);
        let Ok(records) = store.list_oldest_first(repo_id) else {
            return (Vec::new(), 0);
        };

        let mut removed = Vec::new();
        let mut freed = 0_u64;
        for record in records {
            if u64::try_from(total).unwrap_or(u64::MAX) <= max_bytes {
                break;
            }
            if self
                .delete_snapshot_locked(&repo, repo_id, &record)
                .is_err()
            {
                // 删不动（ref 已被外部删掉之类）就停手：继续删只会连累更多快照
                break;
            }
            total -= record.backup_bytes.min(total);
            freed = freed.saturating_add(u64::try_from(record.backup_bytes).unwrap_or(0));
            removed.push(record.id);
        }
        (removed, freed)
    }

    /// 创建之后顺手清理（保留策略 + 总占用）。失败只记日志：
    /// 清理不成功不影响"快照已创建"这个事实。
    fn prune_silently_locked(&self, repo_id: i64) -> Vec<SnapshotId> {
        let mut pruned = match self.prune_locked(repo_id, &self.policy) {
            Ok(ids) => ids,
            Err(error) => {
                tracing::warn!(repo_id = repo_id, error = %error.message(), "快照清理失败");
                Vec::new()
            }
        };
        let (reclaimed, _) = self.enforce_repo_quota_locked(repo_id);
        pruned.extend(reclaimed);
        pruned
    }

    /// 回滚的真正实现（调用方已持有该仓库的锁）。
    ///
    /// # 顺序即语义（T3.9 的失败安全）
    ///
    /// 1. **锚点校验**：锚点没了直接返回 `RefMissing`——一个字节都不动；
    /// 2. **打保护点**：打不出来就中止。宁可不动，不可无保护地动；
    /// 3. **落库"正在回滚 + 当前阶段"**：应用被强杀后，下一次启动能说出停在哪里；
    /// 4. **分阶段执行**（见 [`Self::apply_staged`]）；
    /// 5. **失败 → 退回保护点**：成功 = `RolledBack`；回退也失败 = `Emergency`
    ///    （带上可执行的恢复指引——绝不让用户无从下手）；
    /// 6. **清标记**：只有"没走到这一步"才会留下标记，那正是崩溃的信号。
    ///
    /// 失败时返回 `Ok(report)` 而不是 `Err`：报告里带着阶段结果与恢复指引，
    /// 那才是用户此刻最需要的东西。只有"连第一步都没开始"（快照不存在、
    /// 锚点丢失、保护点打不出来）才返回 `Err`——那些情况下没有任何阶段性事实。
    fn restore_locked(
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

        let store = SnapshotStore::new(&self.database);
        let started = self.now();
        let mut stages: Vec<StageResult> = Vec::new();

        // ---------------------------------------------------------- 阶段 0：保护点
        // 走 create_locked：锁已在本方法手里，重入会死锁。
        // （能恢复的仓库一定有 HEAD 提交，所以这个 create 不会因空仓库失败。）
        let anchor = self.now();
        let pre_restore = match self.create_locked(&SnapshotRequest {
            repo_id,
            workdir: &workdir,
            label: SnapshotKind::PreRestore.key(),
            kind: SnapshotKind::PreRestore,
        }) {
            Ok(outcome) => {
                stages.push(stage_result(
                    RestoreStage::Protection,
                    None,
                    anchor,
                    self.now(),
                ));
                outcome
            }
            Err(error) => {
                stages.push(stage_result(
                    RestoreStage::Protection,
                    Some(&error),
                    anchor,
                    self.now(),
                ));
                // 打不出保护点 = 没有退路：不回滚，如实告诉用户
                return Err(error);
            }
        };

        // 从这里开始，任何"没走完"都会在库里留下痕迹（崩溃恢复的线索）。
        // 落库失败**不**打断回滚：这是一条诊断线索，不是回滚的前提。
        let mark = |stage: RestoreStage| {
            let _ = store.set_restore_progress(record.id, true, Some(stage.key()), Some(started));
        };
        mark(RestoreStage::Head);

        let applied = self.apply_staged(&record, &repo, &workdir, &mark);
        stages.extend(applied.stages.iter().cloned());

        // -------------------------------------- 失败处理：先退回保护点，再决定报告
        let mut rollback_target: Option<SnapshotRecord> = None;
        let mut emergency: Option<EmergencyGuidance> = None;
        let outcome = if applied.failure.is_none() {
            RestoreOutcomeKind::Completed
        } else {
            let failure = applied
                .failure
                .as_ref()
                .map(SnapshotError::message)
                .unwrap_or_else(|| "unknown failure".to_owned());
            tracing::warn!(
                repo_id = repo_id,
                snapshot_id = snapshot_id,
                error = %failure,
                "回滚失败，尝试退回保护点"
            );

            let rolled = self.snapshot_record(repo_id, pre_restore.id).ok().map(
                |(pre_workdir, pre_record)| {
                    // 回退路径不再打点、不再落进度标记：它只是把仓库放回去
                    let result = self.apply_staged(&pre_record, &repo, &pre_workdir, &|_stage| {});
                    (pre_record, result)
                },
            );

            match rolled {
                Some((pre_record, result))
                    if result.failure.is_none() && self.rollback_matches(&pre_record, &repo) =>
                {
                    // 仓库回到了动手之前：用户什么都不用做
                    rollback_target = Some(pre_record);
                    RestoreOutcomeKind::RolledBack
                }
                other => {
                    // 回退也失败：绝不让用户无从下手——给出可执行的恢复指引
                    let target = match other {
                        Some((pre_record, _)) => pre_record,
                        None => record.clone(),
                    };
                    tracing::error!(
                        repo_id = repo_id,
                        snapshot_id = snapshot_id,
                        "回滚失败且退回保护点也失败：需要人工恢复"
                    );
                    emergency = Some(self.emergency_guidance(&record, &target, &workdir));
                    rollback_target = Some(target);
                    RestoreOutcomeKind::Emergency
                }
            }
        };

        // 报告展示的是"仓库现在实际所处的那个快照"的事实
        let shown = rollback_target.as_ref().unwrap_or(&record);
        // 标记必须在返回前清掉：留着会让下一次启动误报"上次回滚没走完"。
        // 真正被强杀的场景根本走不到这里，标记自然留了下来——这正是设计意图。
        let _ = store.set_restore_progress(record.id, false, None, None);

        let report = RestoreReport {
            restored_snapshot_id: record.id,
            head_oid: shown.head_oid.clone(),
            index_tree_oid: shown.index_tree_oid.clone(),
            pre_restore_snapshot_id: Some(pre_restore.id),
            untracked_paths: recorded_untracked(&record).into_iter().collect(),
            untracked_restored: applied.untracked_restored,
            untracked_failed: applied.untracked_failed.clone(),
            untracked_extra: applied.untracked_extra.clone(),
            verified: applied.content_verified && applied.failure.is_none(),
            outcome,
            stages,
            report_lines: Self::report_lines(&record, &applied, outcome, Some(pre_restore.id)),
            emergency,
        };

        if !outcome.is_success() {
            tracing::warn!(
                repo_id = repo_id,
                snapshot_id = snapshot_id,
                outcome = outcome.key(),
                "回滚未成功"
            );
        }
        Ok(report)
    }

    /// 把仓库放回快照记录描述的状态（不打保护点、不写审计、不加锁）。
    ///
    /// # 分阶段执行（T3.9）
    ///
    /// 回滚拆成四个阶段，每个阶段的结果都进报告：
    ///
    /// `head`（`reset --hard`）→ `index`（`read-tree`）→ `untracked`（写回内容）
    /// → `verify`（核对 git 事实）
    ///
    /// 任一步失败就**停止后续步骤**并立刻返回：继续往下做只会让中间态更难收拾。
    /// `progress` 在每个阶段**开始前**调用一次——调用方用它把"现在卡在哪一步"
    /// 落库，于是应用被强杀之后，下一次启动还能说出上次停在哪里。
    ///
    /// **为什么是私有方法**：回滚失败后的"自动退回保护点"要复用同一套步骤，
    /// 而那条路径绝不能再嵌套打点（否则失败链会递归下去）。
    fn apply_staged(
        &self,
        record: &SnapshotRecord,
        repo: &RepoId,
        workdir: &Path,
        progress: &dyn Fn(RestoreStage),
    ) -> AppliedStages {
        let mut applied = AppliedStages::default();

        // ---------------------------------------------------------- 阶段 1/4：HEAD
        progress(RestoreStage::Head);
        let started = self.now();
        let head = if self.should_inject_failure(RestoreStage::Head) {
            Err(injected_failure(RestoreStage::Head))
        } else {
            self.engines
                .write()
                .reset(
                    repo,
                    ResetSpec::to(record.head_oid.clone(), ResetMode::Hard),
                )
                .map_err(|error| git_failure("git reset --hard", &error))
        };
        applied.stages.push(stage_result(
            RestoreStage::Head,
            head.as_ref().err(),
            started,
            self.now(),
        ));
        if let Err(error) = head {
            applied.failure = Some(error);
            return applied;
        }

        // --------------------------------------------------------- 阶段 2/4：索引
        // `reset --hard` 只能把索引带到 HEAD 的树；"已暂存未提交"的内容记录在
        // 快照自己的 `index_tree_oid` 里，必须单独恢复
        progress(RestoreStage::Index);
        let started = self.now();
        let index = if self.should_inject_failure(RestoreStage::Index) {
            Err(injected_failure(RestoreStage::Index))
        } else {
            self.engines
                .write()
                .read_tree(repo, &record.index_tree_oid)
                .map_err(|error| git_failure("git read-tree", &error))
        };
        applied.stages.push(stage_result(
            RestoreStage::Index,
            index.as_ref().err(),
            started,
            self.now(),
        ));
        if let Err(error) = index {
            applied.failure = Some(error);
            return applied;
        }

        // ------------------------------------------- 阶段 3/4：未跟踪内容（T3.8）
        // 内容恢复失败**不**触发回退：HEAD 与索引才是回滚的主体，
        // 个别文件被占用不该把整次回滚推倒重来——如实报告即可。
        progress(RestoreStage::Untracked);
        let started = self.now();
        let manifest = BackupManifest::from_json(&record.manifest_json);
        if !manifest.is_empty() {
            match self.backup_dir(record.repo_id, record.id) {
                Some(backup_dir) if backup_dir.is_dir() => {
                    let summary = backup::restore(&backup_dir, workdir, &manifest);
                    applied.untracked_restored = summary.restored;
                    applied.untracked_failed = summary.failed;
                    applied.content_verified =
                        backup::verify(&backup_dir, workdir, &manifest).is_empty();
                }
                _ => {
                    // 记录说有备份、目录却不在（缓存被清过 / 换机器 / 删库重开）：
                    // 逐条列出，用户至少知道"这些文件回不来"
                    applied.untracked_failed = manifest
                        .entries
                        .iter()
                        .map(|entry| entry.path.clone())
                        .collect();
                    applied.content_verified = false;
                }
            }
        }
        applied.stages.push(stage_result(
            RestoreStage::Untracked,
            None,
            started,
            self.now(),
        ));

        // 当前存在、快照里没有的未跟踪文件：**不删**，只列出交给用户决定
        let recorded: BTreeSet<String> = recorded_untracked(record);
        applied.untracked_extra = match self.current_untracked(repo) {
            Ok(paths) => paths
                .into_iter()
                .filter(|path| !recorded.contains(path))
                .collect(),
            Err(error) => {
                applied.failure = Some(error);
                return applied;
            }
        };

        // --------------------------------------------------------- 阶段 4/4：校验
        progress(RestoreStage::Verify);
        let started = self.now();
        let verified = if self.should_inject_failure(RestoreStage::Verify) {
            Err(injected_failure(RestoreStage::Verify))
        } else {
            self.verify_facts(record, repo)
        };
        let verify_failure = verified.err();
        if verify_failure.is_some() {
            applied.content_verified = false;
        }
        applied.stages.push(stage_result(
            RestoreStage::Verify,
            verify_failure.as_ref(),
            started,
            self.now(),
        ));
        applied.failure = verify_failure;

        applied
    }

    /// 核对"恢复后的 HEAD 与索引是不是快照记录的那两个"。
    ///
    /// 走**读引擎**（libgit2）：与写路径是两条独立实现（T1.2 的差分测试保证
    /// 它们对同一状态给出同一结论），用它校验才有意义——"写完自己再读一遍"
    /// 等于让考生批自己的卷子。
    fn verify_facts(&self, record: &SnapshotRecord, repo: &RepoId) -> Result<(), SnapshotError> {
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
        Ok(())
    }

    /// 捕获仓库指纹（T3.9）：回滚前后的同一性判据。
    ///
    /// 公开的只读入口；宿主/命令层与集成测试都用它回答
    /// "仓库现在到底处在哪个状态"。不加锁：它只读。
    pub fn fingerprint(&self, repo_id: i64) -> Result<RepoFingerprint, SnapshotError> {
        let workdir = self.workdir(repo_id)?;
        self.capture_fingerprint(&RepoId::new(workdir))
    }

    /// 回退之后核对"真的回到了保护点"。
    ///
    /// 为什么光看 git 命令的返回值不够：`apply_staged` 的校验只覆盖 HEAD 与索引，
    /// 而"未跟踪文件到底写回去没有"是另一件事（文件被占用、备份目录缺失都会
    /// 让它悄悄失败）。指纹里恰好有未跟踪集合的哈希——用它把这条也钉住，
    /// 核不上就把结局降级为紧急模式（宁可多给一次指引，也不谎报"已经好了"）。
    fn rollback_matches(&self, pre_record: &SnapshotRecord, repo: &RepoId) -> bool {
        let expected: Vec<String> = recorded_untracked(pre_record).into_iter().collect();
        let expected_hash = hash_paths(expected.iter().map(String::as_str));

        self.capture_fingerprint(repo).is_ok_and(|fingerprint| {
            fingerprint.head_oid.as_deref() == Some(pre_record.head_oid.as_str())
                && fingerprint.untracked_paths_hash == expected_hash
        })
    }

    /// 捕获仓库指纹（T3.9）：回滚前后的同一性判据。
    ///
    /// 关于 `tracked_files_hash`：这里记的是**工作区相对索引的变更集**，
    /// 而不是"全部已跟踪文件的内容哈希"。理由有三，且都比任务书字面的方案更好：
    ///
    /// 1. 判据更贴用途——回滚关心的是"工作区干净了没有"，正是变更集的含义；
    /// 2. 成本与仓库大小无关——内容哈希要读遍整个工作区（GB 级仓库不可接受）；
    /// 3. 大仓库也能算，不必退化成 `None`（"不知道"会让校验失去意义）。
    ///
    /// 变更数超过 [`TRACKED_HASH_LIMIT`] 时仍然返回 `None`：那是"仓库正在
    /// 大规模变动"的信号，此时指纹本就不可靠，如实表示"不知道"更诚实。
    fn capture_fingerprint(&self, repo: &RepoId) -> Result<RepoFingerprint, SnapshotError> {
        let status = self
            .engines
            .read()
            .status(repo, &StatusQuery::default())
            .map_err(|error| {
                SnapshotError::Failed(format!("reading the repository failed: {}", error.message))
            })?;

        let mut untracked: Vec<String> = Vec::new();
        let mut changes: Vec<String> = Vec::new();
        for entry in &status.entries {
            let path = backup::normalize(&entry.path.to_string_lossy());
            match entry.kind {
                EntryKind::Untracked => untracked.push(path),
                EntryKind::Ignored => {}
                _ => {
                    // 变更集里带上状态字符：同一个路径"被修改"与"被删除"是两种不同的事实
                    changes.push(format!("{}:{}", entry.index_status.as_char(), path));
                }
            }
        }
        let tracked_files_hash = if changes.len() > TRACKED_HASH_LIMIT {
            None
        } else {
            Some(hash_paths(changes.iter().map(String::as_str)))
        };

        Ok(RepoFingerprint {
            head_oid: status.branch.oid.clone(),
            head_ref: status.branch.head.clone(),
            // 索引里有未合并条目时 write-tree 会失败：那本身就是一种状态，
            // 用 `None` 表示"索引不可表达"而不是让指纹捕获整个失败
            index_tree_oid: self.engines.write().index_tree(repo).ok(),
            tracked_files_hash,
            untracked_paths_hash: hash_paths(untracked.iter().map(String::as_str)),
            operation_state: match status.operation {
                forgedesk_domain::git::OperationState::None => None,
                other => Some(other.as_str().to_owned()),
            },
        })
    }

    /// 紧急模式的恢复指引：**可复制、可执行**。
    ///
    /// 目标选"回到保护点"而不是"继续回到原快照"：保护点是**动手之前**的状态，
    /// 是安全的落点；原快照的锚点仍在，用户恢复到干净状态之后可以再回滚一次
    /// （那时就是一次全新的、有保护点的回滚）。
    fn emergency_guidance(
        &self,
        requested: &SnapshotRecord,
        target: &SnapshotRecord,
        workdir: &Path,
    ) -> EmergencyGuidance {
        let backup_dir = self.backup_dir(target.repo_id, target.id);
        EmergencyGuidance {
            snapshot_id: requested.id,
            target_snapshot_id: target.id,
            backup_dir: backup_dir.map(|path| path.display().to_string()),
            commands: vec![
                format!("git reset --hard {}", target.head_oid),
                format!("git read-tree {}", target.index_tree_oid),
            ],
            notes: vec![
                format!("在仓库目录里执行：{}", workdir.display()),
                "第一条把 HEAD 与工作区带回去，第二条把索引也带回去（两条只影响这个仓库）。"
                    .to_owned(),
                format!(
                    "这里回到的是动手之前的保护点 #{}；原快照 #{} 的锚点仍在，\
                     恢复到干净状态后可以再回滚一次。",
                    target.id, requested.id
                ),
            ],
        }
    }

    /// 组装人话报告行（UI 直接展示，也是写进日志的内容）。
    fn report_lines(
        record: &SnapshotRecord,
        applied: &AppliedStages,
        outcome: RestoreOutcomeKind,
        pre_restore: Option<SnapshotId>,
    ) -> Vec<String> {
        let mut lines = Vec::new();
        for stage in &applied.stages {
            lines.push(if stage.ok {
                format!("{}: ok ({} ms)", stage.stage.key(), stage.duration_ms)
            } else {
                format!(
                    "{}: failed - {}",
                    stage.stage.key(),
                    stage.detail.as_deref().unwrap_or("unknown error")
                )
            });
        }
        match outcome {
            RestoreOutcomeKind::Completed => {
                lines.push(format!(
                    "restored snapshot {} (HEAD {}, {} untracked file(s) written back)",
                    record.id, record.head_oid, applied.untracked_restored
                ));
            }
            RestoreOutcomeKind::RolledBack => {
                lines.push(format!(
                    "restore of snapshot {} failed; rolled back to the protection point {}",
                    record.id,
                    pre_restore.map_or_else(|| "-".to_owned(), |id| id.to_string())
                ));
            }
            RestoreOutcomeKind::Emergency => {
                lines.push(format!(
                    "restore of snapshot {} failed and the rollback failed too: manual recovery required",
                    record.id
                ));
            }
        }
        lines
    }

    /// 创建快照的真正实现（调用方已持有该仓库的锁）。
    fn create_locked(
        &self,
        request: &SnapshotRequest<'_>,
    ) -> Result<SnapshotOutcome, SnapshotError> {
        let repo = RepoId::new(request.workdir.to_path_buf());
        // 要判断"被忽略文件是否也要备份"，就得让 git 把它们报出来
        // （默认查询不返回它们：可能数以万计）
        let status = self
            .engines
            .read()
            .status(
                &repo,
                &StatusQuery {
                    include_ignored: true,
                },
            )
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

        let untracked_paths: Vec<String> = status
            .entries
            .iter()
            .filter(|entry| entry.kind == EntryKind::Untracked)
            .map(|entry| backup::normalize(&entry.path.to_string_lossy()))
            .collect();
        let ignored_paths: Vec<String> = status
            .entries
            .iter()
            .filter(|entry| entry.kind == EntryKind::Ignored)
            .map(|entry| backup::normalize(&entry.path.to_string_lossy()))
            .collect();
        let untracked_json = serde_json::to_string(&untracked_paths).map_err(|error| {
            SnapshotError::Failed(format!("serializing untracked paths failed: {error}"))
        })?;

        let mut warnings: Vec<SnapshotWarning> = Vec::new();

        // 上次崩溃留下的孤儿目录（复制到一半的 `.tmp-*`）顺手清掉：
        // 每次创建都是一次"启动式"的机会，不需要单独的启动钩子
        let orphans = self.cleanup_orphans_locked(request.repo_id);
        if orphans > 0 {
            warnings.push(SnapshotWarning::OrphansRemoved { count: orphans });
        }

        // 未跟踪内容进备份目录（复制到临时目录，改名发生在入库之后）
        let (staging, skipped) = self.prepare_backup(
            request.repo_id,
            request.workdir,
            &untracked_paths,
            &ignored_paths,
            &mut warnings,
        );

        let (manifest_json, backup_bytes, backed_up) = match staging.as_ref() {
            Some(staged) => (
                staged.manifest.to_json(),
                i64::try_from(staged.manifest.bytes).unwrap_or(i64::MAX),
                staged.manifest.entries.len(),
            ),
            None => ("[]".to_owned(), 0, 0),
        };

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
                manifest_json,
                backup_bytes,
                created_at_ms: self.now(),
            })
            .map_err(|error| SnapshotError::Storage(error.message.clone()))?;

        let name = format!("{SNAPSHOT_REF_PREFIX}{id}");
        if let Err(error) = self.engines.write().update_ref(&repo, &name, &head_oid) {
            let _ = store.delete(id);
            if let Some(staged) = staging {
                let _ = backup::remove_dir(&staged.temporary);
            }
            return Err(SnapshotError::Failed(format!(
                "anchoring the snapshot ref failed: {}",
                error.message
            )));
        }
        store
            .update_ref_column(id, &name)
            .map_err(|error| SnapshotError::Storage(error.message.clone()))?;

        // 把临时目录改名成正式目录（`<root>/<repo_id>/<id>`）：同文件系统内的
        // rename 是原子的，正式目录因此"要么完整、要么不存在"
        if let Some(staged) = staging {
            if let Some(directory) = self.backup_dir(request.repo_id, id) {
                if let Err(error) = fs::rename(&staged.temporary, &directory) {
                    // 改不了名 = 备份内容不可达：把记录回退成"没有内容备份"。
                    // 磁盘与数据库必须一致，否则恢复会去读一个不存在的目录
                    let _ = backup::remove_dir(&staged.temporary);
                    let _ = store.update_backup(id, "[]", 0);
                    warnings.push(SnapshotWarning::BackupDirUnavailable {
                        detail: format!("finalizing the backup directory failed: {error}"),
                    });
                }
            }
        }

        // 保留策略与总占用各清一遍（结果如实回传，界面据此提示"顺手清了旧的"）
        let pruned = self.prune_silently_locked(request.repo_id);

        // 告警同时进日志：自动快照（提交前、合并前…）没有界面通道回传结果，
        // 而"这次快照不含内容备份"是排查回滚问题时最需要知道的一件事
        for warning in &warnings {
            tracing::warn!(
                repo_id = request.repo_id,
                snapshot_id = id,
                kind = warning.kind(),
                "快照创建告警"
            );
        }

        Ok(SnapshotOutcome {
            id,
            backup_bytes: u64::try_from(backup_bytes).unwrap_or(0),
            backed_up,
            untracked_total: untracked_paths.len(),
            skipped,
            warnings,
            pruned,
        })
    }
}

impl SnapshotManager for RefSnapshotManager {
    fn create(&self, request: &SnapshotRequest<'_>) -> Result<SnapshotOutcome, SnapshotError> {
        let lock = self.repo_lock(request.repo_id);
        let _guard = match lock.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        self.create_locked(request)
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

    fn estimate(&self, repo_id: i64) -> Result<SnapshotEstimate, SnapshotError> {
        let workdir = self.workdir(repo_id)?;
        let repo = RepoId::new(workdir.clone());
        let status = self
            .engines
            .read()
            .status(
                &repo,
                &StatusQuery {
                    include_ignored: true,
                },
            )
            .map_err(|error| SnapshotError::Storage(error.message.clone()))?;

        let mut untracked: Vec<String> = Vec::new();
        let mut ignored: Vec<String> = Vec::new();
        for entry in &status.entries {
            let path = backup::normalize(&entry.path.to_string_lossy());
            match entry.kind {
                EntryKind::Untracked => untracked.push(path),
                EntryKind::Ignored => ignored.push(path),
                _ => {}
            }
        }

        let plan = backup::plan(&workdir, &untracked, &ignored, &self.limits);
        // 被忽略文件的体积单独算一次（要按"若开启"来估），
        // 否则用户看不到开启这个开关会付出多大代价
        let ignored_plan = backup::plan(
            &workdir,
            &[],
            &ignored,
            &SnapshotLimits {
                include_ignored: true,
                max_snapshot_bytes: 0,
                max_repo_bytes: 0,
            },
        );

        Ok(SnapshotEstimate {
            untracked_count: if plan.files.is_empty() {
                untracked.len()
            } else {
                plan.files.len()
            },
            untracked_bytes: plan.total_bytes,
            ignored_count: ignored.len(),
            ignored_bytes: ignored_plan.total_bytes,
            include_ignored: self.limits.include_ignored,
            limit_bytes: self.limits.max_snapshot_bytes,
            within_limit: plan.skipped.is_empty(),
            would_skip: plan.skipped.len(),
        })
    }

    fn usage(&self, repo_id: i64) -> Result<SnapshotUsage, SnapshotError> {
        let store = SnapshotStore::new(&self.database);
        let count = store
            .count(repo_id)
            .map_err(|error| SnapshotError::Storage(error.message.clone()))?;
        let bytes = store
            .total_backup_bytes(repo_id)
            .map_err(|error| SnapshotError::Storage(error.message.clone()))?;

        let orphan_dirs = self
            .collect_orphans(repo_id)
            .iter()
            .filter_map(|directory| {
                directory
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .collect();

        Ok(SnapshotUsage {
            repo_id,
            snapshot_count: count,
            backup_bytes: u64::try_from(bytes).unwrap_or(0),
            max_snapshot_bytes: self.limits.max_snapshot_bytes,
            max_repo_bytes: self.limits.max_repo_bytes,
            orphan_dirs,
        })
    }

    fn cleanup(&self, repo_id: i64) -> Result<CleanupOutcome, SnapshotError> {
        let lock = self.repo_lock(repo_id);
        let _guard = match lock.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };

        // 先清孤儿：它们不属于任何快照，删掉不影响"可回滚的东西"
        let orphans = self.collect_orphans(repo_id);
        let mut orphans_removed = 0;
        let mut freed_bytes = 0_u64;
        for directory in &orphans {
            let bytes = backup::dir_bytes(directory);
            if backup::remove_dir(directory).is_ok() {
                orphans_removed += 1;
                freed_bytes = freed_bytes.saturating_add(bytes);
            }
        }

        // 再按保留策略与总占用回收
        let reclaimed = self.prune_locked(repo_id, &self.policy)?;
        let (quota_reclaimed, quota_freed) = self.enforce_repo_quota_locked(repo_id);
        freed_bytes = freed_bytes.saturating_add(quota_freed);

        let store = SnapshotStore::new(&self.database);
        let remaining = store
            .total_backup_bytes(repo_id)
            .map_err(|error| SnapshotError::Storage(error.message.clone()))?;

        let mut reclaimed = reclaimed;
        reclaimed.extend(quota_reclaimed);

        Ok(CleanupOutcome {
            orphans_removed,
            reclaimed,
            freed_bytes,
            remaining_bytes: u64::try_from(remaining).unwrap_or(0),
        })
    }

    fn restore(
        &self,
        repo_id: i64,
        snapshot_id: SnapshotId,
    ) -> Result<RestoreReport, SnapshotError> {
        let lock = self.repo_lock(repo_id);
        let _guard = match lock.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        self.restore_locked(repo_id, snapshot_id)
    }

    fn pending_restore(&self, repo_id: i64) -> Result<Option<PendingRestore>, SnapshotError> {
        SnapshotStore::new(&self.database)
            .restore_pending(repo_id)
            .map_err(|error| SnapshotError::Storage(error.message.clone()))
            .map(|pending| {
                pending.map(|record| PendingRestore {
                    snapshot_id: record.snapshot_id,
                    // 认不出的阶段短名（旧版本写的、手工改过库）当成"未知"：
                    // 界面仍然能提示"上次回滚没走完"，只是说不出停在哪一步
                    stage: record.stage.as_deref().and_then(RestoreStage::from_key),
                    started_at_ms: record.started_at,
                })
            })
    }

    fn abandon_restore(&self, repo_id: i64) -> Result<usize, SnapshotError> {
        SnapshotStore::new(&self.database)
            .clear_restore_progress(repo_id)
            .map_err(|error| SnapshotError::Storage(error.message.clone()))
    }

    fn diff(&self, repo_id: i64, snapshot_id: SnapshotId) -> Result<SnapshotDiff, SnapshotError> {
        let (workdir, record) = self.snapshot_record(repo_id, snapshot_id)?;
        let repo = RepoId::new(workdir.clone());

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

        // 未跟踪内容的三分类（T3.8）：会恢复 / 找不回 / 不会被删
        let manifest = BackupManifest::from_json(&record.manifest_json);
        let recorded = recorded_untracked(&record);
        let mut untracked_restorable = Vec::new();
        let backup_dir = self.backup_dir(repo_id, record.id);
        for entry in &manifest.entries {
            let target = workdir.join(&entry.path);
            let unchanged = backup_dir.as_ref().is_some_and(|directory| {
                let source = directory.join(backup::UNTRACKED_SUBDIR).join(&entry.path);
                source.is_file()
                    && target.is_file()
                    && backup::files_equal(&source, &target).unwrap_or(false)
            });
            if !unchanged {
                untracked_restorable.push(entry.path.clone());
            }
        }

        let backed_paths: HashSet<&str> = manifest
            .entries
            .iter()
            .map(|entry| entry.path.as_str())
            .collect();
        let untracked_missing = recorded
            .iter()
            .filter(|path| !backed_paths.contains(path.as_str()))
            .filter(|path| !workdir.join(path).exists())
            .cloned()
            .collect();
        let untracked_extra = self
            .current_untracked(&repo)?
            .into_iter()
            .filter(|path| !recorded.contains(path))
            .collect();

        Ok(SnapshotDiff {
            head_changed: current_head_oid.as_deref() != Some(record.head_oid.as_str()),
            index_changed: current_index_tree_oid.as_deref()
                != Some(record.index_tree_oid.as_str()),
            current_head_oid,
            current_index_tree_oid,
            ref_missing,
            untracked_restorable,
            untracked_missing,
            untracked_extra,
        })
    }

    fn prune(
        &self,
        repo_id: i64,
        policy: &RetentionPolicy,
    ) -> Result<Vec<SnapshotId>, SnapshotError> {
        let lock = self.repo_lock(repo_id);
        let _guard = match lock.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        self.prune_locked(repo_id, policy)
    }
}

// ---------------------------------------------------------------- 回滚辅助（T3.9）

/// 一次分阶段应用的产物。
#[derive(Debug)]
struct AppliedStages {
    /// 各阶段结果（按执行顺序）。
    stages: Vec<StageResult>,
    /// 第一个失败的阶段（`None` = 全部成功）。
    failure: Option<SnapshotError>,
    /// 写回的未跟踪文件数。
    untracked_restored: usize,
    /// 没能写回的未跟踪文件。
    untracked_failed: Vec<String>,
    /// 当前存在、快照里没有的未跟踪文件。
    untracked_extra: Vec<String>,
    /// 内容与 git 事实的校验是否通过。
    content_verified: bool,
}

impl Default for AppliedStages {
    fn default() -> Self {
        Self {
            stages: Vec::new(),
            failure: None,
            untracked_restored: 0,
            untracked_failed: Vec::new(),
            untracked_extra: Vec::new(),
            // 在还没做任何校验之前按"通过"起步：任何一步失败都会把它改成 false，
            // 而"根本没有内容要校验"（v1 快照、空备份）本来就是通过
            content_verified: true,
        }
    }
}

/// git 写操作失败 → `RestoreFailed`。
///
/// 命令名保留原文（`git reset --hard`）：报告是给人看的，
/// 而这一条正是用户拿去对照 git 文档、或者手动重试时最需要的线索。
fn git_failure(step: &str, error: &forgedesk_domain::AppError) -> SnapshotError {
    SnapshotError::RestoreFailed(format!("{step} failed: {}", error.message))
}

/// 注入故障的报错文本。
///
/// 说清"这是测试注入的"：紧急模式的报告会被贴进 issue 与日志，
/// 一条看起来像真实 git 失败的文本会让人追着不存在的问题跑。
fn injected_failure(stage: RestoreStage) -> SnapshotError {
    SnapshotError::RestoreFailed(format!(
        "injected failure at the '{}' stage (test only)",
        stage.key()
    ))
}

/// 一个阶段的结果（成功也记耗时：报告里"哪一步慢"是有用的事实）。
fn stage_result(
    stage: RestoreStage,
    failure: Option<&SnapshotError>,
    started_ms: i64,
    ended_ms: i64,
) -> StageResult {
    StageResult {
        stage,
        ok: failure.is_none(),
        detail: failure.map(SnapshotError::message),
        duration_ms: (ended_ms - started_ms).max(0),
    }
}

/// 记录里的未跟踪路径（归一化后进集合，便于按路径判断归属）。
fn recorded_untracked(record: &SnapshotRecord) -> BTreeSet<String> {
    serde_json::from_str::<Vec<String>>(&record.untracked_paths)
        .unwrap_or_default()
        .into_iter()
        .map(|path| backup::normalize(&path))
        .collect()
}

/// 系统时间的 Unix 毫秒（测试之外的时间源）。
fn system_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or_default()
}

/// 临时目录名的后缀：进程号 + 纳秒。同一进程内连续创建也不会撞名。
fn temp_token() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("{}-{nanos}", std::process::id())
}
