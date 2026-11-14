//! The NSH view as a `UIView` for the Swift host, behind a C ABI
//! (`include/moshpit_ui.h`, part of the app's `MoshpitFFI` module): create a
//! surface, register the host's native components, run a screen, post to
//! it and hear it, destroy it. Ported from spikes G1 and G5
//! (`crates/spike-ios/src/app.rs`), which modelled it on Foyer's: it wraps
//! `nsh_platform_ios::AppView`, because NSH's own embedding ABI
//! (`NativeShell`) only runs JS bundles.
//!
//! - One surface per NSH view, each with its own [`Channel`] (the spikes had
//!   one bus for every root): a screen hears its own host only.
//! - NSH runs the screens' async tasks on the core's runtime
//!   (`moshpit_runtime::handle`, `SurfaceOptions::runtime`): one set of
//!   worker threads in the app, at user-interactive QoS. The runtime starts
//!   with the first surface, so an app that shows no NSH screen starts none.
//! - The surfaces keep nothing on disk (no data directory, no cache): the
//!   screens are the app's, not apps NSH downloads. Their `asset:/` files are
//!   the app bundle's resources (an icon's preview, as `UIImage(named:)`
//!   finds it).
//! - Props, commands and events are JSON: the control path. No call lets a
//!   panic unwind into the host: it is caught, logged, and the call fails
//!   (`NULL`, `false`).

use std::{
    ffi::{CStr, CString, c_char, c_void},
    panic::AssertUnwindSafe,
    ptr,
    rc::Rc,
    sync::Arc,
};

use nsh_platform_ios::{
    AppView, AssetBase, CacheOptions, ComponentContext, NativeComponent, SurfaceOptions,
};
use objc2::{MainThreadMarker, rc::Retained};
use objc2_core_foundation::CGRect;
use objc2_ui_kit::UIView;

use crate::{
    channel::Channel,
    screen::{Params, Screen},
};

/// `MP_UI_ABI_VERSION` in `include/moshpit_ui.h`: bumped whenever the ABI
/// changes shape, so the app refuses a library it would misread.
pub const ABI_VERSION: u32 = 2;

/// The ABI version the library was built with.
#[unsafe(no_mangle)]
pub extern "C" fn mp_ui_abi_version() -> u32 {
    ABI_VERSION
}

/// Runs `f`, or logs its panic and returns `failed`: no panic unwinds into
/// the host.
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

/// The process's logger ([`crate::host_log`]): stderr, and the host's
/// handler for warnings and errors once it set one.
fn init_logging() {
    crate::host_log::install();
}

/// What hears the screens' warnings and errors (`mp_ui_set_log_handler`):
/// `level` 1 for an error, 2 for a warning (`log::Level`'s order), the
/// record's target and message. Any thread.
pub type MpUiLogHandler = unsafe extern "C" fn(
    context: *mut c_void,
    level: u8,
    target: *const c_char,
    message: *const c_char,
);

/// A log handler and its context, called from any thread.
struct LogHandler {
    handler: MpUiLogHandler,
    context: *mut c_void,
}

// SAFETY: the contract of `mp_ui_set_log_handler`: the handler may be
// called with its context from any thread, at any time until cleared.
unsafe impl Send for LogHandler {}
// SAFETY: as above; it is only ever called.
unsafe impl Sync for LogHandler {}

impl LogHandler {
    fn call(&self, level: log::Level, target: &str, message: &str) {
        let (target, message) = (
            crate::host_log::c_text(target),
            crate::host_log::c_text(message),
        );
        // SAFETY: the contract of `mp_ui_set_log_handler`.
        unsafe { (self.handler)(self.context, level as u8, target.as_ptr(), message.as_ptr()) };
    }
}

/// Sets (NULL: clears) what hears the screens' warnings and errors, and
/// installs the logger if no surface did yet. Once, as the app starts.
///
/// # Safety
///
/// `handler` must be safe to call with `context` from any thread, for as
/// long as it is set (the app sets it once and never clears it).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_ui_set_log_handler(
    handler: Option<MpUiLogHandler>,
    context: *mut c_void,
) {
    guard("mp_ui_set_log_handler", (), || {
        init_logging();
        crate::host_log::set_handler(handler.map(|handler| {
            let handler = LogHandler { handler, context };
            std::sync::Arc::new(move |level: log::Level, target: &str, message: &str| {
                handler.call(level, target, message)
            }) as crate::host_log::Handler
        }));
    })
}

/// The app bundle's resources: the directory of the app's executable (an
/// iOS bundle is flat, `Bundle.main.resourcePath` is that directory).
fn app_resources() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    exe.parent().map(std::path::Path::to_path_buf)
}

/// A surface: an NSH view and its channel (opaque to C).
pub struct MpUiSurface {
    view: AppView,
    channel: Arc<Channel>,
}

impl Drop for MpUiSurface {
    fn drop(&mut self) {
        // The host hears nothing more (a call in progress returns first),
        // then the view stops the app.
        self.channel.close();
    }
}

/// The surface behind `surface`, on the main thread.
///
/// # Safety
///
/// `surface` must come from [`mp_ui_surface_create`] and not be destroyed.
unsafe fn surface<'a>(surface: *mut MpUiSurface) -> Option<&'a MpUiSurface> {
    MainThreadMarker::new()?;
    // SAFETY: guaranteed by the caller.
    unsafe { surface.as_ref() }
}

/// A NUL-terminated string's text, if it is UTF-8.
///
/// # Safety
///
/// `text` must be `NULL` or a NUL-terminated string.
unsafe fn string<'a>(text: *const c_char) -> Option<&'a str> {
    if text.is_null() {
        return None;
    }
    // SAFETY: guaranteed by the caller.
    unsafe { CStr::from_ptr(text) }.to_str().ok()
}

/// Creates a surface with `frame` (it then follows its superview's size):
/// empty until a screen runs. `NULL` off the main thread or if NSH failed
/// to start.
#[unsafe(no_mangle)]
pub extern "C" fn mp_ui_surface_create(frame: CGRect) -> *mut MpUiSurface {
    guard("mp_ui_surface_create", ptr::null_mut(), || {
        init_logging();
        let Some(mtm) = MainThreadMarker::new() else {
            log::error!(target: "moshpit", "mp_ui_surface_create: not on the main thread");
            return ptr::null_mut();
        };
        // `SurfaceOptions` is `#[non_exhaustive]` on NSH main: start from
        // the default and set fields.
        let mut options = SurfaceOptions::default();
        options.runtime = Some(moshpit_runtime::handle());
        options.data_dir = None;
        options.cache = CacheOptions::off();
        options.asset_base = app_resources().map(AssetBase::Directory);
        let view = AppView::new(mtm, frame, options);
        let channel = Channel::open();
        log::info!(target: "moshpit", "ui surface {} made on the core's runtime", channel.id());
        Box::into_raw(Box::new(MpUiSurface { view, channel }))
    })
}

/// The view to add to a hierarchy; owned by the surface. Main thread.
///
/// # Safety
///
/// `surface` must come from [`mp_ui_surface_create`] and not be destroyed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_ui_surface_view(surface: *mut MpUiSurface) -> *mut UIView {
    // SAFETY: guaranteed by the caller.
    match unsafe { self::surface(surface) } {
        Some(surface) => ptr::from_ref(surface.view.view()).cast_mut(),
        None => ptr::null_mut(),
    }
}

/// Runs screen `screen` (`"recent-log"`) with `params` (JSON or `NULL`:
/// `{"locale": …, "theme": {…}}`), replacing what ran. `false` (nothing
/// runs) for an unknown screen or params it cannot read. Main thread.
///
/// # Safety
///
/// `surface` must come from [`mp_ui_surface_create`] and not be destroyed;
/// `screen` a NUL-terminated string; `params` `NULL` or one.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_ui_surface_run(
    surface: *mut MpUiSurface,
    screen: *const c_char,
    params: *const c_char,
) -> bool {
    guard("mp_ui_surface_run", false, || {
        // SAFETY: guaranteed by the caller.
        let (surface, name, params) =
            unsafe { (self::surface(surface), string(screen), string(params)) };
        let Some(surface) = surface else {
            return false;
        };
        let Some(screen) = name.and_then(Screen::named) else {
            log::error!(target: "moshpit", "mp_ui_surface_run: no screen {name:?}");
            return false;
        };
        let params = match Params::parse(params) {
            Ok(params) => params,
            Err(error) => {
                log::error!(target: "moshpit", "mp_ui_surface_run: {error}");
                return false;
            }
        };
        surface.view.set_locale(params.nsh_locale());
        surface.view.set_font_scale(params.font_scale);
        surface.view.run(screen.root(&surface.channel, &params));
        true
    })
}

/// Posts `json` (a JSON object, `{"op": …}`) to the running screen, after
/// what was posted before. `false` (nothing posted, the reason logged) when
/// no screen runs or `json` is not one of its messages. Main thread.
///
/// # Safety
///
/// `surface` must come from [`mp_ui_surface_create`] and not be destroyed;
/// `json` a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_ui_surface_post(
    surface: *mut MpUiSurface,
    json: *const c_char,
) -> bool {
    guard("mp_ui_surface_post", false, || {
        // SAFETY: guaranteed by the caller.
        let (surface, json) = unsafe { (self::surface(surface), string(json)) };
        let (Some(surface), Some(json)) = (surface, json) else {
            return false;
        };
        // A refusal is logged by the channel, once per op.
        surface.channel.post(json).is_ok()
    })
}

/// Whether `json` is one of screen `screen`'s messages, as
/// [`mp_ui_surface_post`] checks it (no surface needed: a host's tests).
///
/// # Safety
///
/// `screen` and `json` must be NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_ui_check_message(screen: *const c_char, json: *const c_char) -> bool {
    guard("mp_ui_check_message", false, || {
        // SAFETY: guaranteed by the caller.
        let (name, json) = unsafe { (string(screen), string(json)) };
        let (Some(screen), Some(json)) = (name.and_then(Screen::named), json) else {
            return false;
        };
        match screen.check(json) {
            Ok(()) => true,
            Err(error) => {
                log::warn!(target: "moshpit", "mp_ui_check_message: {error}");
                false
            }
        }
    })
}

/// What became of the host's posts to a surface's screen
/// (`MpUiSurfaceCounts` in `moshpit_ui.h`).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct MpUiSurfaceCounts {
    /// The screen's view took its inbox: it runs and listens.
    pub heard: bool,
    /// Posts made to the running screen (refused ones not counted).
    pub posted: u64,
    /// Posts its view applied.
    pub applied: u64,
    /// Posts that found its view gone: it stopped listening.
    pub lost: u64,
}

/// Fills `counts` for the running screen; `false` (nothing written) off the
/// main thread. Main thread.
///
/// # Safety
///
/// `surface` must come from [`mp_ui_surface_create`] and not be destroyed;
/// `counts` must point to writable memory for one `MpUiSurfaceCounts`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_ui_surface_counts(
    surface: *mut MpUiSurface,
    counts: *mut MpUiSurfaceCounts,
) -> bool {
    guard("mp_ui_surface_counts", false, || {
        // SAFETY: guaranteed by the caller.
        let Some(surface) = (unsafe { self::surface(surface) }) else {
            return false;
        };
        if counts.is_null() {
            return false;
        }
        let now = surface.channel.counts();
        // SAFETY: guaranteed by the caller; checked non-null.
        unsafe {
            counts.write(MpUiSurfaceCounts {
                heard: now.heard,
                posted: now.posted,
                applied: now.applied,
                lost: now.lost,
            })
        };
        true
    })
}

/// What hears a surface's screen: called with a JSON object (`{"ev": …}`)
/// on NSH's engine thread.
pub type MpUiHostHandler = unsafe extern "C" fn(context: *mut c_void, json: *const c_char);

/// A handler and its context, called from the engine thread.
struct Handler {
    handler: MpUiHostHandler,
    context: *mut c_void,
}

// SAFETY: the contract of `mp_ui_surface_set_handler`: the handler may be
// called with its context from the engine thread.
unsafe impl Send for Handler {}
// SAFETY: as above; it is only ever called, never changed.
unsafe impl Sync for Handler {}

impl Handler {
    /// Calls the host with `json`. (A method, so a closure captures the
    /// whole handler, not its raw pointer alone.)
    fn call(&self, json: &str) {
        let json = CString::new(json).unwrap_or_default();
        // SAFETY: the contract of `mp_ui_surface_set_handler`.
        unsafe { (self.handler)(self.context, json.as_ptr()) };
    }
}

/// Sets (`NULL`: clears) what hears the surface's screen. Clearing waits
/// for a call in progress: once it returns, `context` is free to go. Main
/// thread; not from the handler.
///
/// # Safety
///
/// `surface` must come from [`mp_ui_surface_create`] and not be destroyed;
/// `handler` must be safe to call with `context` from any thread until it
/// is cleared or the surface destroyed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_ui_surface_set_handler(
    surface: *mut MpUiSurface,
    handler: Option<MpUiHostHandler>,
    context: *mut c_void,
) {
    guard("mp_ui_surface_set_handler", (), || {
        // SAFETY: guaranteed by the caller.
        let Some(surface) = (unsafe { self::surface(surface) }) else {
            return;
        };
        let listener = handler.map(|handler| {
            let handler = Handler { handler, context };
            Box::new(move |json: &str| handler.call(json)) as crate::channel::Listener
        });
        surface.channel.set_listener(listener);
    })
}

/// Stops the screen and frees the surface: the handler is not called any
/// more once this returns. Main thread.
///
/// # Safety
///
/// `surface` must be `NULL` or come from [`mp_ui_surface_create`], not used
/// afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_ui_surface_destroy(surface: *mut MpUiSurface) {
    if surface.is_null() || MainThreadMarker::new().is_none() {
        return;
    }
    guard("mp_ui_surface_destroy", (), || {
        // SAFETY: guaranteed by the caller.
        drop(unsafe { Box::from_raw(surface) });
    });
}

// --- Native components -------------------------------------------------------

/// What a component tells its screen (opaque to C): owned by the component.
pub struct MpUiComponentContext(ComponentContext);

/// A native component's callbacks (`moshpit_ui.h`), called on the main
/// thread.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct MpUiComponentCallbacks {
    pub create: unsafe extern "C" fn(
        factory: *mut c_void,
        context: *mut MpUiComponentContext,
    ) -> *mut c_void,
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
    callbacks: MpUiComponentCallbacks,
}

impl Drop for CFactory {
    fn drop(&mut self) {
        if let Some(release) = self.callbacks.release_factory {
            // SAFETY: the contract of `mp_ui_surface_register_component`.
            unsafe { release(self.factory) };
        }
    }
}

/// A component made through C callbacks.
struct CComponent {
    handle: *mut c_void,
    callbacks: MpUiComponentCallbacks,
    view: Retained<UIView>,
}

impl NativeComponent for CComponent {
    fn view(&self) -> &UIView {
        &self.view
    }

    fn set_prop(&mut self, name: &str, value: Option<&nsh_core::JsonValue>) {
        let name = CString::new(name).unwrap_or_default();
        let json = value.map(|v| CString::new(v.to_string()).unwrap_or_default());
        // SAFETY: the contract of `mp_ui_surface_register_component`.
        unsafe {
            (self.callbacks.set_prop)(
                self.handle,
                name.as_ptr(),
                json.as_ref().map_or(ptr::null(), |j| j.as_ptr()),
            )
        };
    }

    fn command(&mut self, name: &str, args: &nsh_core::JsonValue) {
        let name = CString::new(name).unwrap_or_default();
        let json = CString::new(args.to_string()).unwrap_or_default();
        // SAFETY: the contract of `mp_ui_surface_register_component`.
        unsafe { (self.callbacks.command)(self.handle, name.as_ptr(), json.as_ptr()) };
    }
}

impl Drop for CComponent {
    fn drop(&mut self) {
        // SAFETY: the contract of `mp_ui_surface_register_component`.
        unsafe { (self.callbacks.destroy)(self.handle) };
    }
}

/// Registers native component `name` (`native("name")` in a screen):
/// `callbacks.create(factory, context)` makes one for every element created
/// from now on. Main thread.
///
/// # Safety
///
/// `surface` must come from [`mp_ui_surface_create`]; `name` must be a
/// NUL-terminated string; the callbacks must be safe to call on the main
/// thread with `factory` (and the handles `create` returns) until
/// `release_factory`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_ui_surface_register_component(
    surface: *mut MpUiSurface,
    name: *const c_char,
    factory: *mut c_void,
    callbacks: *const MpUiComponentCallbacks,
) -> bool {
    guard("mp_ui_surface_register_component", false, || {
        // SAFETY: guaranteed by the caller.
        let (surface, name) = unsafe { (self::surface(surface), string(name)) };
        let (Some(surface), Some(name), false) = (surface, name, callbacks.is_null()) else {
            return false;
        };
        // SAFETY: guaranteed by the caller.
        let callbacks = unsafe { *callbacks };
        let registration = Rc::new(CFactory { factory, callbacks });
        surface.view.register_component(
            name,
            Rc::new(move |context| {
                let context = Box::into_raw(Box::new(MpUiComponentContext(context)));
                let r = &registration;
                // SAFETY: guaranteed by the caller of the registration.
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
        true
    })
}

/// Emits event `name` with `json` data (`NULL`: null). Any thread.
///
/// # Safety
///
/// `context` must be a live context from `create`; the strings
/// NUL-terminated (`json` valid JSON or `NULL`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_ui_component_emit(
    context: *const MpUiComponentContext,
    name: *const c_char,
    json: *const c_char,
) {
    guard("mp_ui_component_emit", (), || {
        // SAFETY: guaranteed by the caller.
        let (context, name, json) = unsafe { (context.as_ref(), string(name), string(json)) };
        let (Some(context), Some(name)) = (context, name) else {
            return;
        };
        let data = match json {
            None => nsh_core::JsonValue::Null,
            Some(json) => serde_json::from_str(json).unwrap_or_else(|error| {
                log::error!(target: "moshpit", "component event {name}: data is not JSON: {error}");
                nsh_core::JsonValue::Null
            }),
        };
        context.0.emit(name, data);
    });
}

/// Reports the component's own size in points. Any thread.
///
/// # Safety
///
/// `context` must be a live context from `create`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_ui_component_set_size(
    context: *const MpUiComponentContext,
    width: f64,
    height: f64,
) {
    // SAFETY: guaranteed by the caller.
    if let Some(context) = unsafe { context.as_ref() } {
        context.0.set_size(width as f32, height as f32);
    }
}

/// Frees a context.
///
/// # Safety
///
/// `context` must be `NULL` or a context from `create`, not freed before.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mp_ui_component_context_free(context: *mut MpUiComponentContext) {
    if !context.is_null() {
        // SAFETY: guaranteed by the caller.
        drop(unsafe { Box::from_raw(context) });
    }
}
