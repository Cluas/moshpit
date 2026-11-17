# Live Core E2E (E2E-1)

The ignored live integration tests across the workspace exercise real processes
on loopback: private OpenSSH `sshd` instances, `mosh-server`, `herdr`, Go
`push-relay`, and source-built `tmux` (`3.6a` and `3.8`).

## Running on Ubuntu / in CI

The GitHub Actions workflow [`.github/workflows/e2e.yml`](../../.github/workflows/e2e.yml)
runs the full matrix (`3.6a` and `3.8` x `path` and `homebrew`) on `ubuntu-24.04`:

1. `scripts/e2e/host-setup.sh`: installs `openssh-server`, `mosh`, `zsh`,
   `python3`, `lsof`, `sudo`, pinned `Go 1.24.4`, and pinned `herdr v0.9.5`
   into `/opt/homebrew/bin/herdr` (without any distro `tmux` package).
2. `scripts/e2e/install-nextest.sh`: installs the pinned `cargo-nextest`.
3. `scripts/e2e/run.sh <3.6a|3.8> <path|homebrew>`:
   - `path`: symlinks `tmux` at `/usr/local/bin/tmux` (on non-interactive `sshd` PATH).
   - `homebrew`: symlinks `tmux` only at `/opt/homebrew/bin/tmux` so `sshd` exec
     channels must rely on the core's probed PATH (covering B788/B850 regressions).
   - Runs `live`, `live_*`, `openssh`, `handshake_stall`, and `e2e2` (excluding
     the local benchmark `live_window`) with `--profile e2e` (`retries = 0`,
     no hidden flakes) and `MOSHPIT_E2E_FAULTS=1`.
4. `scripts/e2e/verdict.py`: parses `$CARGO_TARGET_DIR/nextest/e2e/junit.xml`
   and exits non-zero if any test outside `scripts/e2e/quarantine.txt` fails or
   if zero tests executed.

## Environment Switches

- `MOSHPIT_E2E_FAULTS=1`: enables fault-injection and long suspension tests
  (`Relay::cut`/`stall`, `ProxiedUdp::faults.blackhole`, `LinkFault`,
  `live_resume`). Set by default in `scripts/e2e/run.sh` and CI; bare local
  `--ignored` runs without it skip only those injected-fault cases.
- `E2E_JOBS`: nextest concurrency (default `2`).
- `E2E_FILTER`: optional extra nextest filterset to narrow a local run.
- `CARGO_TARGET_DIR`: cargo target directory; `run.sh` exports it so `nextest`
  and `verdict.py` read the same `junit.xml`.

## Quarantine Rules (`scripts/e2e/quarantine.txt`)

- Only tracked, open bugs with an explicit bug number and explanation belong in
  `scripts/e2e/quarantine.txt`.
- Quarantined tests still execute on every leg and their outcome is reported,
  but their failure does not turn the job red until the fix lands and removes
  the entry.
- Never run tests as `root` in containers when testing SFTP permission errors
  (`openssh::private_sshd_sftp_says_why_and_a_commands_stderr_stays_apart`), as
  UID 0 bypasses POSIX `0o000` DAC checks.
