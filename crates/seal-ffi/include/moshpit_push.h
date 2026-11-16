// The notification service extension's one call into moshpit-seal (E7b,
// crates/seal-ffi): a push in, what to show out, both JSON (the request
// and the answer are described in crates/seal-ffi/src/lib.rs). The
// extension links libmoshpit_seal_ffi.a alone; the app's library carries
// the same two symbols, for the tests that drive the extension in-process.
#pragma once

#ifdef __cplusplus
extern "C" {
#endif

/// One push: `request` a NUL-terminated UTF-8 JSON object. The answer is
/// the caller's, to free with `moshpit_push_free`; NULL only for a request
/// that isn't a C string.
char *moshpit_push_receive(const char *request);

/// Frees an answer `moshpit_push_receive` returned (NULL is ignored).
void moshpit_push_free(char *answer);

#ifdef __cplusplus
}
#endif
