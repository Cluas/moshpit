#!/usr/bin/env bash
# Build an UNSIGNED .ipa for AltStore / SideStore to re-sign with your
# free Apple ID. No Team ID needed here — the sideloader handles signing
# and (with AltServer running) auto-refreshes before the 7-day expiry.
#
# Output: build/Moshi.ipa  → drag into AltStore, or `altserver` install.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

CONFIG="${1:-Debug}"
DERIVED="build/ipa"
APP="$DERIVED/Build/Products/$CONFIG-iphoneos/Moshi.app"

echo "▶ Building $CONFIG (device, unsigned)…"
xcodebuild -project Moshi.xcodeproj -scheme Moshi -configuration "$CONFIG" \
  -sdk iphoneos -derivedDataPath "$DERIVED" \
  CODE_SIGNING_ALLOWED=NO CODE_SIGNING_REQUIRED=NO build >/dev/null

[ -d "$APP" ] || { echo "✘ build produced no app at $APP" >&2; exit 1; }

# Stamp the build so Settings can show which build is running (git SHA + time;
# "+" = uncommitted changes). Also set CFBundleVersion to the commit count — a
# monotonically increasing build number that's easy to compare between installs.
# Only the PRODUCT's Info.plist is touched (unsigned; AltStore re-signs anyway).
SHA="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
git diff --quiet HEAD 2>/dev/null || SHA="${SHA}+"
STAMP="$SHA · $(date '+%m-%d %H:%M')"
BUILDNUM="$(git rev-list --count HEAD 2>/dev/null || echo 1)"
/usr/libexec/PlistBuddy -c "Add :MoshiBuildStamp string $STAMP" "$APP/Info.plist" 2>/dev/null \
  || /usr/libexec/PlistBuddy -c "Set :MoshiBuildStamp $STAMP" "$APP/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleVersion $BUILDNUM" "$APP/Info.plist" 2>/dev/null || true
echo "▶ Stamped build $BUILDNUM ($STAMP)"

echo "▶ Packaging .ipa…"
rm -rf build/Payload build/Moshi.ipa
mkdir -p build/Payload
cp -R "$APP" build/Payload/
( cd build && zip -qry Moshi.ipa Payload )
rm -rf build/Payload

echo "✓ build/Moshi.ipa  ($(du -h build/Moshi.ipa | cut -f1))"
echo "  Install via AltStore (drag in) or: altserver -u <UDID> build/Moshi.ipa"
