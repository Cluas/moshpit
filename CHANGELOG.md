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
- **Tap to put the cursor where you tapped.** A tap on the terminal now sends
  the click that a mouse-aware program — Claude Code's prompt, vim, less —
  reads as "the cursor goes here", so reaching a character in the middle of a
  long prompt is one tap instead of walking the arrow keys one cell at a time.
  Only programs that asked for the mouse get it: a plain shell would print the
  report into its command line, so it sends nothing there.

### Fixed

- **A URL wrapped across two lines opens as the whole address, not the first
  line's half.** Programs that lay out their own transcript — Claude Code is
  the constant case — print a long URL as two physical lines with a hard
  newline and often an indented continuation, so the terminal's own
  soft-wrap bookkeeping never links them. The plain-link detector now
  continues a URL that runs flush into the right edge onto the next row's
  URL-shaped run, and tags every row with the joined address; a complete URL
  that merely happens to end at the last column is left alone, so tapping it
  keeps working as before.
- **Returning from the background no longer flashes a mis-wrapped frame.**
  Moshpit hands the tmux window back while it is away, so a desktop client
  attaching meanwhile isn't stranded at phone width — but output kept arriving
  the whole time, laid out for whatever width the window then took, and painting
  it into the phone's grid is what made coming back land on one visibly wrong
  frame. That output is no longer painted at all: the screen keeps the last
  frame it drew — a few seconds stale, but correctly wrapped — and is repainted
  from tmux once the window is ours again. Nothing is lost, because a tmux
  pane's scrollback lives on the server and that is what the scroll gesture
  pages. Agent bells still come through while away, so "needs your input" still
  reaches you.
- **The first connect no longer wraps every line a character or two short.**
  Connecting to a session a desktop terminal is also attached to left the tmux
  window at the desktop's width: the program in the pane rendered to that width
  while the phone hard-wrapped each line to its own narrower grid, spilling one
  or two characters onto the next line and pushing box-drawn tables past the
  edge. Tapping the terminal fixed it, because that resize was the only thing
  that ever claimed the window. Moshpit now claims it as the panes are
  discovered, which is what the connect path had always assumed happened — and
  claims it at the grid the app's own layout reports rather than at the rough
  pre-connect estimate, which on a modern phone is a column and a good ten rows
  out.
- **A tmux pane paints on connect instead of waiting for you to raise the
  keyboard.** On some phones the first connect showed nothing but a cursor
  until you tapped the terminal, and then the whole screen appeared at once.
  Two things had to line up. A pane's terminal is created during discovery,
  before the app has laid it out even once, and it was created at a zero frame —
  which is a one-column grid, so the authoritative screen tmux hands over at
  attach was parsed into a single column: a shell prompt collapsed into
  stacked characters, a full-screen agent frame into nothing. Panes are now
  born at the grid the client already reports, so a paint that beats the first
  layout still lands in the shape tmux drew it for. The repair path was
  disabled too: the size a view reports once laid out was compared against the
  pre-connect *estimate* and dropped as "unchanged" whenever the estimate had
  been exactly right, so the repaint that follows a settled size never ran —
  on the phones whose estimate was off by a row it always had, which is why
  this only bit some. The first report from a real view now always commits.
  (Measured while fixing this: tmux emits nothing at all to a control client on
  attach, and nothing for a `refresh-client` that names the size it already
  has — it repaints only when the size actually changes. Nothing in the app may
  assume otherwise.)
- **Swiping the app away no longer types an arrow key into your session.** With
  the keyboard collapsed the shortcut bar sits at its lowest, a finger's width
  above the home indicator, and iOS's swipe-up-to-background could begin on the
  arrow-key joystick rather than below it — which fired a real `←`/`↑` at the
  remote on its way off screen. In Claude Code `←` opens the session switcher,
  so backgrounding the app appeared to navigate the agent by itself. The
  joystick and the scroll thumb now engage only once a touch has held still for
  a moment, which a push against them does and a swipe passing through never
  does. Sessions with the keyboard up were never affected: the bar rides above
  the keyboard, out of the swipe's way.
- **Dictation no longer guesses the wrong language from the interface
  language.** "Automatic" resolved to `Locale.current`, so an English-language
  iPhone loaded an English acoustic model and returned confident nonsense for
  a Chinese speaker, with nothing on screen to explain why. Automatic now
  walks the user's own preferred-language ranking and installed keyboards and
  picks the first language an engine can actually transcribe; on Whisper it
  means real audio-based detection. The dictation overlay also shows the
  engine and language in use, so a wrong language is visible before you speak.

### Changed

- **The keyboard follows where you tap, not whether you tap.** Opening a
  session is usually to read it — history, a link, output to select and copy —
  and on iOS focusing the terminal *is* the keyboard, so every tap taken
  while reading threw one over the very thing being read. Now a tap on the
  input area — the cursor's own rows: Claude Code's prompt box, a shell
  prompt — raises the keyboard and positions the cursor; a tap anywhere else
  is just a tap: links open, double-tap still selects, and clicks reach
  mouse-aware programs with the keyboard down (Claude Code's "jump to bottom
  (click)" now works without summoning a keyboard first). The shortcut bar's
  toggle and Settings → Raise keyboard on open still work as before. (This
  replaces the earlier half-measure that only swallowed taps within two
  seconds of a scroll.)
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
