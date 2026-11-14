# Forked SwiftTerm patches

Moshpit depends on a fork of [SwiftTerm](https://github.com/migueldeicaza/SwiftTerm)
rather than the upstream release. The fork lives at
[github.com/Cluas/SwiftTerm](https://github.com/Cluas/SwiftTerm) on branch
`moshi-hyperlink-underline-fix`, forked from upstream `v1.13.0`. `project.yml`
pins it by commit revision (`packages.SwiftTerm.revision`).

This document records every patch carried on that branch on top of `v1.13.0`,
why each one exists, and whether it looks like a candidate to send back
upstream. It is the expanded, prose version of the numbered comment block in
`project.yml`. If any of these land upstream and a tagged release picks them up,
the corresponding patch can be dropped and the dependency moved back to the
upstream package.

The patches fall into these groups: OSC-8 hyperlink underline fixes (1–2), a
host-side plain-text linkification API (3), iOS tap, focus and selection
behaviour (4, 13, 15–17, 20), IME (input-method, e.g. Pinyin) composition
fixes (5–7, 9, 11), a soft-wrap accessor (8), hold-to-repeat delete over text
the remote owns (10, 12), OSC 52 clipboard reads (14), wide-character write
hygiene (18), a reconnect input reset (19), host-driven pixel scrolling on iOS
(21), hyperlinks wrapped over several rows (22), third-party keyboard edits
(23), partial repaints on iOS (24–28), an erased history the view hears about
(29), a bright-colours setting that takes effect at run time (30), a paste
the host can take over (31), a full reset the view hears about (32), a
large history that costs only the lines it holds (33), a full reset that
frees the terminal and starts at home (34–35), an erase that unwraps the
right row (36), a selection that keeps to its text over erases, resets
and Select All (37–39), round twenty-four's emulator fixes (40–45:
autowrap's wrap mark, the selection over a full history, a switch of
screens and a triple-click, DECCOLM as xterm does it, synchronized output
that copies the screen only), and the iOS view freed when its pane closes
(46), kept away from AutoFill (47), and made for a pointer and a hardware
keyboard on an iPad and on a Mac (48–52), and round twenty-nine's (53–61:
the alternate screen's height, a full history's reused lines, a
selection's last column and a long press's slide, a shorter history,
DECCOLM left to the program, autowrap below the scrolling region, and the
text input's predictions and Command key), and round thirty-one's (62–65:
a cancelled press released for the kitty keyboard protocol, `clear` and
the scroll-invariant line numbers, a grid the host keeps its own, and the
image lines `clear` takes with it), and round thirty-two's (66: screen's
title sequence consumed).

## At a glance

One line per patch. **engine** means the fix lives in SwiftTerm's
platform-neutral emulator code (`Terminal.swift`, `Buffer.swift`,
`SelectionService.swift`): any other front end on the same byte streams —
the Android port included — has to carry the same fix in its own emulator.
**iOS** means UIKit view or text-input code; another platform re-solves the
user-facing problem in its own input stack instead of porting the code.
**Catalyst** means UIKit view code built only for a Mac (Mac Catalyst,
`targetEnvironment(macCatalyst)`).

- **1** · engine · `d94216c` — An OSC-8 close ends the link at its last cell;
  the underline overshot the link text by one column.
- **2** · engine · `0fe3cfb` — An OSC-8 close stops at the link's own text;
  the range could run on across the line and into scrollback.
- **3** · engine · `b550659` — `viewportLineText` / `tagPlainTextLink` let the
  host linkify URLs it detects itself; the built-in implicit-link regex also
  matches bare paths.
- **4** · iOS · `e7315e1` — An unfocused tap hit-tests links before "focus me";
  the first tap on a link was eaten.
- **5** · iOS · `c297b2f` — IME marked text is drawn, underlined, at the
  cursor; it was tracked but never shown.
- **6** · iOS · `a8f1a28` `ca54023` `2f00765` — The real caret sits at the end
  of the composing text; trusting the IME's selection offset made it jitter.
- **7** · iOS · `ca54023` — Delete during a composition edits the marked text
  and sends nothing; it sent real backspaces and wiped the composition.
- **8** · engine · `69829cf` — Public `isRowWrapped(_:)`; hosts joined wrapped
  URLs by guessing from text width, which CJK and emoji broke.
- **9** · iOS · `9dfb96b` — Ending a composition does not snap the caret to
  the buffer cursor, which has not moved before the echo arrives.
- **10** · iOS · `ef247ad` — `hasText` is always true; UIKit gates
  hold-to-repeat delete on it, and the shadow buffer is empty for remote text.
- **11** · iOS · `866078a` — `updateCursorPosition()` leaves the caret to a
  live composition; repaints yanked it to the front mid-word.
- **12** · iOS · `b4bd2c4` — One phantom newline in front of the caret; UIKit
  measures one character back before each delete repeat and stopped on empty.
- **13** · iOS · `f8d2dde` — An unfocused link tap opens the link without
  becoming first responder; the keyboard popped over what was being read.
- **14** · engine · `a5198e8` — OSC 52 read queries reach a `clipboardRequest`
  delegate and are answered by `sendClipboardResponse`; they were parsed as
  base64 and dropped.
- **15** · iOS · `0a10d48` — `focusOnTap` (default true); on iOS focus is the
  keyboard, and a tap taken while reading should stay a tap.
- **16** · iOS · `5be43e4` — Finger-sized selection handles and an edit menu
  that shows unfocused; `UIMenuController` needs first responder.
- **17** · iOS · `cebf9df` — Long-press selects the word under the finger; it
  summoned the keyboard and selected nothing.
- **18** · engine (+ renderer) · `740e797` `b1db6c8` `aa3a70a` `29949bc` —
  Writing over either half of a wide character clears the other half, with
  erase semantics; an orphaned half renders as a bright block that a
  screen-diffing peer (mosh, a TUI over tmux) never repaints.
- **19** · engine · `730eaaa` — `discardPendingInput()` drops a half-read
  UTF-8 or escape sequence and nothing else; after a reconnect the new stream
  was parsed as the dead stream's continuation.
- **20** · iOS · `fa25f83` — Public `closeSelection()`; with `focusOnTap` off
  nothing could dismiss a selection.
- **21** · iOS · `7a6d6db` `fc38113` `68da3aa` — The scroll offset follows
  `yDisp`, plus a pixel scroll API on the device pixel grid; programmatic
  scrolling never moved the picture.
- **22** · engine · `577828e` `b6042c4` — A hyperlink printed over several rows
  selects and copies as one link; the copy carried a row break and an indent
  inside the URL.
- **23** · iOS · `dd7f2cd` — A keyboard is not told about edits it made
  itself, and putting the phantom back is not a Return; voice input stopped
  mid-sentence, and a delete at the front of the buffer pressed Return.
- **24** · iOS · `25a06eb` — A font change repaints the view's own `bounds`,
  not its `frame`; a new font that kept the grid size did not redraw.
- **25** · engine · `c5519e0` — ECH (`CSI n X`) and REP (`CSI n b`) record the
  rows they change in the update range; once repaints were partial (27) their
  cells kept stale text on screen.
- **26** · iOS · `46b268e` — A draw pass lays out only the rows it repaints.
  iOS 27 hands `draw(_:)` the whole bounds whatever was invalidated, so a
  `TerminalViewLayer` records the invalidated rows itself.
- **27** · iOS · `62791d5` — `updateDisplay` invalidates only the rows in the
  update range (the old "TODO iOS"); echoing one character redrew the whole
  screen, about 5 ms of main thread per key on a 70×72 grid, now about 0.5 ms.
- **28** · iOS (Metal) · `963ec1e` — The Metal renderer rebuilds only the
  changed rows and merges dirty ranges instead of overwriting them; a
  cursor-only move produced no new frame.
- **29** · engine · `232e37e` — ED 3 (`CSI 3 J`, erase saved lines) repaints
  the screen and tells the view where the viewport went; after `clear` with
  history the iOS scroll offset stayed on a row that no longer existed and
  every draw came out blank.
- **30** · iOS (+ macOS) · `1021332` — Changing `useBrightColors` drops the
  attribute cache and repaints; set at run time it changed nothing.
- **31** · iOS (+ macOS) · `adecbab` — `pasteRequested(source:text:)` lets the
  host take over Paste and ⌘V, and iOS sends its own paste in one write; over
  tmux `-CC` and herdr the local bracketed-paste flag is never set, so a
  multi-line paste ran line by line.
- **32** · engine · `b9a651b` — A full reset (RIS `ESC c`, DECCOLM) tells the
  view where the viewport went; after `reset` with history the iOS view drew
  blank rows until something scrolled.
- **33** · engine · `57485df` — A resize makes no lines for history nobody
  wrote; with a 50 000-line scrollback the first resize made all 50 030 of
  them, about 96 MB at 80 columns.
- **34** · engine · `a6dc313` — A full reset no longer keeps the terminal
  alive; the fresh buffer's scroll callback held it, so after one `ESC c`,
  DECCOLM or Reset the terminal and its history were never freed.
- **35** · engine · `c932e4d` — A full reset puts the cursor at home and clears
  the alternate screen if it shows; it took the alternate screen's cursor,
  so `reset` after a full-screen program left the prompt mid-screen.
- **36** · engine · `4f01624` — ED 1 from the last column unwraps the screen
  row under the cursor; it cleared the flag of a history line, which then
  copied with a line break inside.
- **37** · engine (+ iOS) · `5a34360` — Erasing the saved lines (ED 3) moves
  the selection with its text (`TerminalDelegate.linesDropped`); after
  `clear` the highlight and the copy landed on other rows.
- **38** · engine (+ iOS) · `5fb1775` — A full reset the remote sends drops
  the selection (`TerminalDelegate.terminalReset`); it pointed into the
  buffer the reset threw away.
- **39** · engine · `d66ca96` — Select All ends on the last line with content;
  with a 10 000-line history every Select All + Copy made all of its lines,
  about 55 MB, and a full last line lost its last cell.
- **40** · engine · `7414775` — Autowrap above the bottom row marks the row
  the text runs on to; with history it marked a line of history, and the
  long line copied as two.
- **41** · engine (+ iOS) · `d379073` — A line scrolling into a full history
  moves the selection with its text (`linesDropped`); it slid a line down the
  text with every line of output.
- **42** · engine (+ iOS) · `7c8ca8d` — Switching between the normal and the
  alternate screen drops the selection; Copy took the other screen's text.
- **43** · engine (+ iOS) · `5685a27` — DECCOLM clears the screen, homes the
  cursor, resets the margins and changes the columns, as xterm does; it was a
  full reset that dropped the history, every mode and the alternate screen.
- **44** · engine · `9492769` — A triple-click selects the whole row; the copy
  lost the row's last cell.
- **45** · engine · `409b40d` — A synchronized update copies the screen, not
  the history; with 10 000 lines every Claude Code frame copied and freed
  about 21 MB, 3–4 ms in a release build.
- **46** · iOS · `122d2ac` — The display link holds the view weakly; every
  closed pane kept its view, its terminal and its history (about 19 MB).
- **47** · iOS · `ce7bb72` — The text input names its own content type, so
  AutoFill offers nothing on it; on a Mac it tried to show one-time codes at
  the terminal's caret.
- **48** · iOS · `f01cce5` — The pointer over the text is an I-beam; it stayed
  the system's default on an iPad's trackpad and on a Mac.
- **49** · Catalyst · `5737fe9` — The pointer's drag selects and a selection
  has no handles; a click-drag dragged the history like a finger.
- **50** · iOS (+ Catalyst) · `6c83adf` — The view is no stop of the keyboard
  focus, and on a Mac Tab and Shift-Tab come to it first; Tab moved the
  focus off the terminal, Shift-Tab never reached the remote.
- **51** · Catalyst · `199dbe2` — Escape reaches the terminal; the text system
  took it as "stop editing" and the keyboard went with it.
- **52** · iOS (+ Catalyst) · `ccd3541` — A held key repeats at the Mac's
  timing, held letters repeat on a Mac, and a cancelled press stops
  repeating; the repeat was fixed at 0.4 s / 0.1 s and letters typed once.
- **53** · engine · `8d8fa23` — The alternate screen is as tall as the
  screen and keeps no history; made 25 rows tall before the terminal's own
  size applied, and left at its height by a resize while empty, it kept up
  to 25 − rows lines (after a resize, the difference) as history.
- **54** · engine · `2d2aeff` — A line scrolling into a full history is
  blank, single width, and marked as continuing the line before only if it
  does; it kept the oldest line's wrap mark and size, and half the lines of
  a long session copied joined to the line before.
- **55** · engine (+ iOS) · `d012520` — A selection that ends in a row's
  last column takes its last character: `setSelection` capped the end at
  `cols − 1`, a search match ending there ended on the next row (a line
  break in the copy; on the bottom row, the text before the match), and no
  iOS drag could reach it. A drag goes to the nearest edge between cells.
- **56** · iOS · `765fcbb` — A long press slides from its own word; with a
  selection still showing, it slid from where that one began.
- **57** · engine · `5cb1261` — A shorter history tells the selection, the
  synchronized-output snapshot and the view what went, and leaves the saved
  cursor and the scroll-invariant numbers alone.
- **58** · engine · `3baf850` — DECCOLM needs mode 40, off until a program
  turns it on, as in xterm; any remote could make the grid 132 or 80
  columns wide.
- **59** · engine · `2b86a49` — Autowrap and IND below the scrolling region
  move the cursor down; they scrolled the region, its top line lost.
- **60** · iOS · `4e948ff` — The text input answers inline predictions,
  math results and Writing Tools with no; left to the system's default,
  each was offered, and taken, typed into the remote.
- **61** · iOS · `754a22d` — A cancelled Command press is not held; links
  underlined and opened on a plain click until ⌘ went down and up again.
- **62** · iOS · `b873d6a` — A cancelled press is released for the kitty
  keyboard protocol; with its event reports on, a program heard a key held
  while ⌘-Tab took the app away go down and never up.
- **63** · engine · `3fa2ffa` — ED 3 (`clear`) keeps the scroll-invariant
  numbers of the lines it leaves; they went back to 0, and a number read
  before `clear` found another line after it.
- **64** · engine · `1d0f672` — `TerminalOptions.hostOwnsColumns`: a host
  whose grid follows its view keeps it; DECCOLM does nothing even after a
  program turns mode 40 on. Off by default (xterm's way); Moshpit turns it
  on.
- **65** · engine · `a90d081` — Lines trimmed from the top (ED 3, a shorter
  history, fewer rows) no longer count as holding images, and are let go;
  the count stayed up, and every scroll did the kitty placement pass and
  every iOS repaint covered the whole view.
- **66** · engine · `7e5b936` — screen's title sequence (`ESC k … ST`) is
  consumed; a shell in tmux sends it for every command (oh-my-zsh), a tmux
  -CC client gets it raw, and every title was printed into the pane.

---

## 1. OSC-8 hyperlink underline overshoot by one column

**Touches:** `Sources/SwiftTerm/Terminal.swift` (the `oscHyperlink` close
handler), plus a regression test in `Tests/SwiftTermTests/OscTests.swift`.

**Why:** When a shell emits an OSC-8 hyperlink and then closes it, SwiftTerm
marked the range of cells the link covers so it can draw the link underline.
The close handler computed the end of that range one column too far, so the
underline drawn under a hyperlink overshot the actual link text by a single
cell at the end of the line.

**Recommendation:** Upstream-mergeable as-is. It is a self-contained
off-by-one correction in the emulator's own OSC handler and ships with a
regression test, so it should apply cleanly against upstream.

---

## 2. OSC-8 hyperlink underline overshoot across the whole line / scrollback

**Touches:** `Sources/SwiftTerm/Terminal.swift` (the `oscHyperlink` close
handler), plus a regression test in `Tests/SwiftTermTests/OscTests.swift`.

**Why:** A second, larger variant of the same close-handler bug. In some cases
the computed link range did not stop at the end of the link text at all and ran
on across the rest of the line and into scrollback, so the underline was
painted over a large span of unrelated content rather than just the hyperlink.

**Recommendation:** Upstream-mergeable as-is. Like patch 1 it is a bounded
correction to the emulator's OSC handling with an accompanying test.

---

## 3. Public API to linkify host-detected plain-text URLs

**Touches:** `Sources/SwiftTerm/Terminal.swift` (adds public API
`viewportLineText` and `tagPlainTextLink`), plus tests in
`Tests/SwiftTermTests/LinkLookupTests.swift`.

**Why:** SwiftTerm only treats a run of cells as a clickable link when the
program explicitly emits an OSC-8 hyperlink. It does ship a built-in
implicit-link regex for bare URLs, but that regex is too permissive — it also
matches bare file paths — so Moshpit cannot safely turn it on globally. Instead
Moshpit detects `http(s)` URLs in the visible text itself and wants to mark just
those as links. This patch adds two public entry points: `viewportLineText`
lets the host read back the text of a viewport line so it can run its own URL
detection, and `tagPlainTextLink` lets the host tag a detected span as a link
that reuses the exact same `cell.hasPayload` code path a real OSC-8 hyperlink
would use — so host-detected links behave identically to emulator-native ones
(hit-testing, underline, opening) without a parallel implementation.

**Recommendation:** Would need generalizing first. The underlying mechanism
(let a host contribute link payloads without emitting OSC-8) is broadly useful,
but the API surface here was shaped around Moshpit's specific need. Upstreaming
it would warrant a review of the method names and signatures and a decision on
how it should relate to the existing built-in implicit-link support before it
becomes a permanent part of SwiftTerm's public API.

---

## 4. Check link hit-test before treating a tap as "just focus me"

**Touches:** `Sources/SwiftTerm/iOS/iOSTerminalView.swift` (the `singleTap`
handler).

**Why:** On iOS, tapping the terminal while it is not the first responder was
treated purely as "give me focus / raise the keyboard," and the tap was
otherwise consumed. If the keyboard had been dismissed and the user tapped a
link, that first tap only refocused the view and silently ate the link
activation — the user had to tap the link a second time for it to open. The fix
makes `singleTap` hit-test for a link *before* falling back to the
focus-only behavior, so a tap on a link opens it even when the view was not yet
focused.

**Recommendation:** Upstream-mergeable as-is. It is a general iOS interaction
correctness fix — any host that renders links benefits — and it is localized to
the tap handler. It may attract a review discussion about tap semantics, but it
is not Moshpit-specific.

---

## 5. Render IME composition text as an underlined overlay at the cursor

**Touches:** `Sources/SwiftTerm/iOS/iOSTextInput.swift` (`setMarkedText` /
`unmarkText`) and `Sources/SwiftTerm/iOS/iOSTerminalView.swift`.

**Why:** For input methods that compose text before committing it (Pinyin and
other phonetic IMEs), UIKit hands the in-progress "marked" text to the view via
`setMarkedText`. SwiftTerm tracked that marked text internally but never drew it
anywhere, so while composing, the user saw nothing on screen at the cursor. This
patch paints an underlined overlay of the composing text at the cursor position
and clears it on `unmarkText`, giving normal visual feedback during composition.

**Recommendation:** Upstream-mergeable as-is (with review). Rendering marked
text is standard, expected behavior for any iOS text input and benefits every
CJK user of SwiftTerm, not just Moshpit. Because it introduces a rendering path
it deserves a visual review upstream, but it is not a Moshpit-specific behavior.

---

## 6. Move the real cursor to the end of the composing text

**Touches:** `Sources/SwiftTerm/iOS/iOSTerminalView.swift` and
`Sources/SwiftTerm/iOS/iOSTextInput.swift` (delivered across the composition
commits, including the follow-up that pins the caret to the end of the marked
string).

**Why:** With the composition overlay from patch 5 in place, the terminal's own
real cursor (`caretView`) needed to track the growing composition instead of
drawing a second, separate caret. The naive approach — tracking a
mid-composition insertion offset — jittered back to the front of the string,
because phonetic IMEs replace the entire marked string on every keystroke and
candidate paging can report a stale `selectedRange`. The fix moves the real
caret to the end of the composing text as it grows rather than trusting the
reported selection offset, so the cursor stays put at the end of what the user
is typing.

**Recommendation:** Upstream-mergeable as-is (with review). This is a general
correctness fix for IME composition on iOS and pairs directly with patch 5. It
is behavior any host would want.

---

## 7. `deleteBackward()` during active composition edits the marked text

**Touches:** `Sources/SwiftTerm/iOS/iOSTerminalView.swift` and
`Sources/SwiftTerm/iOS/iOSTextInput.swift`.

**Why:** While an IME composition was active, `deleteBackward()` sent a real
(and wrong) backspace keypress to the terminal, nuking the whole composition in
one shot. Under hold-to-repeat delete on the Pinyin keyboard this was
especially broken. The fix makes `deleteBackward()` during active composition
remove a single character from the end of the marked text and send nothing to
the terminal, matching how a text field behaves while composing.

**Recommendation:** Upstream-mergeable as-is (with review). Like patches 5–6 it
is a general iOS IME correctness fix, not something specific to Moshpit.

---

## 8. Expose `isRowWrapped(_:)` so hosts can join soft-wrapped rows exactly

**Touches:** `Sources/SwiftTerm/Terminal.swift` (adds public
`isRowWrapped(_:)`).

**Why:** When a URL wraps across two terminal rows because of a soft wrap,
Moshpit needs to join those rows back into one string to open the full link.
Previously it had to guess whether a row was a soft-wrap continuation by
comparing the row's text length against its column width — which broke whenever
a CJK or emoji character earlier on the row threw off the
character-count-vs-column-count comparison, silently truncating the opened link.
The emulator already tracks a soft-wrap bit per row internally; this patch adds
a small public accessor, `isRowWrapped(_:)`, that exposes it, so the host can
ask the emulator directly instead of guessing.

**Recommendation:** Upstream-mergeable as-is. It is a small, clean read-only
accessor that surfaces state SwiftTerm already maintains, with no behavior
change to the emulator itself — a low-risk addition to the public API.

---

## 9. Composition end no longer snaps the caret to a stale buffer position

**Touches:** `Sources/SwiftTerm/iOS/iOSTextInput.swift`.

**Why:** When an IME composition committed, the code snapped the caret to
`updateCursorPosition()`. At that instant the terminal buffer's cursor has not
moved yet — the committed text has only just been sent to the remote, not echoed
back — so the snap yanked the caret backwards to where the composition started,
right as you finished typing a word.

**Recommendation:** Upstream-mergeable as-is, alongside patches 5–7.

---

## 10. `UIKeyInput.hasText` always reports true

**Touches:** `Sources/SwiftTerm/iOS/iOSTerminalView.swift`.

**Why:** UIKit gates the delete key's hold-to-repeat on `hasText`: an input view
that reports "no text" gets one `deleteBackward()` per press and the keyboard
never starts its repeat timer. `textInputStorage` is not the terminal's content
— it is the shadow copy of what has been typed here since the last Return, so it
is empty for everything the *remote* put on the line: a reattached session whose
prompt already holds a half-written command, program output, an echoed paste.
Answering `hasText` from it killed hold-to-delete in exactly the cases where you
most want it, while working normally for text typed on the same line, which
reads as the keyboard breaking at random. A terminal always has a screen behind
the cursor, and what a backspace does with it is the remote's decision.

**Recommendation:** Upstream-mergeable, but see patch 12 — on its own this fixes
only half of the problem.

---

## 11. `updateCursorPosition()` defers to the composition overlay

**Touches:** `Sources/SwiftTerm/Apple/AppleTerminalView.swift` and
`Sources/SwiftTerm/iOS/iOSTextInput.swift`.

**Why:** Patch 9 stopped the caret being yanked back by the *IME*; this stops it
being yanked back by *output*. A pane that keeps repainting — an agent status
line, a tmux resync, a reflow — calls `updateCursorPosition()` repeatedly, and
each call moved the caret from the end of the Pinyin you are composing back to
its front, mid-word. While a composition overlay is on screen, the overlay owns
the caret.

**Recommendation:** Upstream-mergeable as-is, alongside patches 5–7 and 9.

---

## 12. One phantom character in front of the caret keeps delete repeating

**Touches:** `Sources/SwiftTerm/iOS/iOSTerminalView.swift` and
`Sources/SwiftTerm/iOS/iOSTextInput.swift`.

**Why:** Patch 10 was necessary but not sufficient, and the bug it was meant to
fix survived it. UIKit does not consult `hasText` on every repeat tick of the
delete key: before each tick it *measures the document* — the position one
character back from the caret — and stops the moment that range comes back
empty. `position(from:offset:)` clamps to `[0, textInputStorage.count]`, so with
an empty shadow buffer the keyboard asks for one character before the caret,
gets the caret itself back, reads the range as empty, and concludes there is
nothing left to delete.

That is the same shadow buffer as patch 10, empty for the same reasons, so the
symptom was the same: holding delete over a paste or over a history entry
recalled with the up arrow deleted exactly one character. Measured in the
Simulator over 20 pasted characters, holding delete for three seconds sent
**one** backspace byte and deleted **one** character; over ten characters typed
on the same line it sent ten and then went dead the instant the buffer drained.

So the document now always has one character in front of the caret. A newline,
for two reasons: it is a hard word boundary, so the keyboard's accelerated
delete-by-word mode can never select a range that runs from the phantom into
real typed text; and it reads as "start of line" to the prediction / candidate
bar, which is where the caret conceptually is when nothing has been typed yet.
One character is deliberately all the patch keeps — a longer run would give
delete-by-word something to chew through, and every character of it would bill
the remote for a backspace it never received. Deleting the phantom is
indistinguishable from the old empty-buffer path: one `deleteBackward()`, one
backspace byte. Ranged deletes bill only the non-phantom characters in the range
(never zero, or the hold goes dead again). The same three-second hold over 20
pasted characters afterwards sends 25 bytes and clears all 20.

**Recommendation:** Upstream-mergeable together with patch 10 — they are two
halves of one fix, and 10 alone gives a misleading impression of being complete.

---

## 13. A tap on a link opens it without grabbing focus

**Touches:** `Sources/SwiftTerm/iOS/iOSTerminalView.swift` (`singleTap`).

**Why:** Patch 4 made an unfocused tap check for a link before treating the
tap as "focus me" — but it still focused afterwards, so tapping a link with
the keyboard dismissed popped the keyboard over the output being read and
left it up on return from the browser. The unfocused link-tap now opens the
link and deliberately does NOT become first responder.

**Upstream candidate:** yes, same UX argument as patch 4.

## 14. OSC 52 read queries reach the host

**Touches:** `Sources/SwiftTerm/Terminal.swift`, delegate surface.

**Why:** `ESC ] 52 ; c ; ?` (clipboard read) was mis-parsed as base64 content
and dropped. It now surfaces through a `clipboardRequest` delegate hook and
is answered via `Terminal.sendClipboardResponse` — whether the remote gets
the clipboard is the host's call. Groundwork for clipboard-image paste over
SSH.

**Upstream candidate:** yes.

## 15. `focusOnTap` — the host decides whether a tap summons the keyboard

**Touches:** `Sources/SwiftTerm/iOS/iOSTerminalView.swift` (new public
`focusOnTap`, checked in `singleTap`'s unfocused branch).

**Why:** On iOS first responder IS the software keyboard, and the unfocused
branch of `singleTap` unconditionally grabbed it — so in an app whose
terminal is read as much as it is typed at, every tap taken while reading
history summoned a keyboard over the output ("没点输入框也弹键盘"). With
`focusOnTap = false` a tap is only ever a tap: links open (patch 13),
double-tap selection and mouse reporting are untouched, and focus moves only
when the host calls `becomeFirstResponder()` itself — Moshpit's shortcut-bar
keyboard toggle and its raise-keyboard-on-open setting. Default stays `true`
so the fork changes nothing for other consumers.

**Upstream candidate:** yes — as an opt-in flag it costs upstream nothing.

## 16. Selection that works without first responder

**Touches:** `Sources/SwiftTerm/iOS/iOSTerminalView.swift`
(`panSelectionHandler`, `showContextMenu`/`hideContextMenu`, a
`UIEditMenuInteractionDelegate` extension).

**Why:** Two assumptions in the selection flow broke once patch 15 made
"unfocused" the terminal's normal state. The handle-grab window was 3×2
cells — ~16pt under a 9pt font, a third of the 44pt HIG minimum — so
dragging a handle usually missed and re-anchored the selection from the old
pivot. It is now derived from the cell size to stay finger-sized (22pt),
exposed as `selectionHandleTolerance(cellWidth:cellHeight:)` for host tests.
And the copy menu was `UIMenuController`, which refuses to show unless the
view is first responder — double-tap selected a word, then no menu ever
appeared. iOS 16+ uses `UIEditMenuInteraction` (no responder requirement,
same `canPerformAction` source of truth); pre-16 keeps the old path.

**Upstream candidate:** yes, both halves.

## 17. Long-press selects, instead of summoning the keyboard

**Touches:** `Sources/SwiftTerm/iOS/iOSTerminalView.swift` (`longPress`).

**Why:** The old handler called `becomeFirstResponder()` (on iOS, the
software keyboard) and showed a bare paste menu with no selection — the
opposite of what a long press means everywhere else text appears on the
platform. It now selects the word/expression under the finger, extends if
the finger slides before lifting, and shows the edit menu on release, where
paste still lives. No first responder involved — the fourth member of the
"assumed focus" family (menu, handles, deselection came first).

**Upstream candidate:** yes.

## 18. Overwriting half of a wide character clears the other half

**Touches:** `Sources/SwiftTerm/Buffer.swift` (`insertCharacter` and the
`insertAsciiRun` fast path), `Sources/SwiftTerm/Apple/AppleTerminalView.swift`
(`buildAttributedString`), tests in `WideCharHygieneTests.swift`. Four
commits: `740e797`, then `b1db6c8`, `aa3a70a` and `29949bc` for the fill.

**Why:** The read side always fixed up a split fullwidth character; the
write side never did. A narrow character written over a lead left its
width-0 continuation behind, and a write onto a continuation left half a
lead. The orphan renders as a bright one-cell block, and under a peer that
sends only the cells it believes changed (mosh's framebuffer, a TUI over
tmux) nothing ever repaints it: the permanent white blocks after a pane
switch over mosh. The cleared half takes erase semantics, as ECH/EL do:
default foreground and styles, the writer's background, and
`defaultInvertedColor` mapped back to the default. A default fill punched a
hole in a coloured region the application never repaints; the writer's full
attribute leaked inverse video, which is the bright block again. The renderer
also draws any width-0 cell it reaches as a default blank (a legitimate one is
skipped by its lead's width).

**Upstream candidate:** yes, with its tests.

## 19. `discardPendingInput()` forgets a dead stream's unfinished input

**Touches:** `Sources/SwiftTerm/Terminal.swift`.

**Why:** A host that keeps one `Terminal` across reconnects inherited the dead
stream's parse state, a partial UTF-8 sequence or a parser stuck inside a
CSI/OSC/DCS, so the new stream's first bytes were read as its continuation (a
dangling OSC swallows everything up to the next terminator). This resets the
parser and the UTF-8 putback and leaves the screen, scrollback, modes and
cursor alone, unlike `softReset`. Moshpit calls it at every reconnect, before
the new stream feeds.

**Upstream candidate:** yes.

## 20. `closeSelection()` — a host-callable way out of a selection

**Touches:** `Sources/SwiftTerm/iOS/iOSTerminalView.swift`.

**Why:** Tap-to-clear lived only in `singleTap`'s focused branch, so with
`focusOnTap = false` (patch 15) the highlight, the drag gesture and the menu
stayed until something selected over them. Moshpit's own tap handler calls
this.

**Upstream candidate:** yes, alongside patch 15.

## 21. The scroll offset follows `yDisp`; hosts can scroll by pixels

**Touches:** `Sources/SwiftTerm/iOS/iOSTerminalView.swift`. Three commits:
`7a6d6db`, `fc38113`, `68da3aa`.

**Why:** Upstream draws its rows from `contentOffset` (9ad1b19) but pinned the
offset to the last screen on every `scrollTo`, so `scrollUp`/`scrollDown`
moved `yDisp` while not a pixel changed. The offset now follows `yDisp`, and
new public `cellHeight`, `distanceToBottom` and `scrollViewport(by:)` let a
host drive a smooth finger scroll over the scrollback. `yDisp` is synced only
where a host moves the offset on purpose (mirroring every offset change wrote
UIKit's transient rewrites back and snapped a pane to row 0 mid-burst), and
host offsets snap to the device pixel grid (fractional offsets made text
shimmer under the finger on a real device).

**Upstream candidate:** yes for the offset fix; the pixel API with review.

## 22. A hyperlink wrapped over rows selects and copies as one link

**Touches:** `Sources/SwiftTerm/Terminal.swift` (`explicitLinkSpan`),
`Sources/SwiftTerm/SelectionService.swift`, tests in `SelectionTests.swift`.
Two commits: `577828e`, `b6042c4`.

**Why:** Programs that lay out their own text (Claude Code's transcript)
print a long OSC-8 link as several rows, re-opening it after an indent. A
long-press selected a fragment, and a dragged selection copied the URL with a
newline and the indent inside it. The link span now follows the link onto
neighbouring rows, long-press selects all of it, and a selection inside it
copies without the row breaks when the text is part of the URL (a wrapped
prose label keeps its rows). Rows are matched by URL, not by payload: tmux's
`capture-pane -e` re-emits a link with an id of its own. A drag that starts
on the indent still counts as inside the link.

**Upstream candidate:** yes.

## 23. A keyboard's own edits are not reported back to it

**Touches:** `Sources/SwiftTerm/iOS/iOSTerminalView.swift`,
`Sources/SwiftTerm/iOS/iOSTextInput.swift`.

**Why:** UIKit deletes a character for a third-party keyboard by selecting two,
deleting both and inserting the first again, and every step went back to the
keyboard through `inputDelegate` as if the user had edited behind its back;
豆包's voice input ended its session there and dictation stopped mid-sentence.
The delegate now hears only about changes the input system did not make. When
such a delete reached the front of the buffer, UIKit put the phantom newline
(patch 12) back with `insertText("\n")`, which pressed Return on the remote;
that re-insert now only restores the phantom locally.

**Upstream candidate:** yes, together with patch 12.

## 24–28. Partial repaints on iOS

**Touches:** `Sources/SwiftTerm/Apple/AppleTerminalView.swift`,
`Sources/SwiftTerm/iOS/iOSTerminalView.swift` (new `TerminalViewLayer`),
`Sources/SwiftTerm/Terminal.swift` (ECH, REP), the Metal renderer, tests in
`UpdateRangeTests.swift`. Five commits, `25a06eb`…`963ec1e`, on branch
`fix/ios-dirty-rect`.

**Why:** `updateDisplay` on iOS invalidated the whole view on every update
(macOS already invalidated only the changed rows). On an iPhone 16 Pro with
Moshpit's 9 pt font and a 70×72 grid every echoed key cost a full-screen draw
of about 5 ms on the main thread. Now the rows in the terminal's update range
are invalidated, widened by the glyph overflow reach; a full repaint remains
for scrolling, clears, palette changes, the alternate screen, resizes, a
viewport scrolled into history, images on screen, selection changes and
lifecycle events. Partial repaints require the update range to be complete:
ECH and REP did not record their rows (25), which `UpdateRangeTests` now
guards with seeded random output.

**Verification:** an on-device check renders each step with partial
invalidation and compares the layer's pixels byte for byte with a forced full
repaint, over typing, cursor moves, colour, CJK / emoji / combining marks,
full-screen programs, scrolling, selection, IME marked text, OSC 8 links,
pixel scrolling, font and theme changes, resizes and seeded fuzz: 531
comparisons, all identical, with two controls that must and do differ, on
iOS 27.3 (iPhone 16 Pro) and iOS 18.2 (iPad mini 6). `swift test` passes.
Metal was checked by hand but not pixel-proven (its captures are often a
previous frame).

**Upstream candidate:** yes for 24, 25 and 27 with 26; 28 with review.

## 29. Erasing the saved lines tells the view where the viewport went

**Touches:** `Sources/SwiftTerm/Terminal.swift` (`cmdEraseInDisplay`), tests in
`UpdateRangeTests.swift` and `ClearScrollbackViewTests.swift`. Commit
`232e37e`, branch `fix/terminal-round14`.

**Why:** ED 3 (`CSI 3 J`) ends `clear` in a shell (ncurses' E3), Claude Code's
`/clear`, and Moshpit's own herdr pane switch. It trimmed the history and
moved `yBase` and `yDisp` without a word to the view. The iOS view draws its
rows from the scroll offset, which follows `yDisp` only when told (patch 21),
so the offset stayed on the old row, past the end of the trimmed buffer, and
every draw was blank until something scrolled or laid the view out. The
trim now marks the whole screen for repaint and announces the new `yDisp`
through `scrolled(source:yDisp:)`. `UpdateRangeTests`' seeded check now holds
every chunk that moves the viewport to announcing where it went. The iOS
view's offset after a clear is checked in Moshpit's `TerminalViewForkTests`
(iOS only, so not in the fork's `swift test`).

**Upstream candidate:** yes.

## 30. Changing `useBrightColors` takes effect at run time

**Touches:** `Sources/SwiftTerm/iOS/iOSTerminalView.swift`,
`Sources/SwiftTerm/Mac/MacTerminalView.swift`, tests in
`BrightColorsTests.swift`. Commit `1021332`.

**Why:** The view resolves each cell attribute once and caches it, and
`useBrightColors` is read only while resolving, so a change on a live view
showed nowhere. A change now drops the caches and repaints every row
(`colorsChanged`); setting the same value does nothing. Moshpit does not set
it today.

**Upstream candidate:** yes.

## 31. A host can take over the terminal's own paste

**Touches:** `Sources/SwiftTerm/Apple/TerminalViewDelegate.swift`,
`Sources/SwiftTerm/iOS/iOSTerminalView.swift`,
`Sources/SwiftTerm/Mac/MacTerminalView.swift`, tests in `PasteHookTests.swift`.
Commit `adecbab`.

**Why:** The edit menu's Paste and ⌘V on a hardware keyboard bracketed the
paste by the local emulator's `bracketedPasteMode` and sent it straight out.
Over SSH+tmux (a `-CC` attach does not replay the pane's modes) and SSH+herdr
(its frames carry none) that flag is never set, so a multi-line paste ran
line by line; and iOS sent the start marker, the text and the end marker as
three writes, which herdr cannot read as one paste. New
`TerminalViewDelegate.pasteRequested(source:text:)`: return true to deliver the
paste yourself; the default returns false and the view sends it as before —
on iOS now in one write. `paste(text:)` is the same path without the
pasteboard. Moshpit routes it to `ActiveSession.sendPaste` (tmux
`paste-buffer -p`, herdr one complete bracketed paste, mosh and plain SSH the
local flag).

**Upstream candidate:** yes (the hook and the one-write paste).

## 32. A full reset tells the view where the viewport went

**Touches:** `Sources/SwiftTerm/Terminal.swift` (`resetToInitialState`), tests
in `UpdateRangeTests.swift` and `ClearScrollbackViewTests.swift`. Commit
`b9a651b`, branch `fix/terminal-round17`.

**Why:** RIS (`ESC c`, what `reset` and `tput reset` send) and DECCOLM
(`CSI ? 3 h` / `l`, which resizes and then resets) swap in a fresh normal
buffer — no history, the viewport on its first row — without a word to the
view: patch 29's blank screen by another road. The iOS offset stayed on a row
past the end of the new buffer, every draw came out blank until something
scrolled, and the content size kept room for the history that was gone. The
reset now announces the new `yDisp` through `scrolled(source:yDisp:)`, as ED 3
does; it already marked the whole screen for repaint. The iOS view's offset
after a reset is checked in Moshpit's `TerminalViewForkTests`.

**Upstream candidate:** yes.

## 33. A resize makes no lines for history nobody wrote

**Touches:** `Sources/SwiftTerm/Buffer.swift` (`resize`),
`Sources/SwiftTerm/CircularList.swift` (`forEachAllocated`), tests in
`ScrollbackAllocationTests.swift`. Commit `57485df`, branch
`fix/terminal-round17`.

**Why:** The line list keeps a slot per line of the scrollback and makes a
line the first time a slot is read. `Buffer.resize` read every slot up to
`maxLength` — to widen lines, to trim them, and in its post-condition check —
so the first resize of any kind (the keyboard coming up is one) made the whole
history at once. With the 50 000 lines Moshpit asks for, real since
`TerminalScrollback.enlarge` calls `changeScrollback` (B94; it used to stay at
500), that was 50 030 lines of 80 cells, about 96 MB, per pane. The three loops
now visit only the lines that exist; an empty slot still gets its line on
first use, at the width the buffer has then. In a debug build on a Mac: lines
made by a rows-only resize 50 030 → 101, 46 → 13 ms; a wider resize
267 → 7 ms. A full history still costs what it holds: 24 bytes a cell, about
96 MB for 50 000 lines of 80 columns.

**Upstream candidate:** yes.

## 34. A full reset no longer keeps the terminal alive

**Touches:** `Sources/SwiftTerm/Terminal.swift` (`resetNormalBuffer`), a test
in `Memory.swift`. Commit `a6dc313`, branch `fix/terminal-round21`.

**Why:** Every full reset — RIS (`ESC c`, what `reset` sends), DECCOLM either
way, the iOS view's Reset menu item — makes a fresh normal buffer, and
`resetNormalBuffer` gave it the bound method `scroll(isWrapped:)` as its
scroll callback, which holds the terminal strongly while the terminal holds
the buffer. After one reset the terminal, both buffers and the whole
history (about 19 MB for 10 000 lines of 80 columns) were never freed. The
initializer already installed a weak callback; the reset now installs the
same one. B97.

**Upstream candidate:** already fixed upstream, the same line, in commit
`1da8e02`, released in v1.14.0: the patch goes when the fork moves past
v1.13.0.

## 35. A full reset puts the cursor at home

**Touches:** `Sources/SwiftTerm/Terminal.swift` (`setup(isReset:)`), tests in
`FullResetTests.swift`. Commit `c932e4d`.

**Why:** RIS switched to its fresh normal buffer through
`activateNormalBuffer`, the path for leaving the alternate screen, which
copies the alternate screen's cursor. xterm's reset (`ReallyReset` in
charproc.c) leaves the alternate screen, clearing it first if it shows,
clears the screen and puts the cursor at home; tmux 3.6a also ends a reset
at 0,0. So after a program that used mode 47 (no clear on the way out) or
one that died on the alternate screen, `reset` put the prompt where the
program had left its cursor, and in the second case the program's screen
came back the next time anything switched to the alternate screen. The
reset now switches to the fresh buffer directly, cursor at 0,0, and clears
the alternate screen if it was showing. DECCOLM resets the same way and
ends at home, as xterm's does. B99.

**Upstream candidate:** yes.

## 36. ED 1 unwraps the row under the cursor, not a line of history

**Touches:** `Sources/SwiftTerm/Terminal.swift` (`cmdEraseInDisplay`), tests in
`ScreenTests.swift`. Commit `4f01624`.

**Why:** Erase Above (`CSI 1 J`) from the last column erases the cursor's
whole row, so the row under it no longer continues it and loses its wrap
flag. The code indexed `buffer.lines [j + 1]` with `j` the cursor's screen
row (inherited from xterm.js): with history that is a line of it, so a
wrapped line in the history copied and reflowed as two, while the row
under the erased one kept its flag; from the bottom row it hit the buffer
line numbered `rows`. The line is now `yBase + j + 1`, and only when that
row is on the screen. B78.

**Upstream candidate:** yes; xterm.js has the same index.

## 37. Erasing the saved lines moves the selection with its text

**Touches:** `Sources/SwiftTerm/Terminal.swift` (new
`TerminalDelegate.linesDropped(source:count:)`, `cmdEraseInDisplay`),
`Sources/SwiftTerm/SelectionService.swift`, both views, tests in
`ClearScrollbackViewTests.swift` and `SelectionTests.swift`. Commit `5a34360`.

**Why:** A selection is kept as buffer rows. ED 3 (`CSI 3 J`: the end of
`clear`, Claude Code's `/clear`) drops every line above the screen, so each
line left moves up by as many rows; the selection kept its row numbers, and
its highlight and its copy landed on other text, or on rows past the end of
the buffer, which copy as nothing. Moshpit keeps a selection while output
arrives (mouse reporting off), so this showed whenever an erase came in
under one. ED 3 now tells its delegate how many lines it dropped; the views
pass that to the selection, which moves up with its text, or goes if any of
its rows were dropped, since its text is no longer all there (Alacritty
does the same on ED 3). The default implementation of the new delegate
method does nothing. The iOS view's selection after a clear is checked in
Moshpit's `TerminalViewForkTests`. B79.

**Upstream candidate:** yes.

## 38. A full reset the remote sends drops the selection

**Touches:** `Sources/SwiftTerm/Terminal.swift` (new
`TerminalDelegate.terminalReset(source:)`, `resetToInitialState`), both
views, tests in `ClearScrollbackViewTests.swift` and `UpdateRangeTests.swift`.
Commit `5fb1775`.

**Why:** The iOS edit menu's Reset dropped the selection after resetting; a
reset that arrived as output (`reset` in the shell, DECCOLM) did not, and
the selection pointed into the buffer the reset had thrown away: a
highlight over blank rows, then over whatever came next, with Copy still
offered. `resetToInitialState` now tells its delegate last, after the
`scrolled` of patch 32, and both views drop the selection there (iOS also
its drag gesture, as Reset does). Alacritty's reset drops its selection
too. Checked on the iOS view in `TerminalViewForkTests`. B100.

**Upstream candidate:** yes, with 37.

## 39. Select All ends on the last line with content

**Touches:** `Sources/SwiftTerm/SelectionService.swift` (`selectAll`, patch 22's
link stitching), tests in `SelectionTests.swift`. Commit `d66ca96`.

**Why:** Select All ended on the last line the scrollback could hold, not the
last line written. The copy clamps its text to the lines that exist, but
patch 22's link stitching listed a position for every selected cell and
read each one, and reading a slot nobody wrote makes its line: with
Moshpit's 10 000-line history every Select All + Copy made every line of
it, and they stayed. It now ends on the last line with anything on it, at
the column past the last (the end is exclusive: a last line that filled the
row lost its last cell in the copy), and the stitching walks the cells
instead of listing them. Release build on a Mac, 80×50, 10 000-line
history, Select All + Copy, the same text copied: with 1 000 lines of
output 26–47 ms and +55 MB of footprint (9 049 lines made) → 4 ms and
+0.6 MB; with 100 lines 29–80 ms and +57 MB → 0.5 ms and +0.1 MB; with the
history full 61–78 ms and +41 MB → 45–48 ms and +4.8 MB. B98.

**Upstream candidate:** yes.

## 40. Autowrap above the bottom row marks the row it runs on to

**Touches:** `Sources/SwiftTerm/Buffer.swift` (`insertAsciiRun`,
`insertCharacter`), tests in `ScreenTests.swift`. Commit `7414775`, branch
`fix/terminal-round24`.

**Why:** Text that runs past the last column above the bottom row goes on in
the row under it, which is marked as continuing it. Both write paths set the
mark on `buffer.lines [_y]`, `_y` the cursor's screen row (upstream has the
same index, inherited from xterm.js): with history above the screen that is a
line of the history, which then copied run together with the line before it,
while the row that really continues the text stayed unmarked, so the long
line copied as two and a resize could not join it back. After Ctrl-L, and in
any full-screen program, every wrap is above the bottom row. The line is now
`_y + _yBase`, the class of patch 36. B177.

**Upstream candidate:** yes; xterm.js has the same index.

## 41. A line scrolling into a full history moves the selection with its text

**Touches:** `Sources/SwiftTerm/Terminal.swift` (`scroll`), comments in
`SelectionService.swift` and both views, tests in `SelectionTests.swift` and
`ClearScrollbackViewTests.swift`. Commit `d379073`.

**Why:** A selection is kept in buffer rows. Once the history is full every
line that scrolls in pushes the oldest one out and every line left sits a row
earlier; the selection kept its rows, so with each line of output it slid a
line down the text, highlight and copy. Moshpit keeps the selection while
output arrives and asks for 10 000 lines, which a long session fills.
`scroll` now reports the dropped line through patch 37's `linesDropped`, and
the views move the selection, or drop it when its text went. The alternate
screen keeps no history and drops its top row the same way, as xterm.js does.
Checked on the iOS view in Moshpit's `TerminalViewForkTests`. B180.

**Upstream candidate:** yes, with 37.

## 42. Switching screens drops the selection

**Touches:** `Sources/SwiftTerm/Terminal.swift` (the 47/1047/1049 mode
handlers, `activateNormalBuffer`, `activateAltBuffer`), both views'
`bufferActivated`, tests in `SelectionTests.swift` and
`ClearScrollbackViewTests.swift`. Commit `7c8ca8d`.

**Why:** A program that switches to the alternate screen (less, vim, a
full-screen agent) puts other text where the selection was; the selection
kept its rows and Copy took the other screen's text. xterm.js clears the
selection when the active buffer changes; both views now drop it in
`bufferActivated`, which the terminal calls only when the screen really
changed (it was also called for a 1049 h on the alternate screen). Checked on
the iOS view in `TerminalViewForkTests`. B181.

**Upstream candidate:** yes.

## 43. DECCOLM clears the screen and changes the columns

**Touches:** `Sources/SwiftTerm/Terminal.swift` (new `setColumnMode`, the
mode 3 handlers), both views' `sizeChanged(source:)`, tests in
`ColumnModeTests.swift`, `UpdateRangeTests.swift`,
`ClearScrollbackViewTests.swift`, `Memory.swift`. Commit `5685a27`.

**Why:** DECCOLM (`CSI ? 3 h` / `l`) resized and then did a full reset: the
history went, every mode went back to its default (application cursor keys,
bracketed paste, mouse reporting, autowrap, insert) and a program on the
alternate screen was put back on the normal one. xterm (charproc.c,
`srm_DECCOLM` and `set_column_mode`) changes the columns, clears the screen,
makes its lines single width, resets the margins and homes the cursor, and
leaves the rest alone, whether or not the width changes. It does that now
(mode 40 still gates it), tells the view where the viewport went, and the
views drop the selection when the terminal changes its own size, as they do
when they resize. B182.

**Upstream candidate:** yes.

## 44. A triple-click selects the whole row

**Touches:** `Sources/SwiftTerm/SelectionService.swift` (`select(row:)`, the
row mode of `shiftExtend(row:col:)`), tests in `SelectionTests.swift`.
Commit `9492769`.

**Why:** A selection's end is exclusive; the row selection a triple-click
(tap) makes ended at the last column, so the copy lost the row's last cell,
as Select All did before patch 39, and a shift-click extending it forwards
did the same. Both end at `cols`. B183.

**Upstream candidate:** yes.

## 45. A synchronized update copies the screen, not the history

**Touches:** `Sources/SwiftTerm/Terminal.swift` (`snapshotBuffer`, a new
`linesWereDropped` for `scroll` and ED 3), `Sources/SwiftTerm/Buffer.swift`
(new `adoptSnapshotLines`), tests in `SynchronizedOutputTests.swift`. Commit
`409b40d`.

**Why:** While synchronized output (mode 2026) holds an update back, the view
draws a snapshot of the buffer as it was when the update began. The begin
copied every line of the buffer, history included, and the end freed them;
Claude Code wraps every frame in an update. The snapshot is now numbered like
the buffer but copies only the screen and the rows on display; any other row
is copied from the buffer when something reads it (the view scrolled, a copy,
a search), counting the lines dropped from the top since the begin (a row
dropped during the update reads blank until it ends). Release build on a Mac
(M2), a frame that rewrites the screen, begin + frame + end, history 0 /
1 000 / 10 000 lines: 80×24 0.10 / 0.36–0.46 / 3.1–4.3 ms → 0.09 ms flat;
120×50 0.15 / 0.45–0.70 / 4.6–6.6 ms → 0.13–0.14 ms; held from begin to end
at 10 000 lines 20.9 MB → 129 KB (80×24) and 31 MB → 231 KB (120×50). B179.

**Upstream candidate:** yes.

## 46. A view its host lets go is freed

**Touches:** `Sources/SwiftTerm/iOS/iOSTerminalView.swift`
(`setupDisplayUpdates`, a `deinit`), tests in the fork's
`iOSTerminalViewTests.swift` (Mac Catalyst). Commit `122d2ac`.

**Why:** The view made its `CADisplayLink` with itself as the target; a
display link holds its target and the run loop holds the link until it is
invalidated, which only `updateUiClosed()` did, and Moshpit never called it.
Every view ever made lived on with its terminal and its whole history (about
19 MB at 10 000 lines): every closed pane. The link's target now holds the
view weakly and the view invalidates the link when it goes. Proved with weak
references on Mac Catalyst (the view and its terminal outlived their last
reference on the old code and are freed now) and, with a window, in Moshpit's
`TerminalViewForkTests`. B178.

**Upstream candidate:** yes.

## 47. The terminal's text input is nothing AutoFill fills

**Touches:** `Sources/SwiftTerm/iOS/iOSTerminalView.swift`
(`textContentType`, new `TerminalView.terminalContentType`), a test in
`iOSTerminalViewTests.swift`. Commit `ce7bb72`.

**Why:** `textContentType` was nil, "unknown", which leaves AutoFill to guess:
on a Mac it took the terminal for a place to put one-time codes and tried to
show the codes list at its caret (the system log's "Refusing to display OTP
completion list relative to null rect", spike G4). UIKit has no "none"
content type and every one it defines is something AutoFill fills; the view
names its own, "terminal", which no AutoFill source fills. Whether AutoFill
then stays away can only be seen with a code arriving. B176.

**Upstream candidate:** yes, with review of the value.

## 48. The pointer over the terminal is an I-beam

**Touches:** `Sources/SwiftTerm/iOS/iOSTerminalView.swift` (the pointer
interaction's delegate methods, now open), a test in
`iOSTerminalViewTests.swift`. Commit `f01cce5`.

**Why:** The view's pointer interaction answered `regionFor` with nil: over
the text the pointer stayed the system's default on an iPad with a trackpad
and on a Mac, and an interaction a host added further up was never asked. It
answers with the whole view and a vertical beam a line tall. Measured on a
Mac (a Catalyst test app, the real pointer moved through screen sharing):
NSCursor's current cursor is IBeamCursor, arrowCursor on the old code. B173.

**Upstream candidate:** yes.

## 49. On a Mac the pointer's drag selects

**Touches:** `Sources/SwiftTerm/iOS/iOSTerminalView.swift` (a pointer drag
recognizer, `enableSelectionPanGesture`),
`Sources/SwiftTerm/Apple/AppleTerminalView.swift` (no handles drawn), tests
in `iOSTerminalViewTests.swift`. Commit `5737fe9`, Mac Catalyst only.

**Why:** On a Mac a click-drag selected nothing: UIKit handed it to the scroll
view's pan, which dragged the history like a finger, and selecting meant a
long press and handles sized for a finger. The drag now selects from where
the button went down (a program that reports the mouse gets the drag), the
scroll view's pan takes no pointer drags (wheel and trackpad still scroll),
and there are no handles. Spike G4's patch, with the start taken from the
first touch; measured again in a Catalyst test app: a drag over "hello"
selects "hello", nothing on the old code.

**Upstream candidate:** yes, for Catalyst.

## 50. Tab and Shift-Tab stay in the terminal

**Touches:** `Sources/SwiftTerm/iOS/iOSTerminalView.swift`
(`canBecomeFocused`; on Mac Catalyst `keyCommands`, `insertTab(_:)`,
`insertBacktab(_:)`, `canPerformAction`), tests in
`iOSTerminalViewTests.swift`. Commit `6c83adf`.

**Why:** `canBecomeFocused` was always true, so the terminal was a stop of
UIKit's keyboard focus and Tab, sent to the remote, also moved the focus on,
which took the keyboard from the terminal (spike G4 on a Mac; an iPad with
keyboard navigation the same way). It is no stop now. On a Mac with the app
active the focus took Shift-Tab before the view saw it; the view's key
commands for Tab and Shift-Tab have priority over the system and send what
the presses send (Tab, CSI Z, or the kitty protocol's encoding). Measured in
a Catalyst test app next to a text field: Shift-Tab sent nothing on the old
code; now Tab sends 09 and Shift-Tab ESC [ Z and the terminal keeps the
keyboard. B175.

**Upstream candidate:** yes.

## 51. On a Mac, Escape reaches the terminal

**Touches:** `Sources/SwiftTerm/iOS/iOSTerminalView.swift` (on Mac Catalyst:
an Escape key command, `cancelOperation(_:)`), tests in
`iOSTerminalViewTests.swift`. Commit `199dbe2`.

**Why:** With the app active, the text system took Escape as "stop editing":
it resigned the terminal, so Escape never reached vim, less or an agent's
prompt and the keys after it went nowhere until the terminal was clicked
(spike G4). A key command with priority over the system sends what Escape's
press sends (ESC, or the kitty protocol's press and release). Measured in a
Catalyst test app: nothing sent and the keyboard lost on the old code; 1B
sent and the keyboard kept now.

**Upstream candidate:** yes.

## 52. A held key repeats as the platform does

**Touches:** `Sources/SwiftTerm/iOS/iOSTerminalView.swift`
(`keyRepeatTiming`, `startKeyRepeat`, `pressesCancelled`; on Mac Catalyst
`repeatHeldTextKey` and the press-and-hold setting), tests in
`iOSTerminalViewTests.swift`. Commit `ccd3541`.

**Why:** UIKit reports a held key's press once, so the view repeats the keys
it handles itself on a timer, fixed at 0.4 s then every 0.1 s. On a Mac it
now reads the Mac's keyboard settings at every press (NSEvent's
`keyRepeatDelay` / `keyRepeatInterval`, through the runtime); iPadOS gives an
app no access to its Key Repeat settings, so there it stays 0.4 s / 0.1 s. On
a Mac a held letter typed once (the system repeats the text of a held key
only for its own text views; it does repeat Backspace): the view repeats it
at the Mac's timing unless a composition is in progress, and turns the
press-and-hold accent picker off for its app (registration domain), as Mac
terminals do. A cancelled press stopped nothing and its key went on
repeating; it stops. Measured on a Mac (0.25 s, 30 a second), key events
posted to a Catalyst test app, each key held 1 s: an arrow 8 times on the
old code, 24 now; a letter 1 time, 24 now; no doubling with AppKit's
autorepeat key-downs posted too. B174.

**Upstream candidate:** the timing and the cancelled press, yes; the Mac's
letter repeat and the press-and-hold setting need upstream's view of a
library changing an app default.

## 53. The alternate screen is as tall as the screen

**Touches:** `Sources/SwiftTerm/Buffer.swift` (`resize`), tests in a new
`AlternateScreenTests.swift`. Commit `8d8fa23`, branch
`fix/terminal-round29`.

**Why:** xterm makes the alternate screen exactly as large as the display,
with no saved lines: a program on it that scrolls pushes its top row out.
The terminal makes its alternate screen 25 rows tall, the default size,
before its own size applies, and `resize` made the list of an empty buffer
no shorter (the shrink sat with the adjustments for a filled one, as in
xterm.js). On a screen of fewer than 25 rows, a phone's, the first
full-screen program kept up to 25 − rows lines as history the view could
scroll back into; and as the alternate screen is empty whenever the normal
one shows, any resize to fewer rows did the same (a keyboard coming up). An
empty list takes the new size now. Sixty lines printed on the alternate
screen: 3 rows kept 22 lines of history, 10 rows 15, 24 rows 1, a screen
gone from 40 rows to 12 kept 28; now none. B214.

**Upstream candidate:** yes; xterm.js's resize has the same shape.

## 54. A line scrolling into a full history starts blank and unwrapped

**Touches:** `Sources/SwiftTerm/BufferLine.swift` (`clear(with:isWrapped:)`),
`Sources/SwiftTerm/CircularList.swift` (`recycle`),
`Sources/SwiftTerm/Terminal.swift` (`scroll`), tests in `ScreenTests.swift`.
Commit `2d2aeff`.

**Why:** Once the history is full (Moshpit keeps 10 000 lines; the
alternate screen is always full) a line that scrolls in reuses the storage
of the oldest line. Its cells were cleared, but it kept that line's wrap
mark and its line size (DECDWL, DECDHL): after wrapped output, ordinary
lines came out marked as continuing the line before them and copied joined
to it (5 of 10 lines after a run of wrapped ones: "line 2line 3"), a long
line wrapping at the bottom row took an unmarked line and copied as two,
and a line that took a double-width line's storage was drawn double width.
xterm.js copies the new line in, mark and all
(`recycle().copyFrom(newLine)`); xterm clears both. The reused line is now
the blank line `getBlankLine` makes, with the mark the scroll asks for.
B213.

**Upstream candidate:** yes.

## 55. A selection that ends in a row's last column takes its last character

**Touches:** `Sources/SwiftTerm/SelectionService.swift` (`clamp`, which
`setSelection` uses), `Sources/SwiftTerm/SearchService.swift`
(`advancePosition`), `Sources/SwiftTerm/iOS/iOSTerminalView.swift` (new
`calculateSelectionHit`; the long press, the handles, the Mac pointer's
drag), tests in `SelectionTests.swift`, `SearchTests.swift` and
`iOSTerminalViewTests.swift`. Commit `d012520`.

**Why:** A selection's end is exclusive, so one that takes a row's last
cell ends at `cols`; three places capped or moved it, the cause of patches
39 and 44. `setSelection` capped the end at `cols − 1` ("56789" copied as
"5678"). A search match ending in the last column ended at the start of the
next row: the copy took the line break after the match, and on the bottom
row, the buffer's last line, the end fell back before the match and the
selection took the text in front of it ("0123456" for a search for "789");
it ends at `cols` on its own row, as xterm.js's selection model ends one.
And an iOS drag put the end it moves on the cell under the finger, capped
at the last column: that cell was never taken, so no drag could take a
row's last character. A drag puts it on the edge between cells nearest the
finger now, as text selection does and as xterm.js rounds the pointer, the
edge after the last column included; taps, links and mouse reports keep the
cell. B215.

**Upstream candidate:** yes.

## 56. A long press slides from its own word

**Touches:** `Sources/SwiftTerm/iOS/iOSTerminalView.swift` (`longPress`), a
test in `iOSTerminalViewTests.swift`. Commit `765fcbb`.

**Why:** Sliding a long press extends the selection from the long-pressed
word, pivoting on its start, but the pivot was taken only when none was
set: with a selection still showing (another long press, a handle
dragged), a long press on another word and a slide selected everything
from where the old selection began. The long press sets the pivot to its
word. Found with B215.

**Upstream candidate:** yes.

## 57. A shorter history says which lines went

**Touches:** `Sources/SwiftTerm/Buffer.swift` (`changeHistorySize`, which
returns how many lines went), `Sources/SwiftTerm/Terminal.swift`
(`changeScrollback`, `linesWereDropped`), tests in `HistoryTests.swift` and
`SynchronizedOutputTests.swift`. Commit `5cb1261`.

**Why:** `changeScrollback` to fewer lines than the history holds drops the
oldest, and every line left sits that many rows earlier, as after ED 3
(patches 29 and 37) or a line pushed out of a full history (41). Nothing
that keeps rows of the buffer heard of it: a selection kept its rows and
covered other text, the synchronized-output snapshot (45) read kept lines
as blank and dropped rows as lines still there, and a view on that screen
was not told where the viewport went. It also moved the saved cursor up by
as many rows, though the screen does not move, and a line lost its
scroll-invariant number. The count goes through `linesDropped` and to the
snapshot, the view hears `scrolled`, the saved cursor stays, `linesTop`
advances. Moshpit only grows its history (`TerminalScrollback`); this is
for any host that shrinks one. B216.

**Upstream candidate:** yes, with 37 and 45.

## 58. DECCOLM needs mode 40, as in xterm

**Touches:** `Sources/SwiftTerm/Terminal.swift` (`allow80To132`, `setup`),
tests in `ColumnModeTests.swift`; the DECCOLM tests in
`UpdateRangeTests.swift`, `ClearScrollbackViewTests.swift` and `Memory.swift`
turn mode 40 on first. Commit `3baf850`.

**Why:** xterm honours DECCOLM (`CSI ? 3 h` / `l`) only with mode 40 on
(its `c132` resource, false by default), and ignores it otherwise. Mode 40
was on from the start and after every full reset, so any remote could make
the grid 132 or 80 columns wide, and clear the screen, with DECCOLM alone;
Moshpit reports its grid to the remote as the window size. It is off now
until a program turns it on (`CSI ? 40 h`), and again after a full reset;
DECRQM reports it. Nothing in Moshpit relied on the old default (its
`TerminalViewForkTests` DECCOLM test turns mode 40 on). B217.

**Upstream candidate:** yes; a changed default for hosts that relied on it.

## 59. Autowrap and IND below the scrolling region move the cursor down

**Touches:** `Sources/SwiftTerm/Buffer.swift` (`insertAsciiRun`,
`insertCharacter`), `Sources/SwiftTerm/Terminal.swift` (`cmdIndex`, which
NEL uses), tests in `ScreenTests.swift`. Commit `2b86a49`.

**Why:** Only the bottom row of the scrolling region scrolls it. Below the
region the cursor is outside it, and xterm's autowrap there is an index
like IND (`WrapLine`, `xtermIndex`): the cursor moves down to the last row
and nothing scrolls; xterm.js does the same. Both write paths scrolled
whenever the cursor was on or below the region's bottom, and so did IND and
NEL: text that ran past the last column of a row below the region scrolled
the region up, its top line lost, and went on over the start of its own
row. B218.

**Upstream candidate:** yes.

## 60. No inline predictions, math results or Writing Tools on the text input

**Touches:** `Sources/SwiftTerm/iOS/iOSTerminalView.swift`
(`inlinePredictionType`, `mathExpressionCompletionType`,
`writingToolsBehavior`), a test in `iOSTerminalViewTests.swift`. Commit
`4e948ff`.

**Why:** The view turns off what a keyboard changes in typed text
(autocorrection, spell checking, which Moshpit turns back on for the input
methods' candidate bar, smart quotes, dashes, insert and delete) but did
not answer three later traits, and a view that does not answer one gets
the system's default, which offers it: inline predictions (iOS 17), math
results (iOS 18) and Writing Tools (iOS 18). Each, taken, is typed into the
remote; autocorrection off does not turn inline predictions off. The view
answers no to all three, and a host can set them. Checked as UIKit reads
them, on Mac Catalyst; whether a keyboard then offers nothing shows only on
a device. B218.

**Upstream candidate:** yes.

## 61. A cancelled Command press is not held

**Touches:** `Sources/SwiftTerm/iOS/iOSTerminalView.swift`
(`pressesCancelled`, `pressesEnded`, new `commandKeysUp`), a test in
`iOSTerminalViewTests.swift`. Commit `754a22d`.

**Why:** UIKit takes a press back instead of ending it when the app loses
the keyboard with the key down (⌘-Tab away with ⌘ held) or the system
claims the key. The view tracked the Command keys held only in
`pressesEnded`, so a cancelled ⌘ stayed held, and every press after it
counted as a ⌘ press until that key went down and up again: with the
`...WithModifier` link modes links underlined and opened on a plain click,
and hovering reported links as with ⌘ held (Moshpit's `.always` mode is
left with the hover reports). B218.

**Upstream candidate:** yes.

## 62. A cancelled press is released for the kitty keyboard protocol

**Touches:** `Sources/SwiftTerm/iOS/iOSTerminalView.swift`
(`pressesCancelled`, `pressesEnded`, new `sendKittyReleases`), a test in
`iOSTerminalViewTests.swift`. Commit `b873d6a`.

**Why:** With the kitty keyboard protocol's event reports on (flag 2) a
program hears each key go down and up. UIKit takes a press back instead of
ending it when the app loses the keyboard with the key down (held while
⌘-Tab takes the app away) or the system claims the key: `pressesCancelled`,
never `pressesEnded`. Only `pressesEnded` sent the release, so the program
took the key as held for good (a game kept moving, an editor kept the key
down). A cancelled press sends the release an ended one does; without
event reports nothing is sent, as before. B237.

**Upstream candidate:** yes, with 52 and 61.

## 63. Erasing the saved lines keeps the scroll-invariant numbers

**Touches:** `Sources/SwiftTerm/Terminal.swift` (`cmdEraseInDisplay`, case
3), a test in `HistoryTests.swift`. Commit `3fa2ffa`.

**Why:** ED 3 drops the history, and every line left sits that many rows
earlier. It set `linesTop`, the number of the oldest line, back to 0, so
the lines left took the numbers the erased ones had had: a number read
with `getScrollInvariantLine` before `clear` found another line after it,
and the numbers went backwards. A shorter history (57) and a line pushed
out of a full one already advance `linesTop` by the lines that went; ED 3
does the same. Moshpit reads no scroll-invariant numbers today. B238.

**Upstream candidate:** yes.

## 64. A host can keep the columns its own

**Touches:** `Sources/SwiftTerm/TerminalOptions.swift` (`hostOwnsColumns`),
`Sources/SwiftTerm/Terminal.swift` (`columnModeAllowed`, used by DECCOLM
and DECRQM), tests in `ColumnModeTests.swift`. Commit `1d0f672`.

**Why:** Patch 58 left DECCOLM to a program that turns mode 40 on, as xterm
does. A host whose grid follows its view had no way to keep a program from
making it 132 or 80 columns wide (and clearing the screen) with one
`CSI ? 40 h`. With `hostOwnsColumns` DECCOLM does nothing even then, and
DECRQM reports mode 40 off; it is read whenever a program asks, so a host
can set it on a running terminal. Off by default (xterm's behaviour);
Moshpit sets it on every terminal it mints (`TerminalMint.configureInput`):
the phone's screen decides the grid, and the remote is told that size.
B217, the decision recorded with patch 58.

**Upstream candidate:** yes; an option, nothing changes by default.

## 65. Trimmed lines no longer count their images

**Touches:** `Sources/SwiftTerm/CircularList.swift`
(`CircularBufferLineList.trimStart`), tests in `ImageTests.swift`. Commit
`a90d081`.

**Why:** The buffer counts the lines holding images (`hasAnyImages`):
while any do, every scroll runs the kitty placement pass and every iOS
repaint covers the whole view (`setNeedsDisplay(screenRows:)`). A line
recycled out of a full history lowered the count; lines dropped at once
from the top (`trimStart`: ED 3, a shorter history, fewer rows) did not,
so after `clear` erased the lines with images that work went on for the
life of the terminal. `trimStart` reports each line it drops that held
images, as recycling does, and empties its slot: dropped lines were held
until the history filled up over them again (after `clear` with a
10 000-line history, every line it had made) and a resize went on
resizing them. It also counts what it really dropped when asked for more
than there is. B239.

**Upstream candidate:** yes.

## 66. screen's title sequence is consumed, not printed

**Touches:** `Sources/SwiftTerm/EscapeSequenceParser.swift` (a
`screenTitleString` parser state), tests in `ScreenTitleTests.swift`.
Commit `7e5b936`.

**Why:** A shell inside screen or tmux names its window with
`ESC k <title> ST`; oh-my-zsh's `title` does it before every command and at
every prompt when `TERM` is `screen*` or `tmux*`. The multiplexer eats it,
but a tmux control-mode client is handed a pane's bytes raw, and here
`ESC k` fell through to an ESC dispatch: the title was printed into the
pane, so `echo hello` showed `echohello` and a command with no output left
a line of its name (seen in a capture of a private tmux 3.6a -CC session
running zsh with oh-my-zsh). `ESC k` now enters a string state that
consumes the title as tmux does: up to ESC (which starts the ST, or
whatever sequence follows) or BEL; CAN and SUB abort it. The bytes
0x80–0xff in it are the title's UTF-8, never C1 controls — the prompt
arrow U+279C ends in 0x9c, which must not end the title as an 8-bit ST.
Android's Termux got the same fix (W12). B257.

**Upstream candidate:** yes.

---

## Fork maintenance

- **Where the fork lives:** [github.com/Cluas/SwiftTerm](https://github.com/Cluas/SwiftTerm),
  branch `moshi-hyperlink-underline-fix`, forked from upstream `v1.13.0`.
  Since 2026-11-11 the pin points at branch `fix/ios-dirty-rect` (patches
  24–28 on top of `moshi-hyperlink-underline-fix`), pushed on its own because
  the main fork branch's working copy held uncommitted work at the time; merge
  it back into `moshi-hyperlink-underline-fix` and keep both branches.
  Patches 29–31 are on branch `fix/terminal-round14`, on top of
  `fix/ios-dirty-rect`; the pin moves there with them. Patches 32–33 are on
  branch `fix/terminal-round17`, on top of `fix/terminal-round14`. Patches
  34–39 are on branch `fix/terminal-round21`, on top of
  `fix/terminal-round17`. Patches 40–52 are on branch
  `fix/terminal-round24`, on top of `fix/terminal-round21`. Patches 53–61
  are on branch `fix/terminal-round29`, on top of `fix/terminal-round24`.
  Patches 62–65 are on branch `fix/terminal-round31`, on top of
  `fix/terminal-round29`. Patch 66 is on branch `fix/terminal-round32`, on
  top of `fix/terminal-round31`.
- **How `project.yml` pins it today:** by bare commit revision
  (`packages.SwiftTerm.revision`), not by a tag. A bare hash is fragile: if the
  fork branch is ever force-pushed or the pinned commit is garbage-collected,
  the pin can no longer be resolved and the build breaks.
- **Recommended future state:** cut a **tagged release** on the
  `moshi-hyperlink-underline-fix` branch and pin `project.yml` to that tag
  instead of the bare hash. A tag is a stable, GC-anchored reference that
  survives a force-push or garbage collection of loose commits, making the
  dependency reproducible over time.

  > Creating the tag is a git operation on the external fork repository and is
  > being handled separately — this document only records the recommendation.
  > Do not create the tag as part of editing this file.

- **Exit plan:** if the patches above land upstream and a tagged upstream
  release includes them, drop the fork and move the dependency back to
  `migueldeicaza/SwiftTerm` at that release.
