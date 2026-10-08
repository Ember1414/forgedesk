//! 仓库生命周期命令（`repo_*`）。
//!
//! # 本层只做四件事
//!
//! 1. **收敛外部输入**：路径、URL、分支名、模板 id 全部在入口校验
//!    （前端零信任，AGENTS.md §6）。非法输入在这里就返回 `VALIDATION`，
//!    而不是让它走到 git 那里换回一句 `unknown option`。
//! 2. **把领域类型映射成 DTO**：`domain::git` 的类型刻意不派生 `Serialize`
//!    （理由见该模块头），线上形状由这里决定。
//! 3. **决定哪些操作是长任务**：`repo_clone` 走 `JobRunner`（几秒到几分钟），
//!    其余是同步的（打开仓库的目标是 ≤2s）。
//! 4. **不产出用户可见文案**：错误经 `to_app_error` 分类与脱敏，
//!    界面文案由前端按错误码走 i18n。

use std::path::PathBuf;
use std::sync::Arc;

use forgedesk_domain::git::{
    BranchLabel, CloneSpec, InitSpec, RepoAuditReport, RepositoryInfo, Worktree,
};
use forgedesk_domain::{AppError, AppResult, ErrorCode};
use forgedesk_services::audit::op_type;
use forgedesk_services::repository::{InitExtras, LicenseSpec, OpenedRepository};
use forgedesk_services::templates::{GitignoreTemplate, LicenseTemplate};
use forgedesk_services::{AuditArgs, AuditEntry, AuditLog, RepositoryService, GLOBAL_REPO_ID};
use forgedesk_storage::{OperationStore, RepositoryRecord, RepositoryStore};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use crate::jobs::reporter_for;
use crate::state::AppState;
use crate::watch;

/// 最近列表的默认条数。
const DEFAULT_RECENT_LIMIT: usize = 50;

/// 最近列表的最大条数。
///
/// 与存储层的上限一致：一次 IPC 把整张表拉走没有意义，
/// 而"最近"本身就是一个有界的列表。
const MAX_RECENT_LIMIT: usize = 200;

// ---------------------------------------------------------------- DTO

/// 当前分支在界面上的呈现类别。
///
/// 由后端判定而不是让前端拼：空仓库、游离 HEAD、普通分支三种状态的文案依据
/// 不同，让三处界面各拼一遍迟早互不一致（见 `domain::git::BranchLabel`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum BranchLabelDto {
    /// 还没有任何提交。
    Unborn,
    /// 游离 HEAD。
    Detached,
    /// 普通分支。
    Named {
        /// 分支短名。
        name: String,
    },
}

impl From<BranchLabel<'_>> for BranchLabelDto {
    fn from(label: BranchLabel<'_>) -> Self {
        match label {
            BranchLabel::Unborn => Self::Unborn,
            BranchLabel::Detached => Self::Detached,
            BranchLabel::Named(name) => Self::Named {
                name: name.to_owned(),
            },
        }
    }
}

/// 一个工作区。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeDto {
    /// 工作区目录。
    pub path: String,
    /// 当前检出的提交 oid。
    pub head: Option<String>,
    /// 检出的分支短名；游离 HEAD 时为 `null`。
    pub branch: Option<String>,
    /// 是否处于游离 HEAD。
    pub detached: bool,
    /// 是否为裸仓库。
    pub is_bare: bool,
    /// 是否被锁定。
    pub locked: bool,
    /// 是否可被 prune 清理。
    pub prunable: bool,
}

impl From<&Worktree> for WorktreeDto {
    fn from(worktree: &Worktree) -> Self {
        Self {
            path: worktree.path.to_string_lossy().into_owned(),
            head: worktree.head.clone(),
            branch: worktree.branch.clone(),
            detached: worktree.detached,
            is_bare: worktree.is_bare,
            locked: worktree.locked,
            prunable: worktree.prunable,
        }
    }
}

/// 仓库基本信息。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryDto {
    /// 工作区根目录（裸仓库为 `null`）。
    pub workdir: Option<String>,
    /// `.git` 目录（裸仓库即仓库根）。
    pub git_dir: String,
    /// 是否为裸仓库。
    pub is_bare: bool,
    /// 是否还没有任何提交。
    pub is_empty: bool,
    /// HEAD 指向的分支短名。
    pub head: Option<String>,
    /// 是否处于游离 HEAD。
    pub detached: bool,
    /// 当前分支的上游短名。
    pub upstream: Option<String>,
    /// 默认分支短名。
    pub default_branch: Option<String>,
    /// 是否为浅克隆。
    pub is_shallow: bool,
    /// 是否使用 Git LFS。
    pub is_lfs: bool,
    /// 关联工作区列表（主工作区在首位）。
    pub worktrees: Vec<WorktreeDto>,
    /// 界面应展示的分支标签类别。
    pub branch_label: BranchLabelDto,
}

impl From<&RepositoryInfo> for RepositoryDto {
    fn from(info: &RepositoryInfo) -> Self {
        Self {
            workdir: info
                .workdir
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned()),
            git_dir: info.git_dir.to_string_lossy().into_owned(),
            is_bare: info.is_bare,
            is_empty: info.is_empty,
            head: info.head.clone(),
            detached: info.detached,
            upstream: info.upstream.clone(),
            default_branch: info.default_branch.clone(),
            is_shallow: info.is_shallow,
            is_lfs: info.is_lfs,
            worktrees: info.worktrees.iter().map(WorktreeDto::from).collect(),
            branch_label: BranchLabelDto::from(info.branch_label()),
        }
    }
}

/// 一条仓库配置审计发现。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditFindingDto {
    /// 类别（`fsmonitor` / `ssh_command` / …），前端按它选解释文案。
    pub id: String,
    /// 严重程度（`info` / `warning` / `danger`）。
    pub severity: String,
    /// 命中的配置键。
    pub key: String,
    /// 命中的配置值（**已脱敏**）。
    pub value: String,
    /// 来源范围（`local` / `worktree`）。
    pub scope: String,
}

/// 仓库配置审计报告。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoAuditDto {
    /// 全部发现。
    pub findings: Vec<AuditFindingDto>,
    /// 是否存在"会被 git 当命令执行"的配置。
    pub has_danger: bool,
    /// 最高严重程度；无发现时为 `null`。
    pub max_severity: Option<String>,
}

impl From<&RepoAuditReport> for RepoAuditDto {
    fn from(report: &RepoAuditReport) -> Self {
        Self {
            findings: report
                .findings
                .iter()
                .map(|finding| AuditFindingDto {
                    id: finding.id.as_str().to_owned(),
                    severity: finding.severity.as_str().to_owned(),
                    key: finding.key.clone(),
                    value: finding.value.clone(),
                    scope: finding.scope.as_str().to_owned(),
                })
                .collect(),
            has_danger: report.has_danger(),
            max_severity: report
                .max_severity()
                .map(|severity| severity.as_str().to_owned()),
        }
    }
}

/// 打开 / 克隆 / 初始化之后的完整结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenedRepositoryDto {
    /// `repositories` 表里的主键（后续所有 `repo_id` 参数都用它）。
    pub record_id: i64,
    /// 仓库基本信息。
    pub repository: RepositoryDto,
    /// 配置审计结果。
    pub audit: RepoAuditDto,
    /// 系统 git 版本（如 `2.54.0.windows.1`）；解析不出来时为 `null`。
    pub git_version: Option<String>,
    /// git 版本是否满足最低要求。
    pub git_version_supported: bool,
    /// 是否应当提示用户升级 git。
    pub needs_git_upgrade: bool,
}

impl From<OpenedRepository> for OpenedRepositoryDto {
    fn from(opened: OpenedRepository) -> Self {
        Self {
            record_id: opened.record_id,
            repository: RepositoryDto::from(&opened.info),
            audit: RepoAuditDto::from(&opened.audit),
            git_version: opened
                .git_version
                .as_ref()
                .map(|version| version.to_string()),
            git_version_supported: opened.git_version_supported(),
            needs_git_upgrade: opened.needs_git_upgrade(),
        }
    }
}

/// 最近列表里的一项。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentRepositoryDto {
    /// `repositories` 表主键。
    pub id: i64,
    /// 规范化后的仓库路径（裸仓库为仓库根）。
    pub path: String,
    /// 展示名。
    pub name: String,
    /// 默认分支。
    pub default_branch: Option<String>,
    /// 最近一次打开时间（Unix 毫秒）。
    pub last_opened_at: Option<i64>,
    /// 首次登记时间（Unix 毫秒）。
    pub created_at: i64,
    /// 当前会话中是否已打开。
    pub is_open: bool,
}

impl From<&RepositoryRecord> for RecentRepositoryDto {
    fn from(record: &RepositoryRecord) -> Self {
        Self {
            id: record.id,
            path: record.path.clone(),
            name: record.name.clone(),
            default_branch: record.default_branch.clone(),
            last_opened_at: record.last_opened_at,
            created_at: record.created_at,
            // 由调用方按注册表覆盖；单独转换时默认未打开
            is_open: false,
        }
    }
}

/// 长任务创建的结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobIdDto {
    /// 任务 id；进度与结果通过 `job:*` 事件推送。
    pub job_id: String,
}

// ---------------------------------------------------------------- 请求

/// `repo_clone` 的请求体。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloneRequest {
    /// 远端 URL（HTTPS 或 SSH）。
    pub url: String,
    /// 克隆到哪个本地目录。
    pub into: String,
    /// 浅克隆深度；缺省为完整克隆。
    #[serde(default)]
    pub depth: Option<u32>,
    /// 只检出某个分支。
    #[serde(default)]
    pub branch: Option<String>,
    /// 克隆为裸仓库。
    #[serde(default)]
    pub bare: bool,
    /// 递归初始化子模块。
    #[serde(default)]
    pub recurse_submodules: bool,
    /// 只取单个分支的引用。
    #[serde(default)]
    pub single_branch: bool,
    /// 克隆对话框挑定的账号登录名（T4.5）：同 host 多账号时凭据门按它挑人。
    /// 缺省时凭据门按 URL 用户名/第一条凭据解析（与 T2.7 行为一致）。
    #[serde(default)]
    pub login_hint: Option<String>,
}

impl CloneRequest {
    /// 校验并转换成领域参数。
    fn into_spec(self) -> AppResult<CloneSpec> {
        let url = validate_remote_url(&self.url)?;
        let into = validated_path(&self.into, "clone destination")?;

        let mut spec = CloneSpec::new(url, into);
        if let Some(depth) = self.depth {
            if depth == 0 {
                return Err(AppError::new(
                    ErrorCode::Validation,
                    "clone depth must be at least 1",
                ));
            }
            spec = spec.with_depth(depth);
        }
        if let Some(branch) = self
            .branch
            .as_deref()
            .map(str::trim)
            .filter(|b| !b.is_empty())
        {
            validate_branch_name(branch)?;
            spec = spec.with_branch(branch);
        }
        spec = spec.with_submodules(self.recurse_submodules);
        spec = spec.with_single_branch(self.single_branch);

        if self.bare {
            spec.bare = true;
        }
        Ok(spec)
    }
}

/// `repo_init` 的请求体。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitRequest {
    /// 初始化到哪个目录（不存在时创建）。
    pub path: String,
    /// 初始分支名；缺省跟随用户的 `init.defaultBranch` 配置。
    #[serde(default)]
    pub initial_branch: Option<String>,
    /// 是否创建裸仓库。
    #[serde(default)]
    pub bare: bool,
    /// `.gitignore` 模板 id（`rust` / `node` / `python` / `go` / `java`）。
    #[serde(default)]
    pub gitignore: Option<String>,
    /// 许可证模板 id（`MIT` / `Apache-2.0` / `BSD-3-Clause`）。
    #[serde(default)]
    pub license: Option<String>,
    /// 版权持有者（仅当提供了许可证时有意义）。
    #[serde(default)]
    pub license_holder: Option<String>,
    /// 版权年份；缺省取当前年份。
    #[serde(default)]
    pub license_year: Option<u32>,
}

impl InitRequest {
    /// 校验并转换成领域参数与附加文件。
    fn into_parts(self) -> AppResult<(PathBuf, InitSpec, InitExtras)> {
        let path = validated_path(&self.path, "repository path")?;

        let mut spec = InitSpec::new();
        spec.bare = self.bare;
        if let Some(branch) = self
            .initial_branch
            .as_deref()
            .map(str::trim)
            .filter(|b| !b.is_empty())
        {
            validate_branch_name(branch)?;
            spec = spec.with_initial_branch(branch);
        }

        let gitignore = match self.gitignore.as_deref() {
            Some(raw) => Some(parse_gitignore_template(raw)?),
            None => None,
        };

        let license = match self.license.as_deref() {
            Some(raw) => Some(LicenseSpec {
                template: parse_license_template(raw)?,
                year: self.license_year.unwrap_or_else(current_year),
                holder: self.license_holder.unwrap_or_default(),
            }),
            None => None,
        };

        Ok((path, spec, InitExtras { gitignore, license }))
    }
}

// ---------------------------------------------------------------- 命令

/// 从任意目录向上发现仓库（**不写库、不审计**）。
///
/// 能力等级：`ReadOnly`。
///
/// 用途：用户拖入一个子目录时先告诉界面"它属于哪个仓库、当前分支是什么"，
/// 再由界面决定是否真的打开。路径不在任何仓库内时返回 `PATH_NOT_REPO`
/// 并附带"初始化仓库"动作。
#[tauri::command(async)]
pub fn repo_discover(state: State<'_, AppState>, path: String) -> AppResult<RepositoryDto> {
    let path = validated_path(&path, "path")?;
    state
        .repository_service()
        .discover(&path)
        .map(|info| RepositoryDto::from(&info))
}

/// 打开仓库：发现 → 配置审计 → git 版本检查 → 登记到最近列表。
///
/// 能力等级：`ReadOnly`（只读仓库、只写本地登记表；不改仓库状态，因此不需要快照）。
///
/// 审计与版本检查的结果**不会阻塞打开**：它们只是附加提示。
#[tauri::command(async)]
pub fn repo_open(state: State<'_, AppState>, path: String) -> AppResult<OpenedRepositoryDto> {
    let path = validated_path(&path, "path")?;
    let opened = state.repository_service().open(&path)?;

    // 打开成功即开始监听外部变化（终端里的 git 命令、编辑器保存）。
    // 监听失败只记日志：仓库照样能打开，只是少了自动刷新。
    watch::start_for_repo(
        &state.watchers,
        &state.database,
        opened.record_id,
        watch_root(&opened),
    );

    Ok(OpenedRepositoryDto::from(opened))
}

/// 克隆仓库（长任务）。
///
/// 能力等级：`Network`（访问远端并写本地磁盘）。
///
/// 立即返回任务 id；进度经 `job:progress`、结果经 `job:done` / `job:failed`
/// 推送（`docs/API.md` §3）。任务可经 `job_cancel` 取消。
#[tauri::command]
pub fn repo_clone(
    state: State<'_, AppState>,
    app: AppHandle,
    spec: CloneRequest,
) -> AppResult<JobIdDto> {
    let login_hint = spec.login_hint.clone();
    let spec = spec.into_spec()?;

    // 任务体要求 `'static`，因此把需要的共享对象克隆出来而不是借出引用
    let engines = Arc::clone(&state.engines);
    let database = Arc::clone(&state.database);
    let open_repos = Arc::clone(&state.open_repos);

    // 监听要在克隆**成功之后**才启动：克隆过程中目标目录还不存在，
    // 提前 watch 只会得到一个失败
    let watchers = Arc::clone(&state.watchers);
    // 凭据门：任务闭包只拿得到 Arc 克隆（用不了 AppState 上的便捷方法），
    // 漏接这一处就会让私有仓库的克隆永远以 AUTH_REQUIRED 失败
    let credential_gate = state.credential_gate.clone();

    let job_id = state.jobs.spawn(reporter_for(app), move |context| {
        let progress = crate::jobs::progress_sink(&context);
        let opened = {
            let service =
                RepositoryService::new(&engines, RepositoryStore::new(&database), &open_repos)
                    .with_credential_gate(credential_gate.as_deref())
                    .with_login_hint(login_hint);

            // 审计（T1.11）：克隆是 `Network` 级操作，却在任务线程里执行，
            // 因此记录也在这里写。仓库记录 id 要等克隆成功才存在，
            // 所以这条记录挂在"全局"下——args 里的目标路径足以回答"克隆了什么"。
            let operation = AuditLog::new(OperationStore::new(&database)).begin(
                &AuditEntry::new(GLOBAL_REPO_ID, op_type::CLONE).with_args(
                    AuditArgs::new()
                        .text("url", &spec.url)
                        .text("into", &spec.into.display().to_string())
                        .number("depth", spec.depth.map_or(0, i64::from))
                        .flag("bare", spec.bare),
                ),
            );

            // 没有 snapshot_id：克隆的"回滚"是删掉刚建出来的目录重来一次，
            // 而不是本地快照。审计如实写成"不可逆（无快照）"
            let result = service.clone(&spec, &progress);
            if let Some(operation) = operation {
                operation.finish(&result, None);
            }
            result?
        };

        // 服务已释放，这里可以再借数据库来读监听设置
        watch::start_for_repo(&watchers, &database, opened.record_id, watch_root(&opened));
        Ok(OpenedRepositoryDto::from(opened))
    });

    Ok(JobIdDto {
        job_id: job_id.as_str().to_owned(),
    })
}

/// 初始化仓库，并按需生成 `.gitignore` / `LICENSE`。
///
/// 能力等级：`Mutating`（在磁盘上创建仓库并写入文件；**不**创建提交，
/// 因此不需要快照——没有任何可回滚的既有状态）。
#[tauri::command(async)]
pub fn repo_init(state: State<'_, AppState>, spec: InitRequest) -> AppResult<OpenedRepositoryDto> {
    let (path, init_spec, extras) = spec.into_parts()?;

    // 仓库记录 id 要等初始化成功才存在，因此这条记录挂在"全局"下（args 有路径）
    // （DTO 构造挪进闭包：record 的结果形状要求可序列化——这也正是快照 id
    //   能从结果里提取进审计表的同一约束）
    let opened = crate::audit::record(
        &state,
        AuditEntry::new(GLOBAL_REPO_ID, op_type::INIT).with_args(
            AuditArgs::new()
                .text("path", &path.display().to_string())
                .flag("bare", init_spec.bare),
        ),
        || {
            state
                .repository_service()
                .init(&path, &init_spec, &extras)
                .map(OpenedRepositoryDto::from)
        },
    )?;

    // 与 repo_open 同一条纪律：初始化成功就顺手开始监听。
    // 监听根从 DTO 里取：workdir 缺失（裸仓库）时退回 `.git` 目录——
    // 与 `watch_root` 同一规则，只是数据源换了形状
    let watch_path = opened
        .repository
        .workdir
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(&opened.repository.git_dir));
    watch::start_for_repo(
        &state.watchers,
        &state.database,
        opened.record_id,
        &watch_path,
    );

    Ok(opened)
}

/// 最近打开的仓库（按最近打开时间倒序）。
///
/// 能力等级：`ReadOnly`。
#[tauri::command]
pub fn repo_recent_list(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> AppResult<Vec<RecentRepositoryDto>> {
    let limit = limit
        .unwrap_or(DEFAULT_RECENT_LIMIT)
        .clamp(1, MAX_RECENT_LIMIT);

    let recent = state.repository_service().recent(limit)?;
    Ok(recent
        .into_iter()
        .map(|item| RecentRepositoryDto {
            is_open: item.is_open,
            ..RecentRepositoryDto::from(&item.record)
        })
        .collect())
}

/// 从最近列表移除一个仓库（**只删记录，不碰磁盘上的仓库**）。
///
/// 能力等级：`Mutating`（改本地登记表）。
#[tauri::command]
pub fn repo_forget(state: State<'_, AppState>, repo_id: i64) -> AppResult<()> {
    // 删的是本地登记记录（磁盘上的仓库不动），但它同样是写操作，同样要留痕
    crate::audit::record(&state, AuditEntry::new(repo_id, op_type::FORGET), || {
        state.repository_service().forget(repo_id)
    })?;
    // 记录被移除，监听也必须停：句柄是真实资源（操作系统监听 + 一条线程）
    state.watchers.stop(repo_id);
    Ok(())
}

/// 关闭一个已打开的仓库（结束会话内的"已打开"状态）。
///
/// 能力等级：`ReadOnly`（不改数据库、不碰仓库，只改本进程内的会话状态）。
///
/// 终端安全网（T5.2）：该仓库还有活跃终端时拒绝关闭（`VALIDATION` + 计数 hint），
/// 由前端弹确认后再带着"关闭终端"的意图调用 `term_close`/重试——
/// 绝不静默杀掉用户正在用的 shell（里面可能有没跑完的命令）。
#[tauri::command]
pub fn repo_close(state: State<'_, AppState>, repo_id: i64) -> AppResult<()> {
    let active_terminals = state.terminals.active_for_repo(repo_id);
    if active_terminals > 0 {
        return Err(AppError::new(
            ErrorCode::Validation,
            "active terminal sessions are attached to this repository",
        )
        .with_hint(active_terminals.to_string()));
    }
    state.repository_service().close(repo_id)?;
    // 关闭仓库即停止监听：继续监听一个用户已经关掉的仓库既是浪费，
    // 也会让前端收到"没人在看"的事件
    state.watchers.stop(repo_id);
    Ok(())
}

// ---------------------------------------------------------------- 校验与转换

/// 该监听哪个目录：工作区优先，裸仓库退回 `.git` 目录。
///
/// 裸仓库没有工作区，但它有引用与索引——有人往它 `git push` 时界面同样要刷新，
/// 因此不能因为 `workdir` 为空就跳过监听。
fn watch_root(opened: &OpenedRepository) -> &std::path::Path {
    opened
        .info
        .workdir
        .as_deref()
        .unwrap_or(opened.info.git_dir.as_path())
}

/// 校验并收敛一个来自 IPC 的路径。
///
/// 这里只做"形状"校验（空、NUL）：路径**是否存在**、**是否是仓库**由
/// git 引擎判定，在命令层用 `Path::exists` 预判会引入 TOCTOU 且重复一遍逻辑。
fn validated_path(raw: &str, what: &str) -> AppResult<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AppError::new(
            ErrorCode::Validation,
            format!("the {what} is empty"),
        ));
    }
    if trimmed.contains('\0') {
        return Err(AppError::new(
            ErrorCode::Validation,
            format!("the {what} contains a NUL byte"),
        ));
    }
    Ok(PathBuf::from(trimmed))
}

/// 校验远端 URL 的形状。
///
/// 只拦"明显不是 URL"的输入（空、含空白或控制字符、含 NUL）。
/// 具体协议的合法性交给 git：它支持的传输方式（`file://`、`ssh://`、
/// 用户自定义的 `url.<base>.insteadOf`）比我们能枚举的多。
fn validate_remote_url(raw: &str) -> AppResult<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AppError::new(
            ErrorCode::Validation,
            "the remote url is empty",
        ));
    }
    if trimmed.chars().any(|c| c.is_control()) {
        return Err(AppError::new(
            ErrorCode::Validation,
            "the remote url contains control characters",
        ));
    }
    if trimmed.split_whitespace().count() > 1 {
        return Err(
            AppError::new(ErrorCode::Validation, "the remote url contains whitespace")
                .with_hint(trimmed.to_owned()),
        );
    }
    Ok(trimmed.to_owned())
}

/// 校验分支名。
///
/// 规则取自 `git check-ref-format` 的关键几条，在**入口**拦住是为了让用户
/// 拿到 `VALIDATION`（前端按错误码给出"分支名不能包含空格"这类提示），
/// 而不是等 git 回一句 `fatal: '...' is not a valid branch name`。
///
/// 参数以数组传递（`-b` 与值分开），因此不存在 shell 注入；这里防的是
/// "看起来成功了但 git 理解了别的东西"。
fn validate_branch_name(name: &str) -> AppResult<()> {
    let invalid = name.is_empty()
        || name.starts_with('-')
        || name.ends_with('/')
        || name.ends_with('.')
        || name.contains("..")
        || name.contains("@{")
        || name.contains("//")
        || name == "@"
        || name
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || "~^:?*[\\".contains(c));

    if invalid {
        return Err(
            AppError::new(ErrorCode::Validation, "the branch name is not valid")
                .with_hint(name.to_owned()),
        );
    }
    Ok(())
}

/// 解析 `.gitignore` 模板 id。
fn parse_gitignore_template(raw: &str) -> AppResult<GitignoreTemplate> {
    GitignoreTemplate::parse(raw).ok_or_else(|| {
        let supported: Vec<&str> = GitignoreTemplate::ALL.iter().map(|t| t.id()).collect();
        AppError::new(ErrorCode::Validation, "unknown .gitignore template")
            .with_hint(supported.join(","))
    })
}

/// 解析许可证模板 id。
fn parse_license_template(raw: &str) -> AppResult<LicenseTemplate> {
    LicenseTemplate::parse(raw).ok_or_else(|| {
        let supported: Vec<&str> = LicenseTemplate::ALL.iter().map(|t| t.id()).collect();
        AppError::new(ErrorCode::Validation, "unknown license template")
            .with_hint(supported.join(","))
    })
}

/// 当前年份（生成 LICENSE 时的默认值）。
fn current_year() -> u32 {
    time::OffsetDateTime::now_utc().year().max(1970) as u32
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use forgedesk_domain::ErrorCode;

    use super::{
        validate_branch_name, validate_remote_url, validated_path, BranchLabelDto, CloneRequest,
        InitRequest,
    };
    use forgedesk_domain::git::BranchLabel;

    #[test]
    fn empty_paths_and_nul_bytes_are_rejected() {
        assert_eq!(
            validated_path("", "path").unwrap_err().code,
            ErrorCode::Validation
        );
        assert_eq!(
            validated_path("   ", "path").unwrap_err().code,
            ErrorCode::Validation
        );
        assert_eq!(
            validated_path("a\0b", "path").unwrap_err().code,
            ErrorCode::Validation
        );
        // 前后空白会被裁掉：从输入框粘贴路径时很容易带上
        assert_eq!(
            validated_path("  /tmp/x  ", "path")
                .unwrap()
                .to_string_lossy(),
            "/tmp/x"
        );
    }

    #[test]
    fn remote_urls_are_trimmed_and_shape_checked() {
        assert_eq!(
            validate_remote_url("  https://example.com/a.git ").unwrap(),
            "https://example.com/a.git"
        );
        assert_eq!(
            validate_remote_url("").unwrap_err().code,
            ErrorCode::Validation
        );
        assert_eq!(
            validate_remote_url("https://a b/c.git").unwrap_err().code,
            ErrorCode::Validation
        );
        assert_eq!(
            validate_remote_url("https://a\nb").unwrap_err().code,
            ErrorCode::Validation
        );
    }

    #[test]
    fn branch_names_follow_the_git_rules_we_can_check_cheaply() {
        for valid in ["main", "feature/x-1", "release_2.0", "修复/登录"] {
            validate_branch_name(valid).unwrap_or_else(|error| panic!("{valid} 应通过：{error:?}"));
        }

        for invalid in [
            "", "-b", "a b", "a..b", "a@{0}", "a\\b", "a:b", "a?b", "a*b", "a[b", "a~b", "a^b",
            "ends/", "ends.", "a//b", "@",
        ] {
            assert_eq!(
                validate_branch_name(invalid).unwrap_err().code,
                ErrorCode::Validation,
                "{invalid} 应被拒绝"
            );
        }
    }

    #[test]
    fn a_clone_request_becomes_a_spec_with_every_switch_forwarded() {
        let request = CloneRequest {
            login_hint: None,
            url: " https://example.com/a.git ".to_owned(),
            into: " dest ".to_owned(),
            depth: Some(1),
            branch: Some(" release ".to_owned()),
            bare: true,
            recurse_submodules: true,
            single_branch: true,
        };

        let spec = request.into_spec().unwrap();
        assert_eq!(spec.url, "https://example.com/a.git");
        assert_eq!(spec.into.to_string_lossy(), "dest");
        assert_eq!(spec.depth, Some(1));
        assert_eq!(spec.branch.as_deref(), Some("release"));
        assert!(spec.bare);
        assert!(spec.recurse_submodules);
        assert!(spec.single_branch);
    }

    #[test]
    fn a_zero_depth_is_rejected_instead_of_silently_becoming_a_full_clone() {
        let request = CloneRequest {
            login_hint: None,
            url: "https://example.com/a.git".to_owned(),
            into: "dest".to_owned(),
            depth: Some(0),
            branch: None,
            bare: false,
            recurse_submodules: false,
            single_branch: false,
        };
        assert_eq!(request.into_spec().unwrap_err().code, ErrorCode::Validation);
    }

    #[test]
    fn an_empty_branch_string_means_no_branch_rather_than_an_error() {
        let request = CloneRequest {
            login_hint: None,
            url: "https://example.com/a.git".to_owned(),
            into: "dest".to_owned(),
            depth: None,
            branch: Some("   ".to_owned()),
            bare: false,
            recurse_submodules: false,
            single_branch: false,
        };
        assert_eq!(request.into_spec().unwrap().branch, None);
    }

    #[test]
    fn an_init_request_with_templates_is_validated_and_parsed() {
        let request = InitRequest {
            path: "/tmp/new-repo".to_owned(),
            initial_branch: Some("trunk".to_owned()),
            bare: false,
            gitignore: Some("Rust".to_owned()),
            license: Some("mit".to_owned()),
            license_holder: Some("Ada".to_owned()),
            license_year: Some(2026),
        };

        let (path, spec, extras) = request.into_parts().unwrap();
        assert_eq!(path.to_string_lossy(), "/tmp/new-repo");
        assert_eq!(spec.initial_branch.as_deref(), Some("trunk"));
        assert!(!spec.bare);
        assert_eq!(
            extras.gitignore.unwrap().id(),
            "rust",
            "id 解析应大小写不敏感"
        );
        let license = extras.license.unwrap();
        assert_eq!(license.template.id(), "MIT");
        assert_eq!(license.year, 2026);
        assert_eq!(license.holder, "Ada");
    }

    #[test]
    fn unknown_template_ids_are_rejected_and_list_the_supported_ones() {
        let request = InitRequest {
            path: "/tmp/new-repo".to_owned(),
            initial_branch: None,
            bare: false,
            gitignore: Some("cobol".to_owned()),
            license: None,
            license_holder: None,
            license_year: None,
        };
        let error = request.into_parts().unwrap_err();
        assert_eq!(error.code, ErrorCode::Validation);
        assert_eq!(error.hint.as_deref(), Some("rust,node,python,go,java"));

        let request = InitRequest {
            path: "/tmp/new-repo".to_owned(),
            initial_branch: None,
            bare: false,
            gitignore: None,
            license: Some("WTFPL".to_owned()),
            license_holder: None,
            license_year: None,
        };
        let error = request.into_parts().unwrap_err();
        assert_eq!(error.code, ErrorCode::Validation);
        assert_eq!(error.hint.as_deref(), Some("MIT,Apache-2.0,BSD-3-Clause"));
    }

    #[test]
    fn the_license_year_defaults_to_the_current_year() {
        let request = InitRequest {
            path: "/tmp/new-repo".to_owned(),
            initial_branch: None,
            bare: false,
            gitignore: None,
            license: Some("MIT".to_owned()),
            license_holder: None,
            license_year: None,
        };
        let (_, _, extras) = request.into_parts().unwrap();
        assert!(extras.license.unwrap().year >= 2024);
    }

    #[test]
    fn branch_labels_map_to_a_tagged_dto() {
        assert_eq!(
            BranchLabelDto::from(BranchLabel::Unborn),
            BranchLabelDto::Unborn
        );
        assert_eq!(
            BranchLabelDto::from(BranchLabel::Detached),
            BranchLabelDto::Detached
        );
        assert_eq!(
            serde_json::to_value(BranchLabelDto::from(BranchLabel::Named("main"))).unwrap(),
            serde_json::json!({ "kind": "named", "name": "main" })
        );
        assert_eq!(
            serde_json::to_value(BranchLabelDto::Unborn).unwrap(),
            serde_json::json!({ "kind": "unborn" })
        );
    }
}
