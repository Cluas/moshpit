#!/usr/bin/env python3
"""The app icon family: the egg as the primary icon, and every species
peeking over a terminal pane as an alternate, drawn from the pet's own
pixel data.

    python3 -I scripts/gen/generate-pet-icons.py [--sheet /tmp/sheet.png]

The creature is the pet as the app draws it: crates/pet/data/pixel.json
composed by the design's reference renderer (moshpit-design
docs/nsh/ui-mockups/pet/tools/build.py, the file crates/pet's renderer is
golden-tested against frame by frame). Its idle pose, without its limbs:
the pane hides the body, and two paws in the creature's own outline and
body colours grip the pane's top edge instead. A shiny is the same pose with
the genome's shiny palette (the two bands, the shade and the light), the
way the app paints a shiny pet.

One grid for the whole family: 42 x 42 cells of 24 px plus an 8 px margin
(1024), so every sprite pixel lands on whole pixels. The pane, the cursor,
the background and the row the pane cuts at are the same in every icon;
only the creature and its paws change. Icon Composer renders it as Liquid
Glass with the glass toned down (no specular, little translucency), so the
pixels stay crisp and the Dark, Clear and Tinted appearances are real.

Writes (all checked in):
  Moshpit/Resources/AppIcon.icon/                     the egg (primary)
  Moshpit/Resources/AlternateIcons/AppIcon-<Species>.icon/
  Moshpit/Resources/AlternateIcons/AppIcon-<Species>Shiny.icon/
  Moshpit/Resources/AlternateIcons/Previews/IconPreview-<Tag>@3x.png
    240 px picker thumbnails rendered by ictool, one per alternate, one for
    the primary ("Moshpit", the egg), and one for "Classic" (the
    three-panes icon of 1.0.x, kept as an alternate; not drawn here).

The alternates' names go in project.yml's
ASSETCATALOG_COMPILER_ALTERNATE_APPICON_NAMES. Needs Xcode's Icon Composer
(ictool) and the moshpit-design checkout ($MOSHPIT_DESIGN, else
../moshpit-design, else ~/code/moshpit-design). Standard library only.
"""
import argparse
import json
import os
import shutil
import struct
import subprocess
import sys
import zlib

ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
RES = os.path.join(ROOT, "Moshpit", "Resources")
ALT = os.path.join(RES, "AlternateIcons")
PREVIEWS = os.path.join(ALT, "Previews")
PIXEL = os.path.join(ROOT, "crates", "pet", "data", "pixel.json")
ICTOOL = "/Applications/Xcode.app/Contents/Applications/Icon Composer.app/Contents/Executables/ictool"


def design_root():
    for d in (os.environ.get("MOSHPIT_DESIGN"), os.path.join(ROOT, "..", "moshpit-design"),
              os.path.expanduser("~/code/moshpit-design")):
        if d and os.path.isfile(os.path.join(d, "docs/nsh/ui-mockups/pet/tools/build.py")):
            return d
    sys.exit("needs the moshpit-design checkout: set MOSHPIT_DESIGN")


sys.path.insert(0, os.path.join(design_root(), "docs/nsh/ui-mockups/pet/tools"))
import build  # noqa: E402  the reference renderer

# ── The grid ────────────────────────────────────────────────────────────────
S, N, M, W = 24, 42, 8, 1024      # px per cell, cells per side, margin, side
CUT = 17                          # sprite rows >= CUT are behind the pane
ORIGIN = (9, 1)                   # sprite (0, 0) in icon cells
PANE = (5, 18, 38, 36)            # x0, y0, x1, y1 in cells (x1, y1 exclusive)
PANE_CORNER = [3, 1, 1]           # cells cut at each corner, row by row
CURSOR = (8, 21, 2, 4)            # x, y, w, h in cells
PAW = [".oo.", "obbo", "obbo", "oooo"]
PAW_TOP, PAW_X = 17, (12, 26)     # the rounded top above the pane's edge
LIMBS = ("limb", "leg", "foot", "paw", "paw_down", "arm_up")
PAW_FILL = {"dig": "P"}           # Dig's hands are pink; everyone else's are its body

# ── Glass: Liquid Glass, toned down so the pixels stay crisp ─────────────────
def srgb(h):
    r, g, b = (int(h[i:i + 2], 16) / 255 for i in (1, 3, 5))
    return f"extended-srgb:{r:.5f},{g:.5f},{b:.5f},1.00000"


FILLS = [
    {"value": {"linear-gradient": [srgb("#7D7CF0"), srgb("#3F3DB8")]}},
    {"appearance": "dark", "value": {"linear-gradient": [srgb("#4D4BA0"), srgb("#1E1D5A")]}},
]
SOFT = {"lighting": "combined", "shadow": {"kind": "neutral", "opacity": 0.3}, "specular": False}
PANE_GROUP = {**SOFT, "translucency": {"enabled": True, "value": 0.3}}
PIXEL_GROUP = {**SOFT, "translucency": {"enabled": False, "value": 0}}

SPECIES = [("tun", "Tun"), ("coo", "Coo"), ("dig", "Dig"), ("hex", "Hex"), ("ether", "Ether"), ("moth", "Moth")]
SHINY_TRAITS = {"shiny": True, "palette": 0, "pattern": 0, "eyes": 0, "size": 1, "slot": 0, "grown": []}


def rgb(h):
    return (int(h[1:3], 16), int(h[3:5], 16), int(h[5:7], 16))


def png(path, cells):
    """cells: {(cx, cy): (r, g, b)} on the grid; transparent elsewhere."""
    clear = b"\x00\x00\x00\x00"
    raw = bytearray()
    for y in range(W):
        raw += b"\x00"
        cy = (y - M) // S if M <= y < W - M else None
        row = bytearray(clear * M)
        for cx in range(N):
            c = cells.get((cx, cy)) if cy is not None else None
            row += (bytes((*c, 255)) if c else clear) * S
        row += clear * M
        raw += row
    def chunk(k, d):
        return struct.pack(">I", len(d)) + k + d + struct.pack(">I", zlib.crc32(k + d) & 0xFFFFFFFF)
    data = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", W, W, 8, 6, 0, 0, 0))
    data += chunk(b"IDAT", zlib.compress(bytes(raw), 9)) + chunk(b"IEND", b"")
    with open(path, "wb") as f:
        f.write(data)


def pane():
    x0, y0, x1, y1 = PANE
    h, out = y1 - y0, {}
    for j in range(h):
        k = PANE_CORNER[j] if j < len(PANE_CORNER) else (PANE_CORNER[h - 1 - j] if h - 1 - j < len(PANE_CORNER) else 0)
        for x in range(x0 + k, x1 - k):
            out[(x, y0 + j)] = (255, 255, 255)
    return out


def cursor():
    x, y, w, h = CURSOR
    return {(x + i, y + j): (255, 255, 255) for i in range(w) for j in range(h)}


def creature(grid):
    ox, oy = ORIGIN
    return {(ox + x, oy + y): rgb(grid[y][x]) for y in range(CUT) for x in range(24) if grid[y][x]}


def paws(doc, cid, shiny):
    pal = doc["characters"][cid]["palette"]["light"]
    fill = pal[PAW_FILL.get(cid, "b")]
    if shiny and cid not in PAW_FILL:
        fill = doc["genome"]["shiny"]["bottom"]   # the paws sit below the band
    ink = {"o": rgb(pal["o"]), "b": rgb(fill)}
    return {(px + i, PAW_TOP + j): ink[k] for px in PAW_X for j, row in enumerate(PAW) for i, k in enumerate(row) if k != "."}


def pose(doc, cid, traits=None):
    fr = dict(build.species_states(doc, cid)["idle"][0])
    fr["layers"] = [l for l in fr["layers"] if l[0] not in LIMBS]
    return build.compose(doc, cid, fr, "light", traits)


def write_icon(path, char, paw):
    if os.path.isdir(path):
        shutil.rmtree(path)
    assets = os.path.join(path, "Assets")
    os.makedirs(assets)
    png(os.path.join(assets, "pane.png"), pane())
    png(os.path.join(assets, "cursor.png"), cursor())
    png(os.path.join(assets, "creature.png"), char)
    groups = []
    if paw:
        png(os.path.join(assets, "paws.png"), paw)
        groups.append({"layers": [{"glass": True, "image-name": "paws.png", "name": "paws"}], **PIXEL_GROUP})
    groups.append({"layers": [{"glass": True, "image-name": "cursor.png", "name": "cursor"},
                              {"glass": True, "image-name": "pane.png", "name": "pane", "opacity": 0.5}], **PANE_GROUP})
    groups.append({"layers": [{"glass": True, "image-name": "creature.png", "name": "creature"}], **PIXEL_GROUP})
    with open(os.path.join(path, "icon.json"), "w") as f:
        json.dump({"fill-specializations": FILLS, "groups": groups, "supported-platforms": {"squares": "shared"}}, f, indent=2)
        f.write("\n")


def ictool(doc, out, rendition="Default", pt=80, scale=3):
    subprocess.run([ICTOOL, doc, "--export-image", "--output-file", out, "--platform", "iOS",
                    "--rendition", rendition, "--width", str(pt), "--height", str(pt), "--scale", str(scale)],
                   check=True, stdout=subprocess.DEVNULL)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--sheet", help="also write a contact sheet of every icon in all four appearances")
    args = ap.parse_args()
    doc = json.load(open(PIXEL, encoding="utf-8"))

    docs = {"Moshpit": os.path.join(RES, "AppIcon.icon")}
    write_icon(docs["Moshpit"], creature(build.compose(doc, "tun", doc["egg_states"]["egg"][0], "light")), None)
    for cid, name in SPECIES:
        for tag, traits in ((name, None), (name + "Shiny", SHINY_TRAITS)):
            docs[tag] = os.path.join(ALT, f"AppIcon-{tag}.icon")
            write_icon(docs[tag], creature(pose(doc, cid, traits)), paws(doc, cid, traits is not None))
    classic = os.path.join(ALT, "AppIcon-Classic.icon")
    if os.path.isdir(classic):
        docs["Classic"] = classic

    os.makedirs(PREVIEWS, exist_ok=True)
    for tag, path in docs.items():
        ictool(path, os.path.join(PREVIEWS, f"IconPreview-{tag}@3x.png"))
    print("icons", " ".join(docs))

    if args.sheet:
        tmp = os.path.join(os.environ.get("TMPDIR", "/tmp"), "moshpit-pet-icon-sheet")
        os.makedirs(tmp, exist_ok=True)
        cmd = ["magick", "-background", "#777777"]
        for tag, path in docs.items():
            row = []
            for r in ("Default", "Dark", "ClearDark", "TintedDark"):
                out = os.path.join(tmp, f"{tag}-{r}.png")
                ictool(path, out, r, 128, 2)
                row.append(out)
            cmd += ["(", *row, "-bordercolor", "#777777", "-border", "4", "+append", ")"]
        subprocess.run(cmd + ["-append", args.sheet], check=True)
        print("sheet", args.sheet)


if __name__ == "__main__":
    main()
