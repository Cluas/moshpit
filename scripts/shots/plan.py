#!/usr/bin/env python3
"""The Screenshots workflow's plan: which shoot jobs a run needs.

    python3 -I scripts/shots/plan.py <devices> <locales> <shots>

<devices>  both | iphone | ipad
<locales>  all, or locales separated by commas or spaces (en-US,ko)
<shots>    all, or the store's slots by number (01,04,08: the order of each
           device's captions in scripts/shots/captions/); a slot a device
           doesn't have is left out for it

Prints GitHub output lines:

    matrix=[{"device", "group", "locales", "shots"}…]   one shoot job each
    full=true|false                                      the whole set asked

Each device's locales are dealt to jobs in groups, so that the five macOS runners
a public repository gets at once are used: the iPhone's eight shots take
about six minutes a locale, three locales a job; the iPad's two take about
three, with a long simulator boot first, five a job. A full run is then the
build and five shoot jobs side by side.
"""

import json
import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
CAPTIONS = os.path.join(ROOT, "scripts", "shots", "captions")
DEVICES = ("iphone", "ipad")
GROUP = {"iphone": 3, "ipad": 5}


def fail(message):
    print(f"plan: {message}", file=sys.stderr)
    sys.exit(64)


def words(text):
    return [w for w in text.replace(",", " ").split() if w]


def main():
    if len(sys.argv) != 4:
        print(__doc__.split("\n\n")[1], file=sys.stderr)
        return 64
    devices_in, locales_in, shots_in = (a.strip() or "all" for a in sys.argv[1:])
    known = sorted(f[:-5] for f in os.listdir(CAPTIONS) if f.endswith(".json"))
    slots = {}
    for device in DEVICES:
        with open(os.path.join(CAPTIONS, "en-US.json"), encoding="utf-8") as f:
            slots[device] = len(json.load(f)[device])

    if devices_in == "both":
        devices = list(DEVICES)
    elif devices_in in DEVICES:
        devices = [devices_in]
    else:
        fail(f"devices: both, iphone or ipad, not {devices_in!r}")

    if locales_in == "all":
        # The store's order: the captions' own, en-US first.
        locales = ["en-US"] + [l for l in known if l != "en-US"]
    else:
        locales = words(locales_in)
        unknown = [l for l in locales if l not in known]
        if unknown:
            fail(f"no captions for {', '.join(unknown)} (there are: {' '.join(known)})")

    wanted = None
    if shots_in != "all":
        try:
            wanted = sorted({int(w) for w in words(shots_in)})
        except ValueError:
            fail(f"shots: all, or slot numbers like 01,04,08, not {shots_in!r}")
        if not wanted or min(wanted) < 1 or max(wanted) > max(slots.values()):
            fail(f"shots: slots run from 01 to {max(slots.values()):02d}")

    matrix = []
    for device in devices:
        mine = list(range(1, slots[device] + 1)) if wanted is None else [s for s in wanted if s <= slots[device]]
        if not mine:
            continue
        # Dealt round the groups, so the slow ones (the Chinese keyboards)
        # land in different jobs.
        groups = -(-len(locales) // GROUP[device])
        for n in range(groups):
            matrix.append({
                "device": device,
                "group": str(n + 1),
                "locales": " ".join(locales[n::groups]),
                "shots": "all" if wanted is None else ",".join(f"{s:02d}" for s in mine),
            })
    if not matrix:
        fail("nothing to shoot: no device has the slots asked for")
    full = devices_in == "both" and locales_in == "all" and shots_in == "all"
    print("matrix=" + json.dumps(matrix, separators=(",", ":")))
    print("full=" + ("true" if full else "false"))
    return 0


if __name__ == "__main__":
    sys.exit(main())
