//! ForgeDesk 插件 SDK（T6.5）。
//!
//! 把 PLUGIN-API.md 的 ABI 约定封装成插件作者直接可用的形态：
//!
//! - `fd_alloc` 导出（宿主写参数用的分配区）；
//! - `fd.log` / `fd.host_call` / `fd.host_result` 导入的安全封装；
//! - `fd_invoke` / `fd_render_panel` / `fd_on_event` 的返回值打包
//!   （`(结果指针 << 32) | 结果长度`，见 PLUGIN-API.md §3.1）；
//! - 极简 JSON 字符串转义。
//!
//! # 内存布局（插件自己的线性内存）
//!
//! ```text
//! HEAP_BUF  1 MiB  fd_alloc 分配区（宿主写参数；下一次调用即作废）
//! ARG_BUF   1 MiB  host_call 参数暂存（调用期间必须稳定）
//! RESP_BUF  1 MiB  host_result / 命令结果的暂存
//! ```
//!
//! 三个缓冲都是线性内存里的静态分配（64 MiB 上限内绰绰有余）。
//! 宿主在调用返回后立刻读取结果区、之后内容才会被下一次调用覆盖——
//! 这是宿主读取约定（PLUGIN-API.md §4.3），不依赖跨调用保活。

#![no_std]

extern crate alloc;

use alloc::alloc::Layout;
use alloc::borrow::ToOwned;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::panic::PanicInfo;

const ARG_SIZE: usize = 1024 * 1024;
const RESP_SIZE: usize = 1024 * 1024;

static mut ARG_BUF: [u8; ARG_SIZE] = [0; ARG_SIZE];
static mut RESP_BUF: [u8; RESP_SIZE] = [0; RESP_SIZE];

/// freestanding 插件没有 std：全局分配器用 dlmalloc 的 GlobalDlmalloc
/// （wasm 家族后端，需要扩容时走 wasm memory.grow，受宿主 64MiB 上限约束）。
#[global_allocator]
static ALLOCATOR: dlmalloc::GlobalDlmalloc = dlmalloc::GlobalDlmalloc;

/// panic = wasm trap：宿主引擎把它映射为结构化错误并隔离插件
///（插件作者用 log 记录预期错误，panic 只留给真正的 bug）。
#[panic_handler]
fn panic_handler(_info: &PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}

#[link(wasm_import_module = "fd")]
extern "C" {
    #[link_name = "log"]
    fn fd_log(level: i32, ptr: i32, len: i32);
    #[link_name = "host_call"]
    fn fd_host_call(op: i32, ptr: i32, len: i32) -> i32;
    #[link_name = "host_result"]
    fn fd_host_result(out_ptr: i32, out_cap: i32) -> i32;
}

/// 宿主写入参数用的分配（ABI 导出）。
///
/// 直接用全局分配器：dlmalloc 负责复用，插件作者无需理解内部布局。
/// 返回 0 表示失败（宿主把 0 当作"指向零页"的合法指针写入，但参数区
/// 语义要求成功；0 也恰好是 NULL 约定，宿主侧把它当参数错误处理）。
#[no_mangle]
pub extern "C" fn fd_alloc(len: i32) -> i32 {
    if len <= 0 {
        return 0;
    }
    let Ok(layout) = Layout::from_size_align(len as usize, 8) else {
        return 0;
    };
    unsafe { alloc::alloc::alloc(layout) as i32 }
}

/// 打日志（0=debug 1=info 2=warn 3=error；无需权限）。
pub fn log(level: i32, message: &str) {
    let bytes = message.as_bytes();
    let len = bytes.len().min(ARG_SIZE);
    unsafe {
        ARG_BUF[..len].copy_from_slice(&bytes[..len]);
        fd_log(level, ARG_BUF.as_ptr() as i32, len as i32);
    }
}

/// 宿主调用失败：ABI 错误码 + staging 里的详情 JSON。
#[derive(Debug)]
pub struct HostCallError {
    /// ABI 错误码（PLUGIN-API.md §5；0 区间不会出现）。
    pub code: i32,
    /// `{"error":{"code":..,"message":..}}` 形状的详情文本。
    pub detail: String,
}

/// 让插件里的 `?` 直接把错误折叠成 String（detail 优先，错误码兜底）。
impl From<HostCallError> for String {
    fn from(error: HostCallError) -> Self {
        if error.detail.is_empty() {
            format!("host_call failed with code {}", error.code)
        } else {
            error.detail
        }
    }
}

/// 发 toast（需要 `ui:toast` 权限）。
pub fn show_toast(level: &str, message: &str) -> bool {
    host_call(
        14,
        &format!(
            "{{\"level\":{},\"message\":{}}}",
            jstr(level),
            jstr(message)
        ),
    )
    .is_ok()
}

/// 一次宿主调用：JSON 入参 → JSON 结果串。
///
/// `op` 取 PLUGIN-API.md §4.1 的 id；权限在宿主侧按清单∩授权裁决，
/// 被拒时返回 [`HostCallError`]（code = -2）。
pub fn host_call(op: i32, args: &str) -> Result<String, HostCallError> {
    let args_bytes = args.as_bytes();
    if args_bytes.len() > ARG_SIZE {
        return Err(HostCallError {
            code: -6,
            detail: "args too large".to_owned(),
        });
    }
    unsafe {
        ARG_BUF[..args_bytes.len()].copy_from_slice(args_bytes);
    }
    let code = unsafe { fd_host_call(op, ARG_BUF.as_ptr() as i32, args_bytes.len() as i32) };
    if code != 0 {
        let detail = take_result();
        return Err(HostCallError { code, detail });
    }
    Ok(take_result())
}

/// 从宿主 staging 取回结果到 RESP_BUF 并转 String（staging 空返回空串）。
fn take_result() -> String {
    unsafe {
        let required = fd_host_result(0, 0).unsigned_abs() as usize;
        if required == 0 {
            return String::new();
        }
        if required > RESP_SIZE {
            // 超过 SDK 缓冲：诚实截断失败（宿主 staging 仍保留，可重试更大缓冲）
            return String::new();
        }
        let written = fd_host_result(unsafe { RESP_BUF.as_mut_ptr() } as i32, required as i32);
        let written = usize::try_from(written).unwrap_or(0).min(RESP_SIZE);
        unsafe { String::from_utf8_lossy(&RESP_BUF[..written]).into_owned() }
    }
}

/// 把插件命令/面板渲染的结果按 ABI 打包：`(指针 << 32) | 长度`。
pub fn pack_result(text: &str) -> i64 {
    let bytes = text.as_bytes();
    let len = bytes.len().min(RESP_SIZE);
    unsafe {
        RESP_BUF[..len].copy_from_slice(&bytes[..len]);
    }
    ((unsafe { RESP_BUF.as_ptr() } as usize as i64) << 32) | len as i64
}

/// 从宿主写入的参数区读字符串（fd_invoke / fd_on_event 的参数）。
pub fn read_args(ptr: i32, len: i32) -> String {
    let ptr = ptr.max(0) as usize;
    let len = len.max(0) as usize;
    // 宿主保证 [ptr, ptr+len) 在插件线性内存内（写入前做过边界校验）；
    // 这里只能信任指针约定，越界由 wasm 本身的内存访问 trap 兜底
    let slice = unsafe { core::slice::from_raw_parts(ptr as *const u8, len) };
    String::from_utf8_lossy(slice).into_owned()
}

// wasm32-wasip1 的链接器期望 libc 的内存比较符号（compiler_builtins 不提供）；
// freestanding 插件自带最小实现（语义与 C 一致：负/零/正）。
#[no_mangle]
pub extern "C" fn memcmp(a: *const u8, b: *const u8, n: usize) -> i32 {
    let (a, b) = (a as *const u8, b as *const u8);
    for index in 0..n {
        let (left, right) = (unsafe { *a.add(index) }, unsafe { *b.add(index) });
        if left != right {
            return i32::from(left) - i32::from(right);
        }
    }
    0
}

#[no_mangle]
pub extern "C" fn strlen(pointer: *const u8) -> usize {
    let mut length = 0usize;
    unsafe {
        while *pointer.add(length) != 0 {
            length += 1;
        }
    }
    length
}

/// JSON 字符串字面量（含两端引号，转义引号/反斜杠/控制字符）。
pub fn jstr(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jstr_escapes_quotes_and_control_chars() {
        assert_eq!(jstr("a\"b\\c\nd"), "\"a\\\"b\\\\c\\nd\"");
    }
}
