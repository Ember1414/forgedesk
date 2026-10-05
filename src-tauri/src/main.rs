// 生产构建下隐藏 Windows 控制台窗口；调试构建保留控制台以便看日志。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![forbid(unsafe_code)]

//! ForgeDesk 桌面应用入口（Tauri 宿主）。
//!
//! 本 crate 刻意保持**极薄**：只负责初始化日志、打开数据库并迁移、注册命令、启动窗口。
//! 所有业务逻辑都在 `crates/` 下的分层 crate 中（见 AGENTS.md §6）。

use std::path::Path;
use std::sync::{Arc, Mutex};

use forgedesk_commands::{emit_watch_event, AppState, WatcherRegistry};
use forgedesk_diagnostics::SanitizingMakeWriter;
use forgedesk_jobs::JobRunner;
use forgedesk_platform::session::{detect_previous_session, start_session, SessionMarker};
use forgedesk_platform::watcher::NotifyFileWatcher;
use forgedesk_platform::{install_panic_hook, non_blocking_writer, LogFlushGuard, LogPolicy};
use forgedesk_provider::{GitHubHttp, HttpConfig};
use forgedesk_services::repository::OpenRepoRegistry;
use forgedesk_services::{
    accounts::AccountService, host_repos::HostRepoService, CommitPlanRegistry, CredentialGate,
    CredentialsService, GitEngines, LogPageCache, MergePlanRegistry, ResetPlanRegistry,
};
use forgedesk_snapshot::RefSnapshotManager;
use forgedesk_storage::{migrate, Database};
use tauri::{Manager, RunEvent};
use tracing::{error, info, warn};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

/// 数据库文件名（位于应用数据目录）。
const DATABASE_FILE: &str = "forgedesk.db";

/// 凭据索引文件名（位于应用数据目录）。
///
/// 只存"我们保存过哪些凭据"（provider/host/login/类型/时间），**密文在系统凭据库**
/// （红线 R8）。系统凭据库没有统一的枚举接口，因此这份索引是列表功能的唯一来源。
const CREDENTIALS_INDEX_FILE: &str = "credentials.index.json";

/// 应用版本（编译期注入，用于日志、会话标记与 panic 报告）。
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// 日志系统的运行期句柄。
///
/// 两个字段都必须活到进程结束：
/// - `_log_guard`：非阻塞写入的 flush 守卫，drop 时把缓冲写进文件；
/// - `session`：会话标记，正常退出时删除它（不删 = 下次启动判定为异常退出）。
///
/// 用 `Mutex<Option<..>>` 是为了在退出事件里 `take()` 出来显式结束会话——
/// 只在 Drop 里做这件事的话，Tauri 的退出路径与 Rust 的析构顺序会让语义变得不可验证。
struct RuntimeHandles {
    _log_guard: LogFlushGuard,
    session: Mutex<Option<SessionMarker>>,
}

/// askpass 模式（T2.7）：git 把本进程当成凭据提示的应答者拉起时，把答案写到 stdout。
///
/// 返回 `true` 表示"本次启动就是一次 askpass 调用，已处理完毕"。
/// 为什么放在宿主而不是命令层：它发生在任何 Tauri 与数据库初始化**之前**，
/// 而那两者都是命令层的前提条件。协议细节见 `forgedesk_credentials::askpass`。
fn handle_askpass_invocation() -> bool {
    let args: Vec<String> = std::env::args().collect();
    if !forgedesk_credentials::is_askpass_invocation(&args) {
        return false;
    }
    let prompt = forgedesk_credentials::prompt_from_args(&args);
    let answer = forgedesk_credentials::answer_for(&prompt, &|key| std::env::var(key).ok());
    forgedesk_credentials::write_answer(answer.as_deref());
    if answer.is_none() {
        // 认不出的提示语不回答：让 git 直接报认证失败，而不是拿到一个空答案继续
        std::process::exit(1);
    }
    true
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // askpass 模式（T2.7）：见 handle_askpass_invocation。
    if handle_askpass_invocation() {
        return Ok(());
    }

    let builder = tauri::Builder::default()
        // 目录/文件选择对话框（GIT-01/02/03 的"打开/克隆/初始化"入口用）：
        // 权限在 capabilities/default.json 里显式声明（dialog:default 只含打开
        // 选择器，不含保存/消息框之外的任何能力）
        .plugin(tauri_plugin_dialog::init())
        // 日志、panic hook、数据库都在 setup 中初始化：
        // 因为 `app_log_dir()` / `app_data_dir()` 只有在拿到 App 句柄后才可用。
        // 代价是 Tauri 自身在 setup 之前的那几行日志不会被记录——那些是框架内部
        // 初始化信息，对用户与我们排查应用问题都没有价值。
        .setup(|app| {
            let log_dir = app.path().app_log_dir()?;
            let data_dir = app.path().app_data_dir()?;

            // 顺序很重要：先装日志与 panic hook，再打开数据库。
            // 数据库初始化是最容易在启动期失败的一步，它失败时我们要能看到原因。
            let guard = init_logging(&log_dir)?;
            install_panic_hook(&log_dir, APP_VERSION);

            // 上一次是否异常退出：T0.8 只记录事实，M7/T7.5 会据此提供恢复引导
            if let Some(previous) = detect_previous_session(&log_dir) {
                warn!(
                    marker = %previous.marker_path.display(),
                    pid = previous.info.as_ref().map(|info| info.pid),
                    version = previous.info.as_ref().map(|info| info.version.as_str()),
                    started_at = previous.info.as_ref().map(|info| info.started_at),
                    "检测到上次会话未正常退出"
                );
            }

            let session = start_session(&log_dir, APP_VERSION)?;

            let database_path = data_dir.join(DATABASE_FILE);
            let database = Database::open(&database_path)?;
            let report = migrate(&database)?;
            if !report.applied.is_empty() {
                info!(
                    from = report.from_version,
                    to = report.to_version,
                    applied = report.applied.len(),
                    backup = report.backup_path.map(|path| path.display().to_string()),
                    "数据库已迁移"
                );
            }

            info!(
                version = APP_VERSION,
                log_dir = %log_dir.display(),
                database = %database_path.display(),
                "应用已启动"
            );

            // 两个 Git 引擎：读走 libgit2、写走系统 git CLI。
            // 创建失败只可能来自 CLI 引擎（它要起一条驱动异步执行器的线程）；
            // 一个连 git 都起不来的环境，后面每个用例都会失败，因此这里直接
            // 让启动失败并把原因写进日志，而不是让用户在每次操作时各撞一次墙。
            let engines = GitEngines::new().map_err(|error| {
                error!(
                    code = error.code.as_str(),
                    message = %error.message,
                    detail = error.detail.as_deref().unwrap_or_default(),
                    "创建 Git 引擎失败"
                );
                error.to_string()
            })?;

            // 快照管理器与命令层共享同一批引擎与同一个库：
            // 快照是"git 事实 + 一行记录"的组合体，两者必须同源，否则会各说各话
            let engines = Arc::new(engines);
            let database = Arc::new(database);

            // 文件监听（T1.10）：注册表持有"每仓库一份句柄"，事件经 sink 变成
            // `repo:changed`。sink 捕获的是 AppHandle 而不是 App 的借用——
            // 它的生命周期要跟进程一样长，而事件可能在监听线程上发出。
            let handle = app.handle().clone();
            let watchers = Arc::new(WatcherRegistry::new(
                Arc::new(NotifyFileWatcher),
                Arc::new(move |repo_id, event| {
                    emit_watch_event(&handle, repo_id, event);
                }),
            ));

            // 凭据（T2.7）：密文进系统凭据库，索引落在数据目录。
            // askpass 程序是**应用自身**（`current_exe`）；拿不到自身路径时
            // `with_app_askpass` 返回 None，网络操作退化为匿名/SSH（如实降级）。
            let credentials = Arc::new(CredentialsService::keyring(
                data_dir.join(CREDENTIALS_INDEX_FILE),
            ));
            // 上次选的是加密文件、且保险库还在 → 启动即进入"待解锁"。
            // 不这样做会拿一个空 keyring 冒充"没有凭据"，用户会以为保存过的令牌丢了。
            if forgedesk_commands::preferred_backend(&database)
                .as_deref()
                == Some(forgedesk_commands::BACKEND_ENCRYPTED_VAULT)
                && credentials.vault_exists()
            {
                credentials.lock_vault();
                info!("凭据使用加密保险库，等待用户解锁");
            }
            let credential_gate =
                CredentialGate::with_app_askpass(credentials.shared()).map(Arc::new);

            // 账号服务（T4.3/T4.4）与远端仓库服务（T4.5）：令牌密文进凭据库
            // （与上面共享同一存储实例），账号元数据进 accounts 表。
            // 两者共享同一个 HTTP 底座（同一条限流快照与连接池）；
            // HTTP 底座目前跟随系统代理，M6 的代理设置落地后改为从设置读取。
            let provider_http = GitHubHttp::new(HttpConfig::default())?;
            let accounts = Arc::new(AccountService::new(
                Arc::clone(&database),
                credentials.shared(),
                provider_http.clone(),
            ));
            let host_repos = Arc::new(HostRepoService::new(
                Arc::clone(&database),
                credentials.shared(),
                provider_http,
            ));

            // T3.8：快照的未跟踪内容备份落在**应用缓存目录**（PLAN §5.10 的分层：
            // 大文件备份放缓存，不污染用户仓库）。缓存被清掉时快照本身仍然可用
            // （HEAD 与索引的恢复不依赖它），只有未跟踪内容回不来——
            // 这一点由 `snapshot_usage` 与回滚报告如实说明，不会静默。
            let snapshot_backup_root = app.path().app_cache_dir()?.join("snapshots");

            // 插件宿主（T6.4）：组合根服务（真实 HostServices）→ 引擎 → 管理器。
            // 依赖方向：引擎在插件执行线程里回调服务；服务只拿 Arc 克隆。
            let open_repos = Arc::new(OpenRepoRegistry::new());
            let (plugin_manager, plugin_services) = forgedesk_commands::plugins::build_plugin_host(
                Arc::clone(&database),
                Arc::clone(&engines),
                Arc::clone(&open_repos),
                app.path().app_data_dir()?.join("plugins"),
                app.handle().clone(),
            )?;

            app.manage(AppState {
                database: Arc::clone(&database),
                log_dir,
                engines: Arc::clone(&engines),
                jobs: Arc::new(JobRunner::new()),
                open_repos,
                // T1.9：真实的 ref 锚点快照。提交链路"执行前打点"的位置在 T1.7
                // 就已接好，这里只是把"如实回答没有快照"的占位换成实现。
                snapshots: Arc::new(
                    RefSnapshotManager::new(Arc::clone(&engines), Arc::clone(&database))
                        .with_backup_root(snapshot_backup_root),
                ),
                commit_plans: Arc::new(CommitPlanRegistry::new()),
                reset_plans: Arc::new(ResetPlanRegistry::new()),
                merge_plans: Arc::new(MergePlanRegistry::new()),
                log_pages: Arc::new(LogPageCache::new()),
                credentials,
                credential_gate,
                accounts,
                host_repos,
                watchers,
                plugins: Arc::new(plugin_manager),
                plugin_services,
            });

            // 审计的保留策略在**启动时**执行一次（T1.11）：查历史不该顺带删记录，
            // 而第一次打开设置页也不该等一次全表删除。失败只记日志。
            forgedesk_commands::audit::prune_on_startup(&app.state::<AppState>());

            app.manage(RuntimeHandles {
                _log_guard: guard,
                session: Mutex::new(Some(session)),
            });

            Ok(())
        });

    // 命令注册按构建类型分流：演示命令只在开发构建里存在。
    // 这样"用于验证错误链路的入口"不会随正式产物分发给用户——
    // 一个能让应用主动报错的命令没有任何理由存在于发布版里。
    #[cfg(debug_assertions)]
    let builder = builder.invoke_handler(tauri::generate_handler![
        forgedesk_commands::app_version,
        forgedesk_commands::settings_get,
        forgedesk_commands::settings_set,
        forgedesk_commands::settings_all,
        forgedesk_commands::logs_open,
        forgedesk_commands::workspace_status,
        forgedesk_commands::workspace_diff,
        forgedesk_commands::workspace_diff_patch,
        forgedesk_commands::workspace_reveal,
        forgedesk_commands::workspace_stage,
        forgedesk_commands::workspace_unstage,
        forgedesk_commands::workspace_discard,
        forgedesk_commands::plugin_list,
        forgedesk_commands::plugin_install_from_dir,
        forgedesk_commands::plugin_set_enabled,
        forgedesk_commands::plugin_grant,
        forgedesk_commands::plugin_revoke,
        forgedesk_commands::plugin_uninstall,
        forgedesk_commands::plugin_reload,
        forgedesk_commands::plugin_logs,
        forgedesk_commands::plugin_render_panel,
        forgedesk_commands::plugin_invoke_command,
        forgedesk_commands::plugin_registrations,
        forgedesk_commands::commit_prepare,
        forgedesk_commands::commit_execute,
        forgedesk_commands::commit_message_hint,
        forgedesk_commands::commit_amend_context,
        forgedesk_commands::commit_hooks_list,
        forgedesk_commands::snapshot_list,
        forgedesk_commands::snapshot_diff,
        forgedesk_commands::snapshot_usage,
        forgedesk_commands::snapshot_estimate,
        forgedesk_commands::snapshot_create,
        forgedesk_commands::snapshot_restore,
        forgedesk_commands::snapshot_restore_pending,
        forgedesk_commands::snapshot_restore_abandon,
        forgedesk_commands::snapshot_prune,
        forgedesk_commands::snapshot_cleanup,
        forgedesk_commands::audit_list,
        forgedesk_commands::operation_history,
        forgedesk_commands::audit_export,
        forgedesk_commands::audit_prune,
        forgedesk_commands::log_frontend_error,
        forgedesk_commands::logs_tail,
        forgedesk_commands::repo_discover,
        forgedesk_commands::repo_open,
        forgedesk_commands::repo_clone,
        forgedesk_commands::repo_init,
        forgedesk_commands::repo_recent_list,
        forgedesk_commands::repo_forget,
        forgedesk_commands::repo_close,
        forgedesk_commands::job_cancel,
        forgedesk_commands::git_log_page,
        forgedesk_commands::git_log_authors,
        forgedesk_commands::git_branch_list,
        forgedesk_commands::git_fetch,
        forgedesk_commands::git_pull,
        forgedesk_commands::git_push,
        forgedesk_commands::git_remote_list,
        forgedesk_commands::git_remote_add,
        forgedesk_commands::git_remote_remove,
        forgedesk_commands::git_remote_rename,
        forgedesk_commands::git_remote_set_url,
        forgedesk_commands::git_tag_list,
        forgedesk_commands::git_branch_compare,
        forgedesk_commands::git_branch_create,
        forgedesk_commands::git_branch_switch,
        forgedesk_commands::git_branch_rename,
        forgedesk_commands::git_branch_delete,
        forgedesk_commands::git_branch_set_upstream,
        forgedesk_commands::git_tag_create,
        forgedesk_commands::git_tag_delete,
        forgedesk_commands::git_stash_save,
        forgedesk_commands::git_stash_list,
        forgedesk_commands::git_stash_show,
        forgedesk_commands::git_stash_apply,
        forgedesk_commands::git_stash_pop,
        forgedesk_commands::git_stash_drop,
        forgedesk_commands::git_stash_clear,
        forgedesk_commands::git_stash_branch,
        forgedesk_commands::git_cherry_pick,
        forgedesk_commands::git_revert,
        forgedesk_commands::git_merge_prepare,
        forgedesk_commands::git_merge_execute,
        forgedesk_commands::git_merge_continue,
        forgedesk_commands::git_conflict_state,
        forgedesk_commands::git_conflict_file_detail,
        forgedesk_commands::git_conflict_mark_resolved,
        forgedesk_commands::git_conflict_apply_resolution,
        forgedesk_commands::git_conflict_take_side,
        forgedesk_commands::git_conflict_remove_file,
        forgedesk_commands::git_conflict_continue,
        forgedesk_commands::git_conflict_abort,
        forgedesk_commands::git_conflict_skip,
        forgedesk_commands::git_reset_prepare,
        forgedesk_commands::git_reset_execute,
        forgedesk_commands::git_reflog,
        forgedesk_commands::git_reflog_create_branch,
        forgedesk_commands::git_rebase_preview_only,
        forgedesk_commands::git_rebase_execute,
        forgedesk_commands::git_rebase_continue_edit,
        forgedesk_commands::git_rebase_range,
        forgedesk_commands::credentials_list,
        forgedesk_commands::credentials_save,
        forgedesk_commands::credentials_delete,
        forgedesk_commands::credentials_status,
        forgedesk_commands::credential_test_remote,
        forgedesk_commands::credentials_ssh_inventory,
        forgedesk_commands::credentials_vault_create,
        forgedesk_commands::credentials_vault_unlock,
        forgedesk_commands::account_login_with_pat,
        forgedesk_commands::account_device_flow_start,
        forgedesk_commands::account_device_flow_wait,
        forgedesk_commands::account_list,
        forgedesk_commands::account_remove,
        forgedesk_commands::repo_remote_list,
        forgedesk_commands::repo_remote_starred,
        forgedesk_commands::repo_remote_search,
        forgedesk_commands::repo_remote_star,
        forgedesk_commands::repo_remote_fork,
        forgedesk_commands::repo_remote_readme,
        forgedesk_commands::repo_pull_list,
        forgedesk_commands::repo_pull_get,
        forgedesk_commands::repo_pull_reviews,
        forgedesk_commands::repo_pull_merge,
        forgedesk_commands::repo_pull_comments_list,
        forgedesk_commands::repo_pull_comment_create,
        forgedesk_commands::repo_pull_review_submit,
        forgedesk_commands::repo_pull_files,
        forgedesk_commands::repo_pull_review_comments_list,
        forgedesk_commands::repo_pull_review_comment_create,
        forgedesk_commands::repo_pull_review_comment_reply,
        forgedesk_commands::repo_account_binding_get,
        forgedesk_commands::repo_account_binding_set,
        forgedesk_commands::repo_issue_list,
        forgedesk_commands::repo_issue_get,
        forgedesk_commands::repo_issue_body,
        forgedesk_commands::repo_issue_create,
        forgedesk_commands::repo_issue_edit,
        forgedesk_commands::repo_issue_state_set,
        forgedesk_commands::repo_issue_assignees_set,
        forgedesk_commands::repo_issue_comments_list,
        forgedesk_commands::repo_issue_comment_create,
        forgedesk_commands::repo_issue_assignees,
        forgedesk_commands::repo_actions_runs_list,
        forgedesk_commands::repo_actions_run_jobs,
        forgedesk_commands::repo_actions_run_cancel,
        forgedesk_commands::repo_actions_run_rerun,
        forgedesk_commands::repo_actions_job_logs,
        forgedesk_commands::repo_rate_limit_state,
        forgedesk_commands::repo_rate_limit_refresh,
        forgedesk_commands::repo_dashboard,
        forgedesk_commands::debug_throw_error,
        forgedesk_commands::debug_panic,
    ]);

    #[cfg(not(debug_assertions))]
    let builder = builder.invoke_handler(tauri::generate_handler![
        forgedesk_commands::app_version,
        forgedesk_commands::settings_get,
        forgedesk_commands::settings_set,
        forgedesk_commands::settings_all,
        forgedesk_commands::logs_open,
        forgedesk_commands::workspace_status,
        forgedesk_commands::workspace_reveal,
        forgedesk_commands::workspace_stage,
        forgedesk_commands::workspace_unstage,
        forgedesk_commands::workspace_discard,
        forgedesk_commands::plugin_list,
        forgedesk_commands::plugin_install_from_dir,
        forgedesk_commands::plugin_set_enabled,
        forgedesk_commands::plugin_grant,
        forgedesk_commands::plugin_revoke,
        forgedesk_commands::plugin_uninstall,
        forgedesk_commands::plugin_reload,
        forgedesk_commands::plugin_logs,
        forgedesk_commands::plugin_render_panel,
        forgedesk_commands::plugin_invoke_command,
        forgedesk_commands::plugin_registrations,
        forgedesk_commands::commit_prepare,
        forgedesk_commands::commit_execute,
        forgedesk_commands::commit_message_hint,
        forgedesk_commands::commit_amend_context,
        forgedesk_commands::commit_hooks_list,
        forgedesk_commands::snapshot_list,
        forgedesk_commands::snapshot_diff,
        forgedesk_commands::snapshot_usage,
        forgedesk_commands::snapshot_estimate,
        forgedesk_commands::snapshot_create,
        forgedesk_commands::snapshot_restore,
        forgedesk_commands::snapshot_restore_pending,
        forgedesk_commands::snapshot_restore_abandon,
        forgedesk_commands::snapshot_prune,
        forgedesk_commands::snapshot_cleanup,
        forgedesk_commands::audit_list,
        forgedesk_commands::operation_history,
        forgedesk_commands::audit_export,
        forgedesk_commands::audit_prune,
        forgedesk_commands::log_frontend_error,
        forgedesk_commands::logs_tail,
        forgedesk_commands::repo_discover,
        forgedesk_commands::repo_open,
        forgedesk_commands::repo_clone,
        forgedesk_commands::repo_init,
        forgedesk_commands::repo_recent_list,
        forgedesk_commands::repo_forget,
        forgedesk_commands::repo_close,
        forgedesk_commands::job_cancel,
        forgedesk_commands::git_log_page,
        forgedesk_commands::git_log_authors,
        forgedesk_commands::git_branch_list,
        forgedesk_commands::git_fetch,
        forgedesk_commands::git_pull,
        forgedesk_commands::git_push,
        forgedesk_commands::git_remote_list,
        forgedesk_commands::git_remote_add,
        forgedesk_commands::git_remote_remove,
        forgedesk_commands::git_remote_rename,
        forgedesk_commands::git_remote_set_url,
        forgedesk_commands::git_tag_list,
        forgedesk_commands::git_branch_compare,
        forgedesk_commands::git_branch_create,
        forgedesk_commands::git_branch_switch,
        forgedesk_commands::git_branch_rename,
        forgedesk_commands::git_branch_delete,
        forgedesk_commands::git_branch_set_upstream,
        forgedesk_commands::git_tag_create,
        forgedesk_commands::git_tag_delete,
        forgedesk_commands::git_stash_save,
        forgedesk_commands::git_stash_list,
        forgedesk_commands::git_stash_show,
        forgedesk_commands::git_stash_apply,
        forgedesk_commands::git_stash_pop,
        forgedesk_commands::git_stash_drop,
        forgedesk_commands::git_stash_clear,
        forgedesk_commands::git_stash_branch,
        forgedesk_commands::git_cherry_pick,
        forgedesk_commands::git_revert,
        forgedesk_commands::git_merge_prepare,
        forgedesk_commands::git_merge_execute,
        forgedesk_commands::git_merge_continue,
        forgedesk_commands::git_conflict_state,
        forgedesk_commands::git_conflict_file_detail,
        forgedesk_commands::git_conflict_mark_resolved,
        forgedesk_commands::git_conflict_apply_resolution,
        forgedesk_commands::git_conflict_take_side,
        forgedesk_commands::git_conflict_remove_file,
        forgedesk_commands::git_conflict_continue,
        forgedesk_commands::git_conflict_abort,
        forgedesk_commands::git_conflict_skip,
        forgedesk_commands::git_reset_prepare,
        forgedesk_commands::git_reset_execute,
        forgedesk_commands::git_reflog,
        forgedesk_commands::git_reflog_create_branch,
        forgedesk_commands::git_commit_detail,
        forgedesk_commands::git_rebase_preview_only,
        forgedesk_commands::git_rebase_execute,
        forgedesk_commands::git_rebase_continue_edit,
        forgedesk_commands::git_rebase_range,
        forgedesk_commands::credentials_list,
        forgedesk_commands::credentials_save,
        forgedesk_commands::credentials_delete,
        forgedesk_commands::credentials_status,
        forgedesk_commands::credential_test_remote,
        forgedesk_commands::credentials_ssh_inventory,
        forgedesk_commands::credentials_vault_create,
        forgedesk_commands::credentials_vault_unlock,
        forgedesk_commands::account_login_with_pat,
        forgedesk_commands::account_device_flow_start,
        forgedesk_commands::account_device_flow_wait,
        forgedesk_commands::account_list,
        forgedesk_commands::account_remove,
        forgedesk_commands::repo_remote_list,
        forgedesk_commands::repo_remote_starred,
        forgedesk_commands::repo_remote_search,
        forgedesk_commands::repo_remote_star,
        forgedesk_commands::repo_remote_fork,
        forgedesk_commands::repo_remote_readme,
        forgedesk_commands::repo_pull_list,
        forgedesk_commands::repo_pull_get,
        forgedesk_commands::repo_pull_reviews,
        forgedesk_commands::repo_pull_merge,
        forgedesk_commands::repo_pull_comments_list,
        forgedesk_commands::repo_pull_comment_create,
        forgedesk_commands::repo_pull_review_submit,
        forgedesk_commands::repo_pull_files,
        forgedesk_commands::repo_pull_review_comments_list,
        forgedesk_commands::repo_pull_review_comment_create,
        forgedesk_commands::repo_pull_review_comment_reply,
        forgedesk_commands::repo_account_binding_get,
        forgedesk_commands::repo_account_binding_set,
        forgedesk_commands::repo_issue_list,
        forgedesk_commands::repo_issue_get,
        forgedesk_commands::repo_issue_body,
        forgedesk_commands::repo_issue_create,
        forgedesk_commands::repo_issue_edit,
        forgedesk_commands::repo_issue_state_set,
        forgedesk_commands::repo_issue_assignees_set,
        forgedesk_commands::repo_issue_comments_list,
        forgedesk_commands::repo_issue_comment_create,
        forgedesk_commands::repo_issue_assignees,
        forgedesk_commands::repo_actions_runs_list,
        forgedesk_commands::repo_actions_run_jobs,
        forgedesk_commands::repo_actions_run_cancel,
        forgedesk_commands::repo_actions_run_rerun,
        forgedesk_commands::repo_actions_job_logs,
        forgedesk_commands::repo_rate_limit_state,
        forgedesk_commands::repo_rate_limit_refresh,
        forgedesk_commands::repo_dashboard,
    ]);

    let app = builder.build(tauri::generate_context!())?;

    app.run(|handle, event| {
        if let RunEvent::Exit = event {
            // 退出前先把长任务停掉：克隆可能正跑着，直接退出会留下半个仓库，
            // 而"半个仓库"比"没有仓库"更难解释（用户下次打开会看到目录非空）。
            if let Some(state) = handle.try_state::<AppState>() {
                state.jobs.registry().cancel_all();
                state.open_repos.close_all();
                // 释放监听：句柄持有操作系统级的目录监听与一条线程。
                // 进程马上就要退出，但显式停掉能让"立刻重启应用"这条路干净，
                // 也不给"退出时还有线程在跑"留下解释不清的日志。
                state.watchers.stop_all();
            }

            // 正常退出：删除会话标记。留在这里而不是 Drop 里，是因为
            // "正常退出"必须在代码里可见——否则将来有人加了 `std::process::exit`
            // 或提前返回，会话标记会一直残留，用户每次启动都会看到"上次异常退出"。
            if let Some(handles) = handle.try_state::<RuntimeHandles>() {
                if let Ok(mut guard) = handles.session.lock() {
                    if let Some(marker) = guard.take() {
                        if let Err(error) = marker.finish() {
                            warn!(%error, "删除会话标记失败");
                        }
                    }
                }
            }
        }
    });

    Ok(())
}

/// 初始化日志：文件（JSON Lines）+ 控制台（仅调试构建，人类可读）。
///
/// 三点约定：
///
/// 1. **文件始终写**（不只是 release）。T0.8 的 `logs_tail` / `logs_open` 需要真实文件，
///    如果只有 release 才落盘，这两个功能在开发期根本无法验证——而"只在发布版才跑"
///    的代码路径正是最容易坏的那一类。
/// 2. 控制台只在调试构建输出：release 下 Windows 子系统没有控制台，
///    在 macOS/Linux 上从终端启动时多出来的输出对普通用户只是噪音。
/// 3. **所有输出都经过脱敏**（`SanitizingMakeWriter`，红线 R8）。
///    脱敏在写入层完成，因此控制台的自由格式与文件的 JSON 格式共用同一份规则；
///    文件用 JSON 是为了让 `logs_tail` 能解析时间戳与级别并按时间高亮。
fn init_logging(log_dir: &Path) -> Result<LogFlushGuard, Box<dyn std::error::Error>> {
    let (writer, guard, _directory) = non_blocking_writer(log_dir, LogPolicy::default())?;

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let file_layer = tracing_subscriber::fmt::layer()
        .json()
        // 非阻塞写入器 → 按行脱敏 → 文件；顺序不能颠倒（脱敏必须在最靠近落盘的位置）
        .with_writer(SanitizingMakeWriter::new(writer))
        .with_ansi(false)
        .with_target(true);

    #[cfg(debug_assertions)]
    let console_layer = Some(
        tracing_subscriber::fmt::layer()
            .with_writer(SanitizingMakeWriter::new(std::io::stderr))
            .with_target(true),
    );
    #[cfg(not(debug_assertions))]
    let console_layer: Option<tracing_subscriber::fmt::Layer<_>> = None;

    tracing_subscriber::registry()
        .with(filter)
        .with(file_layer)
        .with(console_layer)
        .init();

    Ok(guard)
}
