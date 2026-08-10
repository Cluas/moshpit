# Moshpit Architecture

Moshpit is an iOS SSH / Mosh / tmux terminal client, built with SwiftUI and
targeting iOS 18. It connects to remote servers, runs interactive shells and
CLI coding agents (Claude Code, Codex, …), and surfaces live agent status on
the Lock Screen and Dynamic Island. The project is managed by
[XcodeGen](https://github.com/yonaskolb/XcodeGen) — `project.yml` is the source
of truth and `Moshpit.xcodeproj` is generated from it.

This document is an orientation for new contributors: how the code is layered,
what the core services do, and the handful of designs that are non-obvious
(dual-transport mosh+tmux, the actor concurrency model, capability degradation,
and the widget-extension data bridge). It describes the code as it is — read
the named source files alongside it.

---

## 1. Layering

The app target lives under `Moshpit/`, split into layers by directory. Roughly
top (UI) to bottom (transport):

```
Moshpit/
  App/          @main entry (MoshpitApp.swift), entitlements, assets, icons
  UI/           SwiftUI surfaces: Home, Terminal, Settings, Shortcuts, Keys,
                Island, Brand + design tokens (AppTheme, DesignTokens, Components)
  Views/        Terminal view layer (SwiftTerm bridge / coordinators)
  ViewModels/   TerminalViewModel — per-connection UI state over a transport
  Models/       Value types + stores: ServerConnection, AppSettings,
                ConnectionStore, SessionMetrics, TerminalTheme, TerminalFont,
                TerminalShortcut, keyboard/scrollback/cursor models
  Services/     The engine room (see §2):
      SessionHub.swift          app-wide session orchestrator
      SSHService.swift          auth + SSHSession (Citadel/NIOSSH)
      HostCapabilities.swift    capability probe + cache
      HostKeyValidator.swift    TOFU known-hosts
      KeychainService.swift     credential storage (biometry-gated)
      SSHKeyFactory.swift       key parsing / Secure Enclave
      Mosh/                     real mosh SSP-over-UDP client
      Tmux/                     tmux -CC control-mode client + controller
  Island/       Shared agent-status code (see §6) — compiled into BOTH the app
                and the widget extension

MoshpitIsland/   Widget extension target: Live Activity + home/lock widgets
MoshpitTests/    unit tests    MoshpitUITests/   UI tests
```

**Dependency direction is one-way, top-down.** UI/Views observe ViewModels and
`SessionHub`; ViewModels and the hub drive Services; Services talk to the
network. The `Island/` directory is special: it holds the types and monitor
shared across the app/extension boundary and is compiled into both targets.

Key third-party pieces: **Citadel / swift-nio-ssh** for the SSH transport,
**SwiftTerm** for terminal emulation and rendering, and a bundled **mosh** C++
static library the Swift `Mosh/` code links against.

---

## 2. Services overview

The interesting logic is in `Services/`. Four files carry most of the weight:

| File | Role |
|------|------|
| `SessionHub.swift` | App-wide session registry + lifecycle orchestrator (§3) |
| `SSHService.swift` | Authenticates connections, hands back an `SSHSession` actor |
| `Mosh/MoshTransport.swift` | The mosh State Synchronization Protocol client over UDP |
| `Tmux/TmuxSessionController.swift` + `Tmux/TmuxControlClient.swift` | tmux control-mode (`-CC`) state owner + protocol parser |

`SSHService` is an actor façade that resolves credentials (from
`KeychainService`, with an in-memory cache so frequent auto-reconnects don't
re-prompt Face ID), applies TOFU host-key validation (`HostKeyValidator`), and
returns an `SSHSession` — itself an actor wrapping one authenticated Citadel
client with a PTY-backed channel. `SSHSession` exposes host output as an
`AsyncStream<Data>` (`dataStream`), a `write(_:)` for input, `resize(...)`, a
one-shot `executeCommand(...)` over a fresh exec channel, and an
`onUnexpectedClose` hook that fires only on an unexpected drop (not on
intentional `close()`) to drive auto-reconnect. There is a process-wide
`SSHService.shared` for non-injecting callers; tests construct their own with a
mock `SSHClientProvider` so no socket is opened.

---

## 3. SessionHub — the session orchestrator

`SessionHub` (`@MainActor @Observable`) is the single owner of every live
terminal session. Sessions **outlive navigation**: the Home screen can read the
tmux tree of a connected server while its Terminal screen is not on screen, and
re-entering the Terminal re-attaches to the same in-memory session instead of
reconnecting.

- **Registry.** `sessions: [UUID: ActiveSession]` keyed by connection id.
  `prepare(_:)` creates+registers a session *without* starting the transport
  (so the UI can bind to it before the host-key TOFU prompt fires mid-handshake
  — otherwise the prompt has no observer and the handshake deadlocks).
  `start(_:...)` then opens the transport; `connect(...)` is a convenience that
  does both.
- **`ActiveSession`** (nested, also `@MainActor @Observable`) is one live
  connection. It owns the transport objects, a `TerminalViewModel`, the
  SwiftTerm coordinator(s), remembered appearance (theme/font/cursor for
  transparent reconnects), and the remembered tmux selection
  (`TmuxSelectionStore`, persisted **per connection** so it survives a protocol
  switch or app relaunch). `sendInput(_:)` is the single input entry point —
  it routes keystrokes to whichever transport is live so callers never branch
  on transport type. `scrollActiveTerminal(lines:)` similarly hides the
  wheel-vs-copy-mode scrollback differences between transports.
- **Lifecycle across reconnect / background / foreground.** The hub runs a
  foreground keepalive `Timer` (`setForeground(_:)`) that probes each session
  every ~12s: NAT-warmth, fast death detection, and reconnect-retry in one.
  iOS kills TCP during suspension, so `isConnected` (a local `!closed` flag)
  can lie — the probe actively round-trips a trivial command
  (`isTransportAlive`) rather than trusting it. On foreground return, sessions
  backgrounded longer than ~20s skip the (unreliable on a half-open socket)
  probe and force a fresh reconnect. `connectionDropped()` (fired by
  `SSHSession.onUnexpectedClose`) triggers a transparent reconnect; tmux
  `attach` / `new -A` re-lands on the same server-side session so panes and
  scrollback come right back. `isStopping` / `isReconnecting` guard against a
  keepalive tick and a death callback fighting each other.
- **Window-size pins.** Moshpit pins the tmux window to the phone grid while the
  terminal is on screen. On backgrounding the hub hands those pins back
  (`releaseWindowPinsForBackground`) so a desktop client sharing the window
  isn't stranded at phone width; `repinForeground()` (only for the currently
  `visibleSession`) takes them back on return.

---

## 4. The three transport modes

A `ServerConnection` chooses its transport via `connectionProtocol` (`.ssh` /
`.mosh`) and the `useTmux` flag. That yields three effective modes:

### a. Plain SSH
`ActiveSession.start` opens an `SSHSession`, requests a PTY, and pumps
`session.dataStream` straight into a single SwiftTerm coordinator. Used when
`useTmux` is off (or when tmux is wanted but the host lacks it — see §5). One
shell, one screen, local scrollback.

### b. Real Mosh (UDP) — for roaming and high-latency links
Chosen when the protocol is `.mosh`. `startMosh()` authenticates over SSH with
**no** PTY, execs `mosh-server` to learn the UDP port + session key
(`MoshBootstrap`), drops the SSH connection, and then runs the actual mosh
**State Synchronization Protocol** client in `MoshTransport` over `NWConnection`
(UDP). This is a from-scratch Swift SSP client (`Mosh/`), not a wrapper around
an interactive `mosh` binary: it encrypts/decrypts datagrams (`MoshCrypto` /
`OCB3`), fragments/reassembles (`MoshWire`), zlib-compresses instructions
(`MoshCompression`), maintains the client→server user-input state numbers and
the server→client display state numbers, and applies host bytes to the terminal
**only when a diff's `old_num` matches the state already applied**. Its
`hostStream` mirrors `SSHSession.dataStream`, so the terminal layer consumes
mosh and SSH identically. Mosh's value is roaming (Wi-Fi ↔ cellular, surfaced
via `NWConnection.betterPathUpdateHandler`) and survival across suspend/resume —
SSP is connectionless, so a suspended session usually heals itself.

### c. tmux control mode (`-CC`) — native session/window/pane UI
Chosen when `useTmux` is on over an SSH connection to a host that has tmux.
Instead of a raw shell, `ActiveSession.start` boots `tmux -CC attach`, and
`TmuxSessionController` drives tmux's **control mode**: a line-framed protocol
where tmux reports every session/window/pane event (`%output`, `%window-add`,
`%layout-change`, `%session-changed`, …) and echoes each command inside a
`%begin … %end/%error` block. `TmuxControlClient` is the pure, transport-free,
Observation-free *parser* of that stream (byte-level `%output` handling,
octal-escape decoding, `%begin/%end` block tracking); `TmuxSessionController`
(`@MainActor @Observable`) is the *state owner* that turns those callbacks into
a single observable `TmuxSnapshot` (sessions/windows/panes) plus a pool of
per-pane SwiftTerm `TerminalView`s. This is what powers Moshpit's native
breadcrumb and the Sessions/Windows/Panes sheets — the UI reads live tmux state
instead of scraping a rendered TUI. **Moshpit never creates or modifies tmux
sessions on its own**: it attaches the user's existing sessions (most recent),
and if the server has none it shows an empty state and lets the user create the
first one explicitly.

### The mosh+tmux dual-transport design (the notable one)

You cannot run tmux `-CC` over mosh: mosh transmits *screen diffs*, not the raw
line-framed control stream `-CC` needs (verified empirically). So when the user
wants **both** roaming (mosh) **and** the native session/window/pane UI (tmux
`-CC`), Moshpit runs **two transports at once** against the same tmux server:

```
                    ┌─────────────────────────── remote host ──┐
  mosh UDP  ────────┼──▶ mosh-server ──▶ tmux client (renderer) │
  (renders TUI,     │                         │  same tmux       │
   roams, low-lat)  │                         ▼  daemon/session  │
                    │        tmux -CC control ▲                  │
  sidecar SSH  ─────┼──▶ sshd ──▶ tmux -CC client (control-only) │
  (control stream)  │        pushes %window-add / %layout-change │
                    └───────────────────────────────────────────┘
        │                                    │
        ▼                                    ▼
  SwiftTerm screen                   TmuxSnapshot → breadcrumb + sheets
```

- The **mosh UDP transport renders the interactive tmux TUI** (a plain
  full-screen `tmux attach` inside the mosh shell) — this is what the user sees
  and types into, and it roams.
- A **separate, lightweight sidecar SSH connection** carries a `tmux -CC`
  control stream to the *same* tmux daemon. This control client is created
  `rendersOutput: false` — it deliberately never reports a size, so tmux
  suppresses `%output` to it and excludes it from window sizing. It exists
  purely to push live session/window/pane state into the native breadcrumb and
  sheets (`%window-add`, `%layout-change`, …).

The two are kept in lockstep: the control plane attaches first and lands on a
session; the mosh renderer then attaches the **same** session by name
(preferring the remembered selection so a reconnect returns to the user's pane).
Actions that must move both clients (switch pane/window, scroll, window
resize/pin) go through the `-CC` sidecar, since select-window/pane are
session-scoped and therefore move the mosh renderer too. Scrollback is the
exception — the sidecar's copy-mode doesn't repaint the separate mosh client, so
scrollback is driven with copy-mode keystrokes sent over the mosh keystroke
channel itself. `ActiveSession` owns the sidecar `SSHSession` and closes it on
teardown; `TmuxSelectionStore` lets a reconnect re-target the right session.

---

## 5. Capability probe & degrade

Before committing to tmux `-CC` or mosh, Moshpit probes the host so a missing
dependency degrades gracefully instead of stalling on a failing command.

- **The probe.** `HostCapabilities.probeCommand` runs over the first SSH
  channel and reports `hasTmux`, `hasMoshServer`, the OS, and the first package
  manager found on `PATH` (apt-get / dnf / yum / pacman / apk / brew). Results
  are cached per connection (`HostCapabilityCache`) so a reconnect shows the
  right state immediately while a fresh probe runs.
- **Optimistic default.** `HostCapabilities.unknown` reports everything present.
  A probe that *can't run* keeps the last-known capabilities (or the optimistic
  default on a cold session) — a transient hiccup never degrades a healthy host,
  and, crucially, never *upgrades* a known-degraded host back to "all present"
  (which would retry `-CC attach` on a tmux-less box and hang).
- **Degrade matrix.**
  - `useTmux` but no tmux (and no custom `tmuxPath`) → run a **plain SSH pane**
    (or the bare mosh shell) and raise a `DegradeNotice(.tmux)`.
  - `.mosh` but no `mosh-server` (and no custom `moshServerPath`) →
    `fallbackToSSH`: reuse the already-authenticated SSH session as a plain
    shell — or as SSH+tmux `-CC` if tmux *is* present — and raise
    `DegradeNotice(.moshServer)`.
  - A **custom path** in the connection form means the user vouches for the tool,
    so the degrade check is skipped (the probe only walks `PATH`).
- **User-facing.** A `DegradeNotice` drives a dismissible banner with an
  "Install …" action that opens Install Assist (which offers the right
  `installCommand` for the detected package manager, installing tmux+mosh
  together). After the user installs, `recheckCapabilities()` re-probes over a
  live channel; if none is available (mosh closed its bootstrap SSH) it returns
  nil → "reconnect to apply" rather than falsely reading "everything installed".

---

## 6. Concurrency model

Moshpit leans hard on Swift's actor model to keep transport bytes, terminal
rendering, and UI state from racing.

- **`SessionHub` / `ActiveSession` / `TmuxSessionController` are `@MainActor`.**
  They touch SwiftTerm `TerminalView`s and the SwiftUI tree, both of which
  require the main actor. Parser callbacks hop back via `Task { @MainActor in … }`.
- **`SSHSession`, `SSHService`, `MoshTransport`, and `TmuxControlClient` are
  actors.** They own network/socket state and mutable protocol counters, so
  actor isolation serializes access without locks. Host output crosses back to
  the main actor as an `AsyncStream<Data>` the UI pumps.

### FIFO-ordering primitives — why they exist

Independently-spawned `Task {}`s have **no ordering guarantee relative to each
other**. When several must reach a transport in the exact order they were
issued, Moshpit chains them explicitly instead of firing bare tasks:

- **`ActiveSession.moshWriteChain`** serializes mosh keystroke writes. Without
  it, typing a word and immediately tapping an arrow key could deliver the arrow
  bytes *before* the last character or two — the reported "history search stops
  matching what I typed" bug. Each `send` awaits the previous chained task
  before writing.
- **`TmuxSessionController.writeChain`** does the same for the `-CC` control
  path, so commands hit tmux in the same order their callback slots were queued.
- **`TmuxControlClient.pendingCallbacks`** is the matching FIFO on the *response*
  side: tmux answers **every** control command with a `%begin…%end` block —
  including fire-and-forget ones like `send-keys` — so each command must occupy
  a slot (`nil` = response ignored) or the queue desyncs and replies land in the
  wrong parser.
- **`ActiveSession.moshTmuxAttachTask`** is a *stored* task (not fire-and-forget)
  so `stop()` can cancel the long (~15s) mosh+tmux bootstrap mid-flight;
  otherwise a teardown or protocol switch could leave it opening a second SSH
  connection against a dead session and racing the new session's bootstrap over
  shared `SSHService.shared` state.

There's also `withTimeoutValue(_:_:)` — a task-group race between an operation
and a sleep — used for liveness probes so a dead half-open socket can't hang the
keepalive or attach loops.

---

## 7. The MoshpitIsland widget extension & App Group

`MoshpitIsland/` is a separate widget-extension target that renders the Vibe
Island: a Dynamic Island pill / Lock Screen Live Activity, plus a home/lock
timeline widget, both showing live agent status. The types shared across the
app/extension boundary live in `Moshpit/Island/` (compiled into both targets):
`AgentActivityAttributes` (the Live Activity payload), `AgentWidgetState` +
`AgentWidgetStore` (the App Group snapshot), and `AgentActivityMonitor` (the
`@MainActor` producer that watches tmux panes and computes agent state).

`AgentActivityMonitor` observes each session's `TmuxSessionController` — pane
output, bells, and the precise per-pane `@moshpit_*` hook stamps that coding
agents write into tmux user options — and computes an ordered list of active
agents (working / needs-you / done / idle). On every sync it feeds that list to
the widget extension by **two different mechanisms**, because the two widget
kinds are updated in fundamentally different ways:

1. **ActivityKit push → the Live Activity.** The Dynamic Island pill and Lock
   Screen expanded view are an `Activity<AgentActivityAttributes>`. The monitor
   **pushes** new `ContentState` to it directly from the running app
   (`Activity.request` / `activity.update`) on its ~2s poll. Because that push
   stops the instant iOS suspends the app, updates carry a `staleDate` so the
   views can render an honest "paused" hint instead of a frozen "working"
   forever. The Live Activity buttons (Allow / Deny / Reply / Interrupt via
   `AppIntent`s) route a keystroke back to the live agent pane through
   `SessionHub.deliverAgentInput(...)`, reconnecting first if iOS killed the
   socket.

2. **WidgetKit timeline pulling a JSON snapshot → the home/lock widget.** A
   `StaticConfiguration` timeline widget (`MoshpitStatusWidget`) **cannot** read
   the pushed ActivityKit state — a `TimelineProvider` runs on the OS's own
   schedule, in the extension process, with no access to the app's memory. So
   the monitor also **writes** the same agent list as JSON into a shared **App
   Group** container (`group.com.cluas.moshpit`, via `AgentWidgetStore` backed by
   a suite `UserDefaults`) and calls `WidgetCenter.shared.reloadAllTimelines()`
   to nudge a refresh. The widget's provider `read()`s that snapshot in
   `getTimeline`. On the simulator the App Group works without provisioning; on
   device the App Group capability must be enabled for both the app and the
   extension.

In short: the Live Activity is **pushed live data**; the timeline widget is a
**pulled JSON snapshot** through the App Group — same source, two delivery paths
dictated by what each widget kind can actually see.

---

## 8. Where to start reading

- **A connection's whole life:** `SessionHub.ActiveSession.start` → `startMosh`
  / tmux boot → `keepAlive` / `resumeIfNeeded` / `reconnect` → `stop`.
- **tmux protocol:** `TmuxControlClient.dispatchLine` (parser) and
  `TmuxSessionController`'s snapshot + pane-pool contract at the top of the file.
- **Mosh SSP:** `MoshTransport.handleDatagram` (the diff-apply state machine)
  and `MoshBootstrap`.
- **Widget bridge:** `AgentActivityMonitor.sync` / `writeWidgetSnapshot`, then
  `MoshpitIsland/MoshpitIslandWidget.swift` (Live Activity) and
  `MoshpitIsland/MoshpitStatusWidget.swift` (timeline).
