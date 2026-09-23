//! 进度事件与 `--progress` 输出解析。
//!
//! 网络操作（clone/fetch/pull/push）动辄几十秒，没有进度就是"界面卡住了"。
//! git 把进度写在 **stderr** 上、用 `\r` 做原地刷新（不是 `\n`），
//! 因此 [`GitProcess`](crate::process::GitProcess) 的 stderr 逐行回调
//! 同时按 `\r` 与 `\n` 切分——这一点在 T1.1 就定好了。
//!
//! 解析放在这里而不是散在四个写方法里：四种操作的进度行格式由 git 统一产生
//! （`Counting objects:` / `Receiving objects:` …），一个解析器覆盖全部。

/// 进度所处的阶段。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProgressPhase {
    /// 统计对象（`Counting objects`）。
    Counting,
    /// 压缩对象（`Compressing objects`）。
    Compressing,
    /// 接收对象（`Receiving objects`）。
    Receiving,
    /// 解析增量（`Resolving deltas`）。
    Resolving,
    /// 写入对象（`Writing objects`）。
    Writing,
    /// 引用更新行（`Updating a1b2c3..d4e5f6`、` * [new branch] ...`）。
    ///
    /// 单独一类：它不是百分比进度，而是**结果**（哪个引用动了），
    /// 界面应该把它当"日志"显示而不是当进度条。
    RefUpdate,
    /// 其他（`remote:` 前缀的用户侧输出等）。
    Other,
}

/// 一条进度事件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgressEvent {
    /// 阶段。
    pub phase: ProgressPhase,
    /// 已完成的量（对象数）。解析不出来时为 `None`。
    pub current: Option<u64>,
    /// 总量。git 在开始阶段可能给不出总量，因此是 `None`。
    pub total: Option<u64>,
    /// 原始行（已脱敏）。保留它是为了让"进度卡住"时用户能看到真实输出。
    pub message: String,
}

impl ProgressEvent {
    /// 完成比例（0.0–1.0）。总量未知或为 0 时返回 `None`。
    pub fn ratio(&self) -> Option<f64> {
        match (self.current, self.total) {
            (Some(current), Some(total)) if total > 0 => {
                Some((current as f64 / total as f64).clamp(0.0, 1.0))
            }
            _ => None,
        }
    }
}

/// 解析一行 git 进度输出。
///
/// 无法识别时返回 `None`：调用方应**忽略**它，而不是把它当成 0% 的进度
/// ——否则 `remote: Enumerating objects` 这类行会让进度条来回跳。
pub fn parse_progress_line(line: &str) -> Option<ProgressEvent> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    let phase = if trimmed.starts_with("Counting objects") {
        ProgressPhase::Counting
    } else if trimmed.starts_with("Compressing objects") {
        ProgressPhase::Compressing
    } else if trimmed.starts_with("Receiving objects") {
        ProgressPhase::Receiving
    } else if trimmed.starts_with("Resolving deltas") {
        ProgressPhase::Resolving
    } else if trimmed.starts_with("Writing objects") {
        ProgressPhase::Writing
    } else if trimmed.starts_with("Updating ")
        || trimmed.starts_with("* [new")
        || trimmed.starts_with("+ ")
        || trimmed.starts_with("- [deleted]")
    {
        // 注意：这里比较的是 **trim 之后** 的文本。git 的引用更新行以空格
        // 或 `*`/`+`/`-` 开头，用带前导空格的模式匹配会一条都匹配不上。
        ProgressPhase::RefUpdate
    } else if trimmed.starts_with("remote:") {
        ProgressPhase::Other
    } else {
        return None;
    };

    let (current, total) = parse_counts(trimmed).unzip_pair();

    Some(ProgressEvent {
        phase,
        current,
        total,
        message: trimmed.to_owned(),
    })
}

/// 从 `...(12/34)` 形式的括号里取出已完成量与总量。
///
/// 只认 `(a/b)`：git 的百分比写法有 `12%`、`12% (3/25)`、`100% (25/25), done.`
/// 等多种，而**只有 `(a/b)` 同时给出两个数**。用百分比反推总量会引入舍入误差，
/// 让进度条在 99% 处停住。
fn parse_counts(line: &str) -> Option<(u64, u64)> {
    let open = line.find('(')?;
    let close = line[open..].find('/')? + open;
    let end = line[open..].find(')')? + open;

    let current = line[open + 1..close].trim().parse::<u64>().ok()?;
    let total = line[close + 1..end].trim().parse::<u64>().ok()?;
    Some((current, total))
}

/// 把 `Option<(u64, u64)>` 拆成两个 `Option<u64>`。
trait UnzipPair {
    fn unzip_pair(self) -> (Option<u64>, Option<u64>);
}

impl UnzipPair for Option<(u64, u64)> {
    fn unzip_pair(self) -> (Option<u64>, Option<u64>) {
        match self {
            Some((current, total)) => (Some(current), Some(total)),
            None => (None, None),
        }
    }
}

/// 进度接收器。
///
/// 为什么用闭包而不是 trait：只有一个真实实现（把事件转成 Tauri 事件），
/// 而测试需要的是"把事件收进一个 Vec"。闭包让这两件事用同一个类型表达，
/// 不必为了可测性再造一个 trait 与它的 mock。
///
/// 内部用 `Arc` 持有回调，因此 [`ProgressSink::handler`] 能产出一个
/// `'static` 闭包交给 [`GitRunOpts`](crate::process::GitRunOpts)——
/// 后者要求回调是 `'static`（它会被移动到读取子进程输出的任务里）。
/// 如果这里存的是引用，写方法就无法把进度接进 stderr 回调。
#[derive(Clone, Default)]
pub struct ProgressSink {
    handler: Option<std::sync::Arc<dyn Fn(ProgressEvent) + Send + Sync>>,
}

impl ProgressSink {
    /// 不接收进度（本地操作与测试用）。
    pub fn none() -> Self {
        Self { handler: None }
    }

    /// 用回调创建。
    pub fn new(handler: impl Fn(ProgressEvent) + Send + Sync + 'static) -> Self {
        Self {
            handler: Some(std::sync::Arc::new(handler)),
        }
    }

    /// 是否有接收方。没有接收方时调用方可以跳过进度解析的开销。
    pub fn is_active(&self) -> bool {
        self.handler.is_some()
    }

    /// 投递一条事件。没有接收方时静默丢弃。
    pub fn emit(&self, event: ProgressEvent) {
        if let Some(handler) = &self.handler {
            handler(event);
        }
    }

    /// 解析一行 stderr 并投递（无法识别时忽略）。
    pub fn emit_line(&self, line: &str) {
        if let Some(event) = parse_progress_line(line) {
            self.emit(event);
        }
    }

    /// 生成可直接交给 `GitRunOpts::with_stderr_line_handler` 的回调。
    ///
    /// 返回的闭包克隆了 `Arc`，因此是 `'static`；没有接收方时返回一个空操作。
    pub fn handler(&self) -> impl Fn(&str) + Send + Sync + 'static {
        let handler = self.handler.clone();
        move |line: &str| {
            if let (Some(handler), Some(event)) = (&handler, parse_progress_line(line)) {
                handler(event);
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{parse_progress_line, ProgressEvent, ProgressPhase, ProgressSink};

    #[test]
    fn counting_line_yields_phase_and_counts() {
        let event = parse_progress_line("Counting objects:  12% (3/25)").unwrap();

        assert_eq!(event.phase, ProgressPhase::Counting);
        assert_eq!(event.current, Some(3));
        assert_eq!(event.total, Some(25));
        assert_eq!(event.ratio(), Some(0.12));
    }

    #[test]
    fn receiving_line_with_throughput_still_parses_the_counts() {
        let event =
            parse_progress_line("Receiving objects:  42% (100/238), 1.20 MiB | 500.00 KiB/s")
                .unwrap();

        assert_eq!(event.phase, ProgressPhase::Receiving);
        assert_eq!(event.current, Some(100));
        assert_eq!(event.total, Some(238));
    }

    #[test]
    fn phase_without_counts_keeps_a_none_ratio() {
        let event = parse_progress_line("Resolving deltas: 100% (12/12)").unwrap();
        assert_eq!(event.phase, ProgressPhase::Resolving);

        let no_counts = parse_progress_line("Writing objects: 100%, done.").unwrap();
        assert_eq!(no_counts.phase, ProgressPhase::Writing);
        assert_eq!(no_counts.current, None);
        assert_eq!(no_counts.ratio(), None);
    }

    #[test]
    fn ref_update_lines_are_their_own_phase() {
        let updating = parse_progress_line("Updating a1b2c3..d4e5f6").unwrap();
        let new_branch = parse_progress_line(" * [new branch]      main -> origin/main").unwrap();

        assert_eq!(updating.phase, ProgressPhase::RefUpdate);
        assert_eq!(new_branch.phase, ProgressPhase::RefUpdate);
    }

    #[test]
    fn remote_prefixed_and_unrelated_lines_are_handled() {
        assert_eq!(
            parse_progress_line("remote: Enumerating objects: 5, done.")
                .unwrap()
                .phase,
            ProgressPhase::Other
        );
        // 无关的行必须被忽略，否则进度条会在这些行上乱跳
        assert_eq!(parse_progress_line("fatal: repository not found"), None);
        assert_eq!(parse_progress_line("   "), None);
        assert_eq!(parse_progress_line(""), None);
    }

    #[test]
    fn sink_without_a_handler_swallows_events() {
        let sink = ProgressSink::none();

        assert!(!sink.is_active());
        sink.emit_line("Counting objects: 1% (1/100)");
        sink.emit(parse_progress_line("Counting objects: 1% (1/100)").unwrap());
    }

    #[test]
    fn sink_forwards_only_recognised_lines_to_its_handler() {
        let collected = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = ProgressSink::new({
            let collected = std::sync::Arc::clone(&collected);
            move |event: ProgressEvent| collected.lock().unwrap().push(event.phase)
        });

        sink.emit_line("Counting objects: 1% (1/100)");
        sink.emit_line("fatal: nothing to do");
        sink.emit_line("Receiving objects: 50% (1/2)");

        assert_eq!(
            *collected.lock().unwrap(),
            vec![ProgressPhase::Counting, ProgressPhase::Receiving]
        );
    }

    #[test]
    fn handler_produced_for_the_process_layer_is_static_and_still_forwards() {
        let collected = std::sync::Arc::new(std::sync::Mutex::new(0_usize));
        let sink = ProgressSink::new({
            let collected = std::sync::Arc::clone(&collected);
            move |_| *collected.lock().unwrap() += 1
        });

        // 与 GitRunOpts 的 `'static` 要求一致：把它送进 spawn 的闭包
        let handler = sink.handler();
        std::thread::spawn(move || {
            handler("Counting objects: 1% (1/100)");
            handler("not a progress line");
        })
        .join()
        .unwrap();

        assert_eq!(*collected.lock().unwrap(), 1);
    }
}
