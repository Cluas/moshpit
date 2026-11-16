#!/usr/bin/env bash
# The README's screenshots, from the bare captures (no frames, no captions):
# for each locale, docs/readme/<locale>/hero.png (four phones side by side,
# 1200 px wide, shown at 900) and one 330 px image per gallery shot.
#
#   scripts/readme-shots.sh <raw> [locale...]
#
#   <raw>   a folder holding the screenshot workflow's `shots-*` artifacts,
#           unpacked: <raw>/**/iphone-6.9/<locale>/<nn>-<shot>.png
#   locale  default: all nine. The lock screen and the Dynamic Island come
#           from scripts/shots/external/<locale>/, else en-US's.
#
# Needs ImageMagick (magick), pngquant and oxipng. Run from anywhere.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
raw="${1:?usage: readme-shots.sh <raw> [locale...]}"
shift || true
locales=("$@")
[ ${#locales[@]} -gt 0 ] || locales=(en-US zh-Hans zh-Hant ja ko de-DE es-ES fr-FR pt-BR)
out="$root/docs/readme"
ext="$root/scripts/shots/external"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# The shot for a locale: the capture, else the phone's external one.
find_shot() { # locale name
  local l="$1" n="$2" f
  f="$(find "$raw" -path "*/iphone-6.9/$l/$n.png" -not -path '*/store/*' | head -n1)"
  [ -n "$f" ] || f="$ext/$l/$n.png"
  [ -f "$f" ] || f="$ext/en-US/$n.png"
  [ -f "$f" ] || { echo "missing $n for $l" >&2; return 1; }
  echo "$f"
}

# Rounded corners and a hairline edge, at width $2.
round() { # in width out
  local w="$2" h r
  h=$(( w * 2868 / 1320 )); r=$(( w * 118 / 1000 ))
  magick "$1" -resize "${w}x${h}!" \
    \( -size "${w}x${h}" xc:none -fill white -draw "roundrectangle 0,0 $((w-1)),$((h-1)) $r,$r" \) \
    -alpha set -compose DstIn -composite \
    \( -size "${w}x${h}" xc:none -fill none -stroke '#ffffff30' -strokewidth 2 \
       -draw "roundrectangle 1,1 $((w-2)),$((h-2)) $r,$r" \) \
    -compose Over -composite "$3"
}

squeeze() { # file
  pngquant --force --quality 70-95 --strip --output "$1" "$1"
  oxipng -o4 --strip all -q "$1"
}

for l in "${locales[@]}"; do
  mkdir -p "$out/$l"
  i=0
  for n in 01-lock 03-home 02-pane 05-overview; do
    round "$(find_shot "$l" "$n")" 282 "$work/h$i.png"; i=$((i+1))
  done
  # 4 x 282 + 3 x 24 = 1200
  magick -background none "$work"/h0.png \( -size 24x1 xc:none \) "$work"/h1.png \( -size 24x1 xc:none \) \
    "$work"/h2.png \( -size 24x1 xc:none \) "$work"/h3.png +append "$out/$l/hero.png"
  squeeze "$out/$l/hero.png"
  for n in 01-lock 02-pane 03-home 04-island 05-overview 06-preview 07-pet 08-keyboard; do
    round "$(find_shot "$l" "$n")" 330 "$out/$l/$n.png"
    squeeze "$out/$l/$n.png"
  done
done
