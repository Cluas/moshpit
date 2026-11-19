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

Totals: 79 green, 0 red.

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

Hypotheses, most likely first:

- **H1-a. The input renders as in a non-interactive or CI terminal.**
  The Ink family shows this exact symptom when it decides it is not interactive: no input echo, Enter still submits.
  The lab hit it with Gemini because of `CI=""`.
  Check the user's tmux environment for `CI`, `CONTINUOUS_INTEGRATION` or `BUILD_NUMBER` (`tmux show-env -g`), and Qoder's own TTY checks.
  This would explain why it happens inside tmux only: tmux's global environment can carry variables that the plain SSH login doesn't.
- **H1-b. The keyboard protocol.**
  Qoder asks for kitty flags (`ESC[?u`) and modifyOtherKeys (`ESC[>4;?m`).
  Inside tmux, tmux answers, not Moshpit.
  If, given those answers, Qoder expects letters as key events (CSI u), then plain letters arrive as text it does not insert, while `\r` still submits.
  Moshpit's core does encode kitty and CSI u (`crates/term/src/input.rs`).
  Check this with tmux `extended-keys` on and off, and with the bytes Moshpit sends for a letter after Qoder starts (the input-trace rig).
- **H1-c. Colour.**
  tmux in control mode answers `OSC 11 ;?` with black (see Gotchas).
  If Qoder picks the typed text's colour from the background it was told, and Moshpit actually draws a light background, the text is drawn in an invisible colour.
  The harness counts fg = bg as blank, so a fixture would catch this, but Qoder exits before any typed text exists.
- **Ruled out for the core:** concealed text left on.
  `ESC[8m` is reset by `ESC[0m` within the first 30 bytes, and the box matches tmux.

To settle bug 1, record one real Qoder session on the user's machine: `drive.py`'s tmux path with the user's own account, outside this lab.
Then compare the bytes after typing `h` with tmux's capture.

## Bug 2: over mosh only, the cursor jumps up and down

**Not reproduced on loopback.**
Every mosh fixture is steady, with Moshpit's prediction off and on, and with mosh-client's own prediction.

What the fixtures do show:

| recording | DEC 2026 sync blocks | `?25l` / `?25h` | chunks |
|---|---|---|---|
| codex-ssh | 70 | 25 / 69 | 107 |
| codex-mosh | **0** | 12 / 12 | 31 |
| qwen-ssh | 48 | 35 / 36 | 197 |
| qwen-mosh | **0** | 25 / 25 | 66 |
| copilot-ssh | 37 | 1 / 0 | 80 |
| copilot-mosh | **0** | 2 / 1 | 48 |

Hypotheses:

- **H2-a. mosh drops synchronized output (main hypothesis).**
  Over tmux and ssh, Codex, Qwen and Copilot wrap each redraw in `ESC[?2026h … ESC[?2026l`.
  The core holds publishing during a sync block (`sync_deadline`), so no frame ever shows the cursor halfway through a redraw.
  mosh-server parses the bytes into its own framebuffer and sends screen states, so the markers never reach Moshpit (0 in every mosh fixture).
  A state taken while the CLI is mid-redraw has the cursor at the redraw point, usually the status line or the bottom of the box.
  The next state has it back in the input.
  On a viewer that looks like the cursor jumping down and up.
  On loopback mosh-server collects for about 8 ms and the CLI finishes its write first, so the lab sees whole states.
  Over a real link (send interval tied to RTT, a CLI that writes in several pieces) it does not.
  Next experiment: put a delay relay (the 510 ms rig) between mosh-client and mosh-server, and rerun `codex mosh` and `qwen mosh`.
  Steady should go red.
- **H2-b. Cursor visibility rides on mosh states.**
  Each mosh state carries cursor visibility (`?25l` / `?25h` above, one pair per state).
  A state sampled while the CLI had the cursor shown at a parked position draws it there for one frame.
  This is the same mechanism as H2-a, seen through visibility.
- **H2-c. Prediction overlay at a parked cursor.**
  Moshpit's predictive echo places its guess at the cursor mosh reports.
  If that cursor is parked (H2-a), the guess and its cursor are drawn there until the echo arrives.
  The predicted check passes on loopback.
  It needs the delayed link from H2-a to show a difference.
- A possible fix, for later: over mosh, do not move a visible cursor off the row it was on for less than one RTT (debounce cursor moves).
  Or ask the CLIs to keep the cursor hidden during redraws.
  Both are outside this lab.

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
