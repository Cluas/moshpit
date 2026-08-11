# Changelog

All notable changes to Moshpit are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Voice input** — a mic chip on the terminal bar dictates commands and
  prompts. Speech is transcribed strictly on-device; server-based recognition
  is never used, so audio never leaves the device. Dictation is composed in a
  preview overlay and only typed into the session on Insert — via bracketed
  paste, so multi-line prompts land in agents like Claude Code as one block.
- **Local Whisper as an alternative speech engine** — Settings → VOICE INPUT →
  Recognition now offers a Whisper model running locally through WhisperKit,
  alongside Apple's built-in engines. Apple's transcribers take exactly one
  locale per session and have no code-switching mode, so a sentence that
  doesn't stay in one language — the normal way anyone dictates English
  command names in their own speech — comes out mangled whichever locale is
  picked. One multilingual Whisper model covers ~100 languages and handles the
  mixing natively. Models are chosen and downloaded (with sizes shown up
  front) under Voice Input → Model, stored in Application Support and excluded
  from iCloud backup. Only multilingual variants are offered — the
  English-only `.en` and distil builds would defeat the purpose. For CJK
  languages the session also primes the model to keep Latin-script technical
  words as-is, and Chinese to emit Simplified rather than Traditional.

### Fixed

- **Dictation no longer guesses the wrong language from the interface
  language.** "Automatic" resolved to `Locale.current`, so an English-language
  iPhone loaded an English acoustic model and returned confident nonsense for
  a Chinese speaker, with nothing on screen to explain why. Automatic now
  walks the user's own preferred-language ranking and installed keyboards and
  picks the first language an engine can actually transcribe; on Whisper it
  means real audio-based detection. The dictation overlay also shows the
  engine and language in use, so a wrong language is visible before you speak.

### Changed

- **The mic is a normal shortcut chip.** It used to be pinned beside the
  keyboard toggle, outside the scrolling row — permanently costing bar width
  and the only shortcut nobody could reorder or hide. It is now an ordinary
  built-in chip: reorder it, move it out of the toolbar, or scroll past it.
  Existing installs get it appended to the trailing end of the bar, where it
  already appeared.

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
