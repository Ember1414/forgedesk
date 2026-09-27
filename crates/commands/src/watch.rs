//! 仓库文件监听的注册表与设置（M1 / T1.10）。
//!
//! # 职责边界
//!
//! - 过滤、去抖动、溢出保护在 `forgedesk_platform::watcher` 里完成——那是"事件
//!   该不该上报"的判断；
//! - **本模块决定"谁在监听、监听什么、事件往哪送"**：一个仓库一份句柄，
//!   仓库关闭时句柄被丢弃，事件经统一的 sink 变成 `repo:changed`。
//!
//! # 为什么注册表在命令层而不是 platform
//!
//! 事件要发给前端（需要 `AppHandle`），而 `platform` 刻意不依赖 Tauri。
//! 命令层把"怎么发"注入进来，platform 只负责"发生了什么事"——这样监听逻辑
//! 可以在纯 Rust 测试里跑（注入一个假 watcher 与假 sink 就够了）。
//!
//! # 失败的姿态
//!
//! 监听失败**永远不该让上层操作失败**：仓库照样能打开，只是少了自动刷新
//! （用户仍可手动刷新）。因此这里对外暴露的错误只用于记日志，调用方
//! （`repository` 命令族）不会把它变成用户可见的失败。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use forgedesk_domain::{AppError, AppResult, ErrorCode};
use forgedesk_platform::watcher::{
    FileWatcher, WatchEvent, WatchOptions, WatcherHandle, DEFAULT_DEBOUNCE_MS,
};
use forgedesk_storage::{Database, Scope, SettingsRepository};

/// 设置键：自动刷新开关（`true` / `false`）。
///
/// 关掉之后不启动任何监听，前端改回轮询兜底（见 `WorkspaceStatusPage`）。
/// 需要这个开关的现实理由：网络盘 / 超大仓库上，某些用户宁愿自己按刷新。
pub const AUTO_REFRESH_KEY: &str = "watch.autoRefresh";

/// 设置键：去抖动窗口（毫秒）。
pub const DEBOUNCE_KEY: &str = "watch.debounceMs";

/// 事件接收端：由命令层注入（生产环境把它变成 Tauri 事件）。
pub type WatchSink = Arc<dyn Fn(i64, WatchEvent) + Send + Sync>;

/// 一份正在进行的监听。
#[derive(Debug)]
struct ActiveWatch {
    root: PathBuf,
    options: WatchOptions,
    /// 句柄被丢弃时监听停止（见 `WatcherHandle::drop`）。
    _handle: WatcherHandle,
}

/// 仓库 id → 监听句柄。
pub struct WatcherRegistry {
    watcher: Arc<dyn FileWatcher>,
    sink: WatchSink,
    active: Mutex<BTreeMap<i64, ActiveWatch>>,
}

impl std::fmt::Debug for WatcherRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `sink` 是 `dyn Fn`（没有 Debug）：手写实现，只把"监听实现 + 正在监听谁"
        // 打出来。`AppState` 派生 Debug，因此这里不能省。
        formatter
            .debug_struct("WatcherRegistry")
            .field("watcher", &self.watcher)
            .field("active", &self.active)
            .finish()
    }
}

impl WatcherRegistry {
    /// 绑定一个监听实现与一个事件接收端。
    pub fn new(watcher: Arc<dyn FileWatcher>, sink: WatchSink) -> Self {
        Self {
            watcher,
            sink,
            active: Mutex::new(BTreeMap::new()),
        }
    }

    /// 锁的恢复策略：中毒说明某个线程在持锁时 panic 过。注册表只是
    /// "id → 句柄"的映射，不存在需要整体放弃的半更新状态，
    /// 因此恢复使用而不是让所有监听永久瘫痪。
    fn lock(&self) -> MutexGuard<'_, BTreeMap<i64, ActiveWatch>> {
        self.active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 开始监听一个仓库（重复调用等于用新选项重启）。
    pub fn start(&self, repo_id: i64, root: &Path, options: WatchOptions) -> AppResult<()> {
        let sink = Arc::clone(&self.sink);
        let handle = self
            .watcher
            .watch(
                root,
                options.clone(),
                Arc::new(move |event| sink(repo_id, event)),
            )
            .map_err(|error| {
                AppError::new(ErrorCode::Internal, error.to_string())
                    .with_detail(format!("repo_id: {repo_id}"))
            })?;

        // 旧句柄在**锁外**丢弃：丢弃会 join 监听线程，持锁 join 会把
        // 一个仓库的重启变成所有仓库的等待。
        let previous = {
            let mut active = self.lock();
            active.insert(
                repo_id,
                ActiveWatch {
                    root: root.to_path_buf(),
                    options,
                    _handle: handle,
                },
            )
        };
        drop(previous);
        Ok(())
    }

    /// 停止监听；返回它此前是否在监听。
    pub fn stop(&self, repo_id: i64) -> bool {
        let taken = {
            let mut active = self.lock();
            active.remove(&repo_id)
        };
        taken.is_some()
    }

    /// 停止全部监听（退出应用、或用户关掉自动刷新）。
    pub fn stop_all(&self) {
        let taken = {
            let mut active = self.lock();
            std::mem::take(&mut *active)
        };
        drop(taken);
    }

    /// 是否正在监听某个仓库。
    pub fn is_watching(&self, repo_id: i64) -> bool {
        self.lock().contains_key(&repo_id)
    }

    /// 正在监听的仓库 id（升序）。
    pub fn active_ids(&self) -> Vec<i64> {
        self.lock().keys().copied().collect()
    }

    /// 用新的选项重启全部监听（设置变更后调用）。
    ///
    /// 为什么是"重启"而不是"改选项"：句柄与选项在创建时就交给了底层实现
    /// （notify 的配置不可热改），重启是唯一能保证新窗口真正生效的做法。
    pub fn restart_all(&self, options: &WatchOptions) -> AppResult<()> {
        let entries: Vec<(i64, PathBuf)> = self
            .lock()
            .iter()
            .map(|(repo_id, watch)| (*repo_id, watch.root.clone()))
            .collect();

        for (repo_id, root) in entries {
            self.start(repo_id, &root, options.clone())?;
        }
        Ok(())
    }

    /// 某个仓库当前的选项（测试与诊断用）。
    pub fn options_of(&self, repo_id: i64) -> Option<WatchOptions> {
        self.lock().get(&repo_id).map(|watch| watch.options.clone())
    }
}

/// 监听相关的设置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WatchSettings {
    /// 是否启用自动刷新（关掉后不启动监听）。
    pub auto_refresh: bool,
    /// 去抖动窗口（毫秒）。
    pub debounce_ms: u64,
}

impl Default for WatchSettings {
    fn default() -> Self {
        Self {
            auto_refresh: true,
            debounce_ms: DEFAULT_DEBOUNCE_MS,
        }
    }
}

impl WatchSettings {
    /// 从设置表读取；缺失或损坏时回落默认值。
    ///
    /// 为什么容错而不是报错：这两个值来自设置页，读取发生在**打开仓库**的路径上。
    /// 一个被手工改坏的设置不该让用户打不开仓库。
    pub fn load(database: &Database) -> Self {
        let repository = SettingsRepository::new(database);
        let defaults = Self::default();

        let auto_refresh =
            read_json::<bool>(&repository, AUTO_REFRESH_KEY).unwrap_or(defaults.auto_refresh);
        let debounce_ms =
            read_json::<u64>(&repository, DEBOUNCE_KEY).unwrap_or(defaults.debounce_ms);

        Self {
            auto_refresh,
            debounce_ms,
        }
    }

    /// 对应的监听选项（去抖动值会被收敛到合法范围）。
    pub fn options(&self) -> WatchOptions {
        WatchOptions::with_debounce_ms(self.debounce_ms)
    }

    /// 是否是监听相关的设置键（决定要不要重启监听）。
    pub fn is_watch_key(key: &str) -> bool {
        key == AUTO_REFRESH_KEY || key == DEBOUNCE_KEY
    }
}

/// 把一次监听事件广播给前端。
///
/// 放在命令层而不是宿主：事件名与载荷形状是 IPC 契约的一部分，必须与
/// [`crate::workspace::emit_changed`] 用**同一份定义**——两处各拼一次早晚漂移，
/// 而前端只会按其中一种处理。
pub fn emit_watch_event(app: &tauri::AppHandle, repo_id: i64, event: WatchEvent) {
    crate::workspace::emit_changed(
        app,
        repo_id,
        event.kind,
        event
            .paths
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect(),
    );
}

/// 读取一个 JSON 编码的设置值。
fn read_json<T: serde::de::DeserializeOwned>(
    repository: &SettingsRepository<'_>,
    key: &str,
) -> Option<T> {
    let raw = repository.get(&Scope::Global, key).ok().flatten()?;
    serde_json::from_str::<T>(&raw).ok()
}

/// 给一个已打开的仓库启动监听（关掉开关时什么都不做）。
///
/// 失败只记日志：**监听不上不该让"仓库打开"失败**——用户仍然可以手动刷新，
/// 而弹一句错误会把一次成功的打开变成一次失败的操作。
pub fn start_for_repo(watchers: &WatcherRegistry, database: &Database, repo_id: i64, root: &Path) {
    let settings = WatchSettings::load(database);
    if !settings.auto_refresh {
        return;
    }
    if let Err(error) = watchers.start(repo_id, root, settings.options()) {
        tracing::warn!(
            repo_id = repo_id,
            root = %root.display(),
            error = %error.message,
            "启动文件监听失败：仓库仍可手动刷新"
        );
    }
}

/// 设置变更后重新应用：关掉则全部停止，打开则重启已有的并为其余已打开仓库补上。
///
/// "补上"这一步是必要的：用户可能先关掉自动刷新打开了几个仓库，
/// 再打开开关——那批仓库此时没有任何监听。
pub fn apply_settings(
    watchers: &WatcherRegistry,
    database: &Database,
    open_repo_ids: &[i64],
) -> AppResult<()> {
    let settings = WatchSettings::load(database);
    if !settings.auto_refresh {
        watchers.stop_all();
        return Ok(());
    }

    watchers.restart_all(&settings.options())?;

    for repo_id in open_repo_ids {
        if watchers.is_watching(*repo_id) {
            continue;
        }
        let Some(record) =
            forgedesk_storage::RepositoryStore::new(database).find_by_id(*repo_id)?
        else {
            continue;
        };
        start_for_repo(watchers, database, *repo_id, Path::new(&record.path));
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    use forgedesk_platform::watcher::{
        FileWatcher, WatchCallback, WatchError, WatchEvent, WatchOptions,
    };

    use super::{WatchSettings, WatcherRegistry};

    /// 注入用的 trait 对象别名（`Arc<dyn _>` 不是合法写法）。
    type DynWatcher = Arc<dyn FileWatcher>;

    /// 记录被 watch 的目录，并允许测试按目录手动触发事件。
    #[derive(Default)]
    struct RecordingWatcher {
        watched: Mutex<Vec<std::path::PathBuf>>,
        /// 每个目录只保留最新一次的回调：重启时旧句柄会被丢弃，
        /// 用"同名覆盖"模拟"旧监听不再上报"。
        callbacks: Mutex<std::collections::BTreeMap<std::path::PathBuf, WatchCallback>>,
    }

    impl std::fmt::Debug for RecordingWatcher {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            // 回调不是 Debug：只报"监听了哪些目录"，测试失败时够用
            formatter
                .debug_struct("RecordingWatcher")
                .field("watched", &self.watched())
                .finish()
        }
    }

    impl RecordingWatcher {
        fn watched(&self) -> Vec<std::path::PathBuf> {
            self.watched.lock().unwrap().clone()
        }

        /// 模拟一次来自指定目录的事件。
        fn emit(&self, root: &Path, event: WatchEvent) {
            let callbacks = self.callbacks.lock().unwrap();
            if let Some(callback) = callbacks.get(root) {
                callback(event);
            }
        }
    }

    impl forgedesk_platform::watcher::FileWatcher for RecordingWatcher {
        fn watch(
            &self,
            root: &Path,
            _options: WatchOptions,
            on_event: WatchCallback,
        ) -> Result<forgedesk_platform::watcher::WatcherHandle, WatchError> {
            self.watched.lock().unwrap().push(root.to_path_buf());
            self.callbacks
                .lock()
                .unwrap()
                .insert(root.to_path_buf(), on_event);
            // 句柄本身不监听任何东西：测试只关心注册表的生命周期
            Ok(forgedesk_platform::watcher::WatcherHandle::detached())
        }
    }

    #[test]
    fn starting_twice_keeps_one_watch_and_the_latest_options() {
        let watcher = Arc::new(RecordingWatcher::default());
        let registry =
            WatcherRegistry::new(Arc::clone(&watcher) as DynWatcher, Arc::new(|_, _| {}));

        registry
            .start(
                1,
                Path::new("/repo/one"),
                WatchOptions::with_debounce_ms(100),
            )
            .unwrap();
        registry
            .start(
                1,
                Path::new("/repo/one"),
                WatchOptions::with_debounce_ms(500),
            )
            .unwrap();

        assert!(registry.is_watching(1));
        assert_eq!(registry.active_ids(), vec![1]);
        assert_eq!(
            registry
                .options_of(1)
                .map(|options| options.debounce.as_millis()),
            Some(500),
            "重启必须带上新的选项"
        );
    }

    #[test]
    fn stopping_removes_the_watch_and_is_idempotent() {
        let registry = WatcherRegistry::new(
            Arc::new(RecordingWatcher::default()) as DynWatcher,
            Arc::new(|_, _| {}),
        );

        registry
            .start(7, Path::new("/repo/seven"), WatchOptions::default())
            .unwrap();
        assert!(registry.stop(7));
        assert!(!registry.stop(7), "第二次停止应当返回 false");
        assert!(registry.active_ids().is_empty());
    }

    #[test]
    fn events_carry_the_repository_id_they_came_from() {
        let watcher = Arc::new(RecordingWatcher::default());
        let received: Arc<Mutex<Vec<(i64, WatchEvent)>>> = Arc::new(Mutex::new(Vec::new()));
        let sink_received = Arc::clone(&received);
        let registry = WatcherRegistry::new(
            Arc::clone(&watcher) as DynWatcher,
            Arc::new(move |repo_id, event| {
                sink_received.lock().unwrap().push((repo_id, event));
            }),
        );

        registry
            .start(42, Path::new("/repo/answer"), WatchOptions::default())
            .unwrap();
        watcher.emit(
            Path::new("/repo/answer"),
            WatchEvent {
                kind: forgedesk_platform::watcher::WatchKind::Workspace,
                paths: vec![std::path::PathBuf::from("/repo/answer/src/a.ts")],
            },
        );

        let received = received.lock().unwrap();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].0, 42, "多仓库同时打开时不能张冠李戴");
        assert_eq!(received[0].1.paths.len(), 1);
    }

    #[test]
    fn restarting_every_watch_keeps_all_repositories_alive() {
        let registry = WatcherRegistry::new(
            Arc::new(RecordingWatcher::default()) as DynWatcher,
            Arc::new(|_, _| {}),
        );

        registry
            .start(
                1,
                Path::new("/repo/one"),
                WatchOptions::with_debounce_ms(100),
            )
            .unwrap();
        registry
            .start(
                2,
                Path::new("/repo/two"),
                WatchOptions::with_debounce_ms(100),
            )
            .unwrap();

        registry
            .restart_all(&WatchOptions::with_debounce_ms(1000))
            .unwrap();

        assert_eq!(registry.active_ids(), vec![1, 2]);
        assert_eq!(
            registry
                .options_of(2)
                .map(|options| options.debounce.as_millis()),
            Some(1000)
        );
    }

    #[test]
    fn watch_settings_fall_back_to_defaults_when_storage_is_empty_or_broken() {
        let database = forgedesk_storage::Database::open_in_memory().unwrap();
        forgedesk_storage::migrate(&database).unwrap();
        let repository = forgedesk_storage::SettingsRepository::new(&database);

        // 没写过任何设置
        assert_eq!(WatchSettings::load(&database), WatchSettings::default());

        // 手工改坏的值不该让解析失败（回落默认而不是 panic）
        repository
            .set(
                &forgedesk_storage::Scope::Global,
                super::DEBOUNCE_KEY,
                "\"soon\"",
            )
            .unwrap();
        assert_eq!(
            WatchSettings::load(&database).debounce_ms,
            WatchSettings::default().debounce_ms
        );

        // 合法值生效
        repository
            .set(
                &forgedesk_storage::Scope::Global,
                super::AUTO_REFRESH_KEY,
                "false",
            )
            .unwrap();
        repository
            .set(
                &forgedesk_storage::Scope::Global,
                super::DEBOUNCE_KEY,
                "1200",
            )
            .unwrap();
        let loaded = WatchSettings::load(&database);
        assert!(!loaded.auto_refresh);
        assert_eq!(loaded.debounce_ms, 1200);
        assert_eq!(loaded.options().debounce.as_millis(), 1200);
    }

    #[test]
    fn only_watch_keys_trigger_a_restart() {
        assert!(WatchSettings::is_watch_key(super::AUTO_REFRESH_KEY));
        assert!(WatchSettings::is_watch_key(super::DEBOUNCE_KEY));
        assert!(!WatchSettings::is_watch_key("ui.density"));
    }

    /// 建一个带一条仓库记录的库（`apply_settings` 要按 id 查路径）。
    fn database_with_repo(id: i64, path: &str) -> forgedesk_storage::Database {
        let database = forgedesk_storage::Database::open_in_memory().unwrap();
        forgedesk_storage::migrate(&database).unwrap();
        forgedesk_storage::RepositoryStore::new(&database)
            .upsert(
                &forgedesk_storage::RepositoryUpsert {
                    path: path.to_owned(),
                    name: "fixture".to_owned(),
                    default_branch: Some("main".to_owned()),
                    provider_id: None,
                    size_class: None,
                },
                1_000,
            )
            .unwrap();
        let _ = id;
        database
    }

    #[test]
    fn turning_auto_refresh_off_stops_every_watch() {
        let database = database_with_repo(1, "/repo/one");
        let registry = WatcherRegistry::new(
            Arc::new(RecordingWatcher::default()) as DynWatcher,
            Arc::new(|_, _| {}),
        );
        registry
            .start(1, Path::new("/repo/one"), WatchOptions::default())
            .unwrap();

        forgedesk_storage::SettingsRepository::new(&database)
            .set(
                &forgedesk_storage::Scope::Global,
                super::AUTO_REFRESH_KEY,
                "false",
            )
            .unwrap();
        super::apply_settings(&registry, &database, &[1]).unwrap();

        assert!(
            registry.active_ids().is_empty(),
            "关掉开关后不该还有监听在跑（否则用户以为关了、CPU 还在烧）"
        );
    }

    #[test]
    fn turning_auto_refresh_on_picks_up_repositories_opened_while_it_was_off() {
        let database = database_with_repo(1, "/repo/one");
        forgedesk_storage::SettingsRepository::new(&database)
            .set(
                &forgedesk_storage::Scope::Global,
                super::DEBOUNCE_KEY,
                "1200",
            )
            .unwrap();
        let registry = WatcherRegistry::new(
            Arc::new(RecordingWatcher::default()) as DynWatcher,
            Arc::new(|_, _| {}),
        );

        // 开关打开：本次调用要给"已打开但没有监听"的仓库补上监听
        super::apply_settings(&registry, &database, &[1]).unwrap();

        assert!(registry.is_watching(1));
        assert_eq!(
            registry
                .options_of(1)
                .map(|options| options.debounce.as_millis()),
            Some(1200),
            "新启动的监听必须用设置里的去抖动值"
        );

        // 幂等：再跑一次不会重复监听，也不会报错
        super::apply_settings(&registry, &database, &[1]).unwrap();
        assert_eq!(registry.active_ids(), vec![1]);
    }

    #[test]
    fn start_for_repo_respects_the_switch_and_never_fails_the_caller() {
        let database = database_with_repo(1, "/repo/one");
        let registry = WatcherRegistry::new(
            Arc::new(RecordingWatcher::default()) as DynWatcher,
            Arc::new(|_, _| {}),
        );

        // 开关默认打开 → 会启动
        super::start_for_repo(&registry, &database, 1, Path::new("/repo/one"));
        assert!(registry.is_watching(1));

        // 关掉之后不会再启动新的监听
        forgedesk_storage::SettingsRepository::new(&database)
            .set(
                &forgedesk_storage::Scope::Global,
                super::AUTO_REFRESH_KEY,
                "false",
            )
            .unwrap();
        registry.stop(1);
        super::start_for_repo(&registry, &database, 1, Path::new("/repo/one"));
        assert!(!registry.is_watching(1), "开关关着就不该启动监听");
    }
}
