# Changelog

All notable changes to Moshpit are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Voice input** — a mic key on the terminal bar dictates commands and
  prompts. Speech is transcribed by Apple's models strictly on-device (the
  iOS 26 SpeechAnalyzer stack where available, with on-device fallbacks for
  earlier systems; server-based recognition is never used, so audio never
  leaves the device). A language's model downloads once on first use and then
  works offline. Dictation is composed in a preview overlay and only typed
  into the session on Insert — via bracketed paste, so multi-line prompts
  land in agents like Claude Code as one block. Language is configurable in
  Settings → VOICE INPUT (defaults to the system language).

## [1.0.0]

First public release.

Moshpit is an iOS SSH / Mosh / tmux terminal client for remote development and
AI coding workflows from an iPhone or iPad.

- **Transports** — SSH (password and private-key auth, Keychain-stored and
  Face ID / Touch ID gated), the Mosh UDP transport with roaming and automatic
  reconnect, and deep tmux integration over `tmux -CC` control mode with native
  session / window / pane navigation.
- **Vibe Island** — live AI agent status in the Dynamic Island and a Live
  Activity, with a lock-screen control surface (Allow / Deny / Reply and a done
  ping) so you can drive an agent without opening the app.
- **Terminal** — SwiftTerm-based emulator with selectable themes, adjustable
  fonts, a custom terminal keyboard bar, and scrollback that adapts to mouse
  apps vs. plain shells.
- **Fully free** — no subscriptions, no paywall; every feature is available to
  everyone.
- **Redesigned visual language** — the current Moshpit look and app iconography.

[1.0.0]: https://github.com/Cluas/moshpit/releases/tag/v1.0.0
