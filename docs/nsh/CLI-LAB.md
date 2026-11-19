# CLI lab

The CLI lab records what a Moshpit pane receives when you type into a coding CLI.
It then replays those bytes through the Rust terminal core and checks what a viewer would see.
It exists for two user reports:

1. Qoder 1.1.92 inside tmux shows nothing while you type English, but Enter still sends.
2. Over mosh only, the cursor jumps up and down.

Nothing in the lab uses a real account, the user's HOME, the user's tmux or herdr, or a real model.
Each CLI talks to `fake_model.py` on 127.0.0.1 with a dummy key.

## Run it

Recording needs the CLIs installed (or unpacked under `/tmp/clisrc`), plus tmux, mosh-server and mosh-client.
Replaying needs only cargo.

```sh
# One recording: <cli> <path>. Paths are tmux, ssh, mosh and mosh-tmux.
scripts/cli-lab/drive.py claude tmux --out /tmp/clilab-fx --ready shortcuts
# mosh-client with its own prediction display on:
scripts/cli-lab/drive.py codex mosh --predict always --out /tmp/clilab-fx
# A slow link: a UDP relay (scripts/cli-lab/udp_delay.py) with a 500 ms round
# trip between the mosh client and mosh-server; name gets -rtt500.
scripts/cli-lab/drive.py codex mosh-tmux --rtt 500 --out /tmp/clilab-fx
# Moshpit's own mosh transport instead of mosh-client (name gets -rs): the
# pane bytes are the HostFrames crates/mosh/examples/mosh_harness prints.
cargo build --release -p moshpit-mosh --example mosh_harness
scripts/cli-lab/drive.py codex mosh-tmux --rtt 500 \
  --moshpit $CARGO_TARGET_DIR/release/examples/mosh_harness --out /tmp/clilab-fx
# fake_model.py: LAB_LONG=N appends N lines to the answer, LAB_PACE=S is the
# pause between streamed pieces (default 0.15 s).
LAB_LONG=40 scripts/cli-lab/drive.py codex mosh-tmux --rtt 500 --out /tmp/clilab-fx

# Everything, in parallel per path, with tmux references added (about 4 minutes):
OUT=/tmp/clilab-fx scripts/cli-lab/record-all.sh
cp /tmp/clilab-fx/*.jsonl crates/term/tests/fixtures/cli-lab/

# tmux's reading of a recording, at the end or just before some text:
scripts/cli-lab/tmux_ref.py FIXTURE [--before REGEX]

# The checks:
CARGO_TARGET_DIR=/tmp/target-clilab cargo test -p moshpit-term --test cli_lab
# The table below, and one recording's screen at each key (LAB_ALL=1 shows every frame):
cargo test -p moshpit-term --test cli_lab -- --ignored --nocapture lab_table
LAB=gemini-tmux cargo test -p moshpit-term --test cli_lab -- --ignored --nocapture lab_frames
```

### How the recording works

`drive.py` runs one CLI in a fresh HOME under `/tmp/L/<name>`.
It waits until the CLI's input box shows (`--ready REGEX` on the text without escape sequences, then 1 s of quiet).
Then it types `hello world` one key every 0.25 s, sends Enter, and waits for the reply.

What gets recorded depends on the path:

| path | what the CLI runs in | bytes recorded | keys go in through |
|---|---|---|---|
| tmux | a private server `tmux -L clilab-<name> -f /dev/null` | `%output` of a control-mode client, the way Moshpit's tmux pane gets them | `send-keys` |
| ssh | a bare pty with `SSH_CONNECTION` and `SSH_TTY` set | the raw pty bytes | the pty |
| mosh | mosh-server on 127.0.0.1, read by a real mosh-client | mosh-client's stdout (see below) | mosh-client |
| mosh-tmux | as mosh, with the CLI inside a private tmux | as mosh | as mosh |

For the mosh paths, mosh-client's stdout stands in for the host bytes.
It comes from mosh's frame-diff generator (`Display::new_frame`), the same one that produces the bytes Moshpit's own mosh transport feeds the core.

Each fixture is JSON lines:

- a header: `{cli, path, cols, rows, text, predict}`
- `{"t": ms, "o": base64}`: bytes the pane received
- `{"t": ms, "k": key}`: a key that was sent
- `{"ref": rows, "cursor": [x, y, shown], "at": N}`: tmux's screen after the first N events.
  A ref without `at` is the live `capture-pane` at the end of a tmux recording.
  `tmux_ref.py` makes the others: it writes the bytes with `cat`, under `stty raw -echo`, into a pane of a private tmux, then captures that pane.

Fixtures show `/Users/dev/lab/...` in place of the lab's real paths.
The lab root `/private/tmp/L` has the same length as `/Users/dev/lab`, so columns don't move when the path is replaced.

### The checks

`crates/term/tests/cli_lab.rs` feeds each recording chunk by chunk, each chunk one read at its recorded time.
It publishes a frame when a pane would: once the output held is due (`crates/term/src/coalesce.rs`, see "Frame coalescing" below).
The checks run on what those frames show.
Concealed text (fg = bg) counts as blank, and wide-char spacers are skipped.

- **final**: the core's screen and cursor equal tmux's at every reference (invariant a).
- **typed**: when key k+1 is sent, the input row shows the first k characters (invariant b).
  The input row is the row that begins with the prompt seen just before Enter.
  If the host sent nothing between key k and key k+1, key k is skipped: that is host lag, not the view's fault.
- **steady**: between the first key and Enter, no frame shows less typed text than an earlier frame, and a visible cursor stays on the input row (invariant c: no torn frames, no cursor jumps).
- **cursor**: from the first key to the end, the answer streaming in included, a visible cursor is only ever on the input row (CLI recordings only: a shell's cursor rightly leaves its prompt).
- **predicted** (mosh recordings only): the same replay with Moshpit's predictive echo set to `Always` (srtt 250 ms, each key acked by the next chunk).
  It may differ from the plain replay only on the input row, and the predicted cursor stays on that row.

Each recording × check is its own `#[test]`.
A red one carries `#[ignore = "red: <reason>"]`.

## Results

Versions: tmux 3.6a, mosh 1.4.0, Claude Code 2.1.327, Codex 0.166.0, Gemini CLI 0.22.4, Qwen Code 0.28.0, Copilot CLI 1.0.8, Qoder 1.1.92. Size 80×24.
`*-mosh-pred-always` is mosh-client with its own prediction display on (`MOSH_PREDICTION_DISPLAY=always`).
The "prediction on" column is Moshpit's predictive echo applied in the replay.

| recording | final = tmux | typed visible | steady (no tear/jump) | prediction on |
|---|---|---|---|---|
| shell-tmux | pass | pass | pass | — |
| shell-ssh | pass | pass | pass | — |
| shell-mosh | pass | pass | pass | pass |
| claude-tmux | pass | pass | pass | — |
| claude-ssh | pass | pass | pass | — |
| claude-mosh | pass | pass | pass | pass |
| claude-mosh-tmux | pass | pass | pass | pass |
| claude-mosh-pred-always | pass | pass | pass | pass |
| codex-tmux | pass | pass | pass | — |
| codex-ssh | pass | pass | pass | — |
| codex-mosh | pass | pass | pass | pass |
| codex-mosh-tmux | pass | pass | pass | pass |
| codex-mosh-pred-always | pass | pass | pass | pass |
| gemini-tmux | pass | pass | pass | — |
| gemini-ssh | pass | pass | pass | — |
| gemini-mosh | pass | pass | pass | pass |
| qwen-tmux | pass | pass | pass | — |
| qwen-ssh | pass | pass | pass | — |
| qwen-mosh | pass | pass | pass | pass |
| copilot-tmux | pass | pass | pass | — |
| copilot-ssh | pass | pass | pass | — |
| copilot-mosh | pass | pass | pass | pass |
| qoder-tmux | pass (at the input box) | — | — | — |
| qoder-ssh | pass (at the input box) | — | — | — |
| qoder-mosh | pass (at the input box) | — | — | — |
| codex-mosh-tmux-rtt500-rs | — (no tmux reference) | lags (echo over 500 ms) | pass | — |

Cursor check: claude-tmux, claude-mosh, codex-tmux, codex-mosh, codex-mosh-tmux and codex-mosh-tmux-rtt500-rs pass.
codex-mosh-tmux-rtt500-rs was red until bug 2 (below) was fixed.

Totals: 86 green.

### Per CLI

- **Shell** (`/bin/sh`, the control): all green.
- **Claude Code**: all green on every path, with and without either kind of prediction.
  Over mosh, Claude often hasn't drawn the first key before the second one is sent.
  That is host lag, and the typed check skips it.
- **Codex**: all green.
  It runs with `--no-daemon`: the app-server daemon socket under `/private/tmp/codex-daemon-<uid>` collides when runs go in parallel.
- **Gemini CLI**: all green since frame coalescing.
  Before it, steady was red over tmux and ssh: a torn frame (see below).
- **Qwen Code**: all green.
  An update notice shows up above the input box while you type and moves the box down.
  The checks follow the box by its prompt.
- **Copilot CLI**: all green.
  It types fine; sending a message needs `/login`, so there is no model reply.
  The folder-trust dialog is answered with `--pre Enter`.
- **Qoder 1.1.92**: draws its input box, then exits about 70 ms later with "Custom Provider access is not allowed for the current account."
  The exit comes right after its `OSC 7501 ;?` query, and there is no way to log in without a real account.
  So only its first screens are recorded (`--text -`).
  `tmux_ref.py --before 'Custom.?Provider'` cuts the recording there.
  At that cut the core matches tmux on every path: the input box and the cursor inside it.
- **Skipped**:

  | CLI | reason |
  |---|---|
  | Cursor (`cursor-agent`) | needs a login before it draws anything usable |
  | CodeBuddy 2.185.0 | needs a login; it never draws an input box |
  | iFlow 0.5.19 | the published tarball fails with `ERR_MODULE_NOT_FOUND` |
  | OpenCode | not installed and not in the tarball cache |

### Gotchas found while building the lab

- **Never set `CI`, even to an empty string.**
  Ink-based CLIs (Gemini, Qwen, Claude) treat any `CI` variable as a CI log.
  They print static output and never draw the input box, yet Enter still submits.
  That is exactly the symptom of bug 1 (see H1-a).
- mosh-server starts the command in `$HOME`.
  The driver wraps the command as `sh -c 'cd "$0" && exec "$@"' work ...`, otherwise CLIs see the wrong directory (a trust dialog, or Codex exits).
- tmux 3.6a answers `OSC 10 ;?` and `OSC 11 ;?` itself, with black on black, when the client is in control mode and the outer terminal's colours are unknown.

## Bug 1: Qoder inside tmux shows no typed English, but Enter sends

**Not reproduced.**
Qoder can't be typed into without an account.
Up to its input box, the core draws exactly what tmux draws (qoder-tmux final at the cut: box on row 13, cursor at [3, 13]).

From Qoder's bytes (`qoder-tmux.jsonl`), it starts with:

```
ESC[8m  ESC[?u  ESC]11;? ST  ESC[>q  ESC[>4;?m  ESC[c  ESC[2K CR ESC[0m
```

That is: conceal on, then queries for kitty keyboard flags, background colour, XTVERSION and modifyOtherKeys, then DA1, then erase and reset.
Later it sends `ESC[>0q`, `OSC 7501 ;?` and `ESC[c` again.
On exit it sends `ESC[>4m ESC[<u` (modifyOtherKeys off, pop kitty flags).
It hides the cursor at start (`ESC[?25l`) and shows it only on exit; the caret is drawn by Qoder itself.

### What Qoder's code says (1.1.92 bundle, read, not run)

Qoder's TUI is a fork of Gemini CLI's (its own Ink renderer).
At start-up it queries the terminal (`TerminalCapabilityManager`) and acts on the answers:

- **Keyboard.** It turns the kitty protocol on (`ESC[>1u`, flag 1 only) if the terminal answered `ESC[?u`, else modifyOtherKeys (`ESC[>4;2m`) if the answer to `ESC[>4;?m` was level 2 or more, else neither.
  Flag 1 and modifyOtherKeys level 2 both still send a plain letter as itself.
  In the lab's tmux (default `extended-keys`) it turned on neither.
- **CI.** It goes headless only for `CI=true`, `GITHUB_ACTIONS=true`, a stdin or stdout that is not a TTY, or `-p`.
  Headless means no input box at all.
  Its renderer has no is-in-ci frame suppression (no `CONTINUOUS_INTEGRATION` check anywhere in the bundle).
- **Colour.** With the theme left on `auto` (the default), it picks the theme from the `OSC 11` answer.
  It first takes a theme whose background equals the answer exactly, else a light or dark default by luminance.
  The light themes paint text in a fixed dark colour: "Default Light" is `#000000` on `#FFFFFF`, and others are `#444`, `#5c6166` or `#24292E`.
  It warns only when the theme was set by hand and does not match.

### Conclusion: H1-c, the colour

- **H1-a (CI) is ruled out.** A CI variable makes Qoder headless, with no input box. The user sees the box.
- **H1-b (keyboard) is unlikely.** Even when tmux reports modifyOtherKeys or kitty, Qoder asks for modes in which letters stay plain text.
  The lab sent plain letters to it through tmux with neither mode on.
- **H1-c (colour) fits every part of the report.**
  Inside tmux, tmux answers `OSC 11`, not Moshpit.
  It answers with a client terminal's colour, or black in control mode.
  If the answer is light (for example, the user's Mac terminal is light and is attached to the same tmux), Qoder picks a light theme and types in near-black.
  On Moshpit's dark background, that text is invisible, while Enter still sends.
  Over plain SSH, Moshpit answers itself, with the background it draws.
  The new test `the_background_query_answers_the_background_drawn` (`crates/term/tests/cases.rs`) checks this for a light and a dark palette, so nothing in the core needs fixing.

To confirm on the friend's machine:

1. Inside that tmux, run `printf '\e]11;?\e\\'; cat -v`, then press Enter. An answer like `rgb:ffff/ffff/ffff` (light) confirms the cause.
2. In Qoder, type `/theme` and pick a dark theme, or set `ui.theme` in its settings. If typed text shows again, the cause is confirmed.
3. Which terminal and colour scheme do they use on the desktop, and is that client attached to the same tmux session?

The fix belongs in Qoder: an `auto` theme should not trust `OSC 11` inside tmux, or it should honour `COLORFGBG`, which it already reads.
A user-side workaround: set Qoder's theme explicitly.

## Bug 2 (fixed): over mosh only, the cursor jumps up and down

**Reproduced with Moshpit's own transport over a 500 ms round trip.**
It is not reproduced with mosh-client.
The tests are `codex_mosh_tmux_rtt500_rs_cursor` (`crates/term/tests/cli_lab.rs`), `a_diff_from_an_older_held_state_applies` and `at_a_500_ms_round_trip_every_state_is_shown` (`crates/mosh/tests/transport.rs`).
The first two were red before the fix below.

What was recorded:

- **mosh-client:** claude, codex, gemini and qwen over `mosh` and `mosh-tmux`, with `udp_delay.py` at 500 ms round trip, and with long streamed answers (`LAB_LONG=40`). No cursor left the input row.
- **Moshpit's transport (`mosh_harness live`):** the same set through the relay.
  codex-mosh-tmux-rtt500-rs shows, while the answer streams in, one frame with the visible cursor at the end of "Line 2 of a longer answer" (row 15, col 51), 4 rows above the input row.
  That state's bytes end in `ESC[16;45Hslowly.ESC[?25h`.

Why:

1. Inside tmux, Codex turns DEC 2026 off. tmux redraws the pane with the cursor visible at whatever it writes last, then moves it back to the input.
   mosh-server snapshots that pane into states.
   A state taken mid-redraw has the cursor on the answer.
   mosh-client shows such a state too, but only until the next one, about 250 ms later.
2. Moshpit's transport shows each state for about 750 ms.
   The probe (`date` every 100 ms through the 500 ms relay) received a state every 250 ms, but the transport applied only 5 of every 15:

   ```
   0→1  0→2  0→3  0→4  1→5  1→6  1→7  5→8  5→9  5→10  8→11 …   (old→new, as mosh-server sent them)
   ok   gap  gap  gap  ok   gap  gap  ok   gap  gap   ok
   ```

   For a small diff, mosh-server diffs from the state the client last acked (`attempt_prospective_resend_optimization`: the resend is no bigger, or under 1000 bytes and at most 100 bytes more).
   That ack is a round trip old.
   mosh-client keeps every state since the acked one (`received_states`) and applies each diff to the state it names.
   `Transport` keeps one state (`applied_host_num`), so it drops every diff whose base it has moved past, and waits for its own ack to come round.
   So a round trip plus `ACK_DELAY` passes between frames, and whatever state happened to land stays up 3× longer than on mosh-client.
   On a phone link it is the same, scaled by its round trip.

The fix: `Transport` now keeps a screen per server state, as mosh-client does (`crates/mosh/src/screen.rs`: mosh's framebuffer, the part of its emulator that host bytes use, and its `Display::new_frame`).

- It holds every state from the server's `throwaway_num` to the newest (at most 64), with rows shared between states.
- It applies each diff to the state it names.
- It hands the pane the newest state.
  When the diff is from the state the pane shows (or it changes the size and repaints everything), the pane gets the server's own bytes.
  Otherwise it gets the difference between what the pane shows and the new state, rendered as mosh renders it.
  When a redraw hid frames, it gets a full repaint.
- A state that arrives after a newer one is held for diffs that name it, but never shown or acked.

After the fix, measured on the same path (`codex mosh-tmux --rtt 500 --moshpit`, `LAB_LONG=40`, three recordings):

| | before | after |
|---|---|---|
| states applied | 5 of 15 (`date` probe) | 55 of 55, 56 of 56, 56 of 56, no gaps |
| frames while the answer streams | ~755 ms apart | ~252 ms apart |
| states held at the end | 1 | 2–3 (about 79 KB of screens, shared rows counted once) |

A full 80×24 screen with every row distinct is about 77 KB (1,920 cells of 40 bytes).
A state that changes one row adds that row (about 3 KB) and its row table.

Earlier hypotheses, for the record: H2-a (no DEC 2026 over mosh) is the cause of the mid-redraw states.
H2-b (visibility rides on states) is part of it.
H2-c (prediction overlay) is not needed to show the jump.

## Fixed: Gemini torn frame (tmux, ssh)

Gemini 0.22.4 does not use DEC 2026.
Its Ink renderer erases the whole box (`ESC[2K ESC[1A` × 13) and redraws it in one write.
That write arrives in two reads, at the same millisecond.
Publishing after every read showed the box half erased, with no input row, for one frame.

## Frame coalescing

A pane no longer frames each read pass as it ends (`crates/term/src/coalesce.rs`, used by `pane.rs`).
It frames the output it parsed once one of these holds:

- the stream has been quiet for **4 ms** (`QUIET`) since the last read;
- **16 ms** (`CAP`, one display refresh) have passed since the first byte not framed yet, so a stream that never pauses still paints at display rate;
- the output ended with the end of a synchronized update (`ESC[?2026l`, nothing unsynchronized after it): the remote said the screen is whole, so it is framed at once.

Inside a synchronized update nothing is framed, as before (the 150 ms deadline still applies).
A backlog (every read a full 64 KB) still frames every 16 ms.

Why 4 ms: of the lab's gaps of 20 ms or less between two reads (979 of them, keys excluded), 76% are under 1 ms and 87% at most 4 ms.
A split write arrives back to back, so 4 ms catches it, while a typed echo waits at most 4 ms more.
That is a quarter of a 60 Hz frame: the echo shows one display refresh later only when it lands within 4 ms before a refresh.
CLIs that wrap their redraws in DEC 2026 (Claude Code, Codex, Qwen and Copilot outside tmux) pay nothing: the end of the update frames at once.
