// The C ABI of Moshpit's NSH screens on iOS (ui/src/ios.rs, in the app's one
// Rust library, libmoshpit_ffi.a): an NSH view as a UIView, the native
// components the host registers for it, and its channel with the host.
// Swift imports it as part of the MoshpitFFI module
// (crates/ffi/include/module.modulemap).
//
// A surface runs one screen by name ("recent-log") at a time. The host
// posts the screen its data as JSON objects ({"op": ...}) and hears what the
// screen tells it ({"ev": ...}) through a handler. Props, commands and
// events are JSON: the control path, never bytes. No call lets a Rust panic
// reach the caller: it is logged and the call fails (NULL, false).
#pragma once
#import <UIKit/UIKit.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/// Bumped whenever anything below changes shape. The app checks
/// `mp_ui_abi_version() == MP_UI_ABI_VERSION` before it makes a surface,
/// so a stale libmoshpit_ffi.a is refused instead of misread.
#define MP_UI_ABI_VERSION 2

/// The `MP_UI_ABI_VERSION` the library was built with.
uint32_t mp_ui_abi_version(void);

/// Hears the screens' warnings and errors: `level` 1 for an error, 2 for a
/// warning, the record's target ("moshpit", "js") and message. Called on
/// any thread; it must not call back into the library.
typedef void (*MpUiLogHandler)(void *_Nullable context, uint8_t level, const char *_Nonnull target,
    const char *_Nonnull message);

/// Sets (NULL: clears) what hears the screens' warnings and errors, which
/// otherwise go to stderr only. Once, as the app starts; `handler` must be
/// safe to call with `context` from any thread while it is set.
void mp_ui_set_log_handler(MpUiLogHandler _Nullable handler, void *_Nullable context);

/// An NSH view and its channel (opaque to C). Main thread only.
typedef struct MpUiSurface MpUiSurface;

/// Creates a surface with `frame` (it then follows its superview's size),
/// empty until a screen runs. Starts the core's runtime, which NSH runs the
/// screens' tasks on. NULL off the main thread or if NSH failed to start.
MpUiSurface *_Nullable mp_ui_surface_create(CGRect frame);

/// The UIView to add to a hierarchy; owned by the surface.
UIView *_Nullable mp_ui_surface_view(MpUiSurface *_Nonnull surface) NS_RETURNS_NOT_RETAINED;

/// Runs screen `screen` with `params` (JSON or NULL:
/// {"locale": "zh-Hans", "theme": {"accent": "#6C6BEF", "accentPressed":
/// ..., "screenBG": ..., "terminalBG": ...}}), replacing what ran. Register
/// components first. false (nothing runs) for an unknown screen or params it
/// cannot read.
bool mp_ui_surface_run(MpUiSurface *_Nonnull surface, const char *_Nonnull screen,
    const char *_Nullable params);

/// Posts `json` (a JSON object, {"op": ...}) to the running screen, after
/// what was posted before. false (nothing posted) when no screen runs or
/// `json` is not one of its messages; the reason is logged once per op.
bool mp_ui_surface_post(MpUiSurface *_Nonnull surface, const char *_Nonnull json);

/// Whether `json` is one of screen `screen`'s messages, checked as
/// `mp_ui_surface_post` checks it, with no surface. Any thread.
bool mp_ui_check_message(const char *_Nonnull screen, const char *_Nonnull json);

/// What became of the host's posts to a surface's running screen.
typedef struct {
    /// The screen's view took its inbox: it runs and listens.
    bool heard;
    /// Posts made to it (refused ones not counted).
    uint64_t posted;
    /// Posts its view applied.
    uint64_t applied;
    /// Posts that found its view gone: it stopped listening.
    uint64_t lost;
} MpUiSurfaceCounts;

/// Fills `counts` for the running screen; false (nothing written) off the
/// main thread. Main thread.
bool mp_ui_surface_counts(MpUiSurface *_Nonnull surface, MpUiSurfaceCounts *_Nonnull counts);

/// Hears what a surface's screen tells the host: a JSON object ({"ev":
/// ...}), called on NSH's engine thread; the host hops to its own. It must
/// not call back into the surface.
typedef void (*MpUiHostHandler)(void *_Nullable context, const char *_Nonnull json);

/// Sets (NULL: clears) what hears the surface's screen. Clearing waits for a
/// call in progress: once it returns, `context` may be freed.
void mp_ui_surface_set_handler(MpUiSurface *_Nonnull surface, MpUiHostHandler _Nullable handler,
    void *_Nullable context);

/// Stops the screen and frees the surface; its handler is not called once
/// this returns.
void mp_ui_surface_destroy(MpUiSurface *_Nullable surface);

/// What a native component tells its screen (opaque to C): owned by the
/// component, freed with `mp_ui_component_context_free`.
typedef struct MpUiComponentContext MpUiComponentContext;

/// A native component's callbacks, called on the main thread.
typedef struct {
    /// Makes a component for an element; returns its handle. Owns `context`.
    void *_Nonnull (*_Nonnull create)(void *_Nullable factory, MpUiComponentContext *_Nonnull context);
    /// Its view; it fills the box the screen lays out.
    UIView *_Nonnull (*_Nonnull view)(void *_Nonnull component);
    /// Prop `name` changed to `json` (NULL: removed).
    void (*_Nonnull set_prop)(void *_Nonnull component, const char *_Nonnull name, const char *_Nullable json);
    /// The screen called `ref.command(name, args)`.
    void (*_Nonnull command)(void *_Nonnull component, const char *_Nonnull name, const char *_Nonnull json);
    /// The element went away: release the handle.
    void (*_Nonnull destroy)(void *_Nonnull component);
    /// The registration was replaced or the surface destroyed.
    void (*_Nullable release_factory)(void *_Nullable factory);
} MpUiComponentCallbacks;

/// Registers native component `name` (`native("name")` in a screen):
/// `callbacks.create(factory, context)` makes one for every element created
/// from now on. false if nothing was registered.
bool mp_ui_surface_register_component(MpUiSurface *_Nonnull surface, const char *_Nonnull name,
    void *_Nullable factory, const MpUiComponentCallbacks *_Nonnull callbacks);

/// Emits event `name` with `json` data (NULL: null). Any thread.
void mp_ui_component_emit(const MpUiComponentContext *_Nonnull context, const char *_Nonnull name,
    const char *_Nullable json);

/// Reports the component's own size in points. Any thread.
void mp_ui_component_set_size(const MpUiComponentContext *_Nonnull context, double width, double height);

/// Frees a context (after its component is destroyed).
void mp_ui_component_context_free(MpUiComponentContext *_Nullable context);

#ifdef __cplusplus
}
#endif
