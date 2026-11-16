#!/usr/bin/env python3
"""The Screenshots workflow's plan: which shoot jobs a run needs.

    python3 -I scripts/shots/plan.py <devices> <locales> <shots> [<jobs>]

<devices>  both | iphone | ipad
<locales>  all, or locales separated by commas or spaces (en-US,ko)
<shots>    all, or the store's slots by number (01,04,08: the order of each
           device's captions in scripts/shots/captions/); a slot a device
           doesn't have is left out for it
<jobs>     how many shoot jobs at most (default 3): the account's macOS
           runners are five at once, CI's included, and a run must leave
           CI one

Prints GitHub output lines:

    matrix=[{"device", "group", "locales", "shots"}…]   one shoot job each
    full=true|false                                      the whole set asked
    parallel=<jobs>                                      the matrix's max-parallel

Each device gets a job, and each job left goes to the device whose jobs
would run longest, if one more shortens them: a full run is the iPhone in
three jobs and the iPad in one at four jobs (about 20 and 25 minutes), two
and one at three (about 30 and 25).
"""

import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from slots import TESTS, external_has  # noqa: E402

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
CAPTIONS = os.path.join(ROOT, "scripts", "shots", "captions")
EXTERNAL = os.path.join(ROOT, "scripts", "shots", "external")
DEVICES = ("iphone", "ipad")
# Minutes, from 43816020313's jobs: a job's own start (the runner, the
# staged host, and most of it the simulator's boot) and each shot a locale
# takes. A shot from scripts/shots/external takes none.
START = 5.5
SHOT = {"iphone": 0.8, "ipad": 1.1}


def fail(message):
    print(f"plan: {message}", file=sys.stderr)
    sys.exit(64)


def words(text):
    return [w for w in text.replace(",", " ").split() if w]


def deal(cost, locales, jobs):
    """How many jobs each device gets: one each, then each job left to the
    device whose jobs run longest, while one more shortens them."""
    groups = {device: 1 for device in cost}

    def span(device, n):
        return START + -(-locales // n) * cost[device]

    for _ in range(jobs - len(groups) if groups else 0):
        longest = max(groups, key=lambda d: span(d, groups[d]))
        if span(longest, groups[longest] + 1) >= span(longest, groups[longest]):
            break
        groups[longest] += 1
    return groups


def main():
    if len(sys.argv) not in (4, 5):
        print(__doc__.split("\n\n")[1], file=sys.stderr)
        return 64
    devices_in, locales_in, shots_in = (a.strip() or "all" for a in sys.argv[1:4])
    jobs_in = sys.argv[4].strip() if len(sys.argv) == 5 else "3"
    if not jobs_in.isdigit() or int(jobs_in) < 1:
        fail(f"jobs: a number from 1, not {jobs_in!r}")
    jobs = int(jobs_in)
    known = sorted(f[:-5] for f in os.listdir(CAPTIONS) if f.endswith(".json"))
    with open(os.path.join(CAPTIONS, "en-US.json"), encoding="utf-8") as f:
        names = {device: [c["shot"] for c in captions] for device, captions in json.load(f).items()
                 if device in DEVICES}
    slots = {device: len(names[device]) for device in DEVICES}

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

    mine, cost = {}, {}
    for device in devices:
        mine[device] = list(range(1, slots[device] + 1)) if wanted is None else [s for s in wanted if s <= slots[device]]
        if mine[device]:
            # A locale's minutes: the shots it takes (en-US's externals
            # stand for every locale's).
            taken = [names[device][s - 1] for s in mine[device]]
            cost[device] = SHOT[device] * sum(1 for shot in taken
                                              if shot in TESTS and not external_has(EXTERNAL, "en-US", shot))
    matrix = []
    for device, groups in deal(cost, len(locales), jobs).items():
        # Dealt round the groups, so the slow ones (the Chinese keyboards)
        # land in different jobs.
        for n in range(groups):
            matrix.append({
                "device": device,
                "group": str(n + 1),
                "locales": " ".join(locales[n::groups]),
                "shots": "all" if wanted is None else ",".join(f"{s:02d}" for s in mine[device]),
            })
    if not matrix:
        fail("nothing to shoot: no device has the slots asked for")
    full = devices_in == "both" and locales_in == "all" and shots_in == "all"
    print("matrix=" + json.dumps(matrix, separators=(",", ":")))
    print("full=" + ("true" if full else "false"))
    print(f"parallel={jobs}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
