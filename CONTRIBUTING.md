# Contributing to Moshpit

Thanks for your interest in contributing. Moshpit is an SSH / Mosh / tmux
terminal for iPhone and iPad: a Rust core and screens (`crates/`, `ui/`) under a
thin Swift host (`Moshpit/`, `Extensions/`), targeting iOS 18. This guide covers
the project's workflow and conventions so your change lands cleanly.

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

Code that an extension needs as well as the app (the widget, the notification
service, the share sheet) goes in `Packages/MoshpitKit`, the local Swift
package every target links, and is `public` there. Do not list an app file
into an extension target by path; the one exception, `AgentCycleIntent`,
explains itself in its header.

### One-time setup

```bash
brew install xcodegen
# Rust: rustup reads rust-toolchain.toml and installs the pinned toolchain on
# first use; the app's "Build Rust" phase adds the iOS targets it needs.
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

You also need Xcode 27 (what CI and the App Store build use; the app deploys
to iOS 18).

### Generate, build, and run

```bash
# Regenerate the Xcode project from project.yml
xcodegen generate

# Build for the simulator: the "Build Rust" phase builds crates/ffi into
# libmoshpit_ffi.a first (scripts/rust/build-ios.sh)
xcodebuild -project Moshpit.xcodeproj -scheme Moshpit \
  -sdk iphonesimulator \
  -destination 'platform=iOS Simulator,name=iPhone 18 Pro' build

# Or open in Xcode and Cmd+R
open Moshpit.xcodeproj
```

### Run the tests

Most of the logic is Rust, in `crates/` and `ui/`; its tests run with cargo, as
CI does:

```bash
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test -p moshpit-session     # one crate while you work on it
```

The iOS app's tests are the Moshpit scheme's `MoshpitTests` (unit) and
`MoshpitUITests` targets. Run them locally before opening a PR:

```bash
scripts/rust/test-build-ios.sh    # the Rust phase's profiles and links, without building
xcodebuild test -project Moshpit.xcodeproj -scheme Moshpit \
  -sdk iphonesimulator \
  -destination 'platform=iOS Simulator,name=iPhone 18 Pro'
```

For the unit tests alone, use the `MoshpitUnitTests` scheme instead: it leaves
the UI-test runner out of the build and the install, which is most of the
wait (the reasoning is next to the scheme in `project.yml`).

New behavior should come with tests. The hard system boundaries are already
built for this (see below), so most logic can be exercised without a live
server or a real keychain.

#### A failure unrelated to your change: suspect the simulator first

A simulator that has hosted a long session of test runs degrades, and it does not
degrade by saying so. Observed on 2026-08-25, same commit and same command, one
simulator red and a fresh one green — and the tired one failed three different
ways in a row:

1. named two specific unrelated tests as failing, both of them timing-sensitive
   (`expiredKeystrokesDropOnFlush`, `keepAliveFunnelsThroughResume`), with
   "Restarting after unexpected exit, crash, or test timeout" in the log
2. `Mach error -308 - (ipc/mig) server died` when only those two suites were re-run
3. "Early unexpected exit, operation never finished bootstrapping" — with no
   crash report on either side

The first presentation is the dangerous one: it looks exactly like a real
regression, in named tests, and the names have enough time semantics to make a
plausible story. Believing it costs an hour of reading transport timing code that
was never touched.

So: **if the failures have nothing to do with what you changed — especially
anything about timing, keepalives or expiry — re-run on a simulator that has not
been used today before you suspect the code.** One 60-second run to tell a real
regression from a tired simulator.

(The same shape bit us elsewhere the same day: a device file listing that said
"could not be transferred" was read as "does not exist" until a deliberately
invented filename showed what absence actually looks like. When one failure
resembles another, the cheap move is a control, not a theory.)

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
  mock exercised in `Tests/MoshpitTests/Services/SSHServiceMockTests.swift`.
- **Keychain** — `KeychainBackend` (`Moshpit/Services/KeychainService.swift`),
  with `InMemoryKeychainBackend` and `KeychainService.inMemory(...)` for tests,
  plus a file-vault backend for the simulator.
- **tmux transport** — `TmuxTransport`
  (`Moshpit/Services/Tmux/TmuxTransport.swift`), with `MockTmuxTransport`
  (`Tests/MoshpitTests/Mocks/MockTmuxTransport.swift`).

New code that crosses one of these boundaries should follow the same pattern:
depend on the protocol, not the concrete type, and provide (or reuse) a test
double so the logic around it can be unit-tested without a network, a device
keychain, or a live tmux server. Keep the protocol as narrow as the caller
actually needs — the `TmuxTransport` doc comment explains why a deliberately
minimal surface is a feature, not a limitation.

## Pull requests

- Branch off `main`.
- Keep the change focused; unrelated cleanups belong in their own PR.
- Run `xcodegen generate`, the Rust tests and the app's test suite before pushing.
- Fill out the PR template. Describe what changed and, in the spirit above, why.
- Include a simulator screenshot or short clip for any user-facing UI change.

## README translations

`README.md` is the source. Its translations (`README-zh.md`, and
`README.<locale>.md` for the other languages the app speaks) are each written
natively, not translated line by line, and each opens with the blob of the
`README.md` it follows:

```
<!-- source: README.md <git hash-object README.md> -->
```

When you change `README.md`, update the translations you can and give them the
new blob in their marker. The ones you can't leave as they are: they keep the
old blob, and `python3 scripts/readme-translations.py` lists them as behind (CI
warns on them and never fails). Commands, paths and code names stay as they are
in every language; feature names follow the app's own wording in that language.

## Commit messages

Commits follow [Conventional Commits](https://www.conventionalcommits.org/):
a type, an optional scope, and a short imperative summary, then a body that
says why.

```
feat(terminal): keep a long-press selection when the finger lifts
fix(tmux): stop the scroll view scrolling underneath our own pan
docs: describe how the public history is produced
```

Types in use: `feat`, `fix`, `perf`, `refactor`, `test`, `docs`, `build`,
`ci`, `chore`. Scope is the area touched (`terminal`, `tmux`, `herdr`, `push`,
`island`, `settings`, `i18n`, `release`) and is left out when the change is
repository-wide. The summary stays under about 70 characters; the body
explains the reasoning, the trade-off or the bug's mechanism — the diff
already shows what changed.

## Reporting bugs and requesting features

Use the issue templates under `.github/ISSUE_TEMPLATE/`. A clear repro (host
OS, transport, tmux/mosh versions where relevant) turns a bug report into a
fix much faster.

## License and provenance

Moshpit is licensed under GPL-3.0-only (see [LICENSE](LICENSE)). By submitting a
pull request you agree that your contribution is licensed under the same terms.

Moshpit is developed here. Commits up to the move carry a `Source-Commit`
trailer: they were published from the maintainer's earlier, private
development tree, one commit per development commit, with the original
author, date and message. The App Store release pipeline and store metadata,
and the marketing site, are separate private repositories. Pull requests are
merged into `main` here.

Every push and pull request runs a leak scan (`scripts/oss-scan.py --public`,
in `.github/workflows/leak-scan.yml`) on what it adds: access tokens, App
Store Connect key ids, private-key material, device UDIDs, a person's home
folder, tailnet addresses. Fixtures use `example.com`, `192.0.2.x`,
`198.51.100.x`, `100.64.0.x`, `/Users/dev` and `/home/dev`. The maintainers'
own pushes also go through a pre-push gate (`scripts/setup-hooks.sh` installs
it) with the same scan and a few private lists of their own.
