// Spike G1's Rust side for the iOS host (crates/spike-ios/src/app.rs): the
// NSH app as a UIView, the host's native components, and the bench's burst
// producer. The terminal's bytes go through moshpit_stream.h, not here.
#pragma once
#import <UIKit/UIKit.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/// An app view running the spike's Rust root (opaque to C). Main thread only.
typedef struct MpApp MpApp;

/// Creates the view (empty until `mp_app_run`). Starts the core, whose
/// tokio runtime NSH then runs the UI's tasks on.
MpApp *_Nonnull mp_app_create(CGRect frame);

/// The UIView to add to a hierarchy; owned by the app.
UIView *_Nonnull mp_app_view(MpApp *_Nonnull app) NS_RETURNS_NOT_RETAINED;

/// Runs the Rust root, replacing the running app.
void mp_app_run(MpApp *_Nonnull app);

/// Runs root `name`, replacing the running app: "g1" (G1's status line over
/// the terminal), "g5" (G5's terminal screen in an NSH navigation stack),
/// "g5-embedded" (the terminal screen alone, for a SwiftUI navigation stack
/// to host; its back button tells the host).
void mp_app_run_root(MpApp *_Nonnull app, const char *_Nonnull name);

/// Posts a JSON object (`{"op": ...}`) to every running root, in order. Any
/// thread.
void mp_spike_post(const char *_Nonnull json);

/// Hears what the UI tells the host: a JSON object (`{"ev": ...}`), called
/// on NSH's engine thread.
typedef void (*MpHostHandler)(const char *_Nonnull json);

/// Sets (NULL: clears) what hears the UI.
void mp_spike_set_host_handler(MpHostHandler _Nullable handler);

/// Stops the app and frees the view.
void mp_app_destroy(MpApp *_Nullable app);

/// What a component tells its app (opaque to C): owned by the component,
/// freed with `mp_component_context_free`.
typedef struct MpComponentContext MpComponentContext;

/// A native component's callbacks, called on the main thread.
typedef struct {
    /// Makes a component for an element; returns its handle. Owns `context`.
    void *_Nonnull (*_Nonnull create)(void *_Nullable factory, MpComponentContext *_Nonnull context);
    /// Its view; it fills the box the app lays out.
    UIView *_Nonnull (*_Nonnull view)(void *_Nonnull component);
    /// Prop `name` changed to `json` (NULL: removed).
    void (*_Nonnull set_prop)(void *_Nonnull component, const char *_Nonnull name, const char *_Nullable json);
    /// The app called `ref.command(name, args)`.
    void (*_Nonnull command)(void *_Nonnull component, const char *_Nonnull name, const char *_Nonnull json);
    /// The element went away: release the handle.
    void (*_Nonnull destroy)(void *_Nonnull component);
    /// The registration was replaced or the app view destroyed.
    void (*_Nullable release_factory)(void *_Nullable factory);
} MpComponentCallbacks;

/// Registers native component `name` (`native("name")` in the Rust UI):
/// `callbacks.create(factory, context)` makes one for every element created
/// from now on.
void mp_app_register_component(MpApp *_Nonnull app, const char *_Nonnull name, void *_Nullable factory,
    const MpComponentCallbacks *_Nonnull callbacks);

/// Emits event `name` with `json` data (NULL: null). Any thread.
void mp_component_emit(const MpComponentContext *_Nonnull context, const char *_Nonnull name,
    const char *_Nullable json);

/// Reports the component's own size in points. Any thread.
void mp_component_set_size(const MpComponentContext *_Nonnull context, double width, double height);

/// Frees a context (after the component is destroyed).
void mp_component_context_free(MpComponentContext *_Nullable context);

/// The burst producer's report.
typedef struct {
    uint64_t bytes;      ///< bytes pushed
    uint64_t chunks;     ///< pushes
    uint64_t hash;       ///< FNV-1a 64 of every byte, in push order
    uint64_t push_nanos; ///< first push to last
} MpSpikeBurst;

/// The terminal's stream (the one the core echoes).
uint64_t mp_spike_terminal_stream(void);

/// G5's pane `number`'s stream (1: the shell, else the full-screen app),
/// opened on first ask.
uint64_t mp_spike_pane_stream(uint32_t number);

/// Pushes `text` as a line of output to G5's pane `number`.
void mp_spike_pane_say(uint32_t number, const char *_Nonnull text);

/// Starts the burst: `total` bytes of seed `seed` pushed to the terminal's
/// stream as fast as the core can make them.
void mp_spike_burst_start(uint64_t total, uint64_t seed);

/// Fills `out` with the burst's report; false until the producer reached its
/// last chunk (it records the report before pushing that chunk).
bool mp_spike_burst_report(MpSpikeBurst *_Nonnull out);

/// What a stream's core end took from the view (G4's evidence).
typedef struct {
    uint64_t bytes;      ///< input bytes, all events together
    uint64_t chunks;     ///< input events that carried bytes
    uint64_t resizes;    ///< resizes
    uint16_t cols;       ///< the last resize (0 x 0 before any)
    uint16_t rows;
    uint64_t resized_at; ///< when the core took it (DispatchTime's clock, ns)
} MpSpikeTaken;

/// Fills `out` with what stream `id`'s core end took so far.
void mp_spike_taken(uint64_t id, MpSpikeTaken *_Nonnull out);

/// Copies up to `cap` of the input bytes stream `id`'s core end took,
/// from byte `offset` on, into `buf`; returns how many.
size_t mp_spike_taken_bytes(uint64_t id, uint64_t offset, uint8_t *_Nonnull buf, size_t cap);

#ifdef __cplusplus
}
#endif
