# Changelog

All notable changes to Beacon are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.0.0]

First public release.

Beacon is an iOS SSH / Mosh / tmux terminal client for remote development and
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
- **Redesigned visual language** — the current Beacon look and app iconography.

[1.0.0]: https://github.com/Cluas/beacon/releases/tag/v1.0.0
