# `crates/install` Swift Equivalence Harness

`equivalence.py` cross-checks the Rust installer crate (`crates/install`) against the frozen Swift install sources from commit `81c121a1` kept in `crates/install/harness/swift/frozen/`, plus committed fixtures under `crates/install/tests/fixtures/swift/`.

## Running

```sh
python3 crates/install/harness/equivalence.py run         # all 16 seeds (40,000 cases) + committed + divergences
python3 crates/install/harness/equivalence.py run --quick # 2 seeds (5,000 cases) + committed + divergences
```

Environment: respects `CARGO_TARGET_DIR` (defaults to `<repo>/target`).

## Two-tier Gate

### 1. Hard Gate (exit 1 on any mismatch)

- **Seeded invariant categories** (must match frozen Swift `81c121a1` byte-for-byte across all seeds):
  - `agent`, `caps`, `conf`, `facts`, `install`, `manifest`, `mread`
  - `merge codex`, `remove codex`
  - `cmd backup`, `cmd path`, `cmd prune`, `cmd quote`, `cmd read`, `cmd remove`, `cmd safe`, `cmd sha`, `cmd socket`
- **Committed cases**: all 387 cases in `tests/fixtures/swift/cases.txt` must match `cases.swift.txt` byte-for-byte on the Rust side.
- **Documented divergences**: all 23 cases in `tests/fixtures/swift/divergences.txt` must match `divergences.rust.txt` on the Rust side, and either match `divergences.swift.txt` while differing from Rust (open fixes), match both sides on unchanged ported fixes, or belong to the evolved categories below.

### 2. Evolved Informational Categories (reported per run with B-numbers)

Rust intentionally diverged from the frozen Swift snapshot (`81c121a1`) on the following categories after Swift-side install moved to Rust:

- `probe`: **B875** — POSIX `dash` `command -v` takes only its first operand; Rust probes each binary separately.
- `scripts`: **B443, B566, B633, B637, B700, B784** — host scripts evolved from v1 to v4 (manifest locking, per-installation pairing records, muting switches-off pairings).
- `cmd preflight`: **B374** — adds agent directory/binary probes.
- `cmd write`: **B351, B379** — writes to a same-directory temp file, replaces atomically, and follows symlink targets up to 40 hops.
- `merge {claude,codebuddy,factory,gemini,qoder,qwen}`, `remove {claude,codebuddy,factory,gemini,qoder,qwen}`: **B356** (preserves user hooks sharing a hook group) and **B375** (preserves existing JSON formatting and key order).
- `flow {hooks,pairing,push,stamp,status,unhooks,uninstall}`: **B351–B355, B374, B379, B443, B566, B633, B637, B700, B784** — combine the evolved shell commands, JSON layout preservation, manifest locking, and v4 scripts above.
