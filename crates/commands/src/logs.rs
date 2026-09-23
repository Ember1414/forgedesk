//! 日志相关命令（`logs_*`）。
//!
//! 这两个命令是"用户能自己拿到证据"的入口：出问题时不必让用户去翻隐藏目录，
//! 也不必让他复制一大段控制台输出——直接打开目录或读取末尾若干行即可。
//!
//! 读取路径**必须脱敏**（[`forgedesk_platform::logging::tail`] 内部已做）：
//! 文件里可能混入历史版本写入的内容或第三方库的原始输出，
//! 不能因为"写入时脱敏过"就在读取时放行（红线 R8）。

use forgedesk_domain::AppResult;
use forgedesk_platform::logging::{self, LogLine};
use forgedesk_platform::shell;
use tauri::State;

use crate::state::AppState;

/// 未指定行数时返回的默认行数。
///
/// 为什么是 200：够覆盖一次失败操作的前后文，又不至于把 IPC 通道塞满。
const DEFAULT_TAIL_LINES: usize = 200;

/// 允许请求的最大行数。
///
/// 上限是必须的：这个参数来自前端，没有上限就等于给了"一次 IPC 拉 10MB 文本"的能力。
const MAX_TAIL_LINES: usize = 2000;

/// 把前端请求的行数收敛到合法范围。
///
/// 抽成纯函数是为了能被单测覆盖：边界（0、超大值、缺省）在命令层最难测，
/// 而它们恰恰是"看起来没事、一出事就是极端值"的地方。
pub fn resolve_tail_limit(requested: Option<usize>) -> usize {
    match requested {
        None => DEFAULT_TAIL_LINES,
        // 0 行没有意义：调用方显然想要日志，只是参数写错了
        Some(0) => 1,
        Some(value) => value.min(MAX_TAIL_LINES),
    }
}

/// 在系统文件管理器中打开日志目录。
///
/// 能力等级：`ReadOnly`（不读写仓库、不修改任何数据，只是打开一个文件夹）。
#[tauri::command]
pub fn logs_open(state: State<'_, AppState>) -> AppResult<()> {
    shell::open_in_file_manager(&state.log_dir)
}

/// 读取末尾若干行日志（时间线顺序：旧 → 新）。
///
/// 能力等级：`ReadOnly`。
///
/// `lines` 缺省 200，被夹在 `1..=`[`MAX_TAIL_LINES`] 之间。
/// 返回的每一行都经过脱敏，并带有解析出的时间戳/级别/target，
/// 供前端高亮"错误发生时间附近的行"。
#[tauri::command]
pub fn logs_tail(state: State<'_, AppState>, lines: Option<usize>) -> AppResult<Vec<LogLine>> {
    logging::tail(&state.log_dir, resolve_tail_limit(lines))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{resolve_tail_limit, DEFAULT_TAIL_LINES, MAX_TAIL_LINES};

    #[test]
    fn defaults_when_line_count_is_missing() {
        assert_eq!(resolve_tail_limit(None), DEFAULT_TAIL_LINES);
    }

    #[test]
    fn zero_is_treated_as_one_not_as_empty() {
        // 传 0 的调用方显然想要日志，只是参数写错了；返回空数组会让他以为"没有日志"
        assert_eq!(resolve_tail_limit(Some(0)), 1);
    }

    #[test]
    fn clamps_to_the_maximum() {
        assert_eq!(resolve_tail_limit(Some(50)), 50);
        assert_eq!(resolve_tail_limit(Some(MAX_TAIL_LINES)), MAX_TAIL_LINES);
        assert_eq!(
            resolve_tail_limit(Some(usize::MAX)),
            MAX_TAIL_LINES,
            "前端传超大值时必须夹紧，否则一次 IPC 能拉走整个日志文件"
        );
    }
}
