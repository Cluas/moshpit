# Moshpit

**English** · [中文](README-zh.md)

**Moshpit is an SSH / Mosh / tmux terminal client for iPhone and iPad**, built for
developers who run remote AI coding agents (Claude Code, and other CLI tools) and
want to keep a shell — and an eye on their agents — in their pocket.

Its headline feature is **Vibe Island**: a Live Activity that surfaces the status of
a remote agent session on the Lock Screen and in the Dynamic Island, so you can watch
a long-running build or agent run without keeping the app in the foreground.

Moshpit is written in SwiftUI, targets iOS 18, and is generated with
[XcodeGen](https://github.com/yonaskolb/XcodeGen) — `project.yml` is the source of
truth and `Moshpit.xcodeproj` is generated from it.

## Features

- **A one-time purchase, not a subscription.** One price unlocks every feature —
  no subscription, no in-app purchases, no feature tiers, no per-host limits, and
  no upsell for the parts you actually came for. Moshpit is MIT-licensed as well,
  so [building and sideloading it yourself](docs/install-free-account.md) with a
  free Apple ID is a first-class path rather than a loophole.
- **Mosh with roaming.** Sessions survive network handoff (Wi-Fi ↔ cellular) and IP
  changes, reconnecting automatically instead of dropping your shell.
- **tmux integration.** Attach to existing tmux sessions with native window and pane
  switching, so the app understands your session layout instead of treating it as a
  raw scrollback.
- **Vibe Island Live Activity.** Monitor a remote agent or long-running command from
  the Lock Screen and Dynamic Island — latency, session, and agent status at a glance.
- **4 switchable app themes.** Signal Room, Moshpit Classic, Terminal Green, and Amber
  Console — each pairs an accent color with a matching alternate app icon. The terminal
  itself ships with several built-in color schemes (Dracula, Nord, Solarized Dark,
  Monokai, Tokyo Night, and more).
- **SSH built in.** Pure-Swift SSH via [Citadel](https://github.com/orlandos-nl/Citadel),
  with keys stored in the iOS Keychain and unlocked with Face ID.
- **A terminal that handles real work.** [SwiftTerm](https://github.com/migueldeicaza/SwiftTerm)
  emulation (Moshpit uses a small fork), a custom key bar for `esc`/`ctrl`/`alt`/arrows,
  IME/Pinyin composition support, and tappable links.

## Screenshots

| Servers | Terminal | tmux panes | Vibe Island |
|---|---|---|---|
| ![Servers list](docs/assets/02-home.png) | ![Terminal session](docs/assets/04-terminal.png) | ![tmux panes](docs/assets/06-tmux-panes.png) | ![Live Activity](docs/assets/10-live-activity.png) |

## Getting Started

### Requirements

- macOS with Xcode 26
- [XcodeGen](https://github.com/yonaskolb/XcodeGen): `brew install xcodegen`
- iOS 18 target (device or simulator)

### Build & run

```bash
# 1. Clone the repository
git clone <repository-url>
cd moshpit

# 2. Generate the Xcode project from project.yml
xcodegen generate

# 3. Open the generated project
open Moshpit.xcodeproj
```

In Xcode, select the **Moshpit** scheme, pick a simulator or your device, and press
**⌘R**. The app's bundle identifier is `com.cluas.moshpit` (the Vibe Island widget
extension is `com.cluas.moshpit.island`). Swift Package dependencies — SwiftTerm and
Citadel — are resolved automatically on first build.

### Installing on your own iPhone

You do **not** need a paid Apple Developer account. A free Apple ID is enough to
sideload Moshpit onto your own device, and there's a step-by-step guide covering
signing, on-device install, and optional auto-resigning with AltStore / SideStore:

➡️ **[Install on iPhone with a free Apple ID](docs/install-free-account.md)**

## Architecture

For a deeper tour of the codebase — services, the SwiftUI layout, and how Vibe Island
is wired up — see [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).
