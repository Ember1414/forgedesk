//! 仓库文件监听：把操作系统的文件事件收敛成"界面该刷新什么"（M1 / T1.10）。
//!
//! # 为什么需要它
//!
//! 应用自己的操作能发 `repo:changed`，但**用户在外部改的东西不能**：终端里
//! `git checkout`、编辑器里保存一个文件、别的工具跑 `git add`。没有监听时，
//! 界面只能靠轮询兜底（T1.4 的 15 秒定时刷新），既慢又在大仓库上白烧 CPU。
//!
//! # 三层收敛，缺一不可
//!
//! 1. **过滤**（[`classify_path`]）：仓库里绝大多数写入都是噪音。
//!    `.git/objects`（每次 git 操作都写）、`.git/logs`（reflog）、`node_modules`、
//!    `target` 一律丢弃；`.git/index` 归到"工作区"（它描述的正是暂存状态），
//!    `.git/HEAD`、`.git/refs/**`、`.git/packed-refs` 归到"引用"。
//! 2. **去抖动**（[`Accumulator`]）：`git checkout` 会在一瞬间产生几百个事件，
//!    逐个上报会让界面抖成一团。默认 300 ms 合并窗口，窗口内的路径去重后合成
//!    **至多两个**事件（工作区一个、引用一个）。
//! 3. **溢出保护**：单窗口事件量超过阈值（默认 2000）时不再列路径，改发一个
//!    "大量变更"事件，让界面整体失效并提示手动刷新——大仓库 checkout 就是这样。
//!
//! # 为什么不依赖 Tauri
//!
//! 与 `logging` / `session` 同样的理由：本模块在纯 Rust 测试里跑，
//! 不需要启动桌面运行时。事件怎么送到前端由调用方（命令层）决定。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use notify::event::{MetadataKind, ModifyKind};
use notify::{EventKind, RecursiveMode, Watcher};

/// 默认去抖动窗口（毫秒）——设置项的缺省值。
pub const DEFAULT_DEBOUNCE_MS: u64 = 300;

/// 默认去抖动窗口（任务定义：300 ms）。
pub const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(DEFAULT_DEBOUNCE_MS);

/// 默认的单窗口路径上限（任务定义：> 2000 视为"大量变更"）。
pub const DEFAULT_MAX_PATHS: usize = 2000;

/// 任意层级都忽略的目录名（构建产物、依赖目录）。
const IGNORED_DIR_NAMES: &[&str] = &["node_modules", "target", ".cache", "dist", "build"];

/// `.git` 下直接忽略的子目录：写入频率极高但与"界面要显示什么"无关。
///
/// `objects` 是对象库（每次 git 命令都会写），`logs` 是 reflog（同上），
/// `lfs` 是大文件缓存。三者都丢弃之后，`.git` 下剩下的写入
/// （`index` / `HEAD` / `refs/**` / `packed-refs` / 各种 `*_HEAD` 状态文件）
/// 恰恰都是界面真正关心的。
const IGNORED_GIT_DIRS: &[&str] = &["objects", "logs", "lfs"];

/// 一次变更的类别——决定前端失效哪些查询。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum WatchKind {
    /// 工作区或暂存区变化：状态面板、diff、提交面板要看。
    Workspace,
    /// 引用变化（HEAD、分支、packed-refs）：历史与分支列表要看。
    Refs,
    /// 单窗口变更量超过阈值：无法逐条列举，界面应当整体失效并提示手动刷新。
    Large,
}

impl WatchKind {
    /// 稳定的短名（IPC 与日志用；前端据此走 i18n）。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Workspace => "workspace",
            Self::Refs => "refs",
            Self::Large => "large",
        }
    }
}

/// 去抖动之后的一次变更。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchEvent {
    /// 变更类别。
    pub kind: WatchKind,
    /// 涉及的路径，**相对仓库根**（`Large` 时为空）。
    ///
    /// 相对而不是绝对有两个理由：前端把它与状态面板里的路径比对（那里是相对路径），
    /// 以及绝对路径里带着用户名与目录结构——那是隐私，红线 R8 要求它别到处跑。
    pub paths: Vec<PathBuf>,
}

/// 监听选项。
#[derive(Debug, Clone)]
pub struct WatchOptions {
    /// 合并窗口。
    pub debounce: Duration,
    /// 单窗口路径上限，超过即判定为"大量变更"。
    pub max_paths: usize,
    /// 额外的忽略前缀（相对仓库根；可配置项来自设置）。
    pub extra_ignores: Vec<PathBuf>,
}

impl Default for WatchOptions {
    fn default() -> Self {
        Self {
            debounce: DEFAULT_DEBOUNCE,
            max_paths: DEFAULT_MAX_PATHS,
            extra_ignores: Vec::new(),
        }
    }
}

impl WatchOptions {
    /// 去抖动窗口的下限/上限（设置项来自界面，必须收敛到有意义的范围）。
    pub const MIN_DEBOUNCE_MS: u64 = 50;
    /// 见 [`WatchOptions::MIN_DEBOUNCE_MS`]。
    pub const MAX_DEBOUNCE_MS: u64 = 5_000;

    /// 从"毫秒"构造：越界值收敛到合法范围。
    ///
    /// 为什么收敛而不是报错：这个值来自设置项，用户（或一次手工改坏的存储）
    /// 给出的 0 或 10 分钟都不该让监听失效或让界面停止刷新。
    pub fn with_debounce_ms(debounce_ms: u64) -> Self {
        Self {
            debounce: Duration::from_millis(
                debounce_ms.clamp(Self::MIN_DEBOUNCE_MS, Self::MAX_DEBOUNCE_MS),
            ),
            ..Self::default()
        }
    }
}

/// 监听失败的原因。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WatchError {
    /// 路径不存在或不是目录。
    #[error("cannot watch {path}: {reason}")]
    Path {
        /// 被监听的路径。
        path: PathBuf,
        /// 原因。
        reason: String,
    },
    /// 底层监听器报错（句柄耗尽、权限不足…）。
    #[error("file watcher backend failed: {0}")]
    Backend(String),
}

/// 事件回调。用 `Arc<dyn Fn>` 而不是泛型：句柄要在注册表里按仓库存起来，
/// 泛型参数会一路传染到 `AppState`。
pub type WatchCallback = Arc<dyn Fn(WatchEvent) + Send + Sync>;

/// 文件监听器。
///
/// 抽象成 trait 的理由与 `GitEngine` 一致：注册表（命令层）要能在测试里
/// 注入一个"我说发生什么就发生什么"的假实现，否则生命周期逻辑只能靠
/// 真实文件系统事件来测，那既慢又不稳。
pub trait FileWatcher: Send + Sync + std::fmt::Debug {
    /// 开始监听 `root`（递归），返回句柄；句柄被丢弃即停止监听。
    fn watch(
        &self,
        root: &Path,
        options: WatchOptions,
        on_event: WatchCallback,
    ) -> Result<WatcherHandle, WatchError>;
}

/// 监听句柄：丢弃即停止。
#[derive(Debug)]
pub struct WatcherHandle {
    stop: Arc<AtomicBool>,
    control: Option<Sender<Message>>,
    thread: Option<JoinHandle<()>>,
}

impl WatcherHandle {
    /// 一个不监听任何东西的句柄。
    ///
    /// 存在的理由：注入式测试需要一个"真句柄"来验证生命周期（重启、停止、
    /// 多仓库并存），而假实现不该为此去监听真实目录——那既慢又会引入平台差异。
    pub fn detached() -> Self {
        Self {
            stop: Arc::new(AtomicBool::new(false)),
            control: None,
            thread: None,
        }
    }

    /// 显式停止（与 `Drop` 等价，但会等待监听线程退出）。
    pub fn stop(mut self) {
        self.shutdown();
    }

    /// 内部停止：置标志 + 唤醒线程 + 等待退出。
    ///
    /// 为什么必须 join：监听线程持有操作系统的监听句柄。不等待它退出就返回，
    /// 紧接着对同一个目录重新 `watch`（设置变更时的重启路径）会与旧线程
    /// 竞争，在 Windows 上表现为"新监听收不到事件"。
    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(control) = self.control.take() {
            // 线程可能已经退出：发送失败不是错误
            let _ = control.send(Message::Stop);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for WatcherHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// 基于 `notify` 的实现（三平台各自的后端由它选择）。
#[derive(Debug, Default)]
pub struct NotifyFileWatcher;

impl FileWatcher for NotifyFileWatcher {
    fn watch(
        &self,
        root: &Path,
        options: WatchOptions,
        on_event: WatchCallback,
    ) -> Result<WatcherHandle, WatchError> {
        if !root.is_dir() {
            return Err(WatchError::Path {
                path: root.to_path_buf(),
                reason: "not a directory".to_owned(),
            });
        }

        let (sender, receiver) = mpsc::channel::<Message>();
        let raw_sender = sender.clone();
        let root_owned = root.to_path_buf();
        let ignores = IgnoreRules::new(root, &options.extra_ignores);

        // notify 的回调在它自己的线程上执行：这里只做"收下来"，不做任何判断，
        // 判断与合并全部在下面的线程里完成（回调里做慢活会丢事件）。
        let mut watcher = notify::recommended_watcher(
            move |result: notify::Result<notify::Event>| match result {
                Ok(event) => {
                    let _ = raw_sender.send(Message::Raw {
                        kind: event.kind,
                        paths: event.paths,
                    });
                }
                Err(error) => {
                    let _ = raw_sender.send(Message::Backend(error.to_string()));
                }
            },
        )
        .map_err(|error| WatchError::Backend(error.to_string()))?;

        watcher
            .watch(root, RecursiveMode::Recursive)
            .map_err(|error| WatchError::Backend(error.to_string()))?;

        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread = thread::Builder::new()
            .name("forgedesk-watch".to_owned())
            .spawn(move || {
                // watcher 必须活到这个线程结束：它被 drop 时操作系统才真正停止监听。
                let _watcher = watcher;
                let mut accumulator = Accumulator::new(ignores, &options);
                // 后端错误只在下一个窗口上报一次：它可能每秒来一条（句柄耗尽），
                // 逐条写日志会把日志淹掉，而用户需要的是"这个仓库没监听上"这件事本身
                let mut pending_backend_error: Option<String> = None;

                loop {
                    if thread_stop.load(Ordering::SeqCst) {
                        break;
                    }
                    match receiver.recv_timeout(options.debounce) {
                        Ok(Message::Stop) => break,
                        Ok(Message::Raw { kind, paths }) => {
                            accumulator.push(&kind, paths);
                            // 溢出时立刻上报：此时继续累积已经没有任何意义
                            if accumulator.overflowed() {
                                accumulator.drain(|event| on_event(event));
                            }
                        }
                        Ok(Message::Backend(reason)) => {
                            // 后端错误本身不影响已收集的事件，攒到下一次 flush 时记一次日志
                            pending_backend_error = Some(reason);
                        }
                        Err(RecvTimeoutError::Timeout) => {
                            accumulator.drain(|event| on_event(event));
                            if let Some(reason) = pending_backend_error.take() {
                                tracing::warn!(
                                    root = %root_owned.display(),
                                    reason = %reason,
                                    "文件监听后端报告了一次错误"
                                );
                            }
                        }
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                }

                // 线程退出前把攒下的东西发出去：用户看到的事件宁可稍晚，不可丢失
                accumulator.drain(|event| on_event(event));
            })
            .map_err(|error| WatchError::Backend(error.to_string()))?;

        Ok(WatcherHandle {
            stop,
            control: Some(sender),
            thread: Some(thread),
        })
    }
}

/// 送进监听线程的消息。
enum Message {
    Raw {
        kind: EventKind,
        paths: Vec<PathBuf>,
    },
    Backend(String),
    Stop,
}

/// 忽略规则：目录名（任意层级）+ 额外的相对路径前缀。
#[derive(Debug, Clone)]
struct IgnoreRules {
    root: PathBuf,
    prefixes: Vec<PathBuf>,
}

impl IgnoreRules {
    fn new(root: &Path, extra: &[PathBuf]) -> Self {
        Self {
            root: root.to_path_buf(),
            prefixes: extra.to_vec(),
        }
    }

    /// 给定路径是否该被忽略。
    fn ignores(&self, path: &Path) -> bool {
        let relative = path.strip_prefix(&self.root).unwrap_or(path);

        // 额外前缀（可配置）：按路径前缀匹配，`dist` 这类单段前缀只匹配仓库根下那一层
        if self
            .prefixes
            .iter()
            .any(|prefix| relative.starts_with(prefix))
        {
            return true;
        }

        let mut components = relative.components();
        let first = components.next();
        let second = components.next();

        // `.git` 下的高频子目录
        if first.is_some_and(|component| component.as_os_str() == ".git")
            && second.is_some_and(|component| {
                IGNORED_GIT_DIRS
                    .iter()
                    .any(|ignored| component.as_os_str() == *ignored)
            })
        {
            return true;
        }

        // 任意层级的构建/依赖目录
        relative.components().any(|component| {
            let name = component.as_os_str();
            IGNORED_DIR_NAMES.iter().any(|ignored| name == *ignored)
        })
    }
}

/// 把一次原始事件归到一个类别；`None` 表示"与界面无关，丢弃"。
fn classify(kind: &EventKind, rules: &IgnoreRules, path: &Path) -> Option<WatchKind> {
    if is_noise(kind) {
        return None;
    }
    if rules.ignores(path) {
        return None;
    }

    let relative = path.strip_prefix(&rules.root).unwrap_or(path);
    let mut components = relative.components();
    let first = components.next();

    if first.is_some_and(|component| component.as_os_str() == std::ffi::OsStr::new(".git")) {
        let second = components
            .next()
            .and_then(|component| component.as_os_str().to_str());
        return Some(match second {
            // 索引描述的是"暂存了什么"：它对界面而言就是工作区状态的一部分
            Some("index") | Some("index.lock") => WatchKind::Workspace,
            // 其余（HEAD、refs/**、packed-refs、MERGE_HEAD…）都是引用语境
            _ => WatchKind::Refs,
        });
    }

    Some(WatchKind::Workspace)
}

/// 只读访问类事件与"仅元数据"事件不算内容变化。
///
/// - `Access`：打开/读取文件（编辑器扫目录、git 读对象都会产生）；
/// - `Modify(Metadata(..))`：权限、访问时间、mtime 之类的属性写入。
///   `touch` 一个文件不改变 git 看到的内容，刷新一次状态也不会看到新东西。
///
/// 保留 `MetadataKind::Any`：后端没能细分时宁可多刷一次，也不能漏掉真实改动。
fn is_noise(kind: &EventKind) -> bool {
    match kind {
        EventKind::Access(_) => true,
        EventKind::Modify(ModifyKind::Metadata(metadata)) => !matches!(metadata, MetadataKind::Any),
        // 未知/兜底类别：不猜，交给下游按路径过滤
        _ => false,
    }
}

/// 合并窗口内的累积器。
#[derive(Debug)]
struct Accumulator {
    rules: IgnoreRules,
    max_paths: usize,
    workspace: BTreeSet<PathBuf>,
    refs: BTreeSet<PathBuf>,
    overflowed: bool,
}

impl Accumulator {
    fn new(rules: IgnoreRules, options: &WatchOptions) -> Self {
        Self {
            rules,
            max_paths: options.max_paths.max(1),
            workspace: BTreeSet::new(),
            refs: BTreeSet::new(),
            overflowed: false,
        }
    }

    fn push(&mut self, kind: &EventKind, paths: Vec<PathBuf>) {
        for path in paths {
            let Some(kind) = classify(kind, &self.rules, &path) else {
                continue;
            };
            let bucket = match kind {
                WatchKind::Workspace => &mut self.workspace,
                // classify 不会返回 Large
                WatchKind::Refs | WatchKind::Large => &mut self.refs,
            };
            // 存相对路径：事件要跨过 IPC 到前端，绝对路径既带隐私又不好比对
            let relative = match path.strip_prefix(&self.rules.root) {
                Ok(relative) => relative.to_path_buf(),
                Err(_) => path,
            };
            bucket.insert(relative);
        }

        if self.workspace.len() + self.refs.len() > self.max_paths {
            self.overflowed = true;
        }
    }

    fn overflowed(&self) -> bool {
        self.overflowed
    }

    /// 取出并清空累积的事件。
    ///
    /// 一次窗口**最多**产出两个事件（工作区一个、引用一个），而不是合并成一个：
    /// 类别决定前端失效哪些查询，混在一起会让"只改了工作区"也去刷新历史列表。
    fn drain(&mut self, mut emit: impl FnMut(WatchEvent)) {
        if self.overflowed {
            self.workspace.clear();
            self.refs.clear();
            self.overflowed = false;
            emit(WatchEvent {
                kind: WatchKind::Large,
                paths: Vec::new(),
            });
            return;
        }

        if !self.workspace.is_empty() {
            emit(WatchEvent {
                kind: WatchKind::Workspace,
                paths: std::mem::take(&mut self.workspace).into_iter().collect(),
            });
        }
        if !self.refs.is_empty() {
            emit(WatchEvent {
                kind: WatchKind::Refs,
                paths: std::mem::take(&mut self.refs).into_iter().collect(),
            });
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::fs;
    use std::sync::mpsc;
    use std::time::Instant;

    use super::{
        classify, is_noise, Accumulator, FileWatcher, IgnoreRules, NotifyFileWatcher, WatchEvent,
        WatchKind, WatchOptions,
    };

    fn ignore_rules(root: &str, extra: &[&str]) -> IgnoreRules {
        IgnoreRules::new(
            std::path::Path::new(root),
            &extra
                .iter()
                .map(std::path::PathBuf::from)
                .collect::<Vec<_>>(),
        )
    }

    #[test]
    fn git_bookkeeping_writes_are_dropped_but_refs_and_index_survive() {
        let rules = ignore_rules("/repo", &[]);
        let create = notify::EventKind::Create(notify::event::CreateKind::File);

        // 每次 git 命令都会写这些：丢弃
        assert_eq!(
            classify(
                &create,
                &rules,
                std::path::Path::new("/repo/.git/objects/ab/cd")
            ),
            None
        );
        assert_eq!(
            classify(
                &create,
                &rules,
                std::path::Path::new("/repo/.git/logs/HEAD")
            ),
            None
        );
        assert_eq!(
            classify(
                &create,
                &rules,
                std::path::Path::new("/repo/.git/lfs/tmp/x")
            ),
            None
        );

        // 索引 = 暂存状态 → 工作区
        assert_eq!(
            classify(&create, &rules, std::path::Path::new("/repo/.git/index")),
            Some(WatchKind::Workspace)
        );
        // HEAD / refs → 引用
        assert_eq!(
            classify(&create, &rules, std::path::Path::new("/repo/.git/HEAD")),
            Some(WatchKind::Refs)
        );
        assert_eq!(
            classify(
                &create,
                &rules,
                std::path::Path::new("/repo/.git/refs/heads/main")
            ),
            Some(WatchKind::Refs)
        );
        assert_eq!(
            classify(
                &create,
                &rules,
                std::path::Path::new("/repo/.git/packed-refs")
            ),
            Some(WatchKind::Refs)
        );
    }

    #[test]
    fn build_and_dependency_directories_are_dropped_at_any_depth() {
        let rules = ignore_rules("/repo", &[]);
        let modify = notify::EventKind::Modify(notify::event::ModifyKind::Data(
            notify::event::DataChange::Any,
        ));

        assert_eq!(
            classify(&modify, &rules, std::path::Path::new("/repo/src/app.ts")),
            Some(WatchKind::Workspace)
        );
        assert_eq!(
            classify(
                &modify,
                &rules,
                std::path::Path::new("/repo/node_modules/x/y.js")
            ),
            None
        );
        assert_eq!(
            classify(
                &modify,
                &rules,
                std::path::Path::new("/repo/target/debug/app.exe")
            ),
            None
        );
        // 深层的 node_modules 同样丢弃（monorepo 里到处都是）
        assert_eq!(
            classify(
                &modify,
                &rules,
                std::path::Path::new("/repo/packages/a/node_modules/b/c.js")
            ),
            None
        );

        // 额外忽略前缀来自设置
        let with_extra = ignore_rules("/repo", &["vendor", "docs/generated"]);
        assert_eq!(
            classify(
                &modify,
                &with_extra,
                std::path::Path::new("/repo/vendor/lib.rs")
            ),
            None
        );
        assert_eq!(
            classify(
                &modify,
                &with_extra,
                std::path::Path::new("/repo/docs/generated/api.md")
            ),
            None
        );
        assert_eq!(
            classify(
                &modify,
                &with_extra,
                std::path::Path::new("/repo/docs/hand.md")
            ),
            Some(WatchKind::Workspace)
        );
    }

    #[test]
    fn read_access_and_metadata_only_events_are_noise() {
        use notify::event::{AccessKind, DataChange, MetadataKind, ModifyKind};
        use notify::EventKind;

        assert!(is_noise(&EventKind::Access(AccessKind::Read)));
        assert!(is_noise(&EventKind::Modify(ModifyKind::Metadata(
            MetadataKind::Permissions
        ))));
        assert!(is_noise(&EventKind::Modify(ModifyKind::Metadata(
            MetadataKind::WriteTime
        ))));
        // 内容改动绝不能丢
        assert!(!is_noise(&EventKind::Modify(ModifyKind::Data(
            DataChange::Any
        ))));
        assert!(!is_noise(&EventKind::Create(
            notify::event::CreateKind::File
        )));
        // 后端没细分时保留（宁可多刷一次）
        assert!(!is_noise(&EventKind::Modify(ModifyKind::Metadata(
            MetadataKind::Any
        ))));
    }

    #[test]
    fn the_accumulator_coalesces_a_window_and_splits_by_kind() {
        let mut accumulator =
            Accumulator::new(ignore_rules("/repo", &[]), &WatchOptions::default());
        let modify = notify::EventKind::Modify(notify::event::ModifyKind::Data(
            notify::event::DataChange::Any,
        ));

        accumulator.push(&modify, vec![std::path::PathBuf::from("/repo/src/a.ts")]);
        accumulator.push(&modify, vec![std::path::PathBuf::from("/repo/src/a.ts")]);
        accumulator.push(
            &modify,
            vec![std::path::PathBuf::from("/repo/.git/refs/heads/main")],
        );
        // 噪音与非仓库路径一起进来：必须被丢掉
        accumulator.push(
            &modify,
            vec![std::path::PathBuf::from("/repo/.git/objects/x")],
        );
        accumulator.push(
            &modify,
            vec![std::path::PathBuf::from("/repo/node_modules/a")],
        );

        let mut events: Vec<WatchEvent> = Vec::new();
        accumulator.drain(|event| events.push(event));

        assert_eq!(events.len(), 2, "工作区与引用各一个事件：{events:?}");
        assert_eq!(events[0].kind, WatchKind::Workspace);
        assert_eq!(events[0].paths.len(), 1, "同一路径在一个窗口里只出现一次");
        assert_eq!(events[1].kind, WatchKind::Refs);

        // 取走即清空
        let mut again: Vec<WatchEvent> = Vec::new();
        accumulator.drain(|event| again.push(event));
        assert!(again.is_empty());
    }

    #[test]
    fn too_many_paths_in_one_window_collapse_into_a_single_large_event() {
        let mut accumulator = Accumulator::new(
            ignore_rules("/repo", &[]),
            &WatchOptions {
                max_paths: 3,
                ..WatchOptions::default()
            },
        );
        let modify = notify::EventKind::Modify(notify::event::ModifyKind::Data(
            notify::event::DataChange::Any,
        ));

        for index in 0..5 {
            accumulator.push(
                &modify,
                vec![std::path::PathBuf::from(format!("/repo/src/f{index}.ts"))],
            );
        }
        assert!(accumulator.overflowed(), "超过上限后必须标记溢出");

        let mut events: Vec<WatchEvent> = Vec::new();
        accumulator.drain(|event| events.push(event));

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, WatchKind::Large);
        assert!(events[0].paths.is_empty(), "大量变更不列路径");
        assert!(!accumulator.overflowed(), "上报后标志复位");
    }

    #[test]
    fn debounce_options_clamp_absurd_values_instead_of_disabling_the_watcher() {
        assert_eq!(
            WatchOptions::with_debounce_ms(0).debounce,
            std::time::Duration::from_millis(WatchOptions::MIN_DEBOUNCE_MS)
        );
        assert_eq!(
            WatchOptions::with_debounce_ms(300).debounce,
            std::time::Duration::from_millis(300)
        );
        assert_eq!(
            WatchOptions::with_debounce_ms(600_000).debounce,
            std::time::Duration::from_millis(WatchOptions::MAX_DEBOUNCE_MS)
        );
    }

    #[test]
    fn watching_a_missing_directory_fails_with_the_path_in_the_error() {
        let watcher = NotifyFileWatcher;
        let missing = std::env::temp_dir().join("forgedesk-watch-missing-dir");
        let error = watcher
            .watch(
                &missing,
                WatchOptions::default(),
                std::sync::Arc::new(|_| {}),
            )
            .expect_err("不存在目录必须失败");

        // 错误里带上路径，用户才知道是哪个仓库没监听上
        assert!(
            error.to_string().contains("forgedesk-watch-missing-dir"),
            "{error}"
        );
    }

    #[test]
    fn a_real_write_is_reported_once_and_noise_is_dropped() {
        let root = std::env::temp_dir().join(format!("forgedesk-watch-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join(".git/objects")).unwrap();
        fs::create_dir_all(root.join(".git/refs/heads")).unwrap();
        fs::create_dir_all(root.join("node_modules/pkg")).unwrap();

        let (sender, receiver) = mpsc::channel::<WatchEvent>();
        let watcher = NotifyFileWatcher;
        let handle = watcher
            .watch(
                &root,
                // 窗口调小一点，让用例快一些；语义与默认值相同
                WatchOptions {
                    debounce: std::time::Duration::from_millis(150),
                    ..WatchOptions::default()
                },
                std::sync::Arc::new(move |event| {
                    let _ = sender.send(event);
                }),
            )
            .expect("监听失败");

        // 一次窗口里同时制造：一个真实改动 + 三类噪音
        fs::write(root.join("src/real.ts"), b"export {}\n").unwrap();
        fs::write(root.join(".git/objects/ab-cd"), b"x").unwrap();
        fs::write(root.join("node_modules/pkg/index.js"), b"x").unwrap();

        let first = receiver
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("真实改动必须在一个窗口内上报");

        assert_eq!(
            first.kind,
            WatchKind::Workspace,
            "普通文件改动属于工作区：{first:?}"
        );
        assert!(
            first.paths.iter().any(|path| path.ends_with("src/real.ts")),
            "真实改动要在事件里：{first:?}"
        );
        assert!(
            first
                .paths
                .iter()
                .all(|path| !path.to_string_lossy().contains("node_modules")),
            "依赖目录的写入不该出现在事件里：{first:?}"
        );

        // 噪音路径（.git/objects）不能以任何形式上报
        let deadline = Instant::now() + std::time::Duration::from_millis(600);
        while let Ok(event) =
            receiver.recv_timeout(deadline.saturating_duration_since(Instant::now()))
        {
            assert!(
                event
                    .paths
                    .iter()
                    .all(|path| !path.to_string_lossy().contains("objects")),
                "对象库写入是噪音：{event:?}"
            );
            if Instant::now() >= deadline {
                break;
            }
        }

        handle.stop();
        let _ = fs::remove_dir_all(&root);
    }
}
