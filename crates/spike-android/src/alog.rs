//! The core's log lines, under a tag of their own (`adb logcat -s
//! MoshpitCore`), apart from NSH's (`nsh`).

use std::ffi::{CString, c_char, c_int};

#[link(name = "log")]
unsafe extern "C" {
    fn __android_log_write(prio: c_int, tag: *const c_char, text: *const c_char) -> c_int;
}

const INFO: c_int = 4;
const WARN: c_int = 5;

pub fn info(message: impl AsRef<str>) {
    write(INFO, message.as_ref());
}

pub fn warn(message: impl AsRef<str>) {
    write(WARN, message.as_ref());
}

fn write(priority: c_int, message: &str) {
    let text = CString::new(message.replace('\0', " ")).unwrap_or_default();
    // SAFETY: valid NUL-terminated strings.
    unsafe { __android_log_write(priority, c"MoshpitCore".as_ptr(), text.as_ptr()) };
}
