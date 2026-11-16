#!/usr/bin/env python3
"""The store screenshots on GitHub Pages: check that a Screenshots run made
the whole set, fetch the set that is live, and lay a set out as a site.

    python3 -I scripts/shots/pages.py check <shots> [--device D] [--locales L] [--slots S]
    python3 -I scripts/shots/pages.py fetch <site URL> <shots>
    python3 -I scripts/shots/pages.py site <shots> <site>

<shots> is the run's output (the workflow's `shots` artifact): the composed
store images in <shots>/iphone-6.9/store/<locale>/<nn>-<shot>.png and
<shots>/ipad-13/store/<locale>/…, as scripts/shots/ci-capture.sh leaves them.

check: every locale with captions (scripts/shots/captions/), every slot of
both devices (or the one --device names: each device's job checks its
own), at the store's size: 1320 × 2868 for the iPhone; 2064 × 2752
for the iPad, and 2752 × 2064 for its landscape shot (slots.LANDSCAPE). Exit 0 only when all are
there; otherwise every gap is listed and the exit is 1. A shoot job checks
its own part: --device, --locales (en-US,ko) and --slots (01,05) narrow it.

fetch: the live set from the site (<site URL>/shots/<device>/<locale>/…)
into <shots>, as a run leaves it, for a run that shot only part of the set
to be laid over (the files it made replace the live ones). One the site
doesn't have is left out, and the check after says so.

site: check, then copy the set to <site>/shots/<iphone|ipad>/<locale>/
<nn>-<shot>.png (paths that stay put from run to run, for the READMEs to
link) and write an index page for each.
"""

import concurrent.futures
import html
import json
import os
import shutil
import struct
import sys
import urllib.error
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from slots import LANDSCAPE  # noqa: E402

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
CAPTIONS = os.path.join(ROOT, "scripts", "shots", "captions")
DEVICES = {"iphone": "iphone-6.9", "ipad": "ipad-13"}
# The store's sizes; a landscape shot (slots.LANDSCAPE) is the iPad's frame
# on its side, and only it is (B842: a sideways capture composed upright).
PORTRAIT = {"iphone": (1320, 2868), "ipad": (2064, 2752)}
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


def check(shots, devices=tuple(DEVICES), locales=None, numbers=None):
    gaps = []
    for locale, (_, slots) in captions().items():
        if locales is not None and locale not in locales:
            continue
        for device, folder in DEVICES.items():
            if device not in devices:
                continue
            for name, _ in slots[device]:
                if numbers is not None and int(name[:2]) not in numbers:
                    continue
                path = os.path.join(shots, folder, "store", locale, name)
                if not os.path.isfile(path):
                    gaps.append(f"{device}/{locale}/{name}: missing")
                    continue
                size = png_size(path)
                want = PORTRAIT[device]
                if name.split("-", 1)[1][:-4] in LANDSCAPE:
                    want = (want[1], want[0])
                if size != want:
                    gaps.append(f"{device}/{locale}/{name}: {size[0]}×{size[1]}, not {want[0]}×{want[1]}" if size
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


def fetch(base, shots):
    """The live set into <shots>; how many files came, and the ones that didn't."""
    wanted = [(device, folder, locale, name)
              for locale, (_, slots) in captions().items()
              for device, folder in DEVICES.items()
              for name, _ in slots[device]]

    def one(item):
        device, folder, locale, name = item
        url = f"{base.rstrip('/')}/shots/{device}/{locale}/{name}"
        try:
            with urllib.request.urlopen(url, timeout=60) as reply:
                data = reply.read()
        except (urllib.error.URLError, TimeoutError) as error:
            return f"{device}/{locale}/{name}: {error}"
        target = os.path.join(shots, folder, "store", locale, name)
        os.makedirs(os.path.dirname(target), exist_ok=True)
        with open(target, "wb") as f:
            f.write(data)
        return None

    with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
        results = list(pool.map(one, wanted))
    lost = [r for r in results if r]
    return len(wanted) - len(lost), lost


def options(args, names):
    """Pulls `--name value` pairs out of `args`; None when one is malformed."""
    found = {}
    while len(args) > 1 and args[-2] in names:
        found[args[-2]], args = args[-1], args[:-2]
    return (args, found) if not any(a.startswith("--") for a in args) else (None, None)


def main():
    args, opts = options(sys.argv[1:], ("--device", "--locales", "--slots"))
    if args is None or not args or len(args) != {"check": 2, "fetch": 3, "site": 3}.get(args[0]) or (
        opts and args[0] != "check"
    ):
        print(__doc__.split("\n\n")[1], file=sys.stderr)
        return 64
    if args[0] == "fetch":
        got, lost = fetch(args[1], args[2])
        print(f"pages: fetched {got} images of the live set into {args[2]}")
        for miss in lost[:20]:
            print("  not there: " + miss, file=sys.stderr)
        return 0
    devices = tuple(DEVICES)
    if "--device" in opts:
        if opts["--device"] not in DEVICES:
            print(f"pages: --device: iphone or ipad, not {opts['--device']}", file=sys.stderr)
            return 64
        devices = (opts["--device"],)
    locales = set(opts["--locales"].replace(",", " ").split()) if "--locales" in opts else None
    numbers = None
    if opts.get("--slots", "all") != "all":
        try:
            numbers = {int(n) for n in opts["--slots"].replace(",", " ").split()}
        except ValueError:
            print(f"pages: --slots: numbers like 01,05, not {opts['--slots']}", file=sys.stderr)
            return 64
    if locales is not None and "all" in locales:
        locales = None
    shots = args[1]
    gaps = check(shots, devices, locales, numbers)
    if gaps:
        print(f"pages: the set isn't complete ({len(gaps)} gap(s)); nothing is published:", file=sys.stderr)
        for gap in gaps[:80]:
            print("  " + gap, file=sys.stderr)
        return 1
    which = "both devices" if len(devices) == 2 else f"the {devices[0]}"
    print(f"pages: the set is complete ({which}"
          f"{'' if locales is None else ', ' + ' '.join(sorted(locales))}"
          f"{'' if numbers is None else ', slots ' + ','.join(f'{n:02d}' for n in sorted(numbers))})")
    if args[0] == "site":
        site(shots, args[2])
        print(f"pages: laid out in {args[2]}/shots/")
    return 0


if __name__ == "__main__":
    sys.exit(main())
