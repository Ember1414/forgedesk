//! `Libgit2Engine`：libgit2（进程内库）实现，只承担读操作。
//!
//! 占位：完整实现紧随其后（T1.2 的同一批次）。
//! 在此之前每个方法都返回
//! [`ErrorCode::UnsupportedByEngine`](forgedesk_domain::ErrorCode::UnsupportedByEngine)，
//! 这是**明确失败**而不是静默成功——静默成功会让 services 层的
//! "预览 → 快照 → 执行"链路以为操作已经完成。

use std::path::Path;

use forgedesk_domain::git::{
    Branch, CheckoutSpec, CloneSpec, Commit, CommitSpec, DiffReport, DiffSpec, FetchOutcome,
    FetchSpec, InitSpec, LogQuery, MergeOutcome, MergeSpec, Page, PullOutcome, PullSpec,
    PushOutcome, PushSpec, ReflogEntry, Remote, ReorderSpec, RepoId, RepositoryInfo, ResetSpec,
    StageSpec, StashEntry, StashSpec, StatusReport, Tag,
};
use forgedesk_domain::AppResult;

use super::progress::ProgressSink;
use super::{unsupported, EngineId, GitEngine};

/// libgit2 实现。
#[derive(Debug, Default)]
pub struct Libgit2Engine;

impl Libgit2Engine {
    /// 创建引擎。
    pub fn new() -> Self {
        Self
    }
}

impl GitEngine for Libgit2Engine {
    fn id(&self) -> EngineId {
        EngineId::Libgit2
    }

    fn discover(&self, _path: &Path) -> AppResult<RepositoryInfo> {
        Err(unsupported(EngineId::Libgit2, "discover"))
    }

    fn status(&self, _repo: &RepoId) -> AppResult<StatusReport> {
        Err(unsupported(EngineId::Libgit2, "status"))
    }

    fn diff(&self, _repo: &RepoId, _spec: DiffSpec) -> AppResult<DiffReport> {
        Err(unsupported(EngineId::Libgit2, "diff"))
    }

    fn log(&self, _repo: &RepoId, _query: LogQuery) -> AppResult<Page<Commit>> {
        Err(unsupported(EngineId::Libgit2, "log"))
    }

    fn show(&self, _repo: &RepoId, _revision: &str) -> AppResult<Commit> {
        Err(unsupported(EngineId::Libgit2, "show"))
    }

    fn branch_list(&self, _repo: &RepoId) -> AppResult<Vec<Branch>> {
        Err(unsupported(EngineId::Libgit2, "branch_list"))
    }

    fn tag_list(&self, _repo: &RepoId) -> AppResult<Vec<Tag>> {
        Err(unsupported(EngineId::Libgit2, "tag_list"))
    }

    fn remote_list(&self, _repo: &RepoId) -> AppResult<Vec<Remote>> {
        Err(unsupported(EngineId::Libgit2, "remote_list"))
    }

    fn stash_list(&self, _repo: &RepoId) -> AppResult<Vec<StashEntry>> {
        Err(unsupported(EngineId::Libgit2, "stash_list"))
    }

    fn reflog(&self, _repo: &RepoId, _limit: usize) -> AppResult<Vec<ReflogEntry>> {
        Err(unsupported(EngineId::Libgit2, "reflog"))
    }

    fn init(&self, _path: &Path, _spec: InitSpec) -> AppResult<RepositoryInfo> {
        Err(unsupported(EngineId::Libgit2, "init"))
    }

    fn clone(&self, _spec: CloneSpec, _progress: &ProgressSink) -> AppResult<RepositoryInfo> {
        Err(unsupported(EngineId::Libgit2, "clone"))
    }

    fn stage(&self, _repo: &RepoId, _spec: StageSpec) -> AppResult<()> {
        Err(unsupported(EngineId::Libgit2, "stage"))
    }

    fn unstage(&self, _repo: &RepoId, _spec: StageSpec) -> AppResult<()> {
        Err(unsupported(EngineId::Libgit2, "unstage"))
    }

    fn commit(&self, _repo: &RepoId, _spec: CommitSpec) -> AppResult<String> {
        Err(unsupported(EngineId::Libgit2, "commit"))
    }

    fn reset(&self, _repo: &RepoId, _spec: ResetSpec) -> AppResult<()> {
        Err(unsupported(EngineId::Libgit2, "reset"))
    }

    fn checkout(&self, _repo: &RepoId, _spec: CheckoutSpec) -> AppResult<()> {
        Err(unsupported(EngineId::Libgit2, "checkout"))
    }

    fn merge(&self, _repo: &RepoId, _spec: MergeSpec) -> AppResult<MergeOutcome> {
        Err(unsupported(EngineId::Libgit2, "merge"))
    }

    fn cherry_pick(&self, _repo: &RepoId, _revision: &str) -> AppResult<MergeOutcome> {
        Err(unsupported(EngineId::Libgit2, "cherry_pick"))
    }

    fn revert(&self, _repo: &RepoId, _revision: &str) -> AppResult<MergeOutcome> {
        Err(unsupported(EngineId::Libgit2, "revert"))
    }

    fn stash(&self, _repo: &RepoId, _spec: StashSpec) -> AppResult<()> {
        Err(unsupported(EngineId::Libgit2, "stash"))
    }

    fn fetch(
        &self,
        _repo: &RepoId,
        _spec: FetchSpec,
        _progress: &ProgressSink,
    ) -> AppResult<FetchOutcome> {
        Err(unsupported(EngineId::Libgit2, "fetch"))
    }

    fn pull(
        &self,
        _repo: &RepoId,
        _spec: PullSpec,
        _progress: &ProgressSink,
    ) -> AppResult<PullOutcome> {
        Err(unsupported(EngineId::Libgit2, "pull"))
    }

    fn push(
        &self,
        _repo: &RepoId,
        _spec: PushSpec,
        _progress: &ProgressSink,
    ) -> AppResult<PushOutcome> {
        Err(unsupported(EngineId::Libgit2, "push"))
    }

    fn rebase(
        &self,
        _repo: &RepoId,
        _plan: ReorderSpec,
        _progress: &ProgressSink,
    ) -> AppResult<MergeOutcome> {
        Err(unsupported(EngineId::Libgit2, "rebase"))
    }
}
