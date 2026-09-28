//! 远端同步（T2.6）的集成测试：**本地 bare 仓库当远端**。
//!
//! # 为什么用 bare 仓库而不是真实 GitHub
//!
//! 任务书（docs/AGENT-PROMPTS.md §4.3）的既定决策：五类场景全部用本地 bare
//! 仓库复现；真实 GitHub 的联调清单交付给用户在有网机器上跑。
//!
//! 这不是"退而求其次"：`file://` 远端与 HTTPS 远端走的是 git **同一条
//! receive-pack / upload-pack 代码路径**——refspec 展开、远端引用广告、
//! `non-fast-forward` 拒绝行、冲突的 unmerged 索引状态全部一致。差别只在
//! 认证与网络传输，而这两件事在 T2.7（凭据）与 T2.6 的断网/取消用例里覆盖。
//! 也就是说：本文件钉的是**产品自己的编排**（快照、错误包装、取消、校验），
//! 不是"能不能连上网"。
//!
//! # 本文件钉住的行为（任务书验收项）
//!
//! 1. **正常**：remote CRUD 往返；push 更新远端并设置上游；fetch 拿到引用更新；
//!    pull 快进并留下 `PreSync` 快照。
//! 2. **冲突**：pull 合并冲突返回 `PullOutcome::Conflicted`（含冲突文件清单），
//!    而不是把 git 的非零退出码压成"命令失败"。
//! 3. **non-ff 拒绝**：push 被拒 → `AppError{PUSH_REJECTED}` 且带 3 条修复动作；
//!    按第一条（先拉取）再 `--force-with-lease` 能按用户意图覆盖（红线 R7）。
//! 4. **断网**：不可达远端 → 分类为 `NETWORK`（不是 `Unknown`）。
//! 5. **取消**：`CancellationToken` 触发 → `ErrorCode::Cancelled`，且远端零改动。
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::path::Path;

use forgedesk_domain::git::{
    FetchSpec, MergeKind, PullSpec, PullStrategy, PushSpec, Remote, RepoPath,
};
use forgedesk_domain::ErrorCode;
use forgedesk_git_engine::engine::progress::ProgressSink;
use forgedesk_git_engine::engines::GitEngines;
use forgedesk_services::SyncService;
use forgedesk_snapshot::{
    RestoreReport, RetentionPolicy, SnapshotDiff, SnapshotError, SnapshotId, SnapshotKind,
    SnapshotManager, SnapshotMeta, SnapshotRequest,
};
use support::{commit_all, file_url, git, git_ok, init_repo, write, TempDir};
use tokio_util::sync::CancellationToken;

// ---------------------------------------------------------------- 夹具

/// 测试用快照管理器：只记录"被要求打了哪类快照"。
///
/// 快照内容是 `forgedesk-snapshot` 的责任（已有单测）；本文件要断言的是
/// **编排**——pull 之前必须打 `PreSync`，fetch 不该打（它不动本地历史）。
#[derive(Default)]
struct RecordingSnapshots {
    kinds: std::sync::Mutex<Vec<SnapshotKind>>,
}

impl std::fmt::Debug for RecordingSnapshots {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecordingSnapshots").finish()
    }
}

impl RecordingSnapshots {
    /// 已记录的快照类别短名，按发生顺序。
    fn keys(&self) -> Vec<&'static str> {
        self.kinds
            .lock()
            .unwrap()
            .iter()
            .map(|kind| kind.key())
            .collect()
    }
}

impl SnapshotManager for RecordingSnapshots {
    fn create(&self, request: &SnapshotRequest<'_>) -> Result<SnapshotId, SnapshotError> {
        self.kinds.lock().unwrap().push(request.kind);
        Ok(1)
    }

    fn list(&self, _repo_id: i64, _limit: i64) -> Result<Vec<SnapshotMeta>, SnapshotError> {
        Ok(Vec::new())
    }

    fn restore(
        &self,
        _repo_id: i64,
        _snapshot_id: SnapshotId,
    ) -> Result<RestoreReport, SnapshotError> {
        Err(SnapshotError::NotFound(0))
    }

    fn diff(&self, _repo_id: i64, _snapshot_id: SnapshotId) -> Result<SnapshotDiff, SnapshotError> {
        Err(SnapshotError::NotFound(0))
    }

    fn prune(
        &self,
        _repo_id: i64,
        _policy: &RetentionPolicy,
    ) -> Result<Vec<SnapshotId>, SnapshotError> {
        Ok(Vec::new())
    }
}

/// 借用生命周期的小包装（与 branch.rs 的 `Box::leak` 同一技巧）。
mod forge_setup {
    use std::path::Path;

    use forgedesk_storage::{Database, RepositoryStore, RepositoryUpsert};

    pub struct LeakDatabase {
        inner: Database,
    }

    pub fn leak_database() -> LeakDatabase {
        let database = Database::open_in_memory().expect("打开内存库失败");
        forgedesk_storage::migrate(&database).expect("迁移失败");
        LeakDatabase { inner: database }
    }

    pub fn store(database: &'static LeakDatabase) -> RepositoryStore<'static> {
        // LeakDatabase 永不 drop（Box::leak），所以借用 'static 安全
        RepositoryStore::new(&database.inner)
    }

    pub fn register(store: &RepositoryStore<'_>, path: &Path) -> i64 {
        store
            .upsert(
                &RepositoryUpsert {
                    name: "fixture".to_owned(),
                    path: path.to_string_lossy().to_string(),
                    default_branch: Some("main".to_owned()),
                    provider_id: None,
                    size_class: None,
                },
                0,
            )
            .expect("登记仓库失败")
    }
}

/// 用完整步进的全功能栈组装一个同步服务（引擎 / 内存库 / 记录型快照）。
fn service(dir: &Path) -> (SyncService<'static>, i64, &'static RecordingSnapshots) {
    let engines: &'static GitEngines =
        Box::leak(Box::new(GitEngines::new().expect("创建引擎失败")));
    let database: &'static forge_setup::LeakDatabase =
        Box::leak(Box::new(forge_setup::leak_database()));
    let snapshots: &'static RecordingSnapshots = Box::leak(Box::new(RecordingSnapshots::default()));
    let store = forge_setup::store(database);
    let repo_id = forge_setup::register(&store, dir);
    (
        SyncService::new(engines, store, snapshots),
        repo_id,
        snapshots,
    )
}

/// 建一个 bare 仓库当"远端"，返回它的目录句柄与 `file://` URL。
fn bare_remote(prefix: &str) -> (TempDir, String) {
    let dir = TempDir::new(prefix);
    git_ok(dir.path(), &["init", "--bare", "-q", "-b", "main", "."]);
    let url = file_url(dir.path());
    (dir, url)
}

/// 一个已提交 `a.txt`、并把 `origin` 指向 bare 远端的本地仓库。
fn local_with_origin(
    prefix: &str,
    remote_url: &str,
) -> (
    TempDir,
    i64,
    SyncService<'static>,
    &'static RecordingSnapshots,
) {
    let dir = TempDir::new(prefix);
    init_repo(dir.path());
    write(dir.path(), "a.txt", b"base\n");
    commit_all(dir.path(), "base");

    let (service, repo_id, snapshots) = service(dir.path());
    service
        .remote_add(repo_id, "origin", remote_url)
        .expect("添加远端失败");
    (dir, repo_id, service, snapshots)
}

/// 克隆远端到临时目录并配好提交身份（模拟"另一台机器"）。
fn clone_of(prefix: &str, url: &str) -> TempDir {
    let dir = TempDir::new(prefix);
    git_ok(dir.path(), &["clone", "-q", url, "."]);
    git_ok(dir.path(), &["config", "user.name", "Second Author"]);
    git_ok(dir.path(), &["config", "user.email", "second@example.com"]);
    dir
}

/// 解析某个引用；不存在时返回 `None`（空仓库 / 尚未推送）。
fn revision_of(dir: &Path, revision: &str) -> Option<String> {
    let output = git(dir, &["rev-parse", "--verify", "--quiet", revision]);
    if output.success() {
        Some(output.stdout_lossy().trim().to_owned())
    } else {
        None
    }
}

/// 首次 push 并设置上游（多数用例的前置步骤）。
fn push_initial(service: &SyncService<'_>, repo_id: i64) {
    let outcome = service
        .push(
            repo_id,
            PushSpec::new().with_set_upstream(true),
            &ProgressSink::none(),
            &CancellationToken::new(),
        )
        .expect("首次 push 失败");
    assert!(
        outcome.is_success(),
        "首次 push 不该被拒绝：{:?}",
        outcome.rejections
    );
}

// ---------------------------------------------------------------- 场景 1：正常

#[test]
fn remote_crud_round_trips_through_the_engine() {
    let (remote, url) = bare_remote("sync-crud-remote");
    let (_local, repo_id, service, _snapshots) = local_with_origin("sync-crud-local", &url);

    let remotes = service.remote_list(repo_id).unwrap();
    assert_eq!(remotes.len(), 1);
    assert_remote(&remotes[0], "origin", &url);

    // add → rename → set-url：每一步都读回来断言，避免"写了但没生效"。
    service.remote_add(repo_id, "backup", &url).unwrap();
    service.remote_rename(repo_id, "backup", "mirror").unwrap();
    // 改成一个格式合法但不可达的 URL：本用例只验证配置写入。
    service
        .remote_set_url(repo_id, "mirror", "https://example.com/owner/repo.git")
        .unwrap();

    let remotes = service.remote_list(repo_id).unwrap();
    let mirror = remotes
        .iter()
        .find(|remote| remote.name == "mirror")
        .expect("rename 后应存在 mirror");
    assert_remote(mirror, "mirror", "https://example.com/owner/repo.git");

    service.remote_remove(repo_id, "mirror").unwrap();
    let names: Vec<String> = service
        .remote_list(repo_id)
        .unwrap()
        .into_iter()
        .map(|remote| remote.name)
        .collect();
    assert_eq!(names, vec!["origin".to_owned()]);

    // 校验在服务层（IPC 之后的第二道）：非法名与非法协议都必须在触碰仓库前失败。
    let error = service
        .remote_add(repo_id, "bad name", &url)
        .expect_err("含空格的远端名应被拒绝");
    assert_eq!(error.code, ErrorCode::Validation);

    let error = service
        .remote_add(repo_id, "ftp", "ftp://host/repo.git")
        .expect_err("不支持的协议应被拒绝");
    assert_eq!(error.code, ErrorCode::Validation);

    let _ = remote;
}

#[test]
fn push_then_fetch_then_pull_fast_forwards_from_the_bare_remote() {
    let (remote, url) = bare_remote("sync-ff-remote");
    let (local, repo_id, service, snapshots) = local_with_origin("sync-ff-local", &url);

    push_initial(&service, repo_id);
    assert_eq!(
        revision_of(remote.path(), "refs/heads/main"),
        revision_of(local.path(), "HEAD"),
        "push 之后远端 main 必须指向本地 HEAD"
    );

    // 另一台机器推一个新提交。
    let other = clone_of("sync-ff-other", &url);
    write(other.path(), "b.txt", b"from other\n");
    commit_all(other.path(), "from other");
    git_ok(other.path(), &["push", "-q", "origin", "main"]);

    let fetched = service
        .fetch(
            repo_id,
            FetchSpec::new(),
            &ProgressSink::none(),
            &CancellationToken::new(),
        )
        .unwrap();
    assert_eq!(fetched.remote, "origin");
    assert!(
        fetched.changed_refs() >= 1,
        "fetch 应报告 origin/main 的更新：{:?}",
        fetched.updates
    );
    assert!(
        snapshots.keys().is_empty(),
        "fetch 不动本地历史，不该打快照：{:?}",
        snapshots.keys()
    );

    let pulled = service
        .pull(
            repo_id,
            PullSpec::new(),
            &ProgressSink::none(),
            &CancellationToken::new(),
        )
        .unwrap();
    assert!(!pulled.up_to_date);
    assert_eq!(
        pulled.merge.as_ref().map(|merge| merge.kind),
        Some(MergeKind::FastForward)
    );
    assert!(!pulled.has_conflicts());
    assert_eq!(
        snapshots.keys(),
        vec!["pre-sync"],
        "pull 会移动 HEAD，必须先打 PreSync 快照"
    );
    assert_eq!(
        revision_of(local.path(), "HEAD"),
        revision_of(remote.path(), "refs/heads/main"),
        "快进后本地 HEAD 应与远端一致"
    );
}

// ---------------------------------------------------------------- 场景 2：冲突

#[test]
fn a_conflicting_pull_returns_the_conflicted_outcome_with_file_list() {
    let (remote, url) = bare_remote("sync-conflict-remote");
    let (local, repo_id, service, _snapshots) = local_with_origin("sync-conflict-local", &url);
    push_initial(&service, repo_id);

    // 远端改同一个文件。
    let other = clone_of("sync-conflict-other", &url);
    write(other.path(), "a.txt", b"remote change\n");
    commit_all(other.path(), "remote change");
    git_ok(other.path(), &["push", "-q", "origin", "main"]);

    // 本地改同一个文件的另一处 → 合并必然冲突。
    write(local.path(), "a.txt", b"local change\n");
    commit_all(local.path(), "local change");

    let pulled = service
        .pull(
            repo_id,
            PullSpec::new().with_strategy(PullStrategy::Merge),
            &ProgressSink::none(),
            &CancellationToken::new(),
        )
        .expect("冲突是结果而不是错误：git 的非零退出码不该被压成 Err");

    assert!(pulled.has_conflicts());
    let merge = pulled.merge.as_ref().expect("冲突时应带合并结果");
    assert_eq!(merge.kind, MergeKind::Conflicted);
    assert_eq!(merge.oid, None, "冲突时没有合并提交");
    assert!(
        merge.conflicts.contains(&RepoPath::from("a.txt")),
        "冲突清单应包含 a.txt：{:?}",
        merge.conflicts
    );

    // 冲突留在索引里（M3 的冲突向导要靠它），且快照已经先打过。
    let unmerged = git(local.path(), &["ls-files", "-u"]);
    assert!(unmerged.success());
    assert!(
        unmerged.stdout_lossy().contains("a.txt"),
        "冲突状态必须留在仓库里供 M3 解决"
    );

    let _ = remote;
}

// ---------------------------------------------------------------- 场景 3：non-ff 拒绝

#[test]
fn a_non_fast_forward_push_is_rejected_with_three_fix_actions() {
    let (remote, url) = bare_remote("sync-reject-remote");
    let (local, repo_id, service, _snapshots) = local_with_origin("sync-reject-local", &url);
    push_initial(&service, repo_id);

    // 另一台机器先推一个提交 → 远端领先。
    let other = clone_of("sync-reject-other", &url);
    write(other.path(), "b.txt", b"from other\n");
    commit_all(other.path(), "from other");
    git_ok(other.path(), &["push", "-q", "origin", "main"]);

    // 本地做出分叉提交（历史里没有远端的那个提交）。
    write(local.path(), "a.txt", b"diverged\n");
    commit_all(local.path(), "diverged");

    let error = service
        .push(
            repo_id,
            PushSpec::new(),
            &ProgressSink::none(),
            &CancellationToken::new(),
        )
        .expect_err("non-fast-forward 必须被拒绝");

    assert_eq!(error.code, ErrorCode::PushRejected);
    let detail = error.detail.clone().unwrap_or_default();
    // git 视传输方式给出 "non-fast-forward"（智能传输）或 "fetch first"
    // （本地 receive-pack）——两者都必须被识别为同一类可修复拒绝。
    assert!(
        detail.contains("non-fast-forward") || detail.contains("fetch first"),
        "detail 应保留 git 给出的原因：{:?}",
        error.detail
    );
    let actions: Vec<&str> = error
        .actions
        .iter()
        .map(|action| action.id.as_str())
        .collect();
    assert_eq!(
        actions,
        vec!["fetch-first", "force-with-lease", "cancel"],
        "被拒时必须给出三条可执行的下一步（任务书要求 4）"
    );
    assert!(
        error
            .actions
            .iter()
            .all(|action| action.label_key.starts_with("errors:actions.")),
        "按钮文案必须走 i18n key，不能是硬编码英文"
    );

    // 失败之后本地 HEAD 不动，远端仍是别人的提交。
    assert_ne!(
        revision_of(remote.path(), "refs/heads/main"),
        revision_of(local.path(), "HEAD"),
        "被拒绝的 push 不能改动远端"
    );
    let _ = local;
}

#[test]
fn force_with_lease_after_fetch_overwrites_the_remote_on_purpose() {
    let (remote, url) = bare_remote("sync-lease-remote");
    let (local, repo_id, service, _snapshots) = local_with_origin("sync-lease-local", &url);
    push_initial(&service, repo_id);

    let other = clone_of("sync-lease-other", &url);
    write(other.path(), "b.txt", b"from other\n");
    commit_all(other.path(), "from other");
    git_ok(other.path(), &["push", "-q", "origin", "main"]);

    write(local.path(), "a.txt", b"diverged\n");
    commit_all(local.path(), "diverged");

    // 第一条修复动作：先拉取（更新 remote-tracking ref，不合并）。
    service
        .fetch(
            repo_id,
            FetchSpec::new(),
            &ProgressSink::none(),
            &CancellationToken::new(),
        )
        .unwrap();

    let outcome = service
        .push(
            repo_id,
            PushSpec::new().with_force_with_lease(true),
            &ProgressSink::none(),
            &CancellationToken::new(),
        )
        .expect("fetch 之后 force-with-lease 应被接受");
    assert!(
        outcome.is_success(),
        "force-with-lease 不该再被拒：{:?}",
        outcome.rejections
    );
    assert_eq!(
        revision_of(remote.path(), "refs/heads/main"),
        revision_of(local.path(), "HEAD"),
        "用户明确选择覆盖后，远端应指向本地历史"
    );
    let _ = local;
}

// ---------------------------------------------------------------- 场景 4：断网

#[test]
fn an_unreachable_remote_classifies_as_a_network_error() {
    let (remote, url) = bare_remote("sync-net-remote");
    let (_local, repo_id, service, _snapshots) = local_with_origin("sync-net-local", &url);

    // 127.0.0.1:1 是本地必然被拒绝的端口：不依赖外网，也不会挂住等 DNS 超时。
    service
        .remote_add(repo_id, "broken", "https://127.0.0.1:1/repo.git")
        .unwrap();

    let error = service
        .fetch(
            repo_id,
            FetchSpec::new().with_remote("broken"),
            &ProgressSink::none(),
            &CancellationToken::new(),
        )
        .expect_err("不可达的远端必须失败");

    assert!(
        matches!(error.code, ErrorCode::Network | ErrorCode::AuthRequired),
        "连接失败应分类为网络类错误，实际：{:?}（detail: {:?}）",
        error.code,
        error.detail
    );
    assert!(
        error
            .detail
            .as_deref()
            .unwrap_or_default()
            .contains("127.0.0.1"),
        "detail 应保留失败的主机（否则用户无从排查）：{:?}",
        error.detail
    );
    let _ = remote;
}

// ---------------------------------------------------------------- 场景 5：取消

#[test]
fn a_cancelled_token_stops_network_operations_and_leaves_the_remote_untouched() {
    let (remote, url) = bare_remote("sync-cancel-remote");
    let (_local, repo_id, service, _snapshots) = local_with_origin("sync-cancel-local", &url);

    // 用户"点了取消"：令牌已触发。这条路径最容易被忽略——如果取消只在
    // 进程启动后才被观察，界面上的"取消"就会变成一个什么都不做的按钮。
    let cancelled = CancellationToken::new();
    cancelled.cancel();

    let error = service
        .push(
            repo_id,
            PushSpec::new().with_set_upstream(true),
            &ProgressSink::none(),
            &cancelled,
        )
        .expect_err("已取消的令牌必须让 push 失败");
    assert_eq!(error.code, ErrorCode::Cancelled);
    assert_eq!(
        revision_of(remote.path(), "refs/heads/main"),
        None,
        "取消之后远端不该出现任何引用"
    );

    let error = service
        .fetch(repo_id, FetchSpec::new(), &ProgressSink::none(), &cancelled)
        .expect_err("已取消的令牌必须让 fetch 失败");
    assert_eq!(error.code, ErrorCode::Cancelled);

    let error = service
        .pull(repo_id, PullSpec::new(), &ProgressSink::none(), &cancelled)
        .expect_err("已取消的令牌必须让 pull 失败");
    assert_eq!(error.code, ErrorCode::Cancelled);
}

// ---------------------------------------------------------------- 断言辅助

fn assert_remote(remote: &Remote, name: &str, url: &str) {
    assert_eq!(remote.name, name);
    assert_eq!(remote.fetch_url, url);
    assert_eq!(
        remote.effective_push_url(),
        url,
        "未单独配置 push URL 时应回退到 fetch URL"
    );
}
