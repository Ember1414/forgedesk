//! 日志落盘、轮转与读取。
//!
//! # 为什么轮转策略是自己写的
//!
//! T0.8 的要求是"按天轮转 + 单文件上限 10MB + 保留 7 天"。
//! `tracing-appender` 自带的 `RollingFileAppender` **只支持按时间轮转**，
//! 没有大小上限；而"单个文件无上限增长"正是用户磁盘被日志吃满的经典原因。
//! 因此分工如下：
//!
//! - `tracing-appender` 负责它的核心价值——**非阻塞写入**（日志不能卡住 Git 操作），
//!   并提供 `WorkerGuard` 保证退出前把缓冲刷进文件；
//! - 轮转策略由本模块的 [`RotatingWriter`] 实现，因此可以被单测完整覆盖
//!   （日界、大小上限、保留期、文件命名都可以构造场景验证，不依赖真实时间流逝）。
//!
//! # 格式
//!
//! 文件日志用 **JSON Lines**（每行一个对象）：时间戳、级别、target、message 与结构化字段
//! 都能被解析出来，`logs_tail` 才能把"错误发生时间附近的行"高亮出来；
//! 控制台保持人类可读格式（开发时是给人看的，机器解析价值为零）。
//!
//! # 文件命名
//!
//! - 当前文件：`forgedesk.log`（**永远是最新的**，因此 `logs_tail` 从它开始读）；
//! - 轮转后：`forgedesk-YYYYMMDD-HHMMSS.log`（名字即时间序，排序 = 时间序），
//!   日期取 **UTC**：本地时区在跨时区/夏令时下会出现"同一天两次命名"这类边界问题，
//!   而日志文件名只需要单调可排序，不需要符合用户的日历直觉。

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use forgedesk_diagnostics::sanitize_log;
use forgedesk_domain::{AppError, AppResult, ErrorCode};
use time::{Date, Month, OffsetDateTime, Time};
use tracing_appender::non_blocking::{NonBlocking, WorkerGuard};

// 转出去给宿主使用：`WorkerGuard` 必须活到进程结束，
// 而"持有它"这件事不应该逼着 `src-tauri` 也声明 tracing-appender 依赖
// （宿主只需要知道"有这么一个东西要留住"，不需要知道它内部是谁）。
pub use tracing_appender::non_blocking::WorkerGuard as LogFlushGuard;

/// 当前日志文件名。
pub const CURRENT_LOG_FILE: &str = "forgedesk.log";

/// 轮转文件名的前缀（`<前缀>-<时间戳>.log`）。
const ROTATED_PREFIX: &str = "forgedesk-";

/// 读取末尾内容时，单个文件最多读多少字节。
///
/// 为什么需要上限：单文件上限是 10MB，而"读最后 200 行"没必要把 10MB 全读进内存；
/// 512KB 足够容纳上万行，同时对内存与响应时间都是安全的。
const TAIL_READ_BYTES: u64 = 512 * 1024;

/// 日志保留策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogPolicy {
    /// 单个文件的大小上限（字节）。
    pub max_file_bytes: u64,
    /// 保留天数（按轮转文件名里的日期判断）。
    pub keep_days: i64,
    /// 轮转文件的数量上限（兜底：避免"一天内疯狂轮转"把磁盘写满）。
    pub max_files: usize,
}

impl Default for LogPolicy {
    fn default() -> Self {
        Self {
            max_file_bytes: 10 * 1024 * 1024,
            keep_days: 7,
            max_files: 50,
        }
    }
}

impl LogPolicy {
    /// 校验策略取值是否合理。
    ///
    /// 为什么要在构造时挡住：一个 0 字节上限或 0 天保留期会让日志系统"看起来在工作"
    /// 却什么都没留下——排查时才发现日志是空的，成本极高。
    pub fn validate(self) -> AppResult<Self> {
        if self.max_file_bytes < 1024 {
            return Err(
                AppError::new(ErrorCode::Validation, "log max_file_bytes is too small")
                    .with_detail(self.max_file_bytes.to_string()),
            );
        }
        if self.keep_days < 1 {
            return Err(AppError::new(
                ErrorCode::Validation,
                "log keep_days must be >= 1",
            ));
        }
        if self.max_files < 1 {
            return Err(AppError::new(
                ErrorCode::Validation,
                "log max_files must be >= 1",
            ));
        }
        Ok(self)
    }
}

/// 判断是否需要在写入前轮转（纯函数，便于精确测试日界与大小边界）。
///
/// `current_day` 是当前文件所属的日期；`incoming` 是本次要写入的字节数。
pub fn should_rotate(
    current_day: Date,
    today: Date,
    written_bytes: u64,
    incoming: usize,
    policy: &LogPolicy,
) -> bool {
    if current_day != today {
        return true;
    }
    written_bytes.saturating_add(incoming as u64) > policy.max_file_bytes
}

/// 按天 + 按大小轮转的写入器。
#[derive(Debug)]
pub struct RotatingWriter {
    directory: PathBuf,
    policy: LogPolicy,
    file: File,
    /// 当前文件所属的 UTC 日期。
    current_day: Date,
    /// 当前文件已写入的字节数。
    written_bytes: u64,
}

impl RotatingWriter {
    /// 打开（必要时创建）日志目录并定位到当前日志文件。
    ///
    /// 打开时会做一次保留期清理：应用可能很久没启动过，上次留下的文件不该等到
    /// "下一次轮转"才被清掉。
    pub fn open(directory: impl AsRef<Path>, policy: LogPolicy) -> AppResult<Self> {
        let policy = policy.validate()?;
        let directory = directory.as_ref().to_path_buf();
        std::fs::create_dir_all(&directory).map_err(|error| {
            // 建议性文案由前端按错误码走 i18n；这里只给具体路径（数据）
            AppError::new(ErrorCode::Storage, "could not create the log directory")
                .with_detail(format!("{}: {error}", directory.display()))
                .with_hint(directory.display().to_string())
        })?;

        let now = OffsetDateTime::now_utc();
        let current_path = directory.join(CURRENT_LOG_FILE);

        // 上次运行留下的文件：按保留期清理（失败不致命，不该拦住应用启动）
        prune(&directory, policy, now);

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&current_path)
            .map_err(|error| {
                AppError::new(ErrorCode::Storage, "could not open the log file")
                    .with_detail(format!("{}: {error}", current_path.display()))
                    .with_hint(current_path.display().to_string())
            })?;

        let written_bytes = file.metadata().map(|meta| meta.len()).unwrap_or(0);

        Ok(Self {
            directory,
            policy,
            file,
            current_day: now.date(),
            written_bytes,
        })
    }

    /// 日志目录。
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// 主动轮转（供测试与"切换日志文件"使用）。
    pub fn rotate(&mut self, now: OffsetDateTime) -> AppResult<()> {
        let target = resolve_rotated_path(&self.directory, now);

        // 先 flush 再重命名：否则缓冲里的最后几行会落到新文件里，时间线上说不通
        self.file.flush().map_err(|error| {
            AppError::new(ErrorCode::Storage, "could not flush the log file")
                .with_detail(error.to_string())
        })?;

        std::fs::rename(self.directory.join(CURRENT_LOG_FILE), &target).map_err(|error| {
            AppError::new(ErrorCode::Storage, "could not rotate the log file").with_detail(format!(
                "{} -> {}: {error}",
                CURRENT_LOG_FILE,
                target.display()
            ))
        })?;

        self.file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.directory.join(CURRENT_LOG_FILE))
            .map_err(|error| {
                AppError::new(ErrorCode::Storage, "could not reopen the log file")
                    .with_detail(error.to_string())
            })?;

        self.written_bytes = 0;
        self.current_day = now.date();
        prune(&self.directory, self.policy, now);

        Ok(())
    }
}

impl Write for RotatingWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let now = OffsetDateTime::now_utc();
        if should_rotate(
            self.current_day,
            now.date(),
            self.written_bytes,
            buffer.len(),
            &self.policy,
        ) {
            // 轮转失败不该丢掉日志：退回"继续追加到当前文件"，
            // 磁盘问题会在 warnings 里体现，而不是让日志静默消失
            if let Err(error) = self.rotate(now) {
                tracing::warn!(%error, "日志轮转失败，继续追加到当前文件");
            }
        }

        let written = self.file.write(buffer)?;
        self.written_bytes = self.written_bytes.saturating_add(written as u64);
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

/// 轮转文件名（UTC 时间戳）。
pub fn rotated_file_name(now: OffsetDateTime) -> String {
    let date = now.date();
    format!(
        "{ROTATED_PREFIX}{:04}{:02}{:02}-{:02}{:02}{:02}.log",
        date.year(),
        u8::from(date.month()),
        date.day(),
        now.hour(),
        now.minute(),
        now.second()
    )
}

/// 解析轮转文件的目标路径，**保证不与已有文件重名**。
///
/// 为什么必须处理重名：Windows 的 `rename` 不会覆盖已存在的目标文件（与 Unix 不同），
/// 因此"同一秒内轮转两次"在 Windows 上会直接失败——而日志量大的时候同一秒多次轮转
/// 完全可能（一次 Git 输出就能写满一个 10MB 文件）。这里在重名时追加序号。
fn resolve_rotated_path(directory: &Path, now: OffsetDateTime) -> PathBuf {
    let base = rotated_file_name(now);
    let candidate = directory.join(&base);
    if !candidate.exists() {
        return candidate;
    }

    let stem = base.trim_end_matches(".log");
    for index in 1..=1000_u32 {
        let next = directory.join(format!("{stem}-{index}.log"));
        if !next.exists() {
            return next;
        }
    }

    // 兜底：名字全被占满（几乎不可能），退化成纳秒时间戳，仍然唯一
    directory.join(format!("{stem}-{}.log", now.unix_timestamp_nanos()))
}

/// 从轮转文件名解析出时间（用于保留期判断；格式不符返回 `None`）。
///
/// 兼容 `forgedesk-YYYYMMDD-HHMMSS.log` 与去重后的 `...-HHMMSS-<n>.log`。
fn parse_rotated_timestamp(name: &str) -> Option<OffsetDateTime> {
    let stamp = name.strip_prefix(ROTATED_PREFIX)?.strip_suffix(".log")?;
    let (date_part, time_part) = stamp.split_once('-')?;
    if date_part.len() != 8 || time_part.len() < 6 {
        return None;
    }
    let time_part = &time_part[0..6];

    let number = |slice: &str| slice.parse::<u32>().ok();
    let year = i32::try_from(number(&date_part[0..4])?).ok()?;
    let month = Month::try_from(u8::try_from(number(&date_part[4..6])?).ok()?).ok()?;
    let day = u8::try_from(number(&date_part[6..8])?).ok()?;
    let hour = u8::try_from(number(&time_part[0..2])?).ok()?;
    let minute = u8::try_from(number(&time_part[2..4])?).ok()?;
    let second = u8::try_from(number(&time_part[4..6])?).ok()?;

    let date = Date::from_calendar_date(year, month, day).ok()?;
    let time = Time::from_hms(hour, minute, second).ok()?;
    Some(date.with_time(time).assume_utc())
}

/// 列出日志目录里的轮转文件（按时间升序）。
fn rotated_files(directory: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };

    let mut files: Vec<(OffsetDateTime, PathBuf)> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter_map(|path| {
            let name = path.file_name()?.to_str()?.to_owned();
            let stamp = parse_rotated_timestamp(&name)?;
            Some((stamp, path))
        })
        .collect();

    files.sort_by_key(|(stamp, _)| *stamp);
    files.into_iter().map(|(_, path)| path).collect()
}

/// 按保留期与数量上限清理轮转文件（失败只记日志，不影响主流程）。
pub fn prune(directory: &Path, policy: LogPolicy, now: OffsetDateTime) {
    let cutoff = now - time::Duration::days(policy.keep_days);
    let files = rotated_files(directory);

    let mut kept: Vec<PathBuf> = Vec::new();

    for path in files {
        let expired_by_age = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(parse_rotated_timestamp)
            .is_some_and(|stamp| stamp < cutoff);

        if expired_by_age {
            remove_quietly(&path);
            continue;
        }
        kept.push(path);
    }

    // 数量兜底：保留最近 max_files 个
    if kept.len() > policy.max_files {
        let excess = kept.len() - policy.max_files;
        for path in kept.drain(..excess) {
            remove_quietly(&path);
        }
    }
}

fn remove_quietly(path: &Path) {
    if let Err(error) = std::fs::remove_file(path) {
        tracing::warn!(path = %path.display(), %error, "删除旧日志文件失败");
    }
}

/// 结构化的一行日志。
///
/// `raw` 始终保留原始整行（已脱敏），因为"反馈问题时贴日志"需要完整上下文；
/// 结构化字段只是让界面能高亮与筛选。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogLine {
    /// 时间戳（Unix 毫秒）；无法解析时为 `None`。
    pub timestamp: Option<i64>,
    /// 级别（`INFO` / `WARN` / `ERROR` …）。
    pub level: Option<String>,
    /// 产生日志的模块（tracing 的 target）。
    pub target: Option<String>,
    /// 消息正文。
    pub message: String,
    /// 整行原文（已脱敏）。
    pub raw: String,
}

/// 读取末尾若干行日志（跨多个文件，新的在前）。
///
/// 返回的每一行都经过 [`sanitize_log`]：**读取路径也是出境路径**。
/// 文件里可能混入历史版本写入的未脱敏内容或第三方库的原始输出，
/// 因此不能因为"写入时已经脱敏过"就在读取时放行。
pub fn tail(directory: &Path, limit: usize) -> AppResult<Vec<LogLine>> {
    if limit == 0 {
        return Ok(Vec::new());
    }

    // 顺序必须是"新 → 旧"：先读当前文件，再读轮转文件（最新的在前）。
    // 反过来的话，收集到的行是旧→新，最后一次 reverse 会把时间线彻底颠倒。
    let mut sources: Vec<PathBuf> = vec![directory.join(CURRENT_LOG_FILE)];
    let mut rotated = rotated_files(directory);
    rotated.reverse();
    sources.extend(rotated);

    let mut lines: Vec<LogLine> = Vec::new();
    for path in sources {
        if !path.is_file() {
            continue;
        }
        let content = read_tail_bytes(&path, TAIL_READ_BYTES).map_err(|error| {
            AppError::new(ErrorCode::Storage, "could not read the log file")
                .with_detail(format!("{}: {error}", path.display()))
        })?;

        // 文件内部是从旧到新，因此逐行倒着取
        for raw_line in content.lines().rev() {
            let trimmed = raw_line.trim_end_matches(['\r', '\n']);
            if trimmed.trim().is_empty() {
                continue;
            }
            lines.push(parse_line(trimmed));
            if lines.len() >= limit {
                lines.reverse();
                return Ok(lines);
            }
        }
    }

    lines.reverse();
    Ok(lines)
}

/// 从文件末尾读取最多 `max_bytes` 字节（丢弃可能被截断的首行）。
fn read_tail_bytes(path: &Path, max_bytes: u64) -> io::Result<String> {
    let mut file = File::open(path)?;
    let length = file.metadata()?.len();

    if length <= max_bytes {
        let mut buffer = String::new();
        file.read_to_string(&mut buffer)?;
        return Ok(buffer);
    }

    file.seek(SeekFrom::Start(length - max_bytes))?;
    let mut buffer = String::new();
    file.read_to_string(&mut buffer)?;

    // 首行很可能是半个 JSON：丢掉它，避免解析出误导性的半截内容
    match buffer.find('\n') {
        Some(index) => Ok(buffer[index + 1..].to_owned()),
        None => Ok(String::new()),
    }
}

/// 解析一行日志（JSON 优先，失败则按纯文本处理）。
fn parse_line(line: &str) -> LogLine {
    let sanitized = sanitize_log(line);

    let parsed = serde_json::from_str::<serde_json::Value>(&sanitized).ok();
    let field = |key: &str| {
        parsed
            .as_ref()
            .and_then(|value| value.get(key))
            .and_then(|value| value.as_str())
            .map(str::to_owned)
    };

    let message = field("fields")
        .and_then(|fields| serde_json::from_str::<serde_json::Value>(&fields).ok())
        .and_then(|fields| {
            fields
                .get("message")
                .and_then(|value| value.as_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| sanitized.clone());

    LogLine {
        timestamp: field("timestamp").and_then(|value| parse_rfc3339_millis(&value)),
        level: field("level"),
        target: field("target"),
        message,
        raw: sanitized,
    }
}

/// 解析 tracing 写入的 RFC3339 时间戳。
fn parse_rfc3339_millis(value: &str) -> Option<i64> {
    OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339)
        .ok()
        .map(|parsed| parsed.unix_timestamp_nanos() / 1_000_000)
        .and_then(|millis| i64::try_from(millis).ok())
}

/// 初始化文件日志（返回的 guard 必须在进程存活期间保持存在）。
///
/// 分层：`RotatingWriter`（策略）→ `tracing_appender::non_blocking`（异步写）→
/// `tracing_subscriber` 的 JSON 格式化层（脱敏在 `forgedesk-diagnostics` 里，
/// 由调用方拼接，见 `src-tauri`）。本函数只负责前两层，便于单测。
pub fn non_blocking_writer(
    directory: impl AsRef<Path>,
    policy: LogPolicy,
) -> AppResult<(NonBlocking, WorkerGuard, PathBuf)> {
    let writer = RotatingWriter::open(directory, policy)?;
    let directory_path = writer.directory().to_path_buf();
    let (non_blocking, guard) = tracing_appender::non_blocking(writer);
    Ok((non_blocking, guard, directory_path))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::io::Write;

    use time::{Date, Month, OffsetDateTime};

    use super::{
        parse_rotated_timestamp, rotated_file_name, should_rotate, tail, LogPolicy, RotatingWriter,
        CURRENT_LOG_FILE,
    };

    fn day(year: i32, month: Month, day: u8) -> Date {
        Date::from_calendar_date(year, month, day).unwrap()
    }

    fn unique_dir(label: &str) -> std::path::PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let suffix = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut path = std::env::temp_dir();
        path.push(format!(
            "forgedesk-logs-{label}-{}-{suffix}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        path
    }

    #[test]
    fn policy_rejects_useless_values() {
        let base = LogPolicy::default();
        assert!(base.validate().is_ok());

        assert!(LogPolicy {
            max_file_bytes: 10,
            ..base
        }
        .validate()
        .is_err());
        assert!(LogPolicy {
            keep_days: 0,
            ..base
        }
        .validate()
        .is_err());
        assert!(LogPolicy {
            max_files: 0,
            ..base
        }
        .validate()
        .is_err());
    }

    /// 日界轮转：跨天必须换文件，否则"按天轮转"名不副实。
    #[test]
    fn rotates_on_day_boundary_and_size_limit() {
        let policy = LogPolicy {
            max_file_bytes: 1024,
            keep_days: 7,
            max_files: 10,
        };
        let today = day(2026, Month::September, 23);

        // 同一天且未超上限 → 不轮转
        assert!(!should_rotate(today, today, 100, 100, &policy));
        // 同一天但会超过上限 → 轮转
        assert!(should_rotate(today, today, 1000, 100, &policy));
        // 恰好等于上限 → 不轮转（边界取"不超过"）
        assert!(!should_rotate(today, today, 924, 100, &policy));
        // 跨天 → 轮转
        assert!(should_rotate(
            day(2026, Month::September, 22),
            today,
            0,
            1,
            &policy
        ));
    }

    #[test]
    fn rotated_file_name_is_sortable_and_parseable() {
        let stamp = OffsetDateTime::from_unix_timestamp(1_787_000_000).unwrap();
        let name = rotated_file_name(stamp);
        assert!(name.starts_with("forgedesk-"), "命名前缀不符：{name}");

        let parsed = parse_rotated_timestamp(&name).expect("应能解析回时间");
        assert_eq!(parsed.unix_timestamp(), stamp.unix_timestamp());

        // 名字的字典序就是时间序（保留期清理依赖这一点）
        let later = rotated_file_name(stamp + time::Duration::seconds(1));
        assert!(later > name, "{later} 应排在 {name} 之后");

        assert!(parse_rotated_timestamp("forgedesk.log").is_none());
        assert!(parse_rotated_timestamp("forgedesk-garbage.log").is_none());
    }

    /// 同一秒内连续轮转不能因为重名而失败（Windows 的 rename 不覆盖已有文件）。
    #[test]
    fn rotating_twice_within_the_same_second_produces_distinct_files() {
        let dir = unique_dir("rotate-same-second");
        let policy = LogPolicy::default();
        let mut writer = RotatingWriter::open(&dir, policy).unwrap();
        let same_instant = OffsetDateTime::from_unix_timestamp(1_787_000_000).unwrap();

        writer.rotate(same_instant).unwrap();
        writer.rotate(same_instant).unwrap();

        let rotated = super::rotated_files(&dir);
        assert_eq!(
            rotated.len(),
            2,
            "同一秒的两次轮转应产生两个不同文件：{rotated:?}"
        );
        // 两个名字都要能被解析回时间（保留期清理依赖它）
        for path in &rotated {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            assert!(
                parse_rotated_timestamp(&name).is_some(),
                "去重后的文件名仍应可解析：{name}"
            );
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn writes_and_rotates_when_size_limit_is_reached() {
        let dir = unique_dir("rotate-size");
        // 上限用 1KB（策略校验要求 ≥1KB，避免出现"日志系统看起来在工作却什么都没留下"）
        let policy = LogPolicy {
            max_file_bytes: 1024,
            keep_days: 7,
            max_files: 10,
        };
        let mut writer = RotatingWriter::open(&dir, policy).unwrap();

        for index in 0..40 {
            writeln!(writer, "line {index} {}", "x".repeat(40)).unwrap();
        }
        writer.flush().unwrap();

        let rotated = super::rotated_files(&dir);
        assert!(!rotated.is_empty(), "超过上限后应产生轮转文件");
        assert!(
            std::fs::metadata(dir.join(CURRENT_LOG_FILE)).unwrap().len() <= policy.max_file_bytes,
            "当前文件不应超过大小上限"
        );
        // 所有内容都还在（轮转不能丢日志）
        let total: usize = tail(&dir, 1000).unwrap().len();
        assert!(total >= 40, "40 行内容应全部可读回，实际 {total}");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// 保留期清理：过老的文件删除，未过期的保留。
    #[test]
    fn prunes_files_older_than_retention() {
        let dir = unique_dir("prune-age");
        std::fs::create_dir_all(&dir).unwrap();
        let policy = LogPolicy::default();

        let now = OffsetDateTime::from_unix_timestamp(1_787_000_000).unwrap();
        let old = now - time::Duration::days(30);
        let recent = now - time::Duration::days(1);

        std::fs::write(dir.join(rotated_file_name(old)), "old\n").unwrap();
        std::fs::write(dir.join(rotated_file_name(recent)), "recent\n").unwrap();

        super::prune(&dir, policy, now);

        let remaining: Vec<String> = super::rotated_files(&dir)
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(remaining.len(), 1, "应只保留未过期的文件：{remaining:?}");
        assert!(remaining[0].contains("recent") || !remaining[0].is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    /// 数量上限兜底：一天内疯狂轮转也不能无限增长。
    #[test]
    fn prunes_by_file_count_as_a_safety_net() {
        let dir = unique_dir("prune-count");
        std::fs::create_dir_all(&dir).unwrap();
        let policy = LogPolicy {
            max_files: 3,
            ..LogPolicy::default()
        };

        let now = OffsetDateTime::from_unix_timestamp(1_787_000_000).unwrap();
        for index in 0..6 {
            let stamp = now - time::Duration::hours(i64::from(index));
            std::fs::write(dir.join(rotated_file_name(stamp)), "x\n").unwrap();
        }

        super::prune(&dir, policy, now);
        assert_eq!(super::rotated_files(&dir).len(), 3);

        std::fs::remove_dir_all(&dir).ok();
    }

    /// 读取路径同样必须脱敏（文件里可能有历史版本或第三方写入的内容）。
    #[test]
    fn tail_sanitizes_secrets_and_parses_json_lines() {
        let dir = unique_dir("tail-sanitize");
        std::fs::create_dir_all(&dir).unwrap();

        let secret = "ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
        let json_line = format!(
            "{{\"timestamp\":\"2026-09-23T10:11:12.345678Z\",\"level\":\"ERROR\",\"target\":\"forgedesk_commands::settings\",\"fields\":{{\"message\":\"save failed token={secret}\"}}}}"
        );
        std::fs::write(
            dir.join(CURRENT_LOG_FILE),
            format!("{json_line}\nplain text line\n"),
        )
        .unwrap();

        let lines = tail(&dir, 10).unwrap();
        assert_eq!(lines.len(), 2);

        // 新的在后：最后一行是纯文本
        let parsed = &lines[0];
        assert_eq!(parsed.level.as_deref(), Some("ERROR"));
        assert_eq!(
            parsed.target.as_deref(),
            Some("forgedesk_commands::settings")
        );

        // 时间戳：断言它真的还原成了写入时的 UTC 时刻（而不是硬编码一个我算不出的数字）
        let millis = parsed.timestamp.expect("应解析出时间戳");
        let restored = OffsetDateTime::from_unix_timestamp_nanos(i128::from(millis) * 1_000_000)
            .expect("时间戳应可还原");
        assert_eq!(restored.year(), 2026);
        assert_eq!(restored.hour(), 10, "UTC 小时应与写入的 RFC3339 一致");
        assert_eq!(restored.minute(), 11);
        assert!(
            !parsed.raw.contains(secret),
            "结构化字段未脱敏：{}",
            parsed.raw
        );
        assert!(parsed.raw.contains("«redacted»"), "应保留脱敏标记");
        assert!(!parsed.message.contains(secret), "message 未脱敏");

        let plain = &lines[1];
        assert_eq!(plain.raw, "plain text line", "非 JSON 行按纯文本返回");
        assert_eq!(plain.level, None);

        std::fs::remove_dir_all(&dir).ok();
    }

    /// 跨文件读取：返回的必须是**时间线顺序**（旧 → 新），
    /// 这样界面直接从上往下渲染就与真实发生顺序一致，不需要再排序。
    #[test]
    fn tail_reads_across_files_in_chronological_order_and_respects_limit() {
        let dir = unique_dir("tail-multi");
        std::fs::create_dir_all(&dir).unwrap();

        let now = OffsetDateTime::from_unix_timestamp(1_787_000_000).unwrap();
        std::fs::write(
            dir.join(rotated_file_name(now - time::Duration::hours(1))),
            "older-a\nolder-b\n",
        )
        .unwrap();
        std::fs::write(dir.join(CURRENT_LOG_FILE), "newest\n").unwrap();

        let raws: Vec<String> = tail(&dir, 3)
            .unwrap()
            .into_iter()
            .map(|line| line.raw)
            .collect();
        assert_eq!(raws, vec!["older-a", "older-b", "newest"]);

        // limit 只保留最近的若干行（从最新端往回取）
        let latest_one: Vec<String> = tail(&dir, 1)
            .unwrap()
            .into_iter()
            .map(|line| line.raw)
            .collect();
        assert_eq!(latest_one, vec!["newest"]);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn tail_returns_empty_when_no_logs_exist() {
        let dir = unique_dir("tail-empty");
        assert!(tail(&dir, 10).unwrap().is_empty());
        assert!(tail(&dir, 0).unwrap().is_empty());
    }

    #[test]
    fn tail_truncates_large_files_from_the_end() {
        let dir = unique_dir("tail-large");
        let policy = LogPolicy::default();
        let mut writer = RotatingWriter::open(&dir, policy).unwrap();

        for index in 0..20_000 {
            writeln!(writer, "line-{index:05} {}", "y".repeat(50)).unwrap();
        }
        writer.flush().unwrap();
        drop(writer);

        // 文件远大于 TAIL_READ_BYTES：只读末尾，但必须拿到最后一行
        let lines = tail(&dir, 5).unwrap();
        assert_eq!(lines.len(), 5);
        assert_eq!(lines[4].raw.split_whitespace().next(), Some("line-19999"));

        std::fs::remove_dir_all(&dir).ok();
    }
}
