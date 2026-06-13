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

echo "▶ Packaging .ipa…"
rm -rf build/Payload build/Moshi.ipa
mkdir -p build/Payload
cp -R "$APP" build/Payload/
( cd build && zip -qry Moshi.ipa Payload )
rm -rf build/Payload

echo "✓ build/Moshi.ipa  ($(du -h build/Moshi.ipa | cut -f1))"
echo "  Install via AltStore (drag in) or: altserver -u <UDID> build/Moshi.ipa"
