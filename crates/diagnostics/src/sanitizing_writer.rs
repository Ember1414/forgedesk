//! 日志出境前的脱敏**写入层**。
//!
//! # 为什么脱敏放在写入层，而不是事件格式化层
//!
//! T0.6 最初把脱敏做成 `FormatEvent` 实现（渲染整条事件 → 脱敏 → 写出）。
//! T0.8 要写 JSON 文件日志时这条路走不通：tracing-subscriber 的 JSON 渲染器是
//! `Format<Json, _>`，而它**没有实现 `Default`**（只有 `Format<Full, _>` 有），
//! 也就是说第三方格式化器无法被我们包一层再交给 `event_format`。
//!
//! 写入层是更稳的位置：无论上游用什么格式（可读、JSON、将来别的），
//! 字节流都要经过 `Write`。而且它带来一个额外好处——脱敏单位是**整行**，
//! 因此"秘密被拆成两次 write 调用"这种情况也能被正确抹掉
//! （按事件脱敏时，跨事件的拼接是看不见的）。
//!
//! # 行为约定
//!
//! - 按行缓冲：遇到换行才写出。tracing 每条事件都以换行结尾，因此正常路径没有额外延迟；
//!   没有换行的残行会在 `flush` 与 `Drop` 时补齐（**绝不因为"没换行"就丢掉日志**）。
//! - 非 UTF-8 字节用替换字符处理：宁可让那一行变难看，也不能让日志消失。
//! - `write` 一律返回"已全部接收"：内容已被我们接管，若返回部分写入，
//!   tracing 会重发整块导致重复行。

use std::io::{self, Write};

use tracing_subscriber::fmt::MakeWriter;

use crate::sanitize::sanitize_log;

/// 按行脱敏的写入器。
#[derive(Debug)]
pub struct SanitizingWriter<W: Write> {
    inner: W,
    /// 尚未遇到换行的残行。
    pending: Vec<u8>,
}

impl<W: Write> SanitizingWriter<W> {
    /// 包装一个底层写入器。
    pub const fn new(inner: W) -> Self {
        Self {
            inner,
            pending: Vec::new(),
        }
    }

    /// 把缓冲里没有换行结尾的残行也脱敏写出。
    fn drain_pending(&mut self) -> io::Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let text = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending.clear();
        self.inner.write_all(sanitize_log(&text).as_bytes())
    }
}

impl<W: Write> Write for SanitizingWriter<W> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.pending.extend_from_slice(buffer);

        while let Some(newline) = self.pending.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=newline).collect();
            let text = String::from_utf8_lossy(&line);
            self.inner.write_all(sanitize_log(&text).as_bytes())?;
        }

        // 全部内容都已被接管（见模块头说明）
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.drain_pending()?;
        self.inner.flush()
    }
}

impl<W: Write> Drop for SanitizingWriter<W> {
    fn drop(&mut self) {
        // 尽力而为：Drop 里出错无法上报，但不能因此丢掉最后一行
        let _ = self.drain_pending();
    }
}

/// 把 [`SanitizingWriter`] 接到 tracing 的 `MakeWriter` 体系上。
#[derive(Debug, Clone, Copy, Default)]
pub struct SanitizingMakeWriter<M> {
    inner: M,
}

impl<M> SanitizingMakeWriter<M> {
    /// 包装一个 `MakeWriter`（例如 `tracing_appender::non_blocking` 的返回值，
    /// 或 `std::io::stderr` 这样的函数）。
    pub const fn new(inner: M) -> Self {
        Self { inner }
    }
}

impl<'a, M> MakeWriter<'a> for SanitizingMakeWriter<M>
where
    M: MakeWriter<'a>,
{
    type Writer = SanitizingWriter<M::Writer>;

    fn make_writer(&'a self) -> Self::Writer {
        SanitizingWriter::new(self.inner.make_writer())
    }

    fn make_writer_for(&'a self, meta: &tracing::Metadata<'_>) -> Self::Writer {
        SanitizingWriter::new(self.inner.make_writer_for(meta))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::io::Write;

    use super::{SanitizingMakeWriter, SanitizingWriter};

    /// 内存写入器（与 `Vec<u8>` 的区别：它不会在 `into_inner` 时被 Drop 吞掉缓冲）。
    #[derive(Clone, Default)]
    struct Shared(#[allow(clippy::type_complexity)] std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl Shared {
        fn contents(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
        }
    }

    struct SharedWriter(Shared);

    impl Write for SharedWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0 .0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Shared {
        type Writer = SharedWriter;

        fn make_writer(&'a self) -> Self::Writer {
            SharedWriter(self.clone())
        }
    }

    #[test]
    fn sanitizes_each_completed_line() {
        let sink = Shared::default();
        {
            let mut writer = SanitizingWriter::new(SharedWriter(sink.clone()));
            writer.write_all(b"token=ghp_ABCDEFGH\n").unwrap();
            writer.flush().unwrap();
        }

        let output = sink.contents();
        // 注意：`token=<值>` 属于"键值对"规则，整个值（含 `ghp_` 前缀）都会被抹掉；
        // 只有**单独出现**的令牌才保留前缀（见 sanitize 模块的规则说明）。
        assert_eq!(output, "token=«redacted»\n");
    }

    /// 写入层脱敏的关键优势：秘密被拆成两次 write 也抹得掉。
    #[test]
    fn sanitizes_secrets_split_across_writes() {
        let sink = Shared::default();
        {
            let mut writer = SanitizingWriter::new(SharedWriter(sink.clone()));
            // 一次日志事件可能被分块写出（例如非阻塞写入器的缓冲边界）
            writer.write_all(b"Authorization: Bearer ghp_ABCD").unwrap();
            writer.write_all(b"EFGHIJKLMNOP\nnext line\n").unwrap();
            writer.flush().unwrap();
        }

        let output = sink.contents();
        assert!(
            !output.contains("ghp_ABCDEFGHIJKLMNOP"),
            "跨块秘密未被脱敏：{output}"
        );
        assert!(output.contains("next line"), "第二行不应被影响：{output}");
    }

    #[test]
    fn handles_multiple_lines_in_one_write() {
        let sink = Shared::default();
        {
            let mut writer = SanitizingWriter::new(SharedWriter(sink.clone()));
            writer
                .write_all(b"password=hunter2\nplain line\nhttps://u:p@example.com/x\n")
                .unwrap();
            writer.flush().unwrap();
        }

        let output = sink.contents();
        assert!(!output.contains("hunter2"));
        assert!(!output.contains(":p@example.com"));
        assert!(output.contains("plain line"));
        assert_eq!(output.lines().count(), 3, "三行都应写出：{output}");
    }

    /// 残行（没有换行结尾）不能丢：flush 与 Drop 都要把它补齐。
    #[test]
    fn flushes_incomplete_final_line() {
        let sink = Shared::default();
        {
            let mut writer = SanitizingWriter::new(SharedWriter(sink.clone()));
            writer.write_all(b"token=glpat-ABCDEFG").unwrap();
            // 不写换行，也不显式 flush：靠 Drop 落地
        }

        let output = sink.contents();
        assert!(!output.contains("glpat-ABCDEFG"), "残行未脱敏：{output}");
        assert_eq!(output, "token=«redacted»");
    }

    #[test]
    fn keeps_non_utf8_bytes_instead_of_dropping_the_line() {
        let sink = Shared::default();
        {
            let mut writer = SanitizingWriter::new(SharedWriter(sink.clone()));
            // 二进制输出（例如某个依赖把原始字节写进日志）：不能整行消失
            writer
                .write_all(&[b'h', b'i', 0xff, 0xfe, b'!', b'\n'])
                .unwrap();
            writer.flush().unwrap();
        }

        let output = sink.contents();
        assert!(output.starts_with("hi"), "有效前缀应保留：{output}");
        assert!(output.contains('!'), "行尾应保留：{output}");
    }

    /// 端到端：真实订阅者 + JSON 格式 + 脱敏写入层。
    /// 这一组断言同时锁住了两件事：脱敏生效，且 JSON 仍是合法单行。
    #[test]
    fn json_lines_stay_parseable_after_sanitizing() {
        use tracing_subscriber::prelude::*;

        let sink = Shared::default();
        let subscriber = tracing_subscriber::registry().with(
            tracing_subscriber::fmt::layer()
                .json()
                .with_ansi(false)
                .with_writer(SanitizingMakeWriter::new(sink.clone())),
        );

        tracing::subscriber::with_default(subscriber, || {
            tracing::warn!(
                password = "hunter2",
                repo = "E:/projects/a",
                "settings write failed"
            );
        });

        let output = sink.contents();
        assert!(!output.contains("hunter2"), "密码泄漏：{output}");

        let line = output.lines().next().expect("应输出一行");
        let parsed: serde_json::Value = serde_json::from_str(line).expect("必须是合法 JSON");
        assert_eq!(parsed["level"], "WARN");
        assert_eq!(parsed["fields"]["repo"], "E:/projects/a");
        assert!(
            parsed["timestamp"].as_str().is_some(),
            "时间戳字段必须存在（logs_tail 依赖它）：{line}"
        );
    }

    #[test]
    fn console_format_is_sanitized_end_to_end() {
        use tracing_subscriber::prelude::*;

        let sink = Shared::default();
        let subscriber = tracing_subscriber::registry().with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(SanitizingMakeWriter::new(sink.clone())),
        );

        tracing::subscriber::with_default(subscriber, || {
            tracing::error!(
                token = "ghp_ABCDEFGHIJKLMNOP",
                url = "https://alice:s3cr3t@example.com/a.git",
                "fetch failed"
            );
        });

        let output = sink.contents();
        assert!(output.contains("fetch failed"), "日志正文应保留：{output}");
        assert!(
            !output.contains("ghp_ABCDEFGHIJKLMNOP"),
            "令牌泄漏：{output}"
        );
        assert!(!output.contains("s3cr3t"), "URL 凭据泄漏：{output}");
    }
}
