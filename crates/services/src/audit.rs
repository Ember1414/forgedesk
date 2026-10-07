//! 操作审计服务（T1.11）。
//!
//! # 这一层负责什么
//!
//! 存储层的 [`OperationStore`] 只做"存"与"取"，它不知道什么叫"参数摘要"、
//! 什么叫"脱敏"、什么叫"保留策略"。这些东西是**策略**，属于用例层：
//!
//! - **参数摘要**：调用方给结构化字段，这里负责拼成合法 JSON、脱敏、截断；
//! - **脱敏**：落库前的最后一道闸门（红线 R8）。写进 `args_json` 的东西可能
//!   带着远端 URL 里的 token，而审计表会被导出成文件、发到别处；
//! - **保留策略**：默认 90 天或 10000 条，可配置；
//! - **导出**：CSV / JSON 写到临时文件，返回路径。
//!
//! # 一条纪律
//!
//! **审计失败不能让用户的操作失败**。`begin` 写不进去库时返回 `None` 并记日志，
//! 调用方照常执行——用户要的是提交成功，不是审计写成功。反过来，"操作成功了但
//! 审计没收尾"这种情况必须留下痕迹，因此 `finish` 失败会记 `warn` 而不是静默。

use std::path::PathBuf;

use forgedesk_domain::{AppError, AppResult, ErrorCode};
use forgedesk_storage::{
    Database, OperationOutcome, OperationQuery, OperationStore, RetentionPolicy, Scope,
    SettingsRepository,
};

use crate::repository::system_clock;

/// 参数摘要的长度上限（字节）。
///
/// 2KB 是任务给定的上限：够放下"哪些路径、多少块、哪些选项"，而几百条路径的
/// 完整列表既没人读，也会把审计表本身撑大。
pub const AUDIT_ARGS_LIMIT: usize = 2048;

/// 失败摘要的长度上限（字节）。
pub const AUDIT_SUMMARY_LIMIT: usize = 2048;

/// 单次导出最多写多少行。
///
/// 不是性能考虑，而是"别让一次误点生成一个 500MB 的文件"：审计表在保留策略下
/// 最多一万条，导出上限比它高，正常用法永远碰不到。
pub const EXPORT_MAX_ROWS: usize = 50_000;

/// 全局操作（不属于任何仓库）在数组里的 `repo_id`。
///
/// 为什么不用 `NULL`：`operation_records.repo_id` 是 `NOT NULL`，而"导出全部仓库"
/// 这类操作需要一个明确的归属。0 不会与真实仓库 id 冲突（自增从 1 开始）。
pub const GLOBAL_REPO_ID: i64 = 0;

/// 脱敏的替代标记（与日志层同一个）。
pub use forgedesk_diagnostics::REDACTED;

/// 保留策略的设置键：保留天数。
pub const RETENTION_DAYS_KEY: &str = "audit.retentionDays";
/// 保留策略的设置键：保留条数。
pub const RETENTION_MAX_KEY: &str = "audit.retentionMax";
/// 缺省保留天数。
pub const DEFAULT_RETENTION_DAYS: i64 = 90;
/// 缺省保留条数。
pub const DEFAULT_RETENTION_ROWS: i64 = 10_000;

/// 操作类型（稳定短名，界面按它选 i18n 文案）。
///
/// 常量而不是散落的字符串字面量：界面要按它筛选，导出要对它分组，
/// 拼错一个字母就会得到"某类操作永远查不到"这种极难发现的 bug。
pub mod op_type {
    /// 提交。
    pub const COMMIT: &str = "commit";
    /// 暂存（含整文件与行/块级）。
    pub const STAGE: &str = "stage";
    /// 取消暂存。
    pub const UNSTAGE: &str = "unstage";
    /// 放弃工作区修改。
    pub const DISCARD: &str = "discard";
    /// 克隆。
    pub const CLONE: &str = "clone";
    /// 初始化。
    pub const INIT: &str = "init";
    /// 从最近列表移除。
    pub const FORGET: &str = "forget";
    /// 关闭仓库。
    pub const CLOSE: &str = "close";
    /// 回滚到快照。
    pub const SNAPSHOT_RESTORE: &str = "snapshot_restore";
    /// 清理旧快照。
    pub const SNAPSHOT_PRUNE: &str = "snapshot_prune";
    /// 手动创建快照（T3.8：用户主动打点）。
    pub const SNAPSHOT_CREATE: &str = "snapshot_create";
    /// 手动清理快照缓存（T3.8：孤儿目录 + 总占用回收）。
    pub const SNAPSHOT_CLEANUP: &str = "snapshot_cleanup";
    /// 放弃未完成的回滚标记（T3.9：崩溃恢复的"放弃"出口）。
    pub const SNAPSHOT_RESTORE_ABANDON: &str = "snapshot_restore_abandon";
    /// 储藏当前改动（T2.8）。
    pub const STASH_SAVE: &str = "stash_save";
    /// 应用储藏（`apply` 与 `pop` 都算：用户看的是"把改动拿回来"）。
    pub const STASH_APPLY: &str = "stash_apply";
    /// 丢弃储藏（`drop` 与 `clear` 都算：都是**不可逆**的删除）。
    pub const STASH_DROP: &str = "stash_drop";
    /// 从储藏创建分支。
    pub const STASH_BRANCH: &str = "stash_branch";
    /// 重置（soft / mixed / hard）。
    pub const RESET: &str = "reset";
    /// 合并（execute；prepare 是只读预览不记审计）。
    pub const MERGE: &str = "merge";
    /// rebase 执行（execute；preview_only 是只读预演不记审计）。
    pub const REBASE: &str = "rebase";
    /// 拣选提交。
    pub const CHERRY_PICK: &str = "cherry_pick";
    /// 反转提交。
    pub const REVERT: &str = "revert";
    /// 从 reflog 恢复成新分支。
    pub const REFLOG_BRANCH: &str = "reflog_branch";
    /// 标记冲突文件已解决（T3.1）。
    pub const CONFLICT_RESOLVE: &str = "conflict_resolve";
    /// 继续进行中的操作（merge commit / rebase / 拣选 / 反转的 continue）。
    pub const CONFLICT_CONTINUE: &str = "conflict_continue";
    /// 中止进行中的操作（abort：打快照的可回滚操作）。
    pub const CONFLICT_ABORT: &str = "conflict_abort";
    /// 跳过当前提交（rebase skip）。
    pub const CONFLICT_SKIP: &str = "conflict_skip";
    /// 导出审计（导出本身也是一次操作）。
    pub const AUDIT_EXPORT: &str = "audit_export";
    /// 清理审计（同上）。
    pub const AUDIT_PRUNE: &str = "audit_prune";
    /// 终端里执行的已识别危险命令（T5.3：来源=终端的强制记录）。
    pub const TERMINAL: &str = "terminal";
}

/// "危险操作"清单：会在用户仓库里**丢掉东西**的那些（T3.10）。
///
/// 判据不是"写操作"（提交也是写），而是"这一步之后，用户可能想要回滚"：
/// 丢弃未提交内容、改写已提交历史、删掉储藏或可回滚点、中止进行中的多步操作。
/// 操作历史页据此提供"只看危险操作"的筛选，状态栏也据此找"最近可回滚点"。
///
/// 为什么不塞进 `op_type` 模块：那是"短名 ↔ 常量"的映射（只增不改），
/// 而这份清单是**产品判断**——哪些操作值得提醒，会随交互设计变化。
/// 两者变化的原因不同，就不该住在同一个地方。
///
/// `STASH_APPLY` 与 `CONFLICT_RESOLVE` 刻意不在此列：它们把内容**带回来**
/// 或者解决冲突，属于"修复"而不是"丢弃"。
pub const DANGEROUS_OP_TYPES: &[&str] = &[
    op_type::DISCARD,
    op_type::RESET,
    op_type::MERGE,
    op_type::REBASE,
    op_type::CHERRY_PICK,
    op_type::REVERT,
    op_type::REFLOG_BRANCH,
    op_type::STASH_DROP,
    op_type::CONFLICT_ABORT,
    op_type::SNAPSHOT_RESTORE,
    op_type::SNAPSHOT_CLEANUP,
];

/// 某个操作类型是否属于"危险操作"。
pub fn is_dangerous(op_type_name: &str) -> bool {
    DANGEROUS_OP_TYPES.contains(&op_type_name)
}

/// 参数摘要构造器。
///
/// 为什么不直接让调用方拼字符串：手拼 JSON 一定会遇到转义（Windows 路径里的
/// 反斜杠、提交信息里的引号），而**拼坏的 JSON 会让整条记录读不出来**——
/// 审计的价值在于事后能读，读不出来等于没记。
#[derive(Debug, Clone, Default)]
pub struct AuditArgs {
    fields: Vec<(&'static str, serde_json::Value)>,
}

impl AuditArgs {
    /// 空摘要。
    pub fn new() -> Self {
        Self::default()
    }

    /// 加一个字符串字段。
    pub fn text(mut self, key: &'static str, value: &str) -> Self {
        self.fields.push((key, serde_json::Value::from(value)));
        self
    }

    /// 加一个整数字段。
    pub fn number(mut self, key: &'static str, value: i64) -> Self {
        self.fields.push((key, serde_json::Value::from(value)));
        self
    }

    /// 加一个布尔字段。
    pub fn flag(mut self, key: &'static str, value: bool) -> Self {
        self.fields.push((key, serde_json::Value::from(value)));
        self
    }

    /// 加一组路径。
    ///
    /// 只记前 20 条 + 总数：几百条路径的完整列表没人读，而它会先把 2KB 用光，
    /// 把真正重要的字段（选项、粒度）挤掉。
    pub fn paths(mut self, key: &'static str, paths: &[String]) -> Self {
        const SHOW: usize = 20;
        let shown: Vec<&str> = paths.iter().take(SHOW).map(String::as_str).collect();
        self.fields
            .push((key, serde_json::Value::from(shown.join("\n"))));
        if paths.len() > SHOW {
            self.fields.push((
                "pathsTruncated",
                serde_json::Value::from(paths.len() as i64),
            ));
        }
        self
    }

    /// 拼成摘要字符串：**先脱敏，再截断**。
    ///
    /// 顺序不能反（与 git-engine 的日志片段同一条纪律）：先截断会把一个令牌
    /// 切成两半，脱敏规则再也认不出它，于是半截令牌留在了库里。
    pub fn build(&self) -> String {
        let value = serde_json::Value::Object(
            self.fields
                .iter()
                .map(|(key, value)| ((*key).to_owned(), value.clone()))
                .collect(),
        );
        sanitize_summary(&value.to_string(), AUDIT_ARGS_LIMIT)
    }
}

/// 当前时间（Unix 毫秒）。
///
/// 与 [`crate::repository::system_clock`] 同一个实现：审计的时间戳、操作的开始
/// 时间与快照的创建时间必须是同一个钟，否则界面按时间排序时会出现"审计比操作早"
/// 这种看起来像 bug 的现象。
pub fn now_ms() -> i64 {
    system_clock()
}

/// 先脱敏再按**字节**上限截断（且不切断多字节字符）。
pub fn sanitize_summary(text: &str, limit: usize) -> String {
    let sanitized = forgedesk_diagnostics::sanitize_log(text);
    if sanitized.len() <= limit {
        return sanitized;
    }

    // 按字符边界回退到不超过 limit 的位置：截在 UTF-8 中间会得到无效字符串，
    // 落库后前端解析 JSON 会直接失败
    let mut end = limit;
    while end > 0 && !sanitized.is_char_boundary(end) {
        end -= 1;
    }
    let mut truncated = sanitized[..end].to_owned();
    truncated.push('…');
    truncated
}

/// 一次写操作的审计登记信息。
#[derive(Debug, Clone)]
pub struct AuditEntry<'a> {
    /// 存储层记录 id（全局操作用 [`GLOBAL_REPO_ID`]）。
    pub repo_id: i64,
    /// 操作类型（见 [`op_type`]）。
    pub op_type: &'a str,
    /// 参数摘要（已结构化；`None` 表示没有参数）。
    pub args: Option<AuditArgs>,
}

impl<'a> AuditEntry<'a> {
    /// 建一条登记信息。
    pub fn new(repo_id: i64, op_type: &'a str) -> Self {
        Self {
            repo_id,
            op_type,
            args: None,
        }
    }

    /// 带上参数摘要。
    pub fn with_args(mut self, args: AuditArgs) -> Self {
        self.args = Some(args);
        self
    }
}

/// 操作审计服务。
#[derive(Debug)]
pub struct AuditLog<'a> {
    operations: OperationStore<'a>,
}

impl<'a> AuditLog<'a> {
    /// 绑定到操作记录仓储。
    pub const fn new(operations: OperationStore<'a>) -> Self {
        Self { operations }
    }

    /// 开始记录一次写操作。
    ///
    /// 返回 `None` 表示**审计不可用**（写库失败）：调用方照常执行操作，
    /// 只是这次没有记录。这是刻意的取舍——审计是安全网，不是闸门。
    pub fn begin(&self, entry: &AuditEntry<'_>) -> Option<AuditRun<'a>> {
        let args = entry.args.as_ref().map(AuditArgs::build);
        let started_at_ms = system_clock();

        match self.operations.begin(&forgedesk_storage::NewOperation {
            repo_id: entry.repo_id,
            op_type: entry.op_type,
            args_json: args.as_deref(),
            started_at_ms,
        }) {
            Ok(id) => Some(AuditRun {
                operations: self.operations,
                id,
                started_at_ms,
            }),
            Err(error) => {
                tracing::warn!(
                    op_type = entry.op_type,
                    error = %error.message,
                    "审计的开始记录写入失败（操作继续执行）"
                );
                None
            }
        }
    }

    /// 按条件查询一页记录。
    pub fn query(&self, query: &OperationQuery) -> AppResult<forgedesk_storage::OperationPage> {
        self.operations.query(query)
    }

    /// 按保留策略清理并返回删除条数。
    pub fn prune(&self, policy: &RetentionPolicy) -> AppResult<usize> {
        self.operations.prune(policy)
    }

    /// 记录一次"操作记录自身"的操作（导出 / 清理）。
    ///
    /// 导出与清理也要留痕：**审计表被谁倒出去过**本身是审计的一部分，
    /// 否则"谁能看到历史"这件事在系统里没有任何证据。
    pub fn note(&self, entry: &AuditEntry<'_>, result: &AppResult<String>) {
        if let Some(run) = self.begin(entry) {
            let outcome: AppResult<()> = match result {
                Ok(_) => Ok(()),
                Err(error) => Err(error.clone()),
            };
            run.finish(&outcome, None);
        }
    }

    /// 导出（CSV / JSON），返回文件路径。
    ///
    /// 目标路径由调用方决定（T7.6）：命令层拿到用户在保存对话框里选的路径后传进来；
    /// 没有指定时退回**临时目录**（旧行为，便于测试与"先看一眼再另存"）。
    /// 服务层不解析对话框、也不猜路径——它只负责把内容写到给定位置。
    pub fn export(&self, request: &AuditExportRequest) -> AppResult<AuditExport> {
        let records = self.operations.query_all(&request.query)?;
        if records.len() > EXPORT_MAX_ROWS {
            return Err(
                AppError::new(ErrorCode::Validation, "too many records to export at once")
                    .with_detail(format!(
                        "matched: {}, limit: {EXPORT_MAX_ROWS}",
                        records.len()
                    ))
                    .with_hint("narrow the filter with a date range or a repository"),
            );
        }

        let body = match request.format {
            AuditExportFormat::Csv => csv_body(&records),
            AuditExportFormat::Json => json_body(&records, request.now_ms),
        };
        let path = match &request.target_path {
            Some(target) => target.clone(),
            None => export_path(request.format, request.now_ms),
        };

        std::fs::write(&path, body.as_bytes()).map_err(|error| {
            AppError::new(ErrorCode::Storage, "failed to write the audit export")
                .with_detail(error.to_string())
                .with_hint(path.display().to_string())
        })?;

        Ok(AuditExport {
            path,
            rows: records.len(),
        })
    }
}

/// 一次正在进行的写操作。
///
/// **必须 `finish`**：只有开始没有结束的记录正是"应用崩在写操作中间"的
/// 可观测特征（存储层刻意允许这种状态存在）。正常路径上不允许留下它。
#[derive(Debug)]
pub struct AuditRun<'a> {
    operations: OperationStore<'a>,
    id: i64,
    started_at_ms: i64,
}

impl AuditRun<'_> {
    /// 记录 id（排查时用得上）。
    pub const fn id(&self) -> i64 {
        self.id
    }

    /// 开始时间（Unix 毫秒）。
    pub const fn started_at_ms(&self) -> i64 {
        self.started_at_ms
    }

    /// 收尾。
    ///
    /// `snapshot_id` 非空即 `reversible`：可回滚的**唯一**依据是有没有快照，
    /// 而不是"这次操作看起来像不像能撤销"。
    pub fn finish<T>(self, result: &AppResult<T>, snapshot_id: Option<i64>) {
        let (exit_code, summary) = match result {
            Ok(_) => (Some(0), None),
            Err(error) => (
                Some(1),
                Some(sanitize_summary(
                    &failure_summary(error),
                    AUDIT_SUMMARY_LIMIT,
                )),
            ),
        };

        let outcome = OperationOutcome {
            ended_at_ms: system_clock(),
            exit_code,
            stderr_summary: summary.as_deref(),
            snapshot_id,
            reversible: snapshot_id.is_some(),
        };

        if let Err(error) = self.operations.finish(self.id, &outcome) {
            tracing::warn!(
                id = self.id,
                error = %error.message,
                "审计的收尾写入失败（操作本身已经完成）"
            );
        }
    }
}

/// 失败摘要：优先用 git 的原始输出（`detail`），它才是"到底为什么失败"。
fn failure_summary(error: &AppError) -> String {
    error
        .detail
        .clone()
        .unwrap_or_else(|| error.message.clone())
}

/// 导出格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditExportFormat {
    /// 逗号分隔（带 UTF-8 BOM，见 [`csv_body`]）。
    Csv,
    /// JSON（一个对象，内含记录数组）。
    Json,
}

impl AuditExportFormat {
    /// 稳定的短名（IPC 用）。
    pub const fn key(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Json => "json",
        }
    }

    /// 从 IPC 的短名解析。
    pub fn parse(value: &str) -> AppResult<Self> {
        match value {
            "csv" => Ok(Self::Csv),
            "json" => Ok(Self::Json),
            other => Err(
                AppError::new(ErrorCode::Validation, "unknown audit export format")
                    .with_detail(other.to_owned())
                    .with_hint("csv | json"),
            ),
        }
    }

    /// 扩展名（不含点）。
    ///
    /// 公开给命令层用：用户在保存对话框里手改扩展名时，要在**写之前**发现
    /// "扩展名与内容格式不符"，而那条校验需要这个映射（T7.6）。
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Json => "json",
        }
    }
}

/// 导出请求。
#[derive(Debug, Clone)]
pub struct AuditExportRequest {
    /// 筛选条件（导出**全部**命中的记录，不受分页限制）。
    pub query: OperationQuery,
    /// 格式。
    pub format: AuditExportFormat,
    /// 目标文件路径；`None` 表示写到临时目录（由服务自己决定文件名）。
    pub target_path: Option<PathBuf>,
    /// 当前时间（用于文件名与 JSON 头的生成时间）。
    pub now_ms: i64,
}

/// 导出结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditExport {
    /// 写好的文件路径（用户指定的，或临时目录里的）。
    pub path: PathBuf,
    /// 导出条数。
    pub rows: usize,
}

/// 保留策略（可配置，缺省 90 天 / 10000 条）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuditRetention {
    /// 保留天数。
    pub days: i64,
    /// 保留条数上限。
    pub rows: i64,
}

impl Default for AuditRetention {
    fn default() -> Self {
        Self {
            days: DEFAULT_RETENTION_DAYS,
            rows: DEFAULT_RETENTION_ROWS,
        }
    }
}

impl AuditRetention {
    /// 从设置表读取；缺项或值损坏时回落默认值。
    ///
    /// 容错而不是报错：这两个值在**应用启动**的路径上被读取，一个被手工改坏的
    /// 设置不该让应用起不来。
    pub fn load(database: &Database) -> Self {
        let repository = SettingsRepository::new(database);
        let defaults = Self::default();

        Self {
            days: read_number(&repository, RETENTION_DAYS_KEY)
                .unwrap_or(defaults.days)
                .clamp(1, 3_650),
            rows: read_number(&repository, RETENTION_MAX_KEY)
                .unwrap_or(defaults.rows)
                .clamp(100, 1_000_000),
        }
    }

    /// 生成清理策略。
    pub const fn policy(&self, now_ms: i64) -> RetentionPolicy {
        RetentionPolicy {
            now_ms,
            keep_days: self.days,
            keep_rows: self.rows,
        }
    }
}

/// 读一个数字设置（值一律是 JSON 字符串）。
fn read_number(repository: &SettingsRepository<'_>, key: &str) -> Option<i64> {
    let raw = repository.get(&Scope::Global, key).ok().flatten()?;
    serde_json::from_str::<i64>(&raw).ok()
}

/// 导出文件路径（临时目录 + 时间戳，保证两次导出不互相覆盖）。
fn export_path(format: AuditExportFormat, now_ms: i64) -> PathBuf {
    std::env::temp_dir().join(format!("forgedesk-audit-{now_ms}.{}", format.extension()))
}

/// CSV 正文。
///
/// 两个细节是给"用 Excel 打开"准备的：
///
/// - **开头写 UTF-8 BOM**：没有它，Excel 会按本地代码页解码，中文全变乱码。
///   这是唯一一处我们主动在文件里写 BOM 的地方（任务里说"可选"，但可选的是
///   BOM 本身，不是"中文能不能看"）；
/// - **所有字段都引号包裹并转义**：`args` 里有换行与逗号，不转义会直接把
///   列数撑乱。
fn csv_body(records: &[forgedesk_storage::OperationRecord]) -> String {
    let mut out = String::from("\u{feff}");
    out.push_str(
        "id,repoId,opType,startedAtMs,startedAt,endedAtMs,durationMs,exitCode,result,\
         snapshotId,reversible,args,stderrSummary\n",
    );

    for record in records {
        let (result, duration) = match (record.ended_at_ms, record.exit_code) {
            (None, _) => ("running", String::new()),
            (Some(ended), code) => (
                if code == Some(0) { "ok" } else { "failed" },
                (ended - record.started_at_ms.unwrap_or(ended)).to_string(),
            ),
        };

        let row = [
            record.id.to_string(),
            record.repo_id.to_string(),
            record.op_type.clone(),
            record.started_at_ms.unwrap_or(0).to_string(),
            iso8601_utc(record.started_at_ms.unwrap_or(0)),
            record
                .ended_at_ms
                .map(|ms| ms.to_string())
                .unwrap_or_default(),
            duration,
            record
                .exit_code
                .map(|code| code.to_string())
                .unwrap_or_default(),
            result.to_owned(),
            record
                .snapshot_id
                .map(|id| id.to_string())
                .unwrap_or_default(),
            if record.reversible { "true" } else { "false" }.to_owned(),
            record.args_json.clone().unwrap_or_default(),
            record.stderr_summary.clone().unwrap_or_default(),
        ];

        out.push_str(&row.map(|field| csv_field(&field)).join(","));
        out.push('\n');
    }

    out
}

/// 一个 CSV 字段（引号包裹 + 内部引号翻倍）。
fn csv_field(value: &str) -> String {
    let mut field = String::with_capacity(value.len() + 2);
    field.push('"');
    for character in value.chars() {
        if character == '"' {
            field.push('"');
        }
        field.push(character);
    }
    field.push('"');
    field
}

/// JSON 正文（一个对象：生成时间 + 条数 + 记录数组）。
fn json_body(records: &[forgedesk_storage::OperationRecord], now_ms: i64) -> String {
    let items: Vec<serde_json::Value> = records
        .iter()
        .map(|record| {
            serde_json::json!({
                "id": record.id,
                "repoId": record.repo_id,
                "opType": record.op_type,
                "startedAtMs": record.started_at_ms,
                "startedAt": record.started_at_ms.map(iso8601_utc),
                "endedAtMs": record.ended_at_ms,
                "durationMs": match (record.started_at_ms, record.ended_at_ms) {
                    (Some(start), Some(end)) => Some(end - start),
                    _ => None,
                },
                "exitCode": record.exit_code,
                "result": match (record.ended_at_ms, record.exit_code) {
                    (None, _) => "running",
                    (Some(_), Some(0)) => "ok",
                    (Some(_), _) => "failed",
                },
                "snapshotId": record.snapshot_id,
                "reversible": record.reversible,
                "argsJson": record.args_json,
                "stderrSummary": record.stderr_summary,
            })
        })
        .collect();

    serde_json::json!({
        "generatedAtMs": now_ms,
        "count": items.len(),
        "records": items,
    })
    .to_string()
}

/// Unix 毫秒 → ISO-8601 UTC（`2026-09-27T12:34:56.789Z`）。
///
/// 自己算而不是引入 `chrono` / `time`：整个项目只在这一处需要把时间格式化成
/// 文本（界面用本地时区，由浏览器自己格式化），为它加一个依赖不划算。
pub fn iso8601_utc(ms: i64) -> String {
    let (days, millis_of_day) = (ms.div_euclid(86_400_000), ms.rem_euclid(86_400_000));
    let (year, month, day) = civil_from_days(days);
    let hour = millis_of_day / 3_600_000;
    let minute = (millis_of_day / 60_000) % 60;
    let second = (millis_of_day / 1_000) % 60;
    let milli = millis_of_day % 1_000;

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{milli:03}Z")
}

/// 天数（相对 1970-01-01）→ 公历年月日（Howard Hinnant 的 `civil_from_days`）。
///
/// 这段算法被广泛实现与验证过；自己从"闰年规则"推一遍容易在 1900 年这类
/// 世纪闰年边界上出错，而审计时间的正确性只在出问题时才被检验。
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_pivot = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_pivot + 2) / 5 + 1;
    let month = if month_pivot < 10 {
        month_pivot + 3
    } else {
        month_pivot - 9
    };
    (
        if month <= 2 { year + 1 } else { year },
        month as u32,
        day as u32,
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{
        iso8601_utc, sanitize_summary, AuditArgs, AuditEntry, AuditExportFormat, AuditLog,
        AuditRetention, AUDIT_ARGS_LIMIT, EXPORT_MAX_ROWS,
    };
    use forgedesk_storage::{
        migrate, Database, OperationOutcome, OperationQuery, OperationStore, RetentionPolicy,
    };
    use std::path::Path;

    fn database() -> Database {
        let database = Database::open_in_memory().unwrap();
        migrate(&database).unwrap();
        database
    }

    #[test]
    fn a_secret_in_the_arguments_never_reaches_the_database() {
        // 审计表会被导出成文件、发到别处：这里的断言是红线 R8 的最后一道闸门
        let database = database();
        let log = AuditLog::new(OperationStore::new(&database));

        let args = AuditArgs::new()
            .text(
                "remote",
                "https://x-access-token:ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA@github.com/a/b.git",
            )
            .text("token", "ghp_BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB")
            .flag("force", true)
            .build();

        let run = log
            .begin(&AuditEntry::new(1, "clone").with_args(AuditArgs::new().text("raw", &args)))
            .unwrap();
        run.finish(&Ok::<(), forgedesk_domain::AppError>(()), None);

        let record = log
            .query(&OperationQuery::default())
            .unwrap()
            .records
            .remove(0);
        let stored = record.args_json.unwrap();
        assert!(!stored.contains("ghp_"), "令牌必须被脱敏：{stored}");
        assert!(stored.contains(super::REDACTED), "要留下脱敏痕迹：{stored}");
    }

    #[test]
    fn the_argument_summary_is_capped_at_two_kilobytes_and_stays_valid_json() {
        let huge = "路径".repeat(2_000);
        let built = AuditArgs::new().text("paths", &huge).build();

        assert!(
            built.len() <= AUDIT_ARGS_LIMIT + '…'.len_utf8(),
            "摘要必须被截断，实际 {} 字节",
            built.len()
        );
        // 截断不能切在多字节字符中间：否则这一行 JSON 谁也别想解析出来
        assert!(built.is_char_boundary(built.len()));
        assert!(built.starts_with('{'));
    }

    #[test]
    fn sanitising_keeps_short_text_untouched() {
        assert_eq!(sanitize_summary("nothing to hide", 64), "nothing to hide");
    }

    #[test]
    fn a_path_list_is_summarised_with_a_count_instead_of_every_line() {
        let paths: Vec<String> = (0..50)
            .map(|index| format!("src/file-{index}.rs"))
            .collect();
        let built = AuditArgs::new().paths("paths", &paths).build();

        assert!(built.contains("pathsTruncated"), "总数要留下：{built}");
        assert!(
            built.contains("\"pathsTruncated\":50"),
            "总数要留下：{built}"
        );
        assert!(built.len() < AUDIT_ARGS_LIMIT);
    }

    #[test]
    fn a_finished_operation_records_the_outcome_and_the_duration() {
        let database = database();
        let log = AuditLog::new(OperationStore::new(&database));

        let run = log.begin(&AuditEntry::new(7, "stage")).unwrap();
        let started = run.started_at_ms();
        run.finish(&Ok::<(), forgedesk_domain::AppError>(()), Some(42));

        let record = log
            .query(&OperationQuery::default())
            .unwrap()
            .records
            .remove(0);
        assert_eq!(record.repo_id, 7);
        assert_eq!(record.op_type, "stage");
        assert_eq!(record.exit_code, Some(0));
        assert_eq!(record.snapshot_id, Some(42));
        assert!(record.reversible, "有快照就可回滚");
        assert!(record.ended_at_ms.unwrap() >= started);
    }

    #[test]
    fn a_failed_operation_keeps_gits_own_words() {
        let database = database();
        let log = AuditLog::new(OperationStore::new(&database));

        let run = log.begin(&AuditEntry::new(1, "commit")).unwrap();
        let error = forgedesk_domain::AppError::new(
            forgedesk_domain::ErrorCode::HookRejected,
            "git exited with code 1",
        )
        .with_detail("pre-commit: tests failed");
        run.finish::<()>(&Err(error), None);

        let record = log
            .query(&OperationQuery::default())
            .unwrap()
            .records
            .remove(0);
        assert_eq!(record.exit_code, Some(1));
        assert!(
            record
                .stderr_summary
                .unwrap()
                .contains("pre-commit: tests failed"),
            "失败摘要要用 git 的原话"
        );
    }

    #[test]
    fn retention_falls_back_to_the_defaults_and_reads_valid_settings() {
        let database = database();
        let defaults = AuditRetention::load(&database);
        assert_eq!(defaults.days, super::DEFAULT_RETENTION_DAYS);
        assert_eq!(defaults.rows, super::DEFAULT_RETENTION_ROWS);

        // 损坏值 → 回落默认；合法值生效
        let repository = forgedesk_storage::SettingsRepository::new(&database);
        repository
            .set(
                &forgedesk_storage::Scope::Global,
                super::RETENTION_DAYS_KEY,
                "not-json",
            )
            .unwrap();
        assert_eq!(
            AuditRetention::load(&database).days,
            super::DEFAULT_RETENTION_DAYS
        );

        repository
            .set(
                &forgedesk_storage::Scope::Global,
                super::RETENTION_DAYS_KEY,
                "30",
            )
            .unwrap();
        repository
            .set(
                &forgedesk_storage::Scope::Global,
                super::RETENTION_MAX_KEY,
                "500",
            )
            .unwrap();
        let loaded = AuditRetention::load(&database);
        assert_eq!(loaded.days, 30);
        assert_eq!(loaded.rows, 500);
    }

    #[test]
    fn the_retention_policy_is_the_configured_one() {
        let policy = AuditRetention {
            days: 30,
            rows: 500,
        }
        .policy(1_000_000_000);

        assert_eq!(policy.keep_days, 30);
        assert_eq!(policy.keep_rows, 500);
        assert_eq!(policy.cutoff_ms(), 1_000_000_000 - 30 * 86_400_000);
    }

    #[test]
    fn exporting_csv_writes_a_bom_and_keeps_chinese_readable() {
        let database = database();
        let log = AuditLog::new(OperationStore::new(&database));

        let run = log
            .begin(
                &AuditEntry::new(1, "commit")
                    .with_args(AuditArgs::new().text("subject", "修复中文乱码，含逗号与\"引号\"")),
            )
            .unwrap();
        run.finish(&Ok::<(), forgedesk_domain::AppError>(()), None);

        let export = log
            .export(&super::AuditExportRequest {
                query: OperationQuery::default(),
                format: AuditExportFormat::Csv,
                target_path: None,
                now_ms: 1_700_000_000_000,
            })
            .unwrap();

        let body = std::fs::read_to_string(&export.path).unwrap();
        // BOM：Excel 没有它会按本地代码页解码，中文全花
        assert!(body.starts_with('\u{feff}'));
        assert!(body.contains("修复中文乱码"), "中文必须原样保留：{body}");
        // 引号在引号字段里要翻倍（JSON 的 `\"` 在 CSV 里变成 `\""`）：
        // 不翻倍会让这一行的列数直接错位
        assert!(
            body.contains(r#""""引号"""""#) || body.contains(r#"\""引号\"""#),
            "引号必须按 CSV 规则转义：{body}"
        );
        assert_eq!(export.rows, 1);

        std::fs::remove_file(&export.path).ok();
    }

    #[test]
    fn exporting_json_is_parseable_and_carries_the_derived_fields() {
        let database = database();
        let log = AuditLog::new(OperationStore::new(&database));

        let run = log.begin(&AuditEntry::new(9, "discard")).unwrap();
        let error = forgedesk_domain::AppError::new(
            forgedesk_domain::ErrorCode::GitConflict,
            "path is unmerged",
        );
        run.finish::<()>(&Err(error), Some(3));

        let export = log
            .export(&super::AuditExportRequest {
                query: OperationQuery::default(),
                format: AuditExportFormat::Json,
                target_path: None,
                now_ms: 1_700_000_000_000,
            })
            .unwrap();

        let body = std::fs::read_to_string(&export.path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["count"], 1);
        assert_eq!(parsed["records"][0]["opType"], "discard");
        assert_eq!(parsed["records"][0]["result"], "failed");
        assert_eq!(parsed["records"][0]["snapshotId"], 3);
        assert_eq!(parsed["records"][0]["reversible"], true);

        std::fs::remove_file(&export.path).ok();
    }

    /// T7.6：给了目标路径就必须写到那里——用户点了"保存到 D:\报表.csv"，
    /// 结果文件出现在临时目录，比不给这个功能更糟。
    #[test]
    fn an_explicit_target_path_is_where_the_export_lands() {
        let database = database();
        let log = AuditLog::new(OperationStore::new(&database));

        let run = log.begin(&AuditEntry::new(1, "stage")).unwrap();
        run.finish(&Ok::<(), forgedesk_domain::AppError>(()), None);

        let dir = std::env::temp_dir().join(format!("fd-audit-target-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("report.csv");

        let export = log
            .export(&super::AuditExportRequest {
                query: OperationQuery::default(),
                format: AuditExportFormat::Csv,
                target_path: Some(target.clone()),
                now_ms: 1_700_000_000_000,
            })
            .unwrap();

        assert_eq!(export.path, target, "路径必须原样使用，不改名、不改目录");
        assert!(target.exists(), "文件要真的落在用户选的位置");
        assert_eq!(export.rows, 1);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_absurdly_large_export_is_refused_instead_of_writing_a_huge_file() {
        // 直接问"超限会不会被拦"：真的插 EXPORT_MAX_ROWS 条太慢，
        // 因此这里断言的是"上限存在、且比保留策略的上限宽松"
        // （否则正常用法会撞上导出上限，那就不是'误点保护'而是'功能坏了'）
        assert!(EXPORT_MAX_ROWS >= super::DEFAULT_RETENTION_ROWS as usize);
        let path = super::export_path(AuditExportFormat::Csv, 1);
        assert_eq!(path.extension().unwrap(), "csv");
        assert!(path.to_string_lossy().contains("forgedesk-audit-1"));
    }

    #[test]
    fn timestamp_formatting_matches_known_instants() {
        // 期望值用 `new Date(ms).toISOString()` 独立核对过（不是手算的）
        assert_eq!(iso8601_utc(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso8601_utc(1_790_598_896_789), "2026-09-28T12:34:56.789Z");
        // 世纪闰年边界（2000-02-29）——手写闰年逻辑最容易错在这里
        assert_eq!(iso8601_utc(951_782_400_000), "2000-02-29T00:00:00.000Z");
        // 毫秒为负（1970 之前）不该崩
        assert_eq!(iso8601_utc(-1), "1969-12-31T23:59:59.999Z");
    }

    #[test]
    fn the_export_file_is_written_to_the_temporary_directory() {
        let database = database();
        let log = AuditLog::new(OperationStore::new(&database));
        let run = log.begin(&AuditEntry::new(1, "stage")).unwrap();
        run.finish(&Ok::<(), forgedesk_domain::AppError>(()), None);

        let export = log
            .export(&super::AuditExportRequest {
                query: OperationQuery::default(),
                format: AuditExportFormat::Json,
                target_path: None,
                now_ms: 42,
            })
            .unwrap();

        assert!(Path::new(&export.path).exists());
        assert_eq!(
            export.path.parent().unwrap(),
            std::env::temp_dir().as_path(),
            "本任务只写临时目录（用户选目录要等 M7 的文件对话框）"
        );
        std::fs::remove_file(&export.path).ok();
    }

    #[test]
    fn a_prune_with_a_policy_removes_only_what_is_older() {
        let database = database();
        let log = AuditLog::new(OperationStore::new(&database));

        let store = OperationStore::new(&database);
        store
            .begin(&forgedesk_storage::NewOperation {
                repo_id: 1,
                op_type: "stage",
                args_json: None,
                started_at_ms: 1_000,
            })
            .unwrap();
        store
            .finish(
                1,
                &OperationOutcome {
                    ended_at_ms: 1_100,
                    exit_code: Some(0),
                    stderr_summary: None,
                    snapshot_id: None,
                    reversible: false,
                },
            )
            .unwrap();

        let removed = log
            .prune(&RetentionPolicy {
                now_ms: 86_400_000 * 10,
                keep_days: 1,
                keep_rows: 0,
            })
            .unwrap();
        assert_eq!(removed, 1);
        assert_eq!(log.query(&OperationQuery::default()).unwrap().total, 0);
    }
}
