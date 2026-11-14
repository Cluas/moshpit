//! Our log lines in logcat, under `MoshpitCore` (`adb logcat -s
//! MoshpitCore`), apart from NSH's (`nsh`).

use std::ffi::{CString, c_char, c_int};

#[link(name = "log")]
unsafe extern "C" {
    fn __android_log_write(prio: c_int, tag: *const c_char, text: *const c_char) -> c_int;
}

/// Writes one line at `level`, its target in front.
pub fn write(level: log::Level, target: &str, message: &str) {
    let priority = match level {
        log::Level::Error => 6,
        log::Level::Warn => 5,
        log::Level::Info => 4,
        log::Level::Debug => 3,
        log::Level::Trace => 2,
    };
    let line = if target == "moshpit" {
        message.to_owned()
    } else {
        format!("[{target}] {message}")
    };
    let text = CString::new(line.replace('\0', " ")).unwrap_or_default();
    // SAFETY: valid NUL-terminated strings.
    unsafe { __android_log_write(priority, c"MoshpitCore".as_ptr(), text.as_ptr()) };
}
