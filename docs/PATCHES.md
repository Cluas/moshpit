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
host-side plain-text linkification API (3), an iOS tap/link-focus fix (4),
IME (input-method, e.g. Pinyin) composition fixes (5–7, 9, 11), a soft-wrap
accessor (8), and hold-to-repeat delete over text the remote owns (10, 12).

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

## Fork maintenance

- **Where the fork lives:** [github.com/Cluas/SwiftTerm](https://github.com/Cluas/SwiftTerm),
  branch `moshi-hyperlink-underline-fix`, forked from upstream `v1.13.0`.
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
