//! 会话输出的行式回滚缓冲（T5.2）。
//!
//! 任务书要求"会话退出后保留最后 1000 行输出供查看"。前端 xterm.js 自己也有
//! 缓冲，但后端保留一份意味着：会话死掉后重新打开查看、以及将来任何
//! "把尾部输出贴进诊断/反馈"的能力都不依赖前端是否还挂在那个 tab 上。
//!
//! 为什么按行而不是按字节：查看与诊断都以行为单位；`\r` 刷新的进度条
//! 不产生新行，归并进"正在累积的行"即可。

use std::collections::VecDeque;

use parking_lot::Mutex;

/// 保留的行数上限（T5.2 任务书指定 1000 行）。
pub const SCROLLBACK_MAX_LINES: usize = 1000;

/// 单行字节上限：一行超长（大块 base64、无限进度刷新）时截断，
/// 防止"一行"把回滚缓冲撑到不可控。
const LINE_MAX_BYTES: usize = 8 * 1024;

#[derive(Default)]
struct ScrollbackState {
    /// 已完成的行（不含换行符；最旧的在队头）。
    done: VecDeque<Vec<u8>>,
    /// 正在累积的行（还没有遇到 `\n`）。
    current: Vec<u8>,
}

/// 行式回滚缓冲。
#[derive(Default)]
pub struct Scrollback {
    state: Mutex<ScrollbackState>,
}

impl Scrollback {
    /// 空缓冲。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 追加一段输出。`\n` 收线，行尾 `\r` 去除（CRLF），超长行截断，
    /// 超过 [`SCROLLBACK_MAX_LINES`] 时淘汰最旧的行。
    pub fn push(&self, bytes: &[u8]) {
        let mut state = self.state.lock();
        for &byte in bytes {
            if byte == b'\n' {
                let mut line = std::mem::take(&mut state.current);
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                line.truncate(LINE_MAX_BYTES);
                if state.done.len() == SCROLLBACK_MAX_LINES {
                    state.done.pop_front();
                }
                state.done.push_back(line);
            } else {
                state.current.push(byte);
                state.current.truncate(LINE_MAX_BYTES);
            }
        }
    }

    /// 取最后 `max_lines` 行（含尚未收线的尾行），按输出顺序（旧 → 新）。
    ///
    /// 非法 UTF-8 字节按替换符呈现（终端输出可能含任意控制字节，
    /// 这里是"供查看"，不承诺可解析）。
    #[must_use]
    pub fn tail(&self, max_lines: usize) -> Vec<String> {
        let state = self.state.lock();
        let skip = state.done.len().saturating_sub(max_lines);
        let mut lines: Vec<String> = state
            .done
            .iter()
            .skip(skip)
            .map(|line| String::from_utf8_lossy(line).into_owned())
            .collect();
        if !state.current.is_empty() {
            lines.push(String::from_utf8_lossy(&state.current).into_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    /// 基本分行：CRLF 去除、多段输入正确收线。
    #[test]
    fn pushes_are_split_into_lines_with_crlf_stripped() {
        let scrollback = Scrollback::new();
        scrollback.push(b"first\r\nsec");
        scrollback.push(b"ond\nthird");
        assert_eq!(
            scrollback.tail(100),
            vec!["first", "second", "third"],
            "跨块的行必须拼回来，行尾 \\r 必须去掉"
        );
    }

    /// 超过上限时淘汰最旧的行，且"正在累积的行"计入 tail。
    #[test]
    fn evicts_the_oldest_lines_beyond_the_cap() {
        let scrollback = Scrollback::new();
        for index in 0..(SCROLLBACK_MAX_LINES + 10) {
            scrollback.push(format!("line-{index}\n").as_bytes());
        }
        scrollback.push(b"partial");
        let tail = scrollback.tail(SCROLLBACK_MAX_LINES);
        // 1000 行已收线 + 1 行未收线的 tail（后者不计入上限，它还不是"行"）
        assert_eq!(tail.len(), SCROLLBACK_MAX_LINES + 1);
        assert_eq!(tail[0], format!("line-{}", 10), "最旧的 10 行被淘汰");
        assert_eq!(tail.last().expect("non-empty"), "partial");
    }

    /// 超长行被截断到单行上限，不会把缓冲撑爆。
    #[test]
    fn overly_long_lines_are_truncated() {
        let scrollback = Scrollback::new();
        scrollback.push(&vec![b'x'; LINE_MAX_BYTES * 4]);
        scrollback.push(b"\n");
        let tail = scrollback.tail(10);
        assert_eq!(tail.len(), 1);
        assert!(tail[0].len() <= LINE_MAX_BYTES, "单行必须被截断");
    }

    /// 空缓冲与非 UTF-8 字节都不 panic。
    #[test]
    fn empty_and_non_utf8_input_are_safe() {
        let scrollback = Scrollback::new();
        assert!(scrollback.tail(10).is_empty());
        scrollback.push(&[0xFF, 0xFE, b'\n']);
        assert_eq!(scrollback.tail(10), vec!["\u{FFFD}\u{FFFD}"]);
    }
}
