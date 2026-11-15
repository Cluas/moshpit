# Moshpit

**English** · [中文](README-zh.md)

**Moshpit is an SSH / Mosh / tmux / herdr terminal for iPhone and iPad, for people
who run coding agents on remote machines.** Claude Code, Codex or Gemini keeps
working on the server. When one needs you, an encrypted push on the Lock Screen
takes you to the pane that asked, and the Live Activity there and on the Dynamic
Island shows who is waiting, who is working, and for how long. On the App Store
it is *Moshpit: Terminal for Agents*.

[App Store](https://apps.apple.com/app/id6799896801) ·
[Website](https://moshpit.cluas.eu.org) ·
[Docs](https://moshpit.cluas.eu.org/docs) ·
[Issues](https://github.com/Cluas/moshpit/issues)

## Screenshots

<table>
<tr><td><img src="https://blog.cluas.eu.org/moshpit/shots/iphone/en-US/01-lock.png" width="190" alt="Who needs you and why, right on the Lock Screen."></td><td><img src="https://blog.cluas.eu.org/moshpit/shots/iphone/en-US/02-pane.png" width="190" alt="Tap in, land on the pane that asked."></td><td><img src="https://blog.cluas.eu.org/moshpit/shots/iphone/en-US/03-home.png" width="190" alt="See who&#x27;s working and who&#x27;s waiting."></td><td><img src="https://blog.cluas.eu.org/moshpit/shots/iphone/en-US/04-island.png" width="190" alt="Progress right on the Dynamic Island."></td></tr>
<tr><td><img src="https://blog.cluas.eu.org/moshpit/shots/iphone/en-US/05-overview.png" width="190" alt="Every tmux and herdr pane at a glance."></td><td><img src="https://blog.cluas.eu.org/moshpit/shots/iphone/en-US/06-preview.png" width="190" alt="Open the file your agent just touched."></td><td><img src="https://blog.cluas.eu.org/moshpit/shots/iphone/en-US/07-pet.png" width="190" alt="Hatch a pixel buddy to wait with you."></td><td><img src="https://blog.cluas.eu.org/moshpit/shots/iphone/en-US/08-keyboard.png" width="190" alt="A keyboard made for the terminal."></td></tr>
</table>

The App Store set, made by the Screenshots workflow and refreshed after every complete run (nightly and on each version tag). Every language and the iPad shots: [all screenshots](https://blog.cluas.eu.org/moshpit/shots/).

## What it does

**A real terminal first**

- SSH and Mosh, with Mosh's predictive echo. Sessions survive Wi-Fi/5G handoffs,
  sleep and bad hotel networks, and a connection that lived through the app being
  in the background is picked up again without a reconnect.
- A host that can't be reached says why: the port refused the connection, the
  name didn't resolve (or the Local Network permission is off), it didn't answer
  (check your VPN), or there's no route from this network.
- tmux control mode: native rendering, and windows and panes you can create,
  rename and kill. No tiny fake desktop squeezed onto a phone.
- [herdr](https://github.com/herdrdev/herdr) support: the multiplexer built for
  coding agents, rendered one full-width pane at a time.
- The pane overview: every window and pane of the session at once, each with a
  live picture of its screen, for tmux and herdr alike. Swipe sideways to move
  between panes.
- When the tmux server goes away under you (killed, crashed), Home stops showing
  its sessions and agents as alive, and the terminal offers New Session.
- A terminal drawn with Metal on a fork of wezterm's terminal core: inline images
  (kitty, iTerm2, sixel), find in the scrollback, selection with handles, and
  scrolling through history that keeps its momentum.
- iPad: hosts on the left, the terminal on the right. On a hardware keyboard, ⌘K
  shows all panes, ⌘1–⌘9 picks a window and ⌘W goes back to Home.
- Dictation on the device. Qwen3-ASR (0.6B, a one-time download) hears Chinese
  speech that carries English terms as one sentence, and takes the file names,
  branch and agent on the pane's screen as hot words. Whisper and Apple's
  on-device engines are there too. Audio never leaves the phone.
- Custom themes, seven terminal fonts, a key bar for esc / ctrl / alt / arrows,
  IME composition, tappable links.
- Twenty-two home-screen icons, all in Settings: a pixel egg by default, the
  companion peeking over a pane for each of its six species (each with a shiny
  one), Classic and eight more. When your companion hatches, Home asks whether to
  switch to its species.
- Each host wears a face drawn from its host key, the same key always the same
  face, or an emoji you pick.
- A new iPhone restored from a backup: Moshpit lists the passwords and keys that
  stayed on the old one (keychain items never travel), so you can enter or import
  them again, and the host keys you trusted stay trusted.

**Built for agent work**

- Home puts "Needs you" first: every agent waiting on you, across all your hosts.
  Below it, who is working, for how long, and who is idle.
- Needs-you pushes on the Lock Screen, encrypted end to end and delivered even
  with the app closed. Tap one and you are in the pane that asked.
- Files an agent mentions open on the phone: tap a path in the output to preview
  an image, play a video or read a file, browse its folder, and share it on.
- Start an agent on a fresh git worktree from your phone: pick a repo, name a
  branch, optionally hand it the first prompt.
- Share an image from any app straight into an agent's pane.
- A pixel companion that hatches from an egg on Home, perches on the card of
  whichever agent needs you most, takes on that agent's state, and keeps you
  company on the Dynamic Island and the Live Activity. With it on, each host's
  face becomes a pixel creature, drawn from the same key.
- Agent status needs no host-side setup on herdr. On tmux, once you say yes in
  Host Setup, it installs hooks for the agents it finds on the host: Claude Code,
  Codex, Gemini CLI, Qwen Code, Qoder, Factory and CodeBuddy.

**Honest by design**

- One-time purchase. No subscription, no accounts, no feature tiers.
- Nothing is collected: no analytics, no tracking. Keys stay in the iOS Keychain,
  and traffic goes only to your own servers, plus a push relay that sees ciphertext only.
- Moshpit never installs anything on your host without asking, never creates
  sessions behind your back, and says so when it degrades instead of quietly
  switching transports.

Works with any SSH server. Mosh, tmux and herdr are optional; install guides are built in.

## How it is built

Most of Moshpit is Rust. The core lives in `crates/`: the SSH transport
(`ssh`, on russh), Mosh's protocol (`mosh`), the session layer that dials,
reconnects and resumes (`session`), tmux control mode (`tmux`) and herdr
(`herdr`), the terminal emulator (`term`, our wezterm fork in `vendor/wezterm`),
what the app keeps (`store`), the agents' state machine (`agents`), the
companion (`pet`), and host setup and push pairing (`hostcare`, `install`,
`seal`). The screens are written in Rust too, with [NSH](https://nsh.dev), in
`ui/`, and their models in `crates/app`. On iOS a thin Swift layer hosts all of
it through one static library (`crates/ffi`): the app shell, the terminal view's
Metal drawing and text input, the Keychain, dictation (Qwen3-ASR on sherpa-onnx's
onnxruntime, Whisper on WhisperKit, Apple's speech engines), and the parts iOS
keeps to itself (the Live Activity and widgets, the notification service that
opens pushes through `crates/seal-ffi`, the share extension). An Android app
shares the same core and screens (`crates/android`, `android/`); it is in
development and not released.

## Build from source

You need macOS with Xcode 27 (the app deploys to iOS 18),
[XcodeGen](https://github.com/yonaskolb/XcodeGen) via `brew install xcodegen`,
and Rust through [rustup](https://rustup.rs). The repository pins its toolchain
in `rust-toolchain.toml`, so rustup installs the right one on first use.

```sh
git clone https://github.com/Cluas/moshpit.git
cd moshpit
xcodegen generate        # project.yml is the source of truth; the .xcodeproj is generated
open Moshpit.xcodeproj
```

Pick the **Moshpit** scheme and a simulator, then press ⌘R. The app target's
"Build Rust" phase runs `scripts/rust/build-ios.sh`, which builds `crates/ffi`
into `libmoshpit_ffi.a` for the simulator or the device, adding the Rust target
it needs (`aarch64-apple-ios-sim`, `aarch64-apple-ios`) the first time. Debug
builds use the `app-test` cargo profile, Release builds `app`. The first build
compiles the whole Rust graph and takes a while; later ones are incremental.
The Swift packages resolve on the first build too.

A device build needs your Team ID in `Signing.xcconfig`; the comment in that
file and [docs/install-free-account.md](docs/install-free-account.md) walk
through it, including the free Apple ID path.

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

A few suites skip on an unsigned build (they need the App Group container) or
off the iPad simulator (hardware-keyboard shortcuts), and some UI flows skip
until they are written again for the current screens; the log names each one.

The Android app builds and tests with `android/ci.sh`, which fetches what it
needs (the SDK parts, the NDK, Gradle) beside a JDK 17.

## Repository layout

| Path | What lives there |
|---|---|
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

The app talks to the maintainer's relay by default; [docs/PUSH.md](docs/PUSH.md)
explains what the relay can and cannot see and how to run your own.

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md) for the workflow and the house style,
and [SECURITY.md](SECURITY.md) before reporting anything sensitive. Everyone here
follows the [Code of Conduct](CODE_OF_CONDUCT.md).

## Community

Moshpit is shared on [LINUX DO](https://linux.do), a developer community in Chinese.
Questions and ideas are as welcome there as in the issue tracker.

## License

Moshpit is free software, released under the [GNU General Public License v3.0](LICENSE)
(GPL-3.0-only). You may build, modify and redistribute it under the terms of that
license. The App Store build is published by the maintainer, who holds the copyright.
Third-party components keep their own licenses, listed in [NOTICES.md](NOTICES.md).
