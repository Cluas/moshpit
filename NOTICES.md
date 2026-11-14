# Third-Party Notices

Moshpit is licensed under the GNU General Public License, version 3 only
(GPL-3.0-only; see [LICENSE](LICENSE)). It
incorporates the following third-party components, each under its own license.

## Swift Package Dependencies

### Citadel

- Source: https://github.com/orlandos-nl/Citadel
- License: MIT

Citadel provides the SSH client implementation used by Moshpit.

### WhisperKit

- Source: https://github.com/argmaxinc/WhisperKit
- License: MIT (Copyright © 2024 Argmax, Inc.)

WhisperKit provides the on-device Whisper inference used by the optional local
speech engine under Settings ▸ Voice Input ▸ Recognition. Only its `WhisperKit`
library product is linked; the package's CLI target and that target's
`swift-argument-parser` dependency are not built.

## Downloaded Speech Models

Moshpit ships **no** speech model weights. When you choose the Whisper engine
and download a model, the files are fetched at runtime from the
[`argmaxinc/whisperkit-coreml`](https://huggingface.co/argmaxinc/whisperkit-coreml)
repository on Hugging Face and stored in the app's Application Support
directory. They are Core ML conversions of OpenAI's Whisper models, which are
released by OpenAI under the MIT License; the conversions are distributed by
Argmax under the MIT License. No audio or transcript is uploaded by this
download — it only fetches model files.

Apple's own speech models (the default engine) are part of iOS and are managed
by the system, not by Moshpit.

## Bundled Fonts

Moshpit bundles several programmer monospace fonts, selectable under
Settings ▸ Display ▸ Font. Each font is redistributable and retains its own
license. The per-font license and source table is maintained in
[`Moshpit/Resources/Fonts/LICENSES.md`](Moshpit/Resources/Fonts/LICENSES.md) and
is incorporated here by reference.

In summary, the bundled fonts are covered by the SIL Open Font License 1.1
(JetBrains Mono, Maple Mono, Fira Code, Source Code Pro, IBM Plex Mono,
Anonymous Pro) and the MIT-style Hack Open Font License (Hack). See the
referenced file for full attribution, source URLs, and license text pointers.

## Clean-Room Disclaimer — Mosh State Synchronization Protocol

The Swift sources under `Moshpit/Services/Mosh/` (`MoshBootstrap.swift`,
`MoshCompression.swift`, `MoshCrypto.swift`, `MoshTransport.swift`,
`MoshWire.swift`, `OCB3.swift`) are an **independent, clean-room Swift
reimplementation** of mosh's State Synchronization Protocol (SSP) and its
associated wire format and cryptography.

This implementation was written from the public description of the protocol —
its published specification and RFC-style documented on-the-wire behavior — for
the sole purpose of interoperating with mosh servers. It is **not** derived
from, and does not copy or translate, mosh's own GPL-3.0-licensed C++ source
code.

Because it is an original work implementing an interoperable protocol rather
than a derivative of the mosh codebase, this code is Moshpit's own copyright:
it is distributed under Moshpit's GPL-3.0-only license along with the rest of
the project, and its provenance does not depend on mosh's license.

"mosh" is a project of Keith Winstein and contributors; the name is used here
only to identify the protocol being interoperated with.

## Rust Core and Shared UI

Both apps, iOS and Android, link one Rust library built from `crates/` and
`ui/`.

### wezterm terminal emulation

- Source: https://github.com/wez/wezterm (commit
  `ce8e6b140c2a52c8be1cc31c1274104b01422fdc`), its terminal crates vendored in
  [`vendor/wezterm`](vendor/wezterm) with Moshpit's patches listed in
  [`docs/WEZTERM-PATCHES.md`](docs/WEZTERM-PATCHES.md)
- License: MIT (Copyright (c) 2018-Present Wez Furlong;
  `vendor/wezterm/LICENSE.md` and each crate's own `LICENSE.md`)

The terminal emulator under the new terminal view (`crates/term`).

### NSH

- Source: https://github.com/longbridge/nsh (the revision `Cargo.toml` pins)
- License: Apache-2.0 (Copyright 2026 Longbridge; the repository's
  `LICENSE-APACHE`)

The cross-platform native UI framework the shared screens (`ui/`) are written
in, on iOS and Android alike. On Android its Java runtime is compiled into the
app from the same revision as its Rust half.

## Android App

### Termux terminal-emulator and terminal-view

- Source: https://github.com/termux/termux-app (tag v0.118.3), vendored in
  [`android/termux`](android/termux) with Moshpit's changes listed in its
  `NOTICE`
- License: GPL-3.0-only; the libraries contain code of Terminal Emulator for
  Android (https://github.com/jackpal/Android-Terminal-Emulator) under the
  Apache License 2.0 (`android/termux/LICENSE.md`, `COPYING.GPL-3.0`,
  `LICENSE.Apache-2.0`)

The Android app's terminal view and emulator. Both licenses are compatible
with Moshpit's GPL-3.0-only.

### Rust crates

The Rust core (`crates/`, both apps) links crates from crates.io under the MIT and/or
Apache-2.0 licenses (among them russh, tokio, ring, ssh-key, serde, jni);
`cargo metadata` lists each with its license.

### Google Play services

This build of the Android app links no Google Play services. Push
notifications on Android will come through Firebase Cloud Messaging, which is
not free software: before a build that links it is distributed, `LICENSE`
gains the GPL-3.0 section 7 additional permission to combine Moshpit with
Google Play services and Firebase.
