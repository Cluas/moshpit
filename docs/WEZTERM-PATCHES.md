# Our wezterm-term fork

Moshpit's shared terminal core (`crates/term`, design in
`docs/nsh/TERMINAL-CORE.md`) runs wezterm's terminal emulator. wezterm-term
is not on crates.io and does not build for iOS as it is, so it is vendored
as our fork:

- **Where:** `vendor/wezterm/`, wezterm-term (`term/`) and the 13 crates of
  wezterm's repository it builds on, in upstream's own layout, with a
  workspace manifest of our own (`vendor/wezterm/Cargo.toml`, upstream's
  root manifest cut down to these crates). The root workspace excludes it;
  `crates/term` depends on it by path.
- **Upstream:** <https://github.com/wezterm/wezterm> at the commit in
  `vendor/wezterm/UPSTREAM` (now `ce8e6b140c2a52c8be1cc31c1274104b01422fdc`,
  2026-11-12). License: MIT (`vendor/wezterm/LICENSE.md`), compatible with
  Moshpit's GPL-3.0-only.
- **Import:** `scripts/wezterm/import.sh <upstream checkout> <rev>` copies the
  crates unchanged (only the Unicode data and table generators of `bidi` and
  `wezterm-char-props` are left out). The import commit is `61933440`; every
  change of ours is a commit after it and is listed below.
- **Updating:** branch from the last import commit, run the import at the new
  revision, commit it ("chore(wezterm): import upstream <rev>"), and merge
  that branch into ours: git carries our patches over, and a conflict is
  exactly a place where upstream changed code we changed. Then run
  `cargo test` in `vendor/wezterm` (upstream's own tests, kept passing) and
  `cargo test -p moshpit-term` (ours: the screen cases, the recorded
  streams against tmux, the cut-invariance fuzz).

Every patch is marked `Moshpit:` in the code. Behaviour changes that an
embedder might not want are switches of `TerminalConfiguration` whose
default is upstream's behaviour; `crates/term` turns them on.

## At a glance

- **W1** · build · termwiz's tty modules behind a `terminal` feature, so
  wezterm-term builds for iOS (`termios` has no iOS).
- **W2** · security · kitty images only by direct transmission unless a
  feature allows the client-file mediums.
- **W3** · size · `image` with PNG, JPEG, GIF and WebP only.
- **W4** · switch `writer_thread()` · replies written synchronously, in
  order, without a thread per terminal.
- **W5** · fix · a wide character that does not fit before the right margin
  wraps first.
- **W6** · fix · a grapheme cut by the end of a read joins the cell printed
  last, as if it had come in one read.
- **W7** · switch `host_owns_columns()` · DECCOLM does nothing.
- **W8** · switches `max_image_bytes()`, `kitty_image_storage_limit()` ·
  image memory limits, and a bounded kitty chunk accumulator.
- **W9** · fix · `Screen::with_phys_lines` read the wrong lines (or panicked)
  once the line deque had wrapped.
- **W10** · switch `scroll_on_clear()` · ED 2 moves the screen into the
  history first.
- **W11** · fix · the wrap mark of a line ending in a wide character was
  lost once the line was stored as a vector.
- **W12** · fix · two cells whose texts would form one grapheme are no
  longer read back as one by a clustered line.
- **W13** · fix · HTS past the tab stops panicked.
- **W14** · fix · CUP, HVP and CHA stop at the last column.
- **W15** · fix · a new configuration's history limit reaches the screens.
- **W16** · fix · base64 with or without padding (kitty's own `icat` sends
  it unpadded: no image showed).
- **W17** · fix · ECH keeps a kitty image placed over the cells it erases.
- **W18** · fix · kitty's "report all keys as escape codes" sends text keys
  as escape codes.
- **W19** · perf · a clustered line of ASCII text is cut into cells by the
  byte, not run through the grapheme segmenter (a frame of plain output
  3x cheaper, its parse 2.4x faster).
- **W20** · memory · the images kept only to share identical ones and the
  kitty images no cell shows can be let go (a memory warning, the
  background); placing an image the terminal does not have answers ENOENT.
- **W21** · memory · an image wider than the screen is clipped at the right
  margin (its cells past it kept its data alive after `clear`).
- **W22** · memory · kitty images sent without an id are counted once in the
  store's bytes (each replaced the one before but stayed on the count).
- **W23** · behaviour · for a host mirroring a tmux pane, `ESC [J` from the
  top left (a tmux pane's `clear`) moves the screen into the history first,
  as tmux's `scroll-on-clear` does.
- **W24** · fix · a program coming to the alternate screen starts with no
  kitty keyboard flags; the last one there may have left its own.
- **W25** · memory · an OSC or APC string past 64 MiB is dropped whole,
  its memory let go at once.
- **W0** · lint · `max_value()` to `MAX` (deprecation warnings in every build).

---

## W1. termwiz builds without its tty modules

**Touches:** `termwiz/Cargo.toml`, `termwiz/src/lib.rs`,
`termwiz/src/error.rs`, `vendor/wezterm/Cargo.toml`.

**Why:** wezterm-term uses termwiz only for its key encoding types
(`termwiz::input`) and an error type, but termwiz always compiled its tty
code (Terminal, caps, render, lineedit, istty) and its dependencies; one of
them, `termios` 0.3, has no definitions for iOS, so wezterm-term did not
build for `aarch64-apple-ios` at all. A new `terminal` feature (in
termwiz's default features, so upstream's own builds are unchanged) holds
those modules and `termios`, `terminfo`, `signal-hook`, `nix`,
`filedescriptor`, `fancy-regex`; our workspace takes termwiz without
default features. The `widgets` feature now implies `terminal`.

**Upstream:** worth offering (a "core only" termwiz is useful to any
embedder).

## W2. kitty images: direct transmission only

**Touches:** `wezterm-escape-parser/src/apc.rs`,
`wezterm-escape-parser/Cargo.toml`, `termwiz/Cargo.toml`.

**Why:** `KittyImageData::load_data` existed only with the `kitty-shm`
feature, which also enabled reading the image from a file the program names
(t=f), from a temporary file it then deletes (t=t), or from shared memory
(t=s). Those mediums are for a terminal on the same machine as the program;
Moshpit's programs run on another machine, and must not make the app read,
or delete, its own files. `load_data` now always serves the direct medium
(t=d); `kitty-local-files` (new) allows t=f and t=t, `kitty-shm` implies it
and adds t=s. termwiz's `use_image` no longer turns `kitty-shm` on. Without
them, a transmission naming a local file is refused with an error (the
kitty response says so to the program). `load_data` needs `std`.

**Upstream:** the split is reasonable for any remote-first embedder.

## W3. image formats

**Touches:** `vendor/wezterm/Cargo.toml` (ours).

**Why:** upstream takes `image` 0.25 with its default formats (AVIF, BMP,
DDS, EXR, Farbfeld, HDR, ICO, JPEG, PNG, PNM, QOI, TGA, TIFF, WebP) and
rayon's thread pool. The image protocols bring PNG or raw pixels (kitty),
the sixel decoder makes pixels itself, and iTerm2's `imgcat` sends the file
as it is: PNG, JPEG, GIF and WebP cover them. With W1, the size probe of
`spike/term-core/size` (a stripped cdylib, fat LTO) went from +3.82 MB to
+2.18 MB on Android and builds at all for iOS (+2.02 MB).

## W4. replies without a writer thread

**Touches:** `term/src/config.rs` (`writer_thread`), `term/src/terminalstate/mod.rs`.

**Why:** every `Terminal` started a thread (`ThreadedWriter`) and its
replies — to DA, DSR, colour and size queries, and the key and mouse
encodings — reached the embedder's writer later, from that thread. Moshpit
feeds the terminal on its pane thread and wants the replies in its input
queue in the order the requests came, without one thread per pane. With
`writer_thread()` false the writer is called synchronously by the call that
produces a reply; it must not block (ours appends to a vector).

## W5. a wide character at the margin wraps first

**Touches:** `term/src/terminalstate/performer.rs` (`flush_print`).

**Why:** printing a two-column character with the cursor in the last column
wrote it there: its second half ran past the margin, off screen, and the
cursor stayed on the line. xterm and tmux (3.6a, measured) erase that last
cell, mark the line as continuing, and print the character at the start of
the next line. Chinese and Japanese text hit this on every line that does
not happen to end on an even column.

## W6. graphemes cut by a read

**Touches:** `term/src/terminalstate/performer.rs` (`join_last_print`,
`flush_print`, `perform`, `Drop`), `term/src/terminalstate/mod.rs`
(`LastPrint`, `held_print`).

**Why:** wezterm clusters graphemes within the text of one read. Text that
continues the last grapheme of the previous read — a combining mark, a ZWJ
and the emoji it joins, VS16, a skin tone, the second regional indicator of
a flag — became a grapheme of its own; a zero-width one was dropped. The
screen depended on where the network, tmux or mosh cut the stream (a `-CC`
`%output` line ends anywhere): `e` + U+0301 lost its accent, a family emoji
split into its people and the cursor ran two columns ahead of tmux's.

Now the terminal remembers the cell it printed last while nothing but text
has come since. When the next text starts with what continues that
grapheme, the cursor goes back to the cell and the grown grapheme is
printed over it through the normal path, so it gets the width (wider or
narrower) and the wrap it would have had in one read. A zero-width
grapheme that is dropped between them ends the run. A read that ends with a
Prepend character (U+0600..U+0605 and the like, which attach to what
follows) holds it for the next read.

**Known exception:** wezterm gives `9` + VS16 two columns and `9` + VS16 +
VS16 one. If a read ends between the two selectors at the last column, the
two-column state has already wrapped to the next line and the narrower
final grapheme stays there; in one read it stays on the line. Two variation
selectors in a row are not text programs send; the fuzz test leaves that
pair out.

**Tests:** `crates/term/tests/cases.rs` (`w6_…`), every recorded stream
in six cuttings, and 5 000 rounds of random escape soup in random cuts
(`tests/corpus.rs`).

## W7. DECCOLM does nothing when the host owns the grid

**Touches:** `term/src/config.rs` (`host_owns_columns`),
`term/src/terminalstate/mod.rs`.

**Why:** upstream never resizes for DECCOLM (mode 3) but clears the screen,
homes the cursor and resets the margins, even without mode 40, which xterm
requires. Moshpit's grid is its view's (SwiftTerm fork patches 58 and 64):
with the switch on, a program cannot clear the screen this way either.

## W8. image memory limits

**Touches:** `term/src/config.rs` (`max_image_bytes`,
`kitty_image_storage_limit`), `term/src/terminalstate/{image,kitty,sixel,iterm}.rs`.

**Why:** the limits were constants: 100 MB per decoded image and 320 MB
for the kitty images a program stores ("FIXME: make this configurable").
A phone needs less; `crates/term` sets 32 MB and 64 MB. A chunked kitty
transmission was accumulated without bound until its last chunk; it is now
dropped once it is larger than the image limit allows (base64 and 64 KB of
slack).

## W9. `Screen::with_phys_lines`

**Touches:** `term/src/screen.rs`.

**Why:** the line deque's second slice was indexed with deque indices: once
the deque had wrapped (any terminal that has filled its history), reading a
line from the second slice read another line, or panicked past its end.
`with_phys_lines_mut` already subtracted the first slice's length; the
immutable twin now does too. (Found by `crates/term`, which reads lines
through it.)

## W10. scroll on clear

**Touches:** `term/src/config.rs` (`scroll_on_clear`),
`term/src/terminalstate/mod.rs` (`erase_in_display`).

**Why:** tmux (`scroll-on-clear`, on by default) and alacritty move what
the main screen shows into the history before ED 2 erases it, up to its
last line with text; wezterm erased it. Moshpit's panes mirror tmux's, and
`clear` or Ctrl-L should leave the output within reach (the recorded `ansi`
capture against tmux 3.6a showed 22 lines missing).

## W11. the wrap mark of a line ending in a wide character

**Touches:** `wezterm-surface/src/line/line.rs`
(`set_last_cell_was_wrapped`).

**Why:** a line stored as a vector of cells keeps a filler cell after a
wide character; the wrap mark was set on the last cell — the filler — while
`last_cell_was_wrapped` reads the last cell `visible_cells` yields, the wide
one. The mark was lost: a CJK line edited out of order (any cell rewritten
in place) and then wrapped did not continue on the next line, for copying
and reflow. It is now set on the last visible cell too. (Found by the fuzz
test.)

## W12. cells that would form one grapheme

**Touches:** `wezterm-surface/src/line/clusterline.rs` (`boundary_holds`,
`can_append`), `wezterm-surface/src/line/line.rs` (`set_cell_grapheme`,
`set_cell_impl`, `compress_for_scrollback`).

**Why:** a clustered line keeps its cells' text as one string and finds
the cells again by grapheme segmentation. Two cells whose texts segment as
one grapheme — an emoji ending in ZWJ and an emoji printed after a control
character, two regional indicators printed apart — came back as one cell,
and every cell after it one column off (text drawn at the wrong place,
copied wrong). A cell that would merge with the one before it now makes the
line a vector of cells, and such a line is not compressed into a clustered
one for the history. The check is exact (segmentation of up to 32 bytes
before the boundary and the character after it) with quick answers for
ASCII and the common scripts, so it costs little. (Found by the fuzz test.)

## W13. HTS past the tab stops

**Touches:** `term/src/terminalstate/mod.rs` (`TabStop::set_tab_stop`).

**Why:** setting a tab stop with the cursor past the end of the tab stop
vector (one column past the last, which W14's positions could reach)
indexed out of bounds: a remote program could crash the terminal with
`CSI 1;999H ESC H`. It is ignored now, as `clear` already did.

## W14. CUP, HVP and CHA stop at the last column

**Touches:** `term/src/terminalstate/mod.rs`, `term/src/test/csi.rs`.

**Why:** an absolute horizontal position was clamped one column past the
last ("to account for some resize/rewrap scenarios"), so `CSI 999 G` then
`CSI 10 D` landed one column right of where xterm, tmux and every other
terminal put it, and text printed there was written past the margin. The
CSI commands now stop at the last column (HPA already did); the internal
uses of the extra column (restoring the cursor of a wrapped line after a
resize) are untouched. Upstream's tests for CUP, HVP and CHA expected the
column past the last and are changed to the last.

## W15. a new configuration reaches the screens

**Touches:** `term/src/screen.rs` (`set_config`),
`term/src/terminalstate/mod.rs` (`set_config`).

**Why:** `TerminalState::set_config` replaced the terminal's configuration
but each screen kept the one it was made with, so a changed history limit
never applied (the app's history setting, B216). The screens take the new
configuration, and lines over a smaller limit go from the top at once.

## W16. base64 with or without padding

**Touches:** `wezterm-escape-parser/src/osc.rs` (`base64_decode`).

**Why:** the decoder required canonical padding. kitty's own `kitten icat`
(0.50.0) sends its image data unpadded, as kitty's protocol allows, and
every image it sent was dropped (`crates/term/tests/images.rs`, a fixture
recorded from icat). The decoder now takes either. It serves OSC 52 and
iTerm2's images too.

## W17. ECH keeps kitty images

**Touches:** `term/src/terminalstate/mod.rs` (`Edit::EraseCharacter`).

**Why:** kitty keeps images apart from the text: erasing characters does
not remove an image. wezterm attaches images to the cells they cover, so
ECH erased them with the text. term-image places an image without moving
the cursor (`C=1`) and then erases the text under it with ECH: the image
vanished at once (a fixture recorded from term-image 0.7.2). ECH now keeps
the kitty placements (images with an id) of the cells it blanks; sixel and
iTerm2 images, which are part of the text as in xterm, are erased as
before. EL and ED still remove everything, as upstream.

## W18. kitty: "report all keys" covers text keys

**Touches:** `wezterm-input-types/src/lib.rs` (`KeyEvent::encode_kitty`).

**Why:** with the kitty keyboard protocol's flag 8 ("report all keys as
escape codes") a program asks to hear every key as an escape code, text
keys included, and to receive no text: `a` is `CSI 97;1 u`. wezterm's
encoder still sent the legacy text form (`a`) whenever no other flag
ruled it out. Found when `crates/term` wired wezterm's encoder into its C
ABI for the iOS view (`crates/term/tests/input.rs`); a test of its own in
`wezterm-input-types`.

**Upstream:** worth offering.

## W19. ASCII lines are cut by the byte

**Touches:** `wezterm-surface/src/line/clusterline.rs` (`CellTexts`,
`ClusteredLine::iter`).

**Why:** a clustered line keeps its text as one string and finds its cells
by running the Unicode grapheme segmenter over it, every time its cells
are listed: for every row of every frame the view takes, and for every
row a write turns back into cells. Half of what a frame of plain output
cost was the segmenter (sampled on the Mac). Text that is ASCII and holds
no CR (CR LF is the one ASCII grapheme of two bytes; a terminal's cells
never hold a CR anyway) is one grapheme per byte, so it is cut by the
byte; anything else goes through the segmenter as before. No behaviour
changes: a test checks the fast path against the segmenter. The line's
layout is unchanged (its size test stands). Measured on the Mac (90×50,
coloured ASCII lines): a full-screen frame 0.374 → 0.115 ms, parsing 400
lines 3.52 → 1.40 ms. Found by T4 (TERMINAL-VIEW.md 5.6): a backlog shown
at the display's rate pays one full frame per refresh.

`cluster_line_new` and `cluster_wrap_last` in `wezterm-surface` fail with
or without this patch (their snapshots predate an earlier patch of ours).

**Upstream:** worth offering.

## W20. Letting images go

**Touches:** `term/src/terminalstate/image.rs`
(`TerminalState::release_image_dedup`), `term/src/terminalstate/kitty.rs`
(`kitty_drop_unplaced`, `kitty_image_bytes`, the `Display` action).

**Why:** an image's memory stayed after the screen and the history were
cleared (TERMINAL-VIEW.md §5.8), in three places per terminal: the core's
decoded images (ours, now one budget for the process), kitty's stored
images (protocol state, kept until deleted or over the storage limit),
and the emulator's `image_cache`: the last 16 images, kept so an
identical one shares their data, whatever their size (one large sixel is
tens of MB of pixels). Six tmux panes made that a few hundred MB, what
gets a terminal app killed in the background on a phone.
`release_image_dedup` empties the sharing cache (images still on a cell
keep their data: cells hold it themselves; a later identical image is
stored again rather than shared). `kitty_drop_unplaced` lets go of the
stored kitty images only the store still holds (no cell on either screen
or in the scrollback shows them), with their placements and numbers. A
placement of an image the terminal does not have now answers ENOENT, as
kitty does; it failed silently (only a log line), so a program that
re-placed an image let go on a memory warning could not tell.

**Upstream:** the ENOENT answer, yes; the rest is a host's memory policy.

## W21. Images are clipped at the right margin

**Touches:** `term/src/terminalstate/image.rs` (`assign_image_to_cells`).

**Why:** placing an image wrote a cell for each of its columns, past the
right margin too: the line grew longer than the screen. Those cells are
never drawn, and no erase reaches them (an erase covers the screen's
columns), so they held the image's data for as long as their line stayed
on the screen: `clear` and `ESC [3J` let a 1200 px image go on a wide
screen and kept it on a narrower one (B295: 14.4 MB a pane, found through
tmux on the iPad; most images are wider than a phone). The image is now
clipped there; its texture coordinates advance as before, so what shows
is unchanged.

**Upstream:** yes.

## W22. Anonymous kitty images are counted once

**Touches:** `term/src/terminalstate/kitty.rs`
(`KittyImageState::record_id_to_data`).

**Why:** an image sent without an id is stored under 0 and replaces the
one before it there, but the store's byte count only ever added: three
anonymous images counted three while the store held one. The count is
what the storage limit prunes by (so real images were pruned early) and
what the host reports; on the iPad a memory warning seemed to leave two
large images behind in each tmux pane (TERMINAL-VIEW.md §5.8, B295). The
replaced image's bytes now come off the count.

**Upstream:** yes.

## W23. A tmux pane's clear keeps what tmux keeps

**Touches:** `term/src/config.rs` (`scroll_on_clear_from_home`, default
off), `term/src/terminalstate/mod.rs` (`erase_in_display`,
`scroll_screen_into_history`).

**Why:** a phone's tmux pane mirrors what tmux draws, and its local
history should hold what tmux's own does, so scrolling back after `clear`
shows what a desktop tmux client shows. tmux 3.6a's `scroll-on-clear` (on
by default) moves the screen into the history on `ESC [2J` and on
`ESC [J` with the cursor at the top left, never on the alternate screen;
a tmux pane's terminfo (`tmux-256color`) makes `clear` send `ESC [H
ESC [J`, so that is the clear people type. Our scroll-on-clear (the
fork's patch on ED 2) did only the first. With the switch on, ED 0 from
the top left of the main screen does the same as ED 2; off (every other
host: plain SSH, mosh, herdr), nothing changes. Checked against a private
tmux 3.6a line for line (`crates/term/tests/tmux_clear.rs`). The one
difference left: tmux counts a line as used once a program wrote to it,
spaces included; wezterm keeps no such mark, so written spaces below the
last line with text are not kept.

**Upstream:** no (a host's choice).

## W24. A new alternate screen starts with no keyboard flags

**Touches:** `term/src/terminalstate/mod.rs` (`ScreenOrAlt::activate_alt_screen`).

**Why:** the kitty keyboard protocol keeps a stack of flags per screen, and
a program is meant to pop its own before it leaves the alternate screen.
One that leaves first (crossterm's `LeaveAlternateScreen`, then
`PopKeyboardEnhancementFlags`) pops the main screen's stack instead, and its
flags stay on the alternate screen's: the next program there gets them
without asking, and vim's Escape arrives as `ESC [27;1u` (B576). Coming
from the main screen, the alternate screen's stack is emptied: a program
there starts from nothing and pushes what it wants. The main screen's
stack is untouched, so a shell's own flags (fish) survive a pager.

**Upstream:** worth offering; the protocol's text says only that the two
stacks are separate.
## W25. OSC and APC strings are bounded

**Touches:** `vtparse/src/lib.rs` (`MAX_STRING_BYTES`, `OscState::put`, the
`OscEnd`, `ApcPut` and `ApcEnd` actions).

**Why:** with `std`, the parser kept an OSC or APC string whole until its
terminator, with no limit: a remote copying a huge file through OSC 52
handed the whole of it over, and a string a program opened and never
closed took in everything after it while the screen showed nothing, until
iOS ended the app (B577). Past 64 MiB (room for the largest image the
terminal takes, 32 MiB decoded, in base64) a string is dropped whole: its
bytes are still read up to its end, what it held is let go at once, and it
is not dispatched, since a cut payload means nothing. What follows its end
shows as before.

**Upstream:** worth offering, with the limit theirs to choose.

## W26. A sixel image is bounded before it is drawn

**Touches:** `wezterm-escape-parser/src/parser/sixel.rs` (`MAX_SIXEL_DATA`,
`MAX_SIXEL_RESERVE`, `SixelBuilder::dropped`, `push`, the raster
attributes' reserve), `parser/mod.rs` (`dcs_unhook`).

**Why:** the parser reserved room for width × height sixel commands as
soon as the raster attributes named a size, under its 100 M limit: a
header naming 10000×10000, a few dozen bytes, allocated 762 MB before a
single command came. And it kept every command of an image without a
limit, twelve bytes each: 20 MB of sixel data took 384 MB. The terminal's
own check of the image's size (W8) comes only after all that (B735). The
reserve is now at most 64 Ki commands (a sixel is six pixels: the size
over six), and past 4 Mi commands (about 48 MB of them; a 4096×2048 image
without a repeat is 1.4 Mi) the image is dropped whole: its bytes are still
read to its end, what it held is let go at once, and it is not dispatched.

**Upstream:** worth offering, with the limits theirs to choose.
## W27. The host lets go of kitty images its history no longer shows

**Touches:** `term/src/terminalstate/kitty.rs` (`kitty_drop_data`).

**Why:** the host holds the images of the rows in its history to a budget
and takes them off the oldest rows (B733). A kitty image's placement stays
on record after that, and the store prunes only images with no placement
on record: the ones the host had let go of were kept for good, up to all of
them (100 icat images of 400×400 kept 84 MB past the store's 32 MB limit).
`kitty_drop_data` lets go of the stored images the host names that no cell
shows any more, with their placements and numbers. Images a program stored
and has not placed yet are never among them (W20's `kitty_drop_unplaced`,
for a memory warning, takes those too).

**Upstream:** ours (the host's budget is ours).

## W0. deprecated `max_value()`

**Touches:** `wezterm-escape-parser/src/*.rs`, `bidi/src/lib.rs`.

**Why:** twelve deprecation warnings in every build of the app. `T::MAX`
is what upstream would write today.
