#!/usr/bin/env python3
"""Place a screenshot on the documentation page that describes it.

Kept as a script rather than done by hand because the pages come in pairs (an
English and a Chinese file per page) and the dimensions have to match the file
on disk — crop-shots.py changes them, and a stale width/height attribute
reserves the wrong space while the image loads.

    python3 scripts/place-doc-shots.py --check    # what is missing, place nothing
    python3 scripts/place-doc-shots.py            # place everything below
"""
from __future__ import annotations

import argparse
import pathlib
import re

from PIL import Image

ROOT = pathlib.Path(__file__).resolve().parent.parent
SITE = ROOT / "marketing" / "site"
ASSETS = SITE / "assets"
END = "<!--DOCSHELL:END-->"

# page slug -> (asset, heading id to place it after, alt text)
#
# The alt text describes what is in the frame, not what the page is about: a
# reader who cannot see the image should learn the same thing a sighted reader
# learns from it, which is the state of a screen.
PLACEMENTS: dict[str, tuple[str, str, str]] = {
    "docs-troubleshooting": (
        "31-connection-error.jpg", "s1",
        "A Connection Error alert reading \"The server closed the connection while "
        "setting up SSH. Check that the port really is an SSH server, and that a "
        "firewall or proxy isn't cutting the connection.\" — with the transport "
        "pill above it showing offline",
    ),
    "docs-keys": (
        "30-ssh-keys.jpg", "s1",
        "The SSH Keys screen in Settings, before any key exists: a THIS DEVICE · "
        "SECURE ENCLAVE section reading \"No device key yet — generate one with +\", "
        "an IMPORTED section, and the add control in the corner",
    ),
}


def place(slug: str, asset: str, after_id: str, alt: str, zh: bool) -> str | None:
    path = SITE / (f"{slug}.zh.html" if zh else f"{slug}.html")
    if not path.exists():
        return None
    s = path.read_text()
    if f'src="/{asset}"' in s:
        return None
    src = ASSETS / asset
    if not src.exists():
        return f"! asset missing: {asset}"
    with Image.open(src) as im:
        w, h = im.size
    fig = (
        f'\n<img class="phone rowshot" src="/{asset}" width="{w}" height="{h}" '
        f'loading="lazy" alt="{alt}">\n'
    )
    m = re.search(
        r'(<h3[^>]*id="' + re.escape(after_id) + r'"[^>]*>.*?</h3>\s*<p[^>]*>.*?</p>)',
        s, re.S,
    ) or re.search(r'(<p class="sub">.*?</p>)', s, re.S)
    if not m:
        return f"! no anchor in {path.name}"
    path.write_text(s[: m.end(1)] + fig + s[m.end(1):])
    return f"placed {asset} on {path.name}"


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true")
    args = ap.parse_args()

    for slug, (asset, after_id, alt) in PLACEMENTS.items():
        if args.check:
            src = ASSETS / asset
            have = "yes" if src.exists() else "NO ASSET"
            page = SITE / f"{slug}.html"
            body = page.read_text().split(END)[-1] if page.exists() else ""
            already = f'src="/{asset}"' in body
            print(f"  {slug:24s} asset={have:9s} already placed={already}")
            continue
        for zh in (False, True):
            r = place(slug, asset, after_id, alt, zh)
            if r:
                print("  " + r)


if __name__ == "__main__":
    main()
