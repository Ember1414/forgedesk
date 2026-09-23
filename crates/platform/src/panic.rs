//! panic 处理：把崩溃现场写进独立文件，为 M7 的崩溃恢复留下证据。
//!
//! # 为什么 panic 日志单独成文件
//!
//! 崩溃可能发生在日志系统还没完全就绪的时刻（例如数据库初始化失败），
//! 也可能发生在日志文件已经损坏/写满的时候。把 panic 写进**独立文件**，
//! 好处是：① 不依赖轮转策略；② 用户报问题时只需附上那个小文件；
//! ③ 即使进程随后 abort，文件也已经 flush 落盘。
//!
//! # 与 `panic = "abort"` 的关系
//!
//! release 构建使用 `panic = "abort"`（体积与确定性考虑，见根 `Cargo.toml`），
//! 也就是说 release 下任何 panic 都会终止进程——这正是需要会话标记与 panic 文件的原因。
//! dev 构建是 unwind，单个操作的 panic 不会带走整个应用（T0.8 的手工验收针对 dev）。
//!
//! # 不吞掉默认行为
//!
//! 写入自己的文件后仍调用**上一个 hook**，因此 stderr 上的 panic 输出、
//! `RUST_BACKTRACE` 的行为都与标准库一致；我们只是额外留档。

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use forgedesk_diagnostics::sanitize_log;

/// panic 日志文件名前缀。
pub const PANIC_FILE_PREFIX: &str = "panic-";

/// panic 日志文件名（`panic-<毫秒时间戳>.log`）。
pub fn panic_file_name(timestamp_millis: i64) -> String {
    format!("{PANIC_FILE_PREFIX}{timestamp_millis}.log")
}

/// 当前时间（Unix 毫秒）。
fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or_default()
}

/// 崩溃现场的内容。
pub struct PanicReport<'a> {
    /// panic 消息。
    pub message: &'a str,
    /// 触发位置（`file:line:col`）。
    pub location: Option<&'a str>,
    /// 线程名（主线程为 `main`；命令线程/Tauri 线程会有各自名字）。
    pub thread: &'a str,
    /// 应用版本。
    pub version: &'a str,
    /// 调用栈（未开启 `RUST_BACKTRACE` 时为占位说明）。
    pub backtrace: &'a str,
}

impl PanicReport<'_> {
    /// 渲染为可读文本（**已脱敏**：panic 消息里可能带着 URL 或令牌）。
    pub fn render(&self) -> String {
        format!(
            "ForgeDesk panic report\n\
             timestamp: {}\n\
             version: {}\n\
             thread: {}\n\
             location: {}\n\
             message: {}\n\
             \n\
             backtrace:\n{}\n",
            now_millis(),
            self.version,
            self.thread,
            self.location.unwrap_or("<unknown>"),
            self.message,
            self.backtrace,
        )
    }
}

/// 把崩溃现场写入日志目录，返回文件路径。
///
/// 本函数**绝不 panic**：它运行在 panic 路径上，任何二次失败都会把原始崩溃信息盖掉。
/// 因此所有错误都被吞掉（调用方拿到 `None` 即可），并只做最小必要的 IO。
pub fn write_panic_report(directory: &Path, report: &PanicReport<'_>) -> Option<PathBuf> {
    let path = directory.join(panic_file_name(now_millis()));
    let rendered = sanitize_log(&report.render());

    if std::fs::create_dir_all(directory).is_err() {
        return None;
    }
    if std::fs::write(&path, rendered.as_bytes()).is_err() {
        return None;
    }
    Some(path)
}

/// 安装 panic hook：先留档，再交给上一个 hook（保持默认 stderr 输出与 backtrace 行为）。
///
/// 返回日志目录给调用方，便于日志与测试引用。
pub fn install_panic_hook(directory: impl AsRef<Path>, version: &'static str) -> PathBuf {
    let directory = directory.as_ref().to_path_buf();
    // hook 是 'static 的，因此目录要复制一份进去；返回值给调用方/测试引用
    let hook_directory = directory.clone();
    let previous = std::panic::take_hook();

    std::panic::set_hook(Box::new(move |info| {
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|text| (*text).to_owned())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "<non-string panic payload>".to_owned());

        let location = info.location().map(|location| {
            format!(
                "{}:{}:{}",
                location.file(),
                location.line(),
                location.column()
            )
        });

        let thread = std::thread::current();
        let thread_name = thread.name().unwrap_or("<unnamed>").to_owned();

        // Backtrace::capture() 尊重 RUST_BACKTRACE 环境变量：未开启时它自己就是一行说明，
        // 因此这里不需要额外判断，也不会在默认情况下拖慢崩溃路径。
        let backtrace = std::backtrace::Backtrace::capture().to_string();

        let report = PanicReport {
            message: &message,
            location: location.as_deref(),
            thread: &thread_name,
            version,
            backtrace: &backtrace,
        };

        // 留档失败也要继续走默认 hook：宁可少一份记录，也不能让 panic 输出消失
        let _ = write_panic_report(&hook_directory, &report);

        previous(info);
    }));

    directory
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{panic_file_name, write_panic_report, PanicReport};

    fn unique_dir(label: &str) -> std::path::PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let suffix = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut path = std::env::temp_dir();
        path.push(format!(
            "forgedesk-panic-{label}-{}-{suffix}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        path
    }

    #[test]
    fn panic_file_name_is_stable_and_sortable() {
        assert_eq!(
            panic_file_name(1_787_000_000_000),
            "panic-1787000000000.log"
        );
        // 数字长度固定（毫秒时间戳到 2286 年都是 13 位），因此字典序即时间序
        assert!(panic_file_name(1_787_000_000_001) > panic_file_name(1_787_000_000_000));
    }

    #[test]
    fn report_contains_the_essentials() {
        let report = PanicReport {
            message: "assertion failed: expected 2, got 3",
            location: Some("crates/domain/src/error.rs:42:9"),
            thread: "main",
            version: "0.0.1",
            backtrace: "0: forgedesk_domain::error::tests",
        };
        let rendered = report.render();

        assert!(rendered.contains("assertion failed"));
        assert!(rendered.contains("crates/domain/src/error.rs:42:9"));
        assert!(rendered.contains("0.0.1"));
        assert!(rendered.contains("backtrace"));
    }

    /// panic 消息里常夹带远端 URL（含凭据），必须脱敏后才写盘。
    #[test]
    fn report_is_sanitized_before_writing() {
        let dir = unique_dir("sanitize");
        let secret = "ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
        let message = format!("clone failed for https://alice:{secret}@example.com/repo.git");

        let report = PanicReport {
            message: &message,
            location: None,
            thread: "main",
            version: "0.0.1",
            backtrace: "",
        };

        let path = write_panic_report(&dir, &report).expect("应写出 panic 文件");
        let content = std::fs::read_to_string(&path).unwrap();

        assert!(!content.contains(secret), "panic 文件未脱敏：{content}");
        assert!(content.contains("«redacted»"));
        assert!(content.contains("location: <unknown>"), "缺失字段应有占位");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// T0.8 验收项的自动化版本："触发一次 panic，确认生成 panic 日志，
    /// 且默认的 panic 行为没有被吞掉"。
    ///
    /// 手工验收（在桌面宿主里点一下按钮）覆盖的是"进程是否存活"，
    /// 而这里覆盖的是更难人工观察的两件事：hook 真的被调用、且仍把 panic 继续抛给上层。
    ///
    /// 注意：panic hook 是**进程级**的，因此用例结束前必须恢复原来的 hook，
    /// 否则会影响同进程内并行运行的其它测试。
    #[test]
    fn installed_hook_writes_a_report_without_swallowing_the_panic() {
        let dir = unique_dir("hook");
        let previous = std::panic::take_hook();
        super::install_panic_hook(&dir, "9.9.9-test");

        let outcome = std::panic::catch_unwind(|| {
            panic!("boom from the hook test");
        });

        std::panic::set_hook(previous);

        assert!(
            outcome.is_err(),
            "我们的 hook 不应吞掉 panic（默认行为必须保留）"
        );

        let reports: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
            .expect("日志目录应存在")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name().is_some_and(|name| {
                    name.to_string_lossy().starts_with(super::PANIC_FILE_PREFIX)
                })
            })
            .collect();

        assert_eq!(
            reports.len(),
            1,
            "应留下且只留下一个 panic 文件：{reports:?}"
        );
        let content = std::fs::read_to_string(&reports[0]).unwrap();
        assert!(
            content.contains("boom from the hook test"),
            "应记录 panic 消息：{content}"
        );
        assert!(
            content.contains("version: 9.9.9-test"),
            "应记录版本：{content}"
        );
        assert!(content.contains("thread:"), "应记录线程名：{content}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn writing_failure_returns_none_instead_of_panicking() {
        // 用一个不可能创建目录的路径（把文件当目录用）
        let dir = unique_dir("not-a-dir");
        std::fs::create_dir_all(&dir).unwrap();
        let file_path = dir.join("blocker");
        std::fs::write(&file_path, b"x").unwrap();

        let report = PanicReport {
            message: "boom",
            location: None,
            thread: "main",
            version: "0.0.1",
            backtrace: "",
        };

        assert!(
            write_panic_report(&file_path, &report).is_none(),
            "写不进去时必须返回 None，绝不能二次 panic"
        );

        std::fs::remove_dir_all(&dir).ok();
    }
}
