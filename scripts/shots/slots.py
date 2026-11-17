#!/usr/bin/env python3
"""Which ShotsUITest tests a shoot job runs, and which shots it still lacks.

    python3 -I scripts/shots/slots.py tests <device> <only> <external> <locale…>
    python3 -I scripts/shots/slots.py missing <device> <only> <out> <external> <locale…>

<device>    iphone | ipad
<only>      all, or the store's slots by number (01,05): the order of the
            device's captions (scripts/shots/captions/<locale>.json)
<external>  SHOTS_EXTERNAL (taken elsewhere: the lock screen and the island),
            or "" for none
<out>       the capture's shots, <out>/<locale>/<shot>.png

tests: the tests the asked slots need, one a line; none for a slot every
asked locale has from <external> (compose's own lookup: the locale's, the
folder's, en-US's).

missing: "<test> <locale>" for each asked shot neither <out> nor <external>
has, for the second pass.
"""

import json
import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
CAPTIONS = os.path.join(ROOT, "scripts", "shots", "captions")
# The shots taken in landscape (both iPad split-view shots): their capture
# reads landscape and their store frame is landscape (B842).
LANDSCAPE = {"panes", "home-terminal"}
TESTS = {
    "01-lock": "test91Lock", "02-pane": "test02Pane", "03-home": "test03Home",
    "04-island": "test90Island", "05-overview": "test05Overview",
    "06-preview": "test06Preview", "07-pet": "test07Pet", "08-keyboard": "test08Keyboard",
    "panes": "test05Overview", "home-terminal": "test10HomeAndTerminal",
}


def asked(device, only, locale):
    """The locale's shots for the device, in the store's order, as asked."""
    with open(os.path.join(CAPTIONS, f"{locale}.json"), encoding="utf-8") as f:
        shots = [c["shot"] for c in json.load(f)[device]]
    if only == "all":
        return shots
    slots = {int(s) for s in only.replace(",", " ").split()}
    return [shot for n, shot in enumerate(shots, 1) if n in slots]


def external_has(external, locale, shot):
    if not external:
        return False
    return any(os.path.isfile(os.path.join(external, *p, f"{shot}.png")) for p in ([locale], [], ["en-US"]))


def main():
    if len(sys.argv) < 3 or sys.argv[1] not in ("tests", "missing"):
        print(__doc__.split("\n\n")[1], file=sys.stderr)
        return 64
    if sys.argv[1] == "tests":
        device, only, external, *locales = sys.argv[2:]
        tests = []
        for locale in locales:
            for shot in asked(device, only, locale):
                test = TESTS.get(shot)
                if test and not external_has(external, locale, shot) and test not in tests:
                    tests.append(test)
        print("\n".join(tests))
        return 0
    device, only, out, external, *locales = sys.argv[2:]
    for locale in locales:
        for shot in asked(device, only, locale):
            have = os.path.isfile(os.path.join(out, locale, f"{shot}.png")) or external_has(external, locale, shot)
            if not have and shot in TESTS:
                print(TESTS[shot], locale)
    return 0


if __name__ == "__main__":
    sys.exit(main())
