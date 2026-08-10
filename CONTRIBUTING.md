# Contributing to Moshpit

Thanks for your interest in contributing. Moshpit is an iOS SSH / Mosh / tmux
terminal client built with SwiftUI, targeting iOS 18. This guide covers the
project's workflow and conventions so your change lands cleanly.

By participating you agree to abide by our [Code of Conduct](CODE_OF_CONDUCT.md).

## The xcodegen-first workflow

**The `Moshpit.xcodeproj` is generated. Never hand-edit it.** `project.yml` is
the single source of truth for targets, build settings, Info.plist keys,
entitlements, and Swift Package dependencies. The `.xcodeproj` is regenerated
from it by [XcodeGen](https://github.com/yonaskolb/XcodeGen) and any manual
change you make in Xcode's project editor will be silently discarded the next
time someone runs `xcodegen generate`.

Concretely, if your change involves adding a file to a target, changing a build
setting, adding a dependency, or editing an Info.plist value:

1. Edit `project.yml`.
2. Run `xcodegen generate`.
3. Build and commit — commit `project.yml`, not out-of-band `.xcodeproj` edits.

New source files placed under a path already listed in `project.yml` (e.g.
`Moshpit/`) are picked up automatically on the next `xcodegen generate` — no
project.yml edit needed for those.

### One-time setup

```bash
brew install xcodegen
```

### Generate, build, and run

```bash
# Regenerate the Xcode project from project.yml
xcodegen generate

# Build for the simulator
xcodebuild -project Moshpit.xcodeproj -scheme Moshpit \
  -sdk iphonesimulator \
  -destination 'platform=iOS Simulator,name=iPhone 16 Pro' build

# Or open in Xcode and Cmd+R
open Moshpit.xcodeproj
```

### Run the tests

The test suite is the Moshpit scheme's `MoshpitTests` (unit) and `MoshpitUITests`
targets. Run them locally before opening a PR:

```bash
xcodebuild test -project Moshpit.xcodeproj -scheme Moshpit \
  -sdk iphonesimulator \
  -destination 'platform=iOS Simulator,name=iPhone 16 Pro'
```

New behavior should come with tests. The hard system boundaries are already
built for this (see below), so most logic can be exercised without a live
server or a real keychain.

## Documentation style: explain WHY, not WHAT

This codebase's house style is that comments justify code that isn't
self-evident — they anchor to the specific bug, race, or platform constraint
the code exists to prevent. The code already says *what* it does; a comment
earns its place by saying *why it has to be this way*, so the next person
doesn't "simplify" it back into the bug.

Read a few examples before you write your own. From `SessionHub.swift`, on why
a bootstrap task is stored rather than fire-and-forget:

```
/// … Unlike `pumpTask` this used to be pure fire-and-forget —
/// `stop()` had no handle on it, so switching protocol (or any
/// teardown) while it was still mid-flight left it running against a
/// dead session … — the "switch to mosh+tmux, screen never loads" bug.
```

From `KeychainService.swift`, on why the modern data-protection keychain is
pinned:

```
/// … `kSecUseDataProtectionKeychain` pins us to the modern data-protection
/// keychain, which:
///   - Doesn't require a `keychain-access-groups` entitlement on unsigned
///     simulator builds (legacy path fails with -34018 there).
```

Both tie a specific line of code to a concrete failure it prevents. Aim for
that. Avoid comments that restate the code (`// increment the counter`) — they
rot and add noise. When you fix a bug with a subtle workaround, name the bug in
a comment so it survives future refactors.

## Boundaries are fronted by protocols with test seams

The hard external boundaries — the SSH client, the keychain, and the tmux
control transport — are each fronted by a narrow protocol with a
mock/in-memory implementation for tests:

- **SSH** — `SSHClientProvider` (`Moshpit/Services/SSHService.swift`), with a
  mock exercised in `MoshpitTests/Services/SSHServiceMockTests.swift`.
- **Keychain** — `KeychainBackend` (`Moshpit/Services/KeychainService.swift`),
  with `InMemoryKeychainBackend` and `KeychainService.inMemory(...)` for tests,
  plus a file-vault backend for the simulator.
- **tmux transport** — `TmuxTransport` / `TmuxControlling`
  (`Moshpit/Services/Tmux/`), with `MockTmuxTransport`
  (`MoshpitTests/Mocks/MockTmuxTransport.swift`).

New code that crosses one of these boundaries should follow the same pattern:
depend on the protocol, not the concrete type, and provide (or reuse) a test
double so the logic around it can be unit-tested without a network, a device
keychain, or a live tmux server. Keep the protocol as narrow as the caller
actually needs — the `TmuxTransport` doc comment explains why a deliberately
minimal surface is a feature, not a limitation.

## Pull requests

- Branch off `main`.
- Keep the change focused; unrelated cleanups belong in their own PR.
- Run `xcodegen generate` and the test suite before pushing.
- Fill out the PR template. Describe what changed and, in the spirit above, why.
- Include a simulator screenshot or short clip for any user-facing UI change.

## Reporting bugs and requesting features

Use the issue templates under `.github/ISSUE_TEMPLATE/`. A clear repro (host
OS, transport, tmux/mosh versions where relevant) turns a bug report into a
fix much faster.
