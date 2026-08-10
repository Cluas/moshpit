---
title: "Typing Chinese in a terminal, properly."
description: "Terminal emulators on iOS are usually written by people typing ASCII. Turn on a Pinyin keyboard and the cracks show: the composing text is invisible, the caret jumps to the wrong place mid-word, and hold-to-delete refuses to repeat. Moshpit ships a patched terminal engine specifically to fix that."
---

## What composition should look like

While an IME is composing — Pinyin, Kana, Hangul — the in-progress text belongs to the keyboard, not to the shell. Nothing should reach the remote host until you commit. Moshpit draws that composing text underlined at the cursor, and moves the terminal's real caret to the end of it as it grows, so what you see is where you are.

## What was broken, and is not

- <b>Invisible composition.</b> The engine tracked marked text internally but never rendered it anywhere. It is now painted as an underlined overlay at the cursor.
- <b>The caret jumping backwards.</b> Phonetic IMEs replace the whole marked string on every keystroke, and candidate paging can report a stale selection, so a naive offset tracker jitters to the front of the word. The caret now follows the end of the composing text.
- <b>Output stealing the caret.</b> A pane that repaints often — an agent status line, a resize, a reflow — used to yank the caret from the end of your Pinyin back to its front, mid-word. Repaints now defer to the composition overlay.
- <b>Delete destroying the whole composition.</b> Backspace during composition removed one character from the marked text instead of sending real backspaces that nuked the lot — which is what fixes hold-to-repeat delete under a Pinyin keyboard.
- <b>Commit snapping the caret back.</b> At the instant composition ends the terminal's own cursor has not moved yet, so snapping to it yanked the caret backwards exactly as you finished a word. It no longer snaps.

## Wide characters

CJK glyphs occupy two cells. That matters beyond looks: a URL that wrapped across rows cannot be rejoined by counting characters when a wide glyph earlier on the line throws the count off. Moshpit reads the emulator's own soft-wrap bit instead, so linkifying a wrapped URL does not silently truncate it.

## Honest limits

All of this is client side. If the program on the host does not handle wide characters properly — some TUIs do not — Moshpit renders faithfully what it is sent, including the misalignment. And a terminal font must contain the glyphs: the bundled monospace fonts cover Latin well and CJK unevenly, so the system font fallback does the work for characters they lack.
