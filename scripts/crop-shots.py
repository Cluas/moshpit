#!/usr/bin/env python3
"""Crop phone screenshots to the part that has something on it.

The captures are full 1320×2868 device frames, and most screens do not fill
one. A design review measured the terminal screenshots on the landing page at
1.3%–7% ink: the Mosh shot was five lines of git log above ~530px of nothing.
Shown at a fixed width in a page, that reads as a broken image, and it sits
directly under a line claiming every screen is the shipping app.

So: find the last row that actually contains something, cut there, and let the
image be short and dense instead of tall and empty. Width is never cropped —
a phone screenshot narrower than a phone stops looking like one.

    python3 scripts/crop-shots.py                 # report only
    python3 scripts/crop-shots.py --write         # write marketing/site/assets

The threshold is deliberately generous about what counts as content: the app's
background is near-black (#05060A-ish) with a faint grid, so "brighter than the
background by a margin" finds real UI without being fooled by the grid.
"""
from __future__ import annotations

import argparse
import pathlib
import sys

import numpy as np
from PIL import Image

ROOT = pathlib.Path(__file__).resolve().parent.parent
SRC = ROOT / "marketing" / "shots"
DST = ROOT / "marketing" / "site" / "assets"

# Rows dimmer than this (0-255, mean of the brightest 2% of pixels in the row)
# are treated as empty background rather than content.
INK = 46
# Keep a little breathing room below the last content row.
PAD = 28
# Never crop above this fraction of the device — a shot cropped to a sliver
# stops reading as a phone.
MIN_KEEP = 0.32

# Captures that exist but must not be published, with the reason. Checked by
# opening each one rather than trusting the filename: a capture script that
# taps its way through a UI will happily save a screenshot of the wrong screen
# and report success.
WITHHELD = {
    "09-mosh": "pane is a bare prompt — the staged run never reaches a plain "
               "login shell, and the pasteboard route did not land",
    "31-connection-error": "shows a healthy home screen with a LIVE connection; "
                           "the port override did not take, so no error occurred",
    "32-scrollback": "identical to 07-tmux-terminal to within 0.02% of pixels — "
                     "the swipe did not scroll the pane",
    "33-paste": "identical to 07-tmux-terminal — the paste control never opened; "
                "an earlier attempt captured iOS's own permission alert instead",
}


def content_bottom(img: Image.Image) -> int:
    a = np.asarray(img.convert("L"), dtype=np.uint8)
    h = a.shape[0]
    # Per row, how bright is the brightest sliver? A row of UI has some.
    k = max(1, a.shape[1] // 50)
    rows = np.sort(a, axis=1)[:, -k:].mean(axis=1)
    lit = np.flatnonzero(rows > INK)
    if lit.size == 0:
        return h
    return min(h, int(lit[-1]) + PAD)


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--write", action="store_true", help="write cropped JPEGs")
    ap.add_argument("--quality", type=int, default=82)
    args = ap.parse_args()

    if not SRC.exists():
        sys.exit(f"no source shots at {SRC}")

    for png in sorted(SRC.glob("*.png")):
        if png.stem in WITHHELD:
            print(f"  {png.stem:24s} WITHHELD — {WITHHELD[png.stem]}")
            continue
        img = Image.open(png)
        w, h = img.size
        bottom = max(content_bottom(img), int(h * MIN_KEEP))
        saved = (1 - bottom / h) * 100
        out = DST / f"{png.stem}.jpg"
        note = f"  {png.stem:24s} {w}×{h} → {w}×{bottom}  ({saved:4.1f}% of height dropped)"
        if args.write:
            cropped = img.convert("RGB").crop((0, 0, w, bottom))
            cropped.save(out, "JPEG", quality=args.quality, optimize=True, progressive=True)
            note += f"  → {out.name} {out.stat().st_size // 1024}KB"
        print(note)


if __name__ == "__main__":
    main()
