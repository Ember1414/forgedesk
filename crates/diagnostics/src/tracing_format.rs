//! tracing 的事件格式化层：所有日志在**写出去之前**统一脱敏。
//!
//! 为什么做成格式化层，而不是"在每条日志里手动调用 sanitize_log"：
//! 前者是结构性保证（漏掉一处就漏一条机密），后者是纪律性保证（总会有人忘）。
//! 放在最外层的 formatter 上，任何来源（我们自己的、依赖库的、panic hook 的）
//! 日志都会被同一把筛子过一遍。
//!
//! 代价：多一次字符串分配。日志量相比 Git 仓库操作的开销可以忽略。

use std::fmt;

use tracing::{Event, Subscriber};
// Format 有两个泛型参数：事件格式（Full）与计时器（SystemTime）。
// 必须显式写出，否则 `Format::default()` 无法推断类型。
use tracing_subscriber::fmt::format::{Format, Full, Writer};
use tracing_subscriber::fmt::time::SystemTime;
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::registry::LookupSpan;

use crate::sanitize::sanitize_log;

/// 先按常规格式渲染事件，再对整行文本脱敏后写出的格式化器。
#[derive(Debug, Default, Clone, Copy)]
pub struct SanitizedFormat;

impl<S, N> FormatEvent<S, N> for SanitizedFormat
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
    N: for<'writer> FormatFields<'writer> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        // 先渲染到内存，脱敏后再一次性写出：避免"半个令牌已经写进文件"的窗口
        let mut rendered = String::new();
        {
            // Writer 按值接收（内部持有可变引用），因此用独立作用域包住这次借用
            let buffer = Writer::new(&mut rendered);
            Format::<Full, SystemTime>::default().format_event(ctx, buffer, event)?;
        }

        writer.write_str(&sanitize_log(&rendered))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::sync::{Arc, Mutex};

    use tracing_subscriber::fmt::MakeWriter;
    use tracing_subscriber::prelude::*;

    use super::SanitizedFormat;

    /// 把日志写进内存缓冲区，便于断言最终输出（而不是断言"我们以为会写什么"）。
    #[derive(Clone, Default)]
    struct SharedBuffer(Arc<Mutex<Vec<u8>>>);

    impl SharedBuffer {
        fn contents(&self) -> String {
            let bytes = self.0.lock().unwrap().clone();
            String::from_utf8(bytes).unwrap()
        }
    }

    struct BufferWriter(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for BufferWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for SharedBuffer {
        type Writer = BufferWriter;

        fn make_writer(&'a self) -> Self::Writer {
            BufferWriter(Arc::clone(&self.0))
        }
    }

    #[test]
    fn log_lines_are_sanitized_end_to_end() {
        let buffer = SharedBuffer::default();
        let subscriber = tracing_subscriber::registry().with(
            tracing_subscriber::fmt::layer()
                .with_writer(buffer.clone())
                .with_ansi(false)
                .event_format(SanitizedFormat),
        );

        tracing::subscriber::with_default(subscriber, || {
            tracing::error!(
                token = "ghp_ABCDEFGHIJKLMNOP",
                url = "https://alice:s3cr3t@example.com/a.git",
                "fetch failed"
            );
        });

        let output = buffer.contents();
        assert!(output.contains("fetch failed"), "日志正文应保留：{output}");
        assert!(
            !output.contains("ghp_ABCDEFGHIJKLMNOP"),
            "令牌泄漏：{output}"
        );
        assert!(!output.contains("s3cr3t"), "URL 凭据泄漏：{output}");
    }
}
