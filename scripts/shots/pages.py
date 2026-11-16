#!/usr/bin/env python3
"""The store screenshots on GitHub Pages: check that a Screenshots run made
the whole set, and lay it out as a site.

    python3 -I scripts/shots/pages.py check <shots> [--device iphone|ipad]
    python3 -I scripts/shots/pages.py site <shots> <site>

<shots> is the run's output (the workflow's `shots` artifact): the composed
store images in <shots>/iphone-6.9/store/<locale>/<nn>-<shot>.png and
<shots>/ipad-13/store/<locale>/…, as scripts/shots/ci-capture.sh leaves them.

check: every locale with captions (scripts/shots/captions/), every slot of
both devices (or the one --device names: each device's job checks its
own), at the store's size: 1320 × 2868 for the iPhone; 2064 × 2752
for the iPad, or 2752 × 2064 for a landscape shot. Exit 0 only when all are
there; otherwise every gap is listed and the exit is 1. The publish job runs
only after a complete set, so a partial run leaves the last good set live.

site: check, then copy the set to <site>/shots/<iphone|ipad>/<locale>/
<nn>-<shot>.png (paths that stay put from run to run, for the READMEs to
link) and write an index page for each.
"""

import html
import json
import os
import shutil
import struct
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
CAPTIONS = os.path.join(ROOT, "scripts", "shots", "captions")
DEVICES = {"iphone": "iphone-6.9", "ipad": "ipad-13"}
SIZES = {"iphone": {(1320, 2868)}, "ipad": {(2064, 2752), (2752, 2064)}}
NAMES = {
    "en-US": "English", "zh-Hans": "简体中文", "zh-Hant": "繁體中文", "ja": "日本語",
    "de-DE": "Deutsch", "es-ES": "Español", "fr-FR": "Français", "ko": "한국어",
    "pt-BR": "Português (Brasil)",
}


def captions():
    """{locale: (language, {device: [(file name, headline)]})}, in the store's order."""
    out = {}
    for name in sorted(os.listdir(CAPTIONS)):
        if not name.endswith(".json"):
            continue
        with open(os.path.join(CAPTIONS, name), encoding="utf-8") as f:
            data = json.load(f)
        slots = {}
        for device in DEVICES:
            slots[device] = []
            for index, shot in enumerate(data[device], 1):
                bare = shot["shot"].split("-", 1)[1] if shot["shot"][:2].isdigit() else shot["shot"]
                slots[device].append((f"{index:02d}-{bare}.png", shot["title"]))
        out[name[:-5]] = (data["language"], slots)
    return out


def png_size(path):
    with open(path, "rb") as f:
        head = f.read(24)
    if len(head) < 24 or head[:8] != b"\x89PNG\r\n\x1a\n":
        return None
    return struct.unpack(">II", head[16:24])


def check(shots, devices=tuple(DEVICES)):
    gaps = []
    for locale, (_, slots) in captions().items():
        for device, folder in DEVICES.items():
            if device not in devices:
                continue
            for name, _ in slots[device]:
                path = os.path.join(shots, folder, "store", locale, name)
                if not os.path.isfile(path):
                    gaps.append(f"{device}/{locale}/{name}: missing")
                    continue
                size = png_size(path)
                if size not in SIZES[device]:
                    gaps.append(f"{device}/{locale}/{name}: {size[0]}×{size[1]}" if size
                                else f"{device}/{locale}/{name}: not a PNG")
    return gaps


PAGE = """<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Moshpit screenshots</title>
<style>
  :root {{ color-scheme: dark; --bg: #0b0d10; --fg: #e8eaed; --dim: #9aa0a6; --edge: #2a2f36; }}
  body {{ margin: 0; background: var(--bg); color: var(--fg);
         font: 16px/1.5 -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; }}
  main {{ max-width: 1200px; margin: 0 auto; padding: 24px 16px 48px; }}
  h1 {{ font-size: 24px; margin: 0 0 4px; }}
  p {{ color: var(--dim); margin: 0 0 16px; }}
  nav {{ display: flex; flex-wrap: wrap; gap: 8px 16px; margin-bottom: 24px; }}
  a {{ color: var(--fg); }}
  h2 {{ font-size: 20px; margin: 40px 0 8px; padding-top: 8px; border-top: 1px solid var(--edge); }}
  h3 {{ font-size: 15px; color: var(--dim); font-weight: 600; margin: 16px 0 8px; }}
  .grid {{ display: grid; gap: 12px; grid-template-columns: repeat(auto-fill, minmax(140px, 1fr)); }}
  .grid.ipad {{ grid-template-columns: repeat(auto-fill, minmax(240px, 1fr)); }}
  .grid img {{ width: 100%; height: auto; display: block; border-radius: 8px; border: 1px solid var(--edge); }}
</style>
</head>
<body>
<main>
<h1>Moshpit screenshots</h1>
<p>The App Store set, made by the Screenshots workflow and published here after every complete run
(nightly and on each version tag). Commit {commit}.</p>
<nav>{nav}</nav>
{sections}
</main>
</body>
</html>
"""


def site(shots, out):
    base = os.path.join(out, "shots")
    sections, nav = [], []
    for locale, (language, slots) in captions().items():
        nav.append(f'<a href="#{locale}" lang="{language}">{html.escape(NAMES.get(locale, locale))}</a>')
        parts = [f'<section id="{locale}" lang="{language}">',
                 f"<h2>{html.escape(NAMES.get(locale, locale))}</h2>"]
        for device, folder in DEVICES.items():
            target = os.path.join(base, device, locale)
            os.makedirs(target, exist_ok=True)
            parts.append(f"<h3>{'iPhone' if device == 'iphone' else 'iPad'}</h3>")
            parts.append(f'<div class="grid {device}">')
            for name, title in slots[device]:
                shutil.copyfile(os.path.join(shots, folder, "store", locale, name), os.path.join(target, name))
                parts.append(f'<a href="{device}/{locale}/{name}"><img src="{device}/{locale}/{name}" '
                             f'alt="{html.escape(title)}" loading="lazy"></a>')
            parts.append("</div>")
        parts.append("</section>")
        sections.append("\n".join(parts))
    commit = os.environ.get("GITHUB_SHA", "")[:7] or "(local)"
    with open(os.path.join(base, "index.html"), "w", encoding="utf-8") as f:
        f.write(PAGE.format(commit=html.escape(commit), nav=" ".join(nav), sections="\n".join(sections)))
    with open(os.path.join(out, "index.html"), "w", encoding="utf-8") as f:
        f.write('<!doctype html><meta charset="utf-8"><meta http-equiv="refresh" content="0; url=shots/">'
                '<title>Moshpit screenshots</title><a href="shots/">Moshpit screenshots</a>\n')


def main():
    args = sys.argv[1:]
    devices = tuple(DEVICES)
    if len(args) == 4 and args[0] == "check" and args[2] == "--device" and args[3] in DEVICES:
        devices, args = (args[3],), args[:2]
    if not args or len(args) != {"check": 2, "site": 3}.get(args[0]):
        print(__doc__.split("\n\n")[1], file=sys.stderr)
        return 64
    shots = args[1]
    gaps = check(shots, devices)
    if gaps:
        print(f"pages: the set isn't complete ({len(gaps)} gap(s)); nothing is published:", file=sys.stderr)
        for gap in gaps[:80]:
            print("  " + gap, file=sys.stderr)
        return 1
    total = sum(len(slots[d]) for _, slots in captions().values() for d in devices)
    which = "both devices" if len(devices) == 2 else f"the {devices[0]}"
    print(f"pages: the set is complete: {total} images, {len(captions())} locales, {which}")
    if args[0] == "site":
        site(shots, args[2])
        print(f"pages: laid out in {args[2]}/shots/")
    return 0


if __name__ == "__main__":
    sys.exit(main())
