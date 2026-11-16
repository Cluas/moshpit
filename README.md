<div align="center">

<img src="Moshpit/Resources/AlternateIcons/Previews/IconPreview-Moshpit@3x.png" width="112" alt="Moshpit's icon: a pixel egg peeking over a terminal pane">

# Moshpit

### The terminal for coding agents on your own machines.

**Claude Code, Codex and Gemini keep working on the server. When one needs you, your phone knows.**<br>
SSH, Mosh, tmux and herdr on iPhone and iPad, with an encrypted push for every question<br>
and a Live Activity for who is waiting, who is working, and for how long.

[![App Store](https://img.shields.io/badge/App_Store-iPhone_%26_iPad-111111?logo=apple&logoColor=white)](https://apps.apple.com/app/id6799896801) ![iOS and iPadOS 18+](https://img.shields.io/badge/iOS_%7C_iPadOS-18%2B-111111?logo=apple&logoColor=white) [![License: GPL-3.0-only](https://img.shields.io/badge/license-GPL--3.0--only-111111)](LICENSE)

**[App Store](https://apps.apple.com/app/id6799896801)** &nbsp;·&nbsp; **[Website](https://moshpit.cluas.eu.org)** &nbsp;·&nbsp; **[Docs](https://moshpit.cluas.eu.org/docs)** &nbsp;·&nbsp; **[Issues](https://github.com/Cluas/moshpit/issues)**

**English** · [简体中文](README-zh.md) · [繁體中文](README.zh-Hant.md) · [日本語](README.ja.md) · [한국어](README.ko.md) · [Deutsch](README.de.md) · [Español](README.es.md) · [Français](README.fr.md) · [Português (Brasil)](README.pt-BR.md)

<br>

<img src="https://blog.cluas.eu.org/moshpit/shots/iphone/en-US/01-lock.png" width="250" alt="Who needs you and why, right on the Lock Screen."> <img src="https://blog.cluas.eu.org/moshpit/shots/iphone/en-US/03-home.png" width="250" alt="See who's working and who's waiting."> <img src="https://blog.cluas.eu.org/moshpit/shots/iphone/en-US/02-pane.png" width="250" alt="Tap in, land on the pane that asked.">

</div>

<br>

## Is Moshpit for you?

It is if you run Claude Code, Codex, Gemini CLI or another coding agent on a machine you reach over SSH (a workstation, a server, a box in the cloud), often inside tmux or herdr, and you want to answer the one that's asking from your phone instead of walking back to a laptop:

- **Know when you're needed.** A question reaches the Lock Screen as a push, encrypted on your host and opened on the phone, and one tap lands you in the pane that asked.
- **See every agent at once.** Home lists who needs you across all your hosts, then who is working and for how long, then who is idle.
- **Use a real terminal.** Mosh that holds through network changes, tmux windows and panes managed natively, a key bar, inline images.

It isn't if your agents don't run on a machine you can reach: Moshpit is a terminal, not a place to run an agent or a chat app for a hosted one. It is on iPhone and iPad only; the Android app is in development and not released.

## Quick start

**1 · Install** Moshpit from the [App Store](https://apps.apple.com/app/id6799896801) (a one-time purchase), or [build it yourself](#build-from-source).

**2 · Add a host.** Give it an address and a user, and sign in with a password or an SSH key: generate one on the phone, import one, or paste a PEM. Any SSH server works.

**3 · Set up the host (optional).** *Set up a host* installs the hooks that report each agent's state and pairs the host for pushes. It asks before it changes anything on the host. Codex needs one more step there: run `/hooks` in Codex and trust Moshpit's four entries.

**4 · Add Mosh, tmux or herdr (optional).** Mosh keeps the session through network changes; tmux and herdr give you their panes natively. The app carries an install guide for each.

## Supported agents

| Agent | State | "Needs you" | "Finished" |
| :-- | :-: | :-: | :-: |
| Claude Code | ✓ | ✓ | ✓ |
| Codex | ✓ | ✓ ¹ | ✓ |
| Gemini CLI | ✓ | ✓ ² | ✓ |
| Qwen Code | ✓ | ✓ | ✓ |
| Qoder | ✓ | ✓ | ✓ |
| Factory (`droid`) | ✓ | ✓ | ✓ |
| CodeBuddy | ✓ | ✓ | ✓ |

**State** shows on Home, the Dynamic Island and the Live Activity; **"Needs you"** and **"Finished"** are pushes. ¹ Codex asks through its permission requests. ² Gemini CLI asks only when a tool needs permission.

*Set up a host* installs the hooks for the agents it finds on the host. On tmux, the state on Home comes from them; on herdr, herdr reports it, with no setup at all. The pushes come from the hooks wherever the agent runs: in tmux, in herdr or in a plain shell.

A "needs you" push waits 30 seconds first, so a question you answer at the desk never reaches the phone. A "finished" push says how long the turn ran, and what it answered when the host has `jq`; a turn of three minutes or more chimes, a shorter one arrives silently.

## How it fits together

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/assets/architecture-dark.svg">
  <img src="docs/assets/architecture-light.svg" width="900" alt="How Moshpit fits together: the app on iPhone and iPad (a thin Swift shell over a Rust core) reaches your host over SSH or Mosh, and reads tmux or herdr panes and each agent's state. On the host, the agents' hooks stamp each pane with its state and hand 'needs you' and 'finished' to a script that seals them; the sealed push goes through a relay that sees ciphertext only, then APNs, to the notification service on the phone, which opens it.">
</picture>

The app reaches your host over SSH or Mosh, and drives tmux through its control mode and herdr through its own protocol, so panes are drawn natively and each agent's state arrives as data, not guessed from its output. On the host, each agent's hooks call a small script that stamps the pane with the agent's state and, for "needs you" and "finished", seals a push with the key paired in *Set up a host*. The relay forwards the sealed envelope to APNs and sees ciphertext only; the notification service on the phone opens it. [docs/PUSH.md](docs/PUSH.md) has the protocol end to end.

## What it does

### A terminal first

<p align="center">
<img src="https://blog.cluas.eu.org/moshpit/shots/iphone/en-US/05-overview.png" width="220" alt="Every tmux and herdr pane at a glance."> <img src="https://blog.cluas.eu.org/moshpit/shots/iphone/en-US/08-keyboard.png" width="220" alt="A keyboard made for the terminal.">
</p>

| Feature | What it means |
| :-- | :-- |
| **SSH and Mosh** | Mosh with its predictive echo. Sessions survive Wi-Fi/5G handoffs, sleep and bad hotel networks, and a connection that lived through the app being in the background is picked up again without a reconnect |
| **Says why it can't connect** | The port refused the connection, the name didn't resolve (or the Local Network permission is off), the host didn't answer (check your VPN), or there's no route from this network |
| **tmux control mode** | Native rendering, and windows and panes you can create, rename and kill. No tiny fake desktop squeezed onto a phone |
| **[herdr](https://github.com/herdrdev/herdr)** | The multiplexer built for coding agents, one full-width pane at a time |
| **Pane overview** | Every window and pane of the session at once, each a live picture of its screen, for tmux and herdr alike. Swipe sideways to move between panes |
| **When tmux goes away** | If the tmux server is killed or crashes, Home stops showing its sessions and agents as alive, and the terminal offers New Session |
| **Drawn with Metal** | On a fork of wezterm's terminal core: inline images (kitty, iTerm2, sixel), find in the scrollback, selection with handles, scrolling through history that keeps its momentum |
| **Typing** | A key bar for esc / ctrl / alt / arrows, IME composition, tappable links, custom themes, seven terminal fonts |
| **Hosts** | Each host wears a face drawn from its host key (the same key always the same face) or an emoji you pick. On a new iPhone restored from a backup, Moshpit lists the passwords and keys that stayed on the old one (keychain items never travel) so you can enter or import them again, and the host keys you trusted stay trusted |

### Built for agent work

<p align="center">
<img src="https://blog.cluas.eu.org/moshpit/shots/iphone/en-US/04-island.png" width="220" alt="Progress right on the Dynamic Island.">
</p>

| Feature | What it means |
| :-- | :-- |
| **"Needs you" first** | Home lists every agent waiting on you, across all your hosts. Below it, who is working, for how long, and who is idle |
| **Pushes on the Lock Screen** | Encrypted end to end and delivered even with the app closed. Tap one and you are in the pane that asked |
| **Live Activity** | On the Lock Screen and the Dynamic Island: who is waiting, who is working, and for how long |
| **A fresh worktree** | Start an agent on a new git worktree from your phone: pick a repo, name a branch, optionally hand it the first prompt |
| **Share into a pane** | Share an image from any app straight into an agent's pane |

### Files an agent mentions

<p align="center">
<img src="https://blog.cluas.eu.org/moshpit/shots/iphone/en-US/06-preview.png" width="220" alt="Open the file your agent just touched.">
</p>

Tap a path in an agent's output and the file opens on the phone: an image previews, a video plays, text reads as text. From there you can browse its folder and share the file on.

### Voice

Dictation runs on the device. **Qwen3-ASR** (0.6B, a one-time download) hears Chinese speech that carries English terms as one sentence, and takes the file names, branch and agent on the pane's screen as hot words. **Whisper** and Apple's on-device engines are there too. Audio never leaves the phone.

### iPad

<p align="center">
<img src="https://blog.cluas.eu.org/moshpit/shots/ipad/en-US/02-home-terminal.png" width="300" alt="Home and terminal, side by side.">
</p>

Hosts on the left, the terminal on the right. On a hardware keyboard, <kbd>⌘K</kbd> shows all panes, <kbd>⌘1</kbd>–<kbd>⌘9</kbd> picks a window and <kbd>⌘W</kbd> goes back to Home.

### The companion and the icons

<p align="center">
<img src="https://blog.cluas.eu.org/moshpit/shots/iphone/en-US/07-pet.png" width="220" alt="Hatch a pixel buddy to wait with you.">
</p>

| Feature | What it means |
| :-- | :-- |
| **A pixel companion** | It hatches from an egg on Home, perches on the card of whichever agent needs you most, takes on that agent's state, and keeps you company on the Dynamic Island and the Live Activity. With it on, each host's face becomes a pixel creature, drawn from the same key |
| **Twenty-two icons** | All in Settings: a pixel egg by default, the companion peeking over a pane for each of its six species (each with a shiny one), Classic and eight more. When your companion hatches, Home asks whether to switch to its species |

Every screenshot here is from the App Store set, made by the Screenshots workflow and refreshed after every complete run. Every language and the iPad: [all screenshots](https://blog.cluas.eu.org/moshpit/shots/).

## Honest by design

- One-time purchase. No subscription, no accounts, no feature tiers.
- Nothing is collected: no analytics, no tracking. Keys stay in the iOS Keychain, and traffic goes only to your own servers, plus a push relay that sees ciphertext only.
- Moshpit never installs anything on your host without asking, never creates sessions behind your back, and says so when it degrades instead of quietly switching transports.

## How it is built

Most of Moshpit is Rust. The core lives in `crates/`: the SSH transport (`ssh`, on russh), Mosh's protocol (`mosh`), the session layer that dials, reconnects and resumes (`session`), tmux control mode (`tmux`) and herdr (`herdr`), the terminal emulator (`term`, our wezterm fork in `vendor/wezterm`), what the app keeps (`store`), the agents' state machine (`agents`), the companion (`pet`), and host setup and push pairing (`hostcare`, `install`, `seal`). The screens are written in Rust too, with [NSH](https://nsh.dev), in `ui/`, and their models in `crates/app`.

On iOS a thin Swift layer hosts all of it through one static library (`crates/ffi`): the app shell, the terminal view's Metal drawing and text input, the Keychain, dictation (Qwen3-ASR on sherpa-onnx's onnxruntime, Whisper on WhisperKit, Apple's speech engines), and the parts iOS keeps to itself (the Live Activity and widgets, the notification service that opens pushes through `crates/seal-ffi`, the share extension). An Android app shares the same core and screens (`crates/android`, `android/`); it is in development and not released.

<details>
<summary><b>Repository layout</b></summary>

<br>

| Path | What lives there |
| :-- | :-- |
| `crates/` | The Rust core, one crate per concern: `ssh`, `mosh`, `session`, `tmux`, `herdr`, `term`, `stream` (a pane's bytes between the core and the terminal view), `store`, `agents`, `pet` (the companion), `seal` (the push envelope), `seal-ffi` (the notification service's one call into it), `hostcare`, `install`, `app` (the screens' models), `runtime`, `copy` (the rules the Chinese copy keeps), `ffi` (the static library iOS links) and `android` (the Android app's Rust half) |
| `ui/` | The screens, written in Rust with NSH and shared by iOS and Android |
| `vendor/wezterm/` | Our fork of wezterm's terminal emulator, behind `crates/term`; [docs/WEZTERM-PATCHES.md](docs/WEZTERM-PATCHES.md) lists what it changes |
| `Moshpit/` | The iOS host: the app shell, the Swift side of each screen, the terminal view's Metal drawing and text input, the Keychain, dictation (Qwen3-ASR, Whisper, Apple's engines) |
| `Extensions/` | The three app extensions: `MoshpitIsland/` (Live Activity and Lock Screen widget, Vibe Island), `MoshpitPush/` (notification service that decrypts pushes on the device), `MoshpitShare/` (hand an image to an agent's pane) |
| `Packages/MoshpitKit/` | Local Swift package the app and all three extensions link: logging, the Live Activity payload, the sealed-push envelope and pairing store |
| `android/` | The Android app (Gradle), in development |
| `Tests/` | `MoshpitTests/` unit tests, `MoshpitUITests/` UI tests |
| `push-relay/` | The stateless Go relay between your host and APNs, with its deployment manifest under `deploy/` |
| `docs/` | [ARCHITECTURE.md](docs/ARCHITECTURE.md), [PUSH.md](docs/PUSH.md) (the push protocol end to end), [WEZTERM-PATCHES.md](docs/WEZTERM-PATCHES.md), `nsh/` (design notes on the Rust core and the NSH screens), design notes |
| `scripts/` | `rust/` the iOS library's build and its Swift bindings, `host/` the shell installed on your dev host, `gen/` generators, `verify/` end-to-end checks, `spikes/` experiments, `wezterm/` the fork's upstream imports, `release/` archive tooling |

</details>

The app talks to the maintainer's relay by default; [docs/PUSH.md](docs/PUSH.md) explains what the relay can and cannot see and how to run your own.

## Build from source

You need macOS with Xcode 27 (the app deploys to iOS 18), [XcodeGen](https://github.com/yonaskolb/XcodeGen) via `brew install xcodegen`, and Rust through [rustup](https://rustup.rs). The repository pins its toolchain in `rust-toolchain.toml`, so rustup installs the right one on first use.

```sh
git clone https://github.com/Cluas/moshpit.git
cd moshpit
xcodegen generate        # project.yml is the source of truth; the .xcodeproj is generated
open Moshpit.xcodeproj
```

Pick the **Moshpit** scheme and a simulator, then press ⌘R. The app target's "Build Rust" phase runs `scripts/rust/build-ios.sh`, which builds `crates/ffi` into `libmoshpit_ffi.a` for the simulator or the device, adding the Rust target it needs (`aarch64-apple-ios-sim`, `aarch64-apple-ios`) the first time. Debug builds use the `app-test` cargo profile, Release builds `app`. The first build compiles the whole Rust graph and takes a while; later ones are incremental. The Swift packages resolve on the first build too.

A device build needs your Team ID in `Signing.xcconfig`; the comment in that file and [docs/install-free-account.md](docs/install-free-account.md) walk through it, including the free Apple ID path.

### Tests

The Rust side, as CI runs it:

```sh
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test -p moshpit-term        # one crate
```

The iOS app, as CI runs it on the Xcode 27 image:

```sh
scripts/rust/test-build-ios.sh    # the Rust phase's profiles and links, without building
xcodebuild test -project Moshpit.xcodeproj -scheme Moshpit \
  -destination 'platform=iOS Simulator,name=iPhone 18 Pro' CODE_SIGNING_ALLOWED=NO
```

A few suites skip on an unsigned build (they need the App Group container) or off the iPad simulator (hardware-keyboard shortcuts), and some UI flows skip until they are written again for the current screens; the log names each one.

The Android app builds and tests with `android/ci.sh`, which fetches what it needs (the SDK parts, the NDK, Gradle) beside a JDK 17.

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md) for the workflow and the house style, and [SECURITY.md](SECURITY.md) before reporting anything sensitive. Everyone here follows the [Code of Conduct](CODE_OF_CONDUCT.md).

Moshpit is shared on [LINUX DO](https://linux.do), a developer community in Chinese. Questions and ideas are as welcome there as in the issue tracker.

## License

Moshpit is free software, released under the [GNU General Public License v3.0](LICENSE) (GPL-3.0-only). You may build, modify and redistribute it under the terms of that license. The App Store build is published by the maintainer, who holds the copyright. Third-party components keep their own licenses, listed in [NOTICES.md](NOTICES.md).
