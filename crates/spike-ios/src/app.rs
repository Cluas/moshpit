//! The NSH app as a `UIView` for the Swift host, behind a C ABI
//! (`include/moshpit_spike.h`): create the view, register the host's native
//! components (the terminal), run the Rust root, destroy it. Modelled on
//! Foyer's (`apps/ios/rust`), which wraps `nsh_platform_ios::AppView`
//! because NSH's own embedding ABI (`NativeShell`) only runs JS bundles.
//!
//! Props, commands and events here are JSON, as NSH's are: they are the
//! control path. The bytes go through `moshpit_stream`'s C ABI instead.

use std::{
    ffi::{CStr, CString, c_char, c_void},
    rc::Rc,
    sync::{Arc, LazyLock, Mutex},
};

use nsh_platform_ios::{AppView, ComponentContext, NativeComponent, Root, SurfaceOptions};
use objc2::{MainThreadMarker, rc::Retained};
use objc2_core_foundation::CGRect;
use objc2_ui_kit::UIView;

use crate::core::core;

/// An app view (opaque to C).
pub struct MpApp {
    view: AppView,
}

/// Logs to stderr (the device console): warnings, and everything of ours.
fn init_logging() {
    struct Stderr;
    impl log::Log for Stderr {
        fn enabled(&self, m: &log::Metadata<'_>) -> bool {
            m.level() <= log::Level::Warn || m.target().starts_with("moshpit")
        }
        fn log(&self, r: &log::Record<'_>) {
            if self.enabled(r.metadata()) {
                eprintln!("[{}] {}: {}", r.target(), r.level(), r.args());
            }
        }
        fn flush(&self) {}
    }
    static LOGGER: Stderr = Stderr;
    if log::set_logger(&LOGGER).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
}

fn main_thread() -> MainThreadMarker {
    MainThreadMarker::new().expect("app view functions must be called on the main thread")
}

/// Creates an app view with `frame` (it then follows its superview's size).
/// Starts the core first: NSH runs the UI's tasks on the core's runtime.
#[unsafe(no_mangle)]
pub extern "C" fn mp_app_create(frame: CGRect) -> *mut MpApp {
    init_logging();
    // `SurfaceOptions` is `#[non_exhaustive]` on NSH main: start from the
    // default and set fields.
    let mut options = SurfaceOptions::default();
    options.runtime = Some(core().handle());
    options.fps = std::env::var_os("MOSHPIT_SPIKE_FPS").is_some();
    let view = AppView::new(main_thread(), frame, options);
    log::info!(target: "moshpit", "app view created on the core's runtime");
    Box::into_raw(Box::new(MpApp { view }))
}

/// The view to add to a hierarchy. Owned by `app`.
///
/// # Safety
///
/// `app` must come from [`mp_app_create`] and not be destroyed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_app_view(app: *mut MpApp) -> *mut UIView {
    // SAFETY: guaranteed by the caller.
    let app = unsafe { &*app };
    std::ptr::from_ref(app.view.view()).cast_mut()
}

/// Runs the spike's Rust root, replacing the running app.
///
/// # Safety
///
/// `app` must come from [`mp_app_create`] and not be destroyed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_app_run(app: *mut MpApp) {
    // SAFETY: guaranteed by the caller.
    let app = unsafe { &*app };
    app.view.run(Root::of::<crate::ui::SpikeRoot>());
}

/// Runs root `name`, replacing the running app: `"g1"` (G1's status line
/// over the terminal), `"g5"` (G5's terminal screen in an NSH navigation
/// stack), `"g5-embedded"` (the terminal screen alone, for a SwiftUI
/// navigation stack to host; its back button tells the host).
///
/// # Safety
///
/// `app` must come from [`mp_app_create`] and not be destroyed; `name` a
/// NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_app_run_root(app: *mut MpApp, name: *const c_char) {
    // SAFETY: guaranteed by the caller.
    let (app, name) = unsafe { (&*app, CStr::from_ptr(name)) };
    match name.to_bytes() {
        b"g5" => app.view.run(Root::of::<crate::screen::G5Root>()),
        b"g5-embedded" => app.view.run(Root::of::<crate::screen::G5Embedded>()),
        _ => app.view.run(Root::of::<crate::ui::SpikeRoot>()),
    }
}

// --- Between the host and the UI ----------------------------------------------

/// What the host posts to every running root (JSON text), in order.
static HOST_BUS: LazyLock<tokio::sync::broadcast::Sender<Arc<str>>> =
    LazyLock::new(|| tokio::sync::broadcast::channel(256).0);

/// The host's messages from now on, for a root to listen to.
pub(crate) fn host_messages() -> tokio::sync::broadcast::Receiver<Arc<str>> {
    HOST_BUS.subscribe()
}

/// Posts `json` (a JSON object, `{"op": ...}`) to every running root, after
/// what was posted before. Any thread; roots not listening miss it.
///
/// # Safety
///
/// `json` must be a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_spike_post(json: *const c_char) {
    // SAFETY: guaranteed by the caller.
    let json = unsafe { CStr::from_ptr(json) }.to_string_lossy();
    let _ = HOST_BUS.send(json.into());
}

/// What the UI tells the host: called with a JSON object (`{"ev": ...}`) on
/// the engine thread. The host hops to its own thread.
pub type MpHostHandler = unsafe extern "C" fn(json: *const c_char);

static HOST_HANDLER: Mutex<Option<MpHostHandler>> = Mutex::new(None);

/// Sets (or, `NULL`, clears) what hears the UI's [`tell_host`].
#[unsafe(no_mangle)]
pub extern "C" fn mp_spike_set_host_handler(handler: Option<MpHostHandler>) {
    *HOST_HANDLER.lock().unwrap_or_else(|e| e.into_inner()) = handler;
}

/// Tells the host `value` (a JSON object), if it listens.
pub(crate) fn tell_host(value: serde_json::Value) {
    let handler = *HOST_HANDLER.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(handler) = handler {
        let json = CString::new(value.to_string()).unwrap_or_default();
        // SAFETY: the handler's contract (`mp_spike_set_host_handler`).
        unsafe { handler(json.as_ptr()) };
    }
}

/// Stops the app and frees `app`.
///
/// # Safety
///
/// `app` must be `NULL` or come from [`mp_app_create`], not used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_app_destroy(app: *mut MpApp) {
    if app.is_null() {
        return;
    }
    let _mtm = main_thread();
    // SAFETY: guaranteed by the caller.
    drop(unsafe { Box::from_raw(app) });
}

// --- Native components -------------------------------------------------------

/// What a component tells its app (opaque to C): owned by the component.
pub struct MpComponentContext(ComponentContext);

/// A native component's callbacks (`moshpit_spike.h`), called on the main
/// thread.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MpComponentCallbacks {
    pub create:
        unsafe extern "C" fn(factory: *mut c_void, context: *mut MpComponentContext) -> *mut c_void,
    pub view: unsafe extern "C" fn(component: *mut c_void) -> *mut UIView,
    pub set_prop:
        unsafe extern "C" fn(component: *mut c_void, name: *const c_char, json: *const c_char),
    pub command:
        unsafe extern "C" fn(component: *mut c_void, name: *const c_char, json: *const c_char),
    pub destroy: unsafe extern "C" fn(component: *mut c_void),
    pub release_factory: Option<unsafe extern "C" fn(factory: *mut c_void)>,
}

/// A registration: the factory pointer and its callbacks.
struct CFactory {
    factory: *mut c_void,
    callbacks: MpComponentCallbacks,
}

impl Drop for CFactory {
    fn drop(&mut self) {
        if let Some(release) = self.callbacks.release_factory {
            // SAFETY: the contract of `mp_app_register_component`.
            unsafe { release(self.factory) };
        }
    }
}

/// A component made through C callbacks.
struct CComponent {
    handle: *mut c_void,
    callbacks: MpComponentCallbacks,
    view: Retained<UIView>,
}

impl NativeComponent for CComponent {
    fn view(&self) -> &UIView {
        &self.view
    }

    fn set_prop(&mut self, name: &str, value: Option<&nsh_core::JsonValue>) {
        let name = CString::new(name).unwrap_or_default();
        let json = value.map(|v| CString::new(v.to_string()).unwrap_or_default());
        // SAFETY: the contract of `mp_app_register_component`.
        unsafe {
            (self.callbacks.set_prop)(
                self.handle,
                name.as_ptr(),
                json.as_ref().map_or(std::ptr::null(), |j| j.as_ptr()),
            )
        };
    }

    fn command(&mut self, name: &str, args: &nsh_core::JsonValue) {
        let name = CString::new(name).unwrap_or_default();
        let json = CString::new(args.to_string()).unwrap_or_default();
        // SAFETY: the contract of `mp_app_register_component`.
        unsafe { (self.callbacks.command)(self.handle, name.as_ptr(), json.as_ptr()) };
    }
}

impl Drop for CComponent {
    fn drop(&mut self) {
        // SAFETY: the contract of `mp_app_register_component`.
        unsafe { (self.callbacks.destroy)(self.handle) };
    }
}

/// Registers native component `name` (`native("name")` in the Rust UI):
/// `callbacks.create(factory, context)` makes one for every element created
/// from now on.
///
/// # Safety
///
/// `app` must come from [`mp_app_create`]; `name` must be a NUL-terminated
/// string; the callbacks must be safe to call on the main thread with
/// `factory` (and the handles `create` returns) until `release_factory`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_app_register_component(
    app: *mut MpApp,
    name: *const c_char,
    factory: *mut c_void,
    callbacks: *const MpComponentCallbacks,
) {
    // SAFETY: guaranteed by the caller.
    let (app, name, callbacks) = unsafe { (&*app, CStr::from_ptr(name), *callbacks) };
    let Ok(name) = name.to_str() else {
        return log::error!(target: "moshpit", "component name is not UTF-8");
    };
    let registration = Rc::new(CFactory { factory, callbacks });
    app.view.register_component(
        name,
        Rc::new(move |context| {
            let context = Box::into_raw(Box::new(MpComponentContext(context)));
            let r = &registration;
            // SAFETY: guaranteed by the caller of `mp_app_register_component`.
            let handle = unsafe { (r.callbacks.create)(r.factory, context) };
            // SAFETY: as above; the component owns its view.
            let view = unsafe { Retained::retain((r.callbacks.view)(handle)) }
                .expect("a component's view");
            Box::new(CComponent {
                handle,
                callbacks: r.callbacks,
                view,
            })
        }),
    );
}

/// Emits event `name` with `json` data (`NULL`: null). Any thread.
///
/// # Safety
///
/// `context` must be a live context from `create`; the strings
/// NUL-terminated (`json` valid JSON or `NULL`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_component_emit(
    context: *const MpComponentContext,
    name: *const c_char,
    json: *const c_char,
) {
    // SAFETY: guaranteed by the caller.
    let (context, name) = unsafe { (&*context, CStr::from_ptr(name)) };
    let data = if json.is_null() {
        nsh_core::JsonValue::Null
    } else {
        // SAFETY: guaranteed by the caller.
        let json = unsafe { CStr::from_ptr(json) }.to_string_lossy();
        serde_json::from_str(&json).unwrap_or_else(|err| {
            log::error!(target: "moshpit", "component event data is not JSON: {err}");
            nsh_core::JsonValue::Null
        })
    };
    context.0.emit(&name.to_string_lossy(), data);
}

/// Reports the component's own size in points. Any thread.
///
/// # Safety
///
/// `context` must be a live context from `create`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_component_set_size(
    context: *const MpComponentContext,
    width: f64,
    height: f64,
) {
    // SAFETY: guaranteed by the caller.
    let context = unsafe { &*context };
    context.0.set_size(width as f32, height as f32);
}

/// Frees a context.
///
/// # Safety
///
/// `context` must be `NULL` or a context from `create`, not freed before.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_component_context_free(context: *mut MpComponentContext) {
    if !context.is_null() {
        // SAFETY: guaranteed by the caller.
        drop(unsafe { Box::from_raw(context) });
    }
}

// --- The bench ---------------------------------------------------------------

/// The burst producer's report ([`crate::core::BurstReport`]).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct MpSpikeBurst {
    pub bytes: u64,
    pub chunks: u64,
    pub hash: u64,
    pub push_nanos: u64,
}

/// The terminal's stream (the one the core echoes).
#[unsafe(no_mangle)]
pub extern "C" fn mp_spike_terminal_stream() -> u64 {
    core().terminal()
}

/// G5's pane `number`'s stream (1: the shell, else the TUI), opened on
/// first ask.
#[unsafe(no_mangle)]
pub extern "C" fn mp_spike_pane_stream(number: u32) -> u64 {
    core().pane(number)
}

/// Pushes `text` as a line of output to G5's pane `number`.
///
/// # Safety
///
/// `text` must be a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_spike_pane_say(number: u32, text: *const c_char) {
    // SAFETY: guaranteed by the caller.
    let text = unsafe { CStr::from_ptr(text) }.to_string_lossy();
    core().pane_say(number, &text);
}

/// Starts the burst: `total` bytes of seed `seed` pushed to the terminal's
/// stream as fast as the core can make them.
#[unsafe(no_mangle)]
pub extern "C" fn mp_spike_burst_start(total: u64, seed: u64) {
    core().start_burst(total, seed);
}

/// Fills `out` with the burst's report; `false` until the producer reached
/// its last chunk (it records the report before pushing that chunk).
///
/// # Safety
///
/// `out` must be valid for a write of an `MpSpikeBurst`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_spike_burst_report(out: *mut MpSpikeBurst) -> bool {
    let Some(report) = core().burst_report() else {
        return false;
    };
    if out.is_null() {
        return false;
    }
    // SAFETY: guaranteed by the caller.
    unsafe {
        out.write(MpSpikeBurst {
            bytes: report.bytes,
            chunks: report.chunks,
            hash: report.hash,
            push_nanos: report.push_nanos,
        })
    };
    true
}

// --- G4: what the core took ---------------------------------------------------

/// What a stream's core end took from the view ([`crate::probe::Taken`]),
/// in counts.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct MpSpikeTaken {
    /// Input bytes, all events together.
    pub bytes: u64,
    /// Input events that carried bytes.
    pub chunks: u64,
    /// Resizes.
    pub resizes: u64,
    /// The last resize (0 × 0 before any) and when the core took it
    /// (`DispatchTime`'s clock, nanoseconds).
    pub cols: u16,
    pub rows: u16,
    pub resized_at: u64,
}

/// Fills `out` with what stream `id`'s core end took so far.
///
/// # Safety
///
/// `out` must be valid for a write of an `MpSpikeTaken`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_spike_taken(id: u64, out: *mut MpSpikeTaken) {
    if out.is_null() {
        return;
    }
    let taken = crate::probe::with_taken(id, |t| {
        t.map_or_else(MpSpikeTaken::default, |t| {
            let (cols, rows, resized_at) = t.resizes.last().copied().unwrap_or_default();
            MpSpikeTaken {
                bytes: t.bytes.len() as u64,
                chunks: t.chunks,
                resizes: t.resizes.len() as u64,
                cols,
                rows,
                resized_at,
            }
        })
    });
    // SAFETY: guaranteed by the caller.
    unsafe { out.write(taken) };
}

/// Copies up to `cap` of the input bytes stream `id`'s core end took,
/// starting at byte `offset`, into `buf`; returns how many.
///
/// # Safety
///
/// `buf` must be valid for writes of `cap` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_spike_taken_bytes(
    id: u64,
    offset: u64,
    buf: *mut u8,
    cap: usize,
) -> usize {
    if buf.is_null() {
        return 0;
    }
    crate::probe::with_taken(id, |t| {
        let Some(t) = t else { return 0 };
        let start = usize::try_from(offset)
            .unwrap_or(usize::MAX)
            .min(t.bytes.len());
        let n = (t.bytes.len() - start).min(cap);
        // SAFETY: guaranteed by the caller; `n` is within both buffers.
        unsafe { std::ptr::copy_nonoverlapping(t.bytes[start..].as_ptr(), buf, n) };
        n
    })
}
