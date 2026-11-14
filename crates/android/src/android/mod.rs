//! The JNI the Java host calls (`android/app/src/main/java/com/cluas/
//! moshpit`): NSH's Rust entry, `Core`'s entry points, the byte channel
//! ([`stream`]), and the Java platform the controller calls back
//! ([`java::JavaPlatform`]).
//!
//! A surface runs this way: the controller pushes a page
//! ([`crate::platform::Platform::push`]); the host makes an `NshView`,
//! registers its native components, calls `Core.prepare(surface)` and then
//! `view.runRustApp(…)`, which calls `Native.runRust(handle)` on the same
//! thread — defined here: it takes the surface `prepare` named, hands NSH
//! the core's runtime for the view and runs the surface's root. A view's
//! `handle` is NSH's private field, so the surface rides a thread-local
//! between the two calls.
//!
//! No call lets a panic unwind into the VM: it is caught and logged, and the
//! call returns its empty value.

pub mod alog;
mod java;
mod stream;

use std::cell::Cell;
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, OnceLock};

use moshpit_stream::Mark;
use nsh::jni::{
    JNIEnv,
    objects::{JClass, JObject, JString},
    sys::{jboolean, jlong, jstring},
};

use crate::app::{Core, Options, Pending, journal};

static CORE: OnceLock<Arc<Core>> = OnceLock::new();

thread_local! {
    /// The surface the next `Native.runRust` on this thread runs.
    static PREPARED: Cell<Option<u64>> = const { Cell::new(None) };
}

/// Runs `f`, or logs its panic and returns `failed`.
fn guard<T>(call: &str, failed: T, f: impl FnOnce() -> T) -> T {
    match std::panic::catch_unwind(AssertUnwindSafe(f)) {
        Ok(value) => value,
        Err(panic) => {
            let message = panic
                .downcast_ref::<&str>()
                .map(|s| (*s).to_owned())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_default();
            log::error!(target: "moshpit", "{call} panicked: {message}");
            failed
        }
    }
}

fn core() -> Option<&'static Arc<Core>> {
    CORE.get()
}

fn read(env: &mut JNIEnv<'_>, text: &JString<'_>) -> Option<String> {
    if text.is_null() {
        return None;
    }
    env.get_string(text).ok().map(String::from)
}

/// `Native.runRust` of NSH's runtime: the surface `Core.prepare` named runs
/// in the view with native handle `handle`.
#[unsafe(no_mangle)]
extern "system" fn Java_com_longbridge_nsh_Native_runRust(
    _: JNIEnv<'_>,
    _: JClass<'_>,
    handle: jlong,
) {
    guard("runRust", (), || {
        let Some(surface) = PREPARED.with(Cell::take) else {
            log::error!(target: "moshpit", "runRust: no surface prepared");
            return;
        };
        let Some(core) = core() else {
            return;
        };
        let Some(pending) = core.take_pending(surface) else {
            log::warn!(target: "moshpit", "runRust: surface {surface} has nothing to run");
            return;
        };
        // SAFETY: Java passes its view's live handle, on the view's UI
        // thread, before the view loads an app (this call is the load).
        unsafe { nsh::set_runtime(handle, Some(moshpit_runtime::handle())) };
        let Pending::Screen {
            screen,
            params,
            channel,
        } = pending;
        let root = screen.root(&channel, &params);
        nsh::run_root(handle, root);
        core.surface_ready(surface);
    });
}

/// `Core.start(platform, version, deviceLabel, debuggable)`: the controller
/// on `platform`, Home on screen. Again (the activity made anew): the old
/// pages go and Home is pushed again.
#[unsafe(no_mangle)]
extern "system" fn Java_com_cluas_moshpit_Core_start(
    mut env: JNIEnv<'_>,
    _: JClass<'_>,
    platform: JObject<'_>,
    version: JString<'_>,
    device: JString<'_>,
    debuggable: jboolean,
) {
    guard("Core.start", (), || {
        journal::init();
        let version = read(&mut env, &version).unwrap_or_default();
        let device = read(&mut env, &device).unwrap_or_else(|| "Android".to_owned());
        if let Some(core) = core() {
            core.restart();
            return;
        }
        let Some(platform) = java::JavaPlatform::new(&mut env, &platform) else {
            log::error!(target: "moshpit", "Core.start: no platform");
            return;
        };
        let options = Options {
            version,
            device_label: device,
            debuggable: debuggable != 0,
            ..Options::default()
        };
        let core = Core::new(Arc::new(platform), options);
        if CORE.set(core.clone()).is_ok() {
            log::info!(target: "moshpit", "core up on {} threads", moshpit_runtime::WORKER_THREADS);
            core.start();
        }
    });
}

#[unsafe(no_mangle)]
extern "system" fn Java_com_cluas_moshpit_Core_prepare(
    _: JNIEnv<'_>,
    _: JClass<'_>,
    surface: jlong,
) {
    PREPARED.with(|p| p.set(Some(surface as u64)));
}

#[unsafe(no_mangle)]
extern "system" fn Java_com_cluas_moshpit_Core_gone(_: JNIEnv<'_>, _: JClass<'_>, surface: jlong) {
    guard("Core.gone", (), || {
        if let Some(core) = core() {
            core.surface_gone(surface as u64);
        }
    });
}

#[unsafe(no_mangle)]
extern "system" fn Java_com_cluas_moshpit_Core_back(_: JNIEnv<'_>, _: JClass<'_>) -> jboolean {
    guard("Core.back", 0, || {
        u8::from(core().is_some_and(|c| c.back()))
    })
}

#[unsafe(no_mangle)]
extern "system" fn Java_com_cluas_moshpit_Core_answer(
    mut env: JNIEnv<'_>,
    _: JClass<'_>,
    token: jlong,
    ok: jboolean,
    text: JString<'_>,
) {
    guard("Core.answer", (), || {
        let text = read(&mut env, &text);
        if let Some(core) = core() {
            core.answer(token as u64, ok != 0, text);
        }
    });
}

#[unsafe(no_mangle)]
extern "system" fn Java_com_cluas_moshpit_Core_lifecycle(
    _: JNIEnv<'_>,
    _: JClass<'_>,
    foreground: jboolean,
) {
    guard("Core.lifecycle", (), || {
        if let Some(core) = core() {
            core.lock().sessions.set_foreground(foreground != 0);
        }
    });
}

#[unsafe(no_mangle)]
extern "system" fn Java_com_cluas_moshpit_Core_networkChanged(_: JNIEnv<'_>, _: JClass<'_>) {
    guard("Core.networkChanged", (), || {
        if let Some(core) = core() {
            core.lock().sessions.network_changed();
        }
    });
}

#[unsafe(no_mangle)]
extern "system" fn Java_com_cluas_moshpit_Core_unlocked(_: JNIEnv<'_>, _: JClass<'_>) {
    guard("Core.unlocked", (), || {
        if let Some(core) = core() {
            core.lock().sessions.device_unlocked();
        }
    });
}

#[unsafe(no_mangle)]
extern "system" fn Java_com_cluas_moshpit_Core_share(
    mut env: JNIEnv<'_>,
    _: JClass<'_>,
    text: JString<'_>,
) {
    guard("Core.share", (), || {
        if let (Some(core), Some(text)) = (core(), read(&mut env, &text)) {
            core.lock().share(text);
        }
    });
}

#[unsafe(no_mangle)]
extern "system" fn Java_com_cluas_moshpit_Core_keyField(
    mut env: JNIEnv<'_>,
    _: JClass<'_>,
    kind: JString<'_>,
    text: JString<'_>,
) {
    guard("Core.keyField", (), || {
        let (Some(kind), Some(text)) = (read(&mut env, &kind), read(&mut env, &text)) else {
            return;
        };
        if let Some(core) = core() {
            core.lock().key_field(&kind, text);
        }
    });
}

/// `Core.picked(token, bytes)`: the file the document picker asked for
/// under `token` (null: none).
#[unsafe(no_mangle)]
extern "system" fn Java_com_cluas_moshpit_Core_picked(
    env: JNIEnv<'_>,
    _: JClass<'_>,
    token: jlong,
    bytes: nsh::jni::objects::JByteArray<'_>,
) {
    guard("Core.picked", (), || {
        let bytes = (!bytes.is_null())
            .then(|| env.convert_byte_array(&bytes).ok())
            .flatten();
        if let Some(core) = core() {
            core.picked(token as u64, bytes);
        }
    });
}

/// `Core.ctrlUsed()`: the sticky ctrl went with a key.
#[unsafe(no_mangle)]
extern "system" fn Java_com_cluas_moshpit_Core_ctrlUsed(_: JNIEnv<'_>, _: JClass<'_>) {
    guard("Core.ctrlUsed", (), || {
        if let Some(core) = core() {
            core.lock().ctrl_used();
        }
    });
}

/// `Core.debug(json)`: the test hooks of a debuggable build
/// ([`crate::app::Core::debug`]); their answer as JSON.
#[unsafe(no_mangle)]
extern "system" fn Java_com_cluas_moshpit_Core_debug(
    mut env: JNIEnv<'_>,
    _: JClass<'_>,
    json: JString<'_>,
) -> jstring {
    let answer = guard("Core.debug", String::new(), || {
        let Some(json) = read(&mut env, &json) else {
            return String::new();
        };
        core().map(|core| core.debug(&json)).unwrap_or_default()
    });
    env.new_string(answer)
        .map(JString::into_raw)
        .unwrap_or(std::ptr::null_mut())
}

/// A mark the view read in stream `stream`'s output: to the controller, on
/// the core's runtime (never on the main thread, which is reading).
pub(crate) fn view_mark(stream: u64, mark: Mark) {
    if let Some(core) = core() {
        let core = core.clone();
        moshpit_runtime::handle().spawn(async move {
            core.lock().view_mark(stream, mark);
        });
    }
}
