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

Versions: tmux 3.6a, mosh 1.4.0, Claude Code 2.1.327, Codex 0.166.0, Gemini CLI 0.22.4, Qwen Code 0.28.0, Copilot CLI 1.0.8, Qoder 1.1.92, CodeBuddy Code 2.185.0, Cursor Agent 2025.09.18-7ae6800, OpenCode 1.19.6. Size 80×24.
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
| codebuddy-tmux | pass | pass | pass | — |
| codebuddy-ssh | pass | pass | pass | — |
| codebuddy-mosh | pass | pass | pass | pass |
| codebuddy-mosh-tmux | pass | pass | pass | pass |
| cursor-tmux | pass | pass | pass | — |
| cursor-ssh | pass | pass | pass | — |
| cursor-mosh | pass | pass | pass | pass |
| cursor-mosh-tmux | pass | pass | pass | pass |
| opencode-tmux | pass | pass | pass | — |
| opencode-ssh | pass | pass | pass | — |
| opencode-mosh | pass | pass | pass | pass |
| opencode-mosh-tmux | pass | pass | pass | pass |
| codex-mosh-tmux-rtt500-rs | — (no tmux reference) | lags (echo over 500 ms) | pass | — |

Cursor check: claude-tmux, claude-mosh, codex-tmux, codex-mosh, codex-mosh-tmux, codex-mosh-tmux-rtt500-rs, and every codebuddy, cursor and opencode recording pass.
codex-mosh-tmux-rtt500-rs was red until bug 2 (below) was fixed.

Totals: 140 green.

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
- **CodeBuddy 2.185.0**: all green, with a real (fake) model reply.
  No login: `CODEBUDDY_BASE_URL=<fake>/v1`, `CODEBUDDY_API_KEY` and `CODEBUDDY_MODEL` together send it to OpenAI chat completions (streamed) on `fake_model.py`.
  A scratch `~/.codebuddy/settings.json` with `{"trustedDirectories": [<work>]}` skips the folder-trust dialog.
  One message makes three POSTs (the second-to-last is a prompt suggestion).
  It still tries to send telemetry to the internet ("Report Service" TLS errors in its log), so it is not hermetic.
- **Cursor Agent 2025.09.18-7ae6800**: all green at the input box; it types, but there is no model reply.
  What it takes to get there without an account:
  - `-e <fake>`: `CURSOR_API_ENDPOINT` alone is not enough, because the `-e` option's default (`https://api2.cursor.sh`) wins for the login.
  - `CURSOR_API_KEY`: exchanged at `POST /auth/exchange_user_api_key` for an access and refresh token. `fake_model.py` returns an unsigned JWT; Cursor reads only its `exp`.
  - A private copy of the bundle (`/tmp/clisrc/cursor-agent-2025.09.18-lab`, outside the repo) with the macOS keychain token store turned off (`if(p7t()==="darwin")return new fke(e)` → `if(0)...`), so the tokens go to `~/.cursor/auth.json` in the scratch home.
    The stock build writes them with `security add-generic-password` into the login keychain, which hangs under the lab and would touch the real keychain.
  - Every Connect-RPC unary call (`/aiserver.v1.*`: privacy mode, team settings, models, default model, server config, download URL) gets an empty protobuf message; analytics `TrackEvents` gets `{}`.
  - A trusted workspace: the empty file `~/.cursor/projects/<work path, non-alphanumerics as ->/.workspace-trusted`.
  - A model in `$XDG_CONFIG_HOME/cursor/cli-config.json` (`agent.v1.ModelDetails` JSON).
    Without it the empty model list makes Cursor print "No model found" and a Node `PromiseRejectionHandledWarning` to stderr in the middle of its first frames.
    That moved the box down a row compared with tmux's own capture, but the core still matched tmux's replay of the same bytes: not a Moshpit bug.
  - No reply: Enter posts to `/agent.v1.AgentService/RunSSE` and `/aiserver.v1.BidiService/BidiAppend` (with `network.useHttp1ForAgent`), Cursor's own protobuf agent protocol.
    Stubbing that is out of scope, so the typed and steady checks cover typing only.
- **OpenCode 1.19.6**: all green, with a real (fake) model reply.
  It is a native binary (npm `opencode-ai` pulls in `opencode-darwin-arm64`), installed with `npm install --prefix /tmp/clisrc/opencode opencode-ai`; its TUI is its own renderer, not Ink.
  It uses the alternate screen, wraps each redraw in DEC 2026, and shows the real cursor in its input box.
  No login: a scratch `$XDG_CONFIG_HOME/opencode/opencode.json` defines a provider `lab` (`npm: "@ai-sdk/openai-compatible"`, `options.baseURL` = `fake_model.py`/v1, a dummy key) as the only enabled one, with `autoupdate: false` and `share: "disabled"`.
  `XDG_DATA_HOME`, `XDG_CACHE_HOME` and `XDG_STATE_HOME` point into the scratch home too.
  The env turns off update checks, the models.dev fetch, LSP downloads, default plugins and reading Claude Code's files (`OPENCODE_DISABLE_*`).
  One message makes two streamed POSTs to `/v1/chat/completions` (the title, then the answer).
  At start it runs `npm install @opencode-ai/plugin` in its config dir; `npm_config_registry=http://127.0.0.1:9/` makes that fail fast.
  With it, `lsof` shows no connection off 127.0.0.1 during a run, and the screen is unchanged.
  The cursor check ignores indent: before the first message the box is centred (3 columns in), and after it the session view draws the box 2 columns in.
  The cursor is in the box in both, so this is a layout change, not a jump.
- **Skipped**:

  | CLI | reason |
  |---|---|
  | iFlow 0.5.19 | the published tarball fails with `ERR_MODULE_NOT_FOUND` |

### Gotchas found while building the lab

- **Never set `CI`, even to an empty string.**
  Ink-based CLIs (Gemini, Qwen, Claude) treat any `CI` variable as a CI log.
  They print static output and never draw the input box, yet Enter still submits.
  That is exactly the symptom of bug 1 (see H1-a).
- mosh-server starts the command in `$HOME`.
  The driver wraps the command as `sh -c 'cd "$0" && exec "$@"' work ...`, otherwise CLIs see the wrong directory (a trust dialog, or Codex exits).
- tmux 3.6a answers `OSC 10 ;?` and `OSC 11 ;?` itself, with black on black, when the client is in control mode and the outer terminal's colours are unknown.

## Bug 1: Qoder inside tmux shows no typed English, but Enter sends

**Not reproduced**, including with typing (below).
Before the typing lab, the core draws exactly what tmux draws (qoder-tmux final at the cut: box on row 13, cursor at [3, 13]).

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

### Conclusion so far: not in Moshpit's tmux path; cause still open

A later report (the friend's screen recording) adds: while typing, the input box's cursor **never moves**, and Enter still sends.
That rules H1-c out as the whole story: invisible text would still move Qoder's caret.
It points at the incremental frames never reaching the screen, while Enter's own repaint (below) shows the result.

**Reaching Qoder's input box in the lab.**
Qoder refuses a custom provider unless the account's server-side switch allows it (`allow_byok >= 2`), and `/login` needs a real account.
The lab runs a private copy of the 1.1.92 bundle with that one check disabled: `if(!0!==s)throw new mD(...)` becomes `if(0)throw new mD(...)`.
The renderer is untouched.
The rest of the recipe:

- `HOME` is a scratch directory whose `.qoder/settings.json` has a provider pointing at `http://127.0.0.1:9/v1` (nothing listens there), `security.auth.useExternal: true`, `folderTrust.enabled: false` and `general.disableAutoUpdate: true`.
- The run passes `--fake-responses <empty file>`.
- Keep `TMUX`, `TMUX_PANE` and `TERM_PROGRAM` from the pane.
  An `env -i` that drops them hides the real path: with `TERM_PROGRAM=tmux`, Qoder turns on the kitty protocol (`ESC[>1u`) and modifyOtherKeys (`ESC[>4;2m`) at start.
- Qoder still makes network calls (telemetry, changelog), so it is not a hermetic lab.

**The probe.**
`crates/tmux/examples/cli_type_probe.rs` runs that through the phone's real path:

- a private `tmux -L` server;
- the real `Controller` in control mode, at the phone's grid, launching from the shell with `view_return_sent`;
- keys sent one at a time with `send_input` (`send-keys -H`);
- every `%output` byte fed into a real `moshpit_term` core, with the core's query answers sent back as the app sends them.

The probe can delay the answers (`QPROBE_DELAY`) or drop them (`QPROBE_NOANSWER`).
It ends by printing tmux's `capture-pane` next to the core's rows and cursor.

**What was tried. Every case typed correctly,** with the core's screen and cursor exactly matching tmux's (`> hello world`, cursor after it, 46×30 and 80×24):

- the phone attached before Qoder started, and after;
- the dark and the light theme;
- tmux `extended-keys` left off, `on`, `always`, and `csi-u` format;
- the core's answers sent at once, after 250 ms, or never;
- Qoder's `OSC 11` background poll set to every second (`ui.terminalBackgroundPollingInterval: 1`; it is 60 s by default), so polls and their double answers land mid-typing.

**What Qoder does when a key arrives, from its bytes:**

- It hides the real cursor (`cursor_flag` 0) and draws its own caret as a coloured cell.
- Each keystroke repaints with a small diff frame made only of cursor-relative moves (CUU/CUD/CUF/CUB, CR, CRLF) from where it parked the cursor, which is the caret.
- Inside tmux it uses no synchronized output: `TMUX` set means off, and neither tmux's nor Moshpit's XTVERSION is on its list.
- A key received while it believes the terminal lost focus refocuses it.
  Being unfocused only slows its animation tick.

**Why Enter "fixes" it.**
`view_return_sent` arms a repaint from tmux's `capture-pane` 450 ms after Return.
So any frame the screen missed while typing shows up after Enter.
That fits the report whatever dropped the frames.

**Two answers to every query.**
On a tmux pane, tmux answers Qoder's queries itself:

- `OSC 11` with black;
- XTVERSION `tmux 3.6a`;
- DA1 `ESC[?1;2;4c`.

Moshpit's core answers too, one round trip later, through `send-keys -H`:

- kitty flags `ESC[?0u`;
- `OSC 11` with the background it draws;
- XTVERSION `Moshpit 0.1.0`;
- DA1;
- focus-in `ESC[I` when the program enables focus reporting.

Qoder's queries finish on tmux's DA1, so Moshpit's answers arrive unasked.
In every case above, Qoder parsed them without typing anything into the box.
Moshpit's later `OSC 11` may also be the one its background poll acts on; this is not checked.
Herdr panes have their own answering turned off (`answersQueries`); tmux panes do not.
This is left as is, because nothing showed it doing harm.

**H1-a, H1-b and H1-c.**

- H1-a (CI) stays ruled out.
- H1-b (keyboard modes) is now tested against tmux's real encodings, and is ruled out for plain letters.
- H1-c (colour) is real: on a dark screen, the light theme's text and caret are near-invisible.
  It does not explain a caret that never moves.

**To ask the friend:**

1. Which build: App Store 1.0.8, which uses the Swift tmux path and SwiftTerm, or a Canary (Rust)?
   The probe covers only the Rust path.
2. After typing a few letters without pressing Enter, does the text show up if you change the screen? For example, rotate the phone, switch tmux windows and back, or leave the app and come back.
   If it does, Qoder sent the frames and Moshpit dropped them.
3. In the same tmux, does a plain shell echo as you type? Does Claude Code's input box?
4. `tmux -V`, the output of `tmux show -g status pane-border-status extended-keys`, `tmux show -s extended-keys`, and `tmux list-clients`.
   Is another device or desktop terminal attached to the same session?
5. Inside that tmux, run `printf '\e]11;?\e\\'; cat -v` and press Enter: light or dark answer?
   Does `/theme` with a dark theme change anything?
6. A keyboard other than Apple's English one (a pinyin keyboard in English mode, or a third-party keyboard)?
   A composition shows as an underlined label and is sent only on commit; Enter commits it.

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
