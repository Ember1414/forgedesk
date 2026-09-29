//! 仓库生命周期用例：发现、打开、克隆、初始化、最近列表。
//!
//! # 这一层在做什么
//!
//! 每个用例都是"纯逻辑（domain）+ IO（git-engine / storage）"的组合，
//! 而组合里最有价值的部分是**顺序与容错策略**：
//!
//! | 用例 | 顺序 | 容错策略 |
//! | --- | --- | --- |
//! | [`RepositoryService::open`] | 发现 → 审计 → 版本检查 → 登记 | 审计与版本检查失败**不阻塞打开**（它们只是附加提示） |
//! | [`RepositoryService::clone`] | 目标检查 → 克隆 → 审计 → 登记 | 目标非空**提前失败**，而不是等传输结束才报错 |
//! | [`RepositoryService::init`] | init → 写模板 → 登记 | 模板**只写新文件**，已存在的 `.gitignore` / `LICENSE` 一律不动 |
//!
//! # 为什么 `discover` 走 CLI 引擎
//!
//! 读操作默认走 libgit2，但 `discover` 是例外：libgit2 只暴露工作区**名称**，
//! 拿不到关联工作区的路径与 HEAD。理由与证据见 `docs/GIT-ENGINE-DIFF.md` §4.1。
//!
//! # 为什么写操作都在这里阻塞执行
//!
//! `clone` / `init` 是阻塞的（它们同步等待 git 子进程）。调用方
//! （`commands` 层）负责把它们放进 `JobRunner` 的线程里，并在此之前
//! 把 `job:progress` 事件接上——本层不感知"任务"，只负责"把这一步做对"。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use forgedesk_domain::git::{
    audit_config, CloneSpec, GitVersion, InitSpec, RepoAuditReport, RepoId, RepositoryInfo,
};
use forgedesk_domain::{AppError, AppResult, ErrorCode, FixAction};
use forgedesk_git_engine::engine::{GitEngine, ProgressSink};
use forgedesk_storage::{RepositoryRecord, RepositoryStore, RepositoryUpsert};

use crate::credentials::{CredentialContext, CredentialGate};
use crate::engines::GitEngines;
use crate::templates::{sanitize_holder, GitignoreTemplate, LicenseTemplate};

/// 毫秒级时间源。
///
/// 做成可注入的，是为了让"最近列表按打开时间排序"这件事能被**确定性地**测试：
/// 依赖真实时钟意味着两条记录可能落在同一毫秒里，测试会随机失败。
pub type MillisClock = Arc<dyn Fn() -> i64 + Send + Sync>;

/// 系统时钟。
///
/// `pub(crate)` 而不是私有：提交服务（T1.7）也要用它，而两个实现各写一份
/// "取当前毫秒"正是那种"看似无害、实则让时间语义分叉"的重复。
pub(crate) fn system_clock() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// 当前会话中"已打开"的仓库集合。
///
/// 为什么需要它：`repo_close` 必须真的做点什么。M1 还没有文件监听
/// （T1.10 才有），但"哪些仓库正开着"是**后端**的事实——它决定
/// 最近列表里的高亮、以及 T1.10 起要监听哪些目录。放进前端 store 会与
/// AGENTS §6「单一真相源」冲突。
///
/// 注意：这是**会话内**状态，不落库。应用重启后没有任何仓库是打开的。
#[derive(Debug, Default)]
pub struct OpenRepoRegistry {
    open: Mutex<BTreeSet<i64>>,
}

impl OpenRepoRegistry {
    /// 空集合。
    pub fn new() -> Self {
        Self::default()
    }

    /// 标记为已打开。
    pub fn open(&self, id: i64) {
        if let Ok(mut open) = self.open.lock() {
            open.insert(id);
        }
    }

    /// 取消"已打开"标记；返回它此前是否打开着。
    pub fn close(&self, id: i64) -> bool {
        self.open
            .lock()
            .map(|mut open| open.remove(&id))
            .unwrap_or(false)
    }

    /// 是否已打开。
    pub fn is_open(&self, id: i64) -> bool {
        self.open
            .lock()
            .map(|open| open.contains(&id))
            .unwrap_or(false)
    }

    /// 当前打开的仓库 id（升序）。
    pub fn ids(&self) -> Vec<i64> {
        self.open
            .lock()
            .map(|open| open.iter().copied().collect())
            .unwrap_or_default()
    }

    /// 全部关闭（应用退出时调用）。
    pub fn close_all(&self) {
        if let Ok(mut open) = self.open.lock() {
            open.clear();
        }
    }
}

/// 打开/克隆/初始化之后的完整结果。
///
/// 把 `RepositoryInfo` 与"打开时才顺便得到的两条附加信息"放在一起返回，
/// 而不是让界面再发两次 IPC——每次往返都是可见延迟（M1 验收：打开 ≤ 2s）。
#[derive(Debug, Clone)]
pub struct OpenedRepository {
    /// 仓库基本信息。
    pub info: RepositoryInfo,
    /// `repositories` 表里的主键。
    pub record_id: i64,
    /// 仓库配置审计结果（危险项警告，**不阻塞打开**）。
    pub audit: RepoAuditReport,
    /// 系统 git 版本；解析不出来时为 `None`。
    pub git_version: Option<GitVersion>,
}

impl OpenedRepository {
    /// 系统 git 版本是否满足最低要求。
    ///
    /// 版本未知时返回 `true`：把"读不到版本"当成"版本太低"会在用户的
    /// git 包装脚本稍有问题时弹出无法消除的警告。
    pub fn git_version_supported(&self) -> bool {
        self.git_version
            .as_ref()
            .map(GitVersion::is_supported)
            .unwrap_or(true)
    }

    /// 是否应当提示用户升级 git。
    pub fn needs_git_upgrade(&self) -> bool {
        !self.git_version_supported()
    }
}

/// 最近列表里的一项。
#[derive(Debug, Clone)]
pub struct RecentRepository {
    /// 数据库里的记录。
    pub record: RepositoryRecord,
    /// 当前会话中是否已打开。
    pub is_open: bool,
}

/// 初始化仓库时可选生成的许可证。
#[derive(Debug, Clone)]
pub struct LicenseSpec {
    /// 许可证模板。
    pub template: LicenseTemplate,
    /// 版权年份。
    pub year: u32,
    /// 版权持有者（会被清理成单行）。
    pub holder: String,
}

/// 初始化仓库时可选生成的附加文件。
#[derive(Debug, Clone, Default)]
pub struct InitExtras {
    /// 按语言生成 `.gitignore`。
    pub gitignore: Option<GitignoreTemplate>,
    /// 生成 `LICENSE`。
    pub license: Option<LicenseSpec>,
}

impl InitExtras {
    /// 是否需要写任何文件。
    pub fn is_empty(&self) -> bool {
        self.gitignore.is_none() && self.license.is_none()
    }
}

/// 仓库生命周期用例。
pub struct RepositoryService<'a> {
    engines: &'a GitEngines,
    store: RepositoryStore<'a>,
    open: &'a OpenRepoRegistry,
    /// 凭据门（T2.7）：克隆也要用保存过的凭据，否则私有仓库克隆必然失败。
    credentials: Option<&'a CredentialGate>,
    clock: MillisClock,
}

impl<'a> RepositoryService<'a> {
    /// 绑定引擎、仓储与"已打开"集合。
    pub fn new(
        engines: &'a GitEngines,
        store: RepositoryStore<'a>,
        open: &'a OpenRepoRegistry,
    ) -> Self {
        Self {
            engines,
            store,
            open,
            credentials: None,
            clock: Arc::new(system_clock),
        }
    }

    /// 接上凭据门（T2.7）：克隆私有仓库时按 URL 解析并注入凭据。
    #[must_use]
    pub fn with_credentials(mut self, credentials: &'a CredentialGate) -> Self {
        self.credentials = Some(credentials);
        self
    }

    /// 接上凭据门；`None`（宿主拿不到自身可执行文件路径）时退化为匿名/SSH。
    ///
    /// 存在的理由：调用方持有的往往是 `Option<Arc<CredentialGate>>`（任务闭包里
    /// 只能克隆 Arc），用这个形态可以一行接上，不必在每处写一遍 `match`。
    #[must_use]
    pub fn with_credential_gate(mut self, credentials: Option<&'a CredentialGate>) -> Self {
        self.credentials = credentials;
        self
    }

    /// 替换时间源（测试用）。
    #[must_use]
    pub fn with_clock(mut self, clock: MillisClock) -> Self {
        self.clock = clock;
        self
    }

    fn now(&self) -> i64 {
        (self.clock)()
    }

    // ------------------------------------------------------------ 发现

    /// 从任意目录向上发现仓库（不写库、不审计）。
    ///
    /// 路径不在任何仓库内时返回 [`ErrorCode::PathNotRepo`]，并附带
    /// "初始化仓库"的修复动作——这是 M1 验收里"打开非仓库给出明确错误与动作"
    /// 的那一条。
    pub fn discover(&self, path: &Path) -> AppResult<RepositoryInfo> {
        self.engines
            .write()
            .discover(path)
            .map_err(|error| with_init_action(error, path))
    }

    // ------------------------------------------------------------ 打开

    /// 打开仓库：发现 → 审计 → 版本检查 → 登记 → 标记为已打开。
    pub fn open(&self, path: &Path) -> AppResult<OpenedRepository> {
        let info = self.discover(path)?;
        self.finish_open(info)
    }

    /// 克隆仓库：目标检查 → 解析凭据 → 克隆 → 审计 → 登记。
    ///
    /// 阻塞调用；调用方负责放进 `JobRunner` 并接上进度。
    ///
    /// 凭据按 **spec 里的 URL** 解析：克隆时还没有仓库，拿不到远端列表，
    /// 而这正是"第一次接触远端"的路径——私有仓库没有凭据就必然失败。
    pub fn clone(&self, spec: &CloneSpec, progress: &ProgressSink) -> AppResult<OpenedRepository> {
        precheck_clone_target(&spec.into)?;
        let context = CredentialContext::resolve(self.credentials, &spec.url, None)?;
        let info = match self
            .engines
            .write()
            .clone(spec.clone(), progress, context.auth())
        {
            Ok(info) => info,
            Err(error) => return Err(context.describe_failure(error)),
        };
        context.note_success();
        self.finish_open(info)
    }

    /// 初始化仓库，并按需生成 `.gitignore` / `LICENSE`。
    ///
    /// 阻塞调用（`git init` 很快，但走同一套调用约定更不容易出错）。
    pub fn init(
        &self,
        path: &Path,
        spec: &InitSpec,
        extras: &InitExtras,
    ) -> AppResult<OpenedRepository> {
        let info = self.engines.write().init(path, spec.clone())?;

        if !extras.is_empty() {
            // 裸仓库没有工作区，模板无处可写；静默跳过会让用户以为文件生成了
            let workdir = info.workdir.clone().ok_or_else(|| {
                AppError::new(
                    ErrorCode::Validation,
                    "cannot generate files for a bare repository",
                )
                .with_hint(info.git_dir.to_string_lossy().into_owned())
                .with_retryable(false)
            })?;
            write_extras(&workdir, extras)?;
        }

        self.finish_open(info)
    }

    /// 打开流程的公共收尾：审计 → 版本检查 → 登记 → 标记已打开。
    fn finish_open(&self, info: RepositoryInfo) -> AppResult<OpenedRepository> {
        let audit = self.audit(&info.id);
        let git_version = self.engines.git_version().ok();
        let record_id = self.register(&info)?;
        self.open.open(record_id);

        Ok(OpenedRepository {
            info,
            record_id,
            audit,
            git_version,
        })
    }

    /// 仓库配置审计。
    ///
    /// 读不到配置时返回空报告而不是错误：审计是**附加提示**，
    /// 因为读不到 `.git/config` 就让"打开仓库"失败，用户会以为仓库坏了。
    fn audit(&self, repo: &RepoId) -> RepoAuditReport {
        match self.engines.write().repository_config(repo) {
            Ok(entries) => audit_config(&entries),
            Err(error) => {
                tracing::debug!(%error, repo = %repo, "读取仓库配置失败，跳过审计");
                RepoAuditReport::default()
            }
        }
    }

    /// 写入或更新 `repositories` 表，返回记录 id。
    fn register(&self, info: &RepositoryInfo) -> AppResult<i64> {
        let path = info.workdir.clone().unwrap_or_else(|| info.git_dir.clone());
        let normalized = normalize_repo_path(&path);

        self.store.upsert(
            &RepositoryUpsert {
                path: normalized.to_string_lossy().into_owned(),
                name: display_name(&normalized, info.is_bare),
                default_branch: info.default_branch.clone(),
                // 托管平台与体量分级由 M2 / M4 填充
                provider_id: None,
                size_class: None,
            },
            self.now(),
        )
    }

    // ------------------------------------------------------------ 列表与关闭

    /// 最近打开的仓库（`last_opened_at` 倒序，最近在前）。
    pub fn recent(&self, limit: usize) -> AppResult<Vec<RecentRepository>> {
        let records = self.store.recent(limit)?;
        Ok(records
            .into_iter()
            .map(|record| RecentRepository {
                is_open: self.open.is_open(record.id),
                record,
            })
            .collect())
    }

    /// 从列表中移除（**只删记录，不碰磁盘上的仓库**）。
    pub fn forget(&self, id: i64) -> AppResult<()> {
        self.require_record(id)?;
        self.store.forget(id)?;
        self.open.close(id);
        Ok(())
    }

    /// 关闭仓库（结束会话内的"已打开"状态；不删记录、不碰磁盘）。
    pub fn close(&self, id: i64) -> AppResult<()> {
        self.require_record(id)?;
        self.open.close(id);
        Ok(())
    }

    /// 记录必须存在，否则 [`ErrorCode::NotFound`]。
    ///
    /// 为什么显式校验：`DELETE` 影响 0 行不会报错，界面却会以为"移除成功"
    /// 而把一个仍在列表里的仓库从视图里抹掉。
    fn require_record(&self, id: i64) -> AppResult<RepositoryRecord> {
        self.store.find_by_id(id)?.ok_or_else(|| {
            AppError::new(ErrorCode::NotFound, "the repository is not in the list")
                .with_hint(id.to_string())
                .with_retryable(false)
        })
    }
}

// ------------------------------------------------------------ 辅助

/// 给"不是仓库"的错误补上路径与可点击的修复动作。
///
/// 只对 [`ErrorCode::PathNotRepo`] 生效：给别的错误挂一个"初始化仓库"按钮
/// 会让用户在一个与初始化无关的问题上点进死胡同。
fn with_init_action(mut error: AppError, path: &Path) -> AppError {
    if error.code != ErrorCode::PathNotRepo {
        return error;
    }

    // `discover` 的失败来自 git 的退出码，`detail` 里是它的 stderr，
    // 而用户此刻最需要的是确认"是哪个路径不是仓库"——路径由调用方知道，
    // 因此在这里补上（hint 只放数据，不放建议性散文）
    if error.hint.is_none() {
        error = error.with_hint(path.to_string_lossy().into_owned());
    }

    error.with_action(
        FixAction::new("repo.init", "errors.actions.initRepo", "repo_init")
            .with_args(serde_json::json!({ "spec": { "path": path.to_string_lossy() } })),
    )
}

/// 克隆前的目标目录检查。
///
/// git 自己也会拒绝非空目录，但它的报错出现在**传输开始之后**——用户等了
/// 几十秒才看到"目录非空"。提前判断把这条诊断变成一次立即的失败。
///
/// 目录**存在但为空**是允许的（git 也允许）；不存在同样允许（git 会创建）。
fn precheck_clone_target(target: &Path) -> AppResult<()> {
    if !target.exists() {
        return Ok(());
    }

    let mut entries = std::fs::read_dir(target).map_err(|error| {
        AppError::new(
            ErrorCode::PermissionDenied,
            "cannot read the clone destination directory",
        )
        .with_detail(error.to_string())
        .with_hint(target.to_string_lossy().into_owned())
    })?;

    if entries.next().is_some() {
        return Err(AppError::new(
            ErrorCode::Validation,
            "the clone destination already exists and is not empty",
        )
        .with_hint(target.to_string_lossy().into_owned())
        .with_retryable(false));
    }

    Ok(())
}

/// 写初始化时的附加文件。
fn write_extras(workdir: &Path, extras: &InitExtras) -> AppResult<()> {
    if let Some(template) = extras.gitignore {
        write_new_file(&workdir.join(".gitignore"), template.contents())?;
    }
    if let Some(license) = &extras.license {
        let contents = license
            .template
            .render(license.year, &sanitize_holder(&license.holder));
        write_new_file(&workdir.join("LICENSE"), &contents)?;
    }
    Ok(())
}

/// 写一个**新**文件；已存在时原样保留。
///
/// 为什么"已存在就跳过"而不是覆盖：`git init` 可以在一个已有内容的目录里执行，
/// 那里可能已经有用户精心维护的 `.gitignore`。覆盖它属于数据丢失。
fn write_new_file(path: &Path, contents: &str) -> AppResult<()> {
    if path.exists() {
        return Ok(());
    }
    std::fs::write(path, contents).map_err(|error| {
        AppError::new(ErrorCode::Storage, "could not write the generated file")
            .with_detail(error.to_string())
            .with_hint(path.to_string_lossy().into_owned())
    })
}

/// 规范化仓库路径：解析符号链接，并去掉 Windows 的 `\\?\` 前缀。
///
/// 为什么需要：`repositories.path` 是**唯一键**。同一个目录的两种写法
/// （符号链接、`\\?\` 前缀）会变成两条记录，用户的"最近仓库"里就会出现
/// 两个看起来完全一样的条目。
pub fn normalize_repo_path(path: &Path) -> PathBuf {
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    strip_verbatim_prefix(canonical)
}

/// 去掉 `\\?\` 前缀（Windows）。
///
/// `std::fs::canonicalize` 在 Windows 上返回 verbatim 路径（`\\?\E:\a`），
/// 直接存进数据库会让界面显示一串用户看不懂的前缀，也让"手输路径"永远匹配不上。
#[cfg(windows)]
fn strip_verbatim_prefix(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    let Some(rest) = text.strip_prefix(r"\\?\") else {
        return path;
    };
    // UNC 路径要还原成 `\\server\share` 形式，否则 `\\?\UNC\srv\share` 会被
    // 剥成 `UNC\srv\share`，变成一个相对路径
    match rest.strip_prefix(r"UNC\") {
        Some(unc) => PathBuf::from(format!(r"\\{unc}")),
        None => PathBuf::from(rest.to_owned()),
    }
}

/// 非 Windows 平台不需要处理 verbatim 前缀。
#[cfg(not(windows))]
fn strip_verbatim_prefix(path: PathBuf) -> PathBuf {
    path
}

/// 展示名：目录名；裸仓库去掉 `.git` 后缀。
fn display_name(path: &Path, is_bare: bool) -> String {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| path.to_string_lossy().into_owned());

    if is_bare {
        if let Some(stripped) = name.strip_suffix(".git") {
            if !stripped.is_empty() {
                return stripped.to_owned();
            }
        }
    }
    name
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::path::Path;

    use super::{display_name, normalize_repo_path, OpenRepoRegistry};

    #[test]
    fn open_registry_tracks_and_releases_ids() {
        let registry = OpenRepoRegistry::new();
        assert!(!registry.is_open(1));

        registry.open(3);
        registry.open(1);
        registry.open(3); // 重复打开是幂等的
        assert_eq!(registry.ids(), vec![1, 3]);

        assert!(registry.close(3));
        assert!(!registry.close(3), "重复关闭应报告'此前未打开'");
        assert_eq!(registry.ids(), vec![1]);

        registry.close_all();
        assert!(registry.ids().is_empty());
    }

    #[test]
    fn display_name_uses_the_last_segment() {
        assert_eq!(
            display_name(Path::new("/home/u/projects/api"), false),
            "api"
        );
        assert_eq!(
            display_name(Path::new(r"E:\work\service"), false),
            "service"
        );
    }

    #[test]
    fn a_bare_repository_drops_the_git_suffix() {
        assert_eq!(display_name(Path::new("/srv/repo.git"), true), "repo");
        // 只对裸仓库剥后缀：普通仓库的目录名就该原样显示
        assert_eq!(display_name(Path::new("/srv/repo.git"), false), "repo.git");
        // 只叫 `.git` 的目录不该被剥成空字符串
        assert_eq!(display_name(Path::new("/srv/.git"), true), ".git");
    }

    #[test]
    fn normalising_resolves_the_path_and_drops_the_verbatim_prefix() {
        let normalized = normalize_repo_path(Path::new("."));
        assert!(normalized.is_absolute(), "规范化后应是绝对路径");

        #[cfg(windows)]
        assert!(
            !normalized.to_string_lossy().starts_with(r"\\?\"),
            "不应把 verbatim 前缀写进数据库：{normalized:?}"
        );
    }

    #[test]
    fn normalising_a_missing_path_falls_back_to_the_input() {
        let missing = std::env::temp_dir().join("forgedesk-normalize-missing-xyz");
        assert_eq!(normalize_repo_path(&missing), missing);
    }
}
