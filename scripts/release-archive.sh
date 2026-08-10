#!/usr/bin/env bash
# Archive and export a SIGNED App Store build → build/AppStore/Moshpit.ipa
#
# The counterpart to scripts/build-ipa.sh, which builds an UNSIGNED .ipa for
# AltStore/SideStore to re-sign with a free Apple ID. This is the paid
# Developer Program path instead: Release configuration, Apple Distribution
# identity, and a build number App Store Connect will actually accept.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

ARCHIVE="build/Moshpit.xcarchive"
EXPORT_DIR="build/AppStore"
OPTS="build/ExportOptions.plist"

# The build number is the commit count — unique and monotonically increasing by
# construction, so a release never depends on someone remembering to bump it
# (App Store Connect rejects a build number it has already seen for a version).
#
# It is passed as a command-line build setting rather than written into
# project.yml because command-line settings apply to EVERY target in the build:
# that is what makes the app and the MoshpitIsland extension come out with the
# identical CFBundleVersion that App Store validation insists on. Override with
# MOSHPIT_BUILD=n to re-upload after a rejection without a dummy commit.
BUILD="${MOSHPIT_BUILD:-$(git rev-list --count HEAD)}"

TEAM="$(sed -n 's/^[[:space:]]*DEVELOPMENT_TEAM[[:space:]]*=[[:space:]]*//p' Signing.xcconfig | tr -d '[:space:]')"
if [ -z "$TEAM" ]; then
  echo "✘ DEVELOPMENT_TEAM is empty in Signing.xcconfig." >&2
  echo "  Fill in the 10-character Team ID of the paid account: ./scripts/team-id.sh" >&2
  exit 1
fi

# A stale .xcodeproj is a silent footgun in a project where project.yml is the
# source of truth — refuse rather than ship yesterday's Info.plist.
if [ project.yml -nt Moshpit.xcodeproj/project.pbxproj ]; then
  echo "✘ project.yml is newer than Moshpit.xcodeproj — run: xcodegen generate" >&2
  exit 1
fi

git diff --quiet HEAD 2>/dev/null || \
  echo "⚠ working tree is dirty — this archive will not match commit $(git rev-parse --short HEAD)"

echo "▶ Archiving Release (build $BUILD, team $TEAM)…"
rm -rf "$ARCHIVE" "$EXPORT_DIR"
mkdir -p build
xcodebuild -project Moshpit.xcodeproj -scheme Moshpit \
  -configuration Release -destination 'generic/platform=iOS' \
  -archivePath "$ARCHIVE" \
  -allowProvisioningUpdates \
  CURRENT_PROJECT_VERSION="$BUILD" \
  archive

# Prove the lockstep rather than trusting it: a CFBundleVersion mismatch between
# the app and its embedded extension is an upload rejection, and the whole point
# of overriding the setting globally above is to make that impossible.
plist_get() { /usr/libexec/PlistBuddy -c "Print :$2" "$1/Info.plist"; }
APP="$ARCHIVE/Products/Applications/Moshpit.app"
APP_BUILD="$(plist_get "$APP" CFBundleVersion)"
for appex in "$APP"/PlugIns/*.appex; do
  [ -d "$appex" ] || continue
  EXT_BUILD="$(plist_get "$appex" CFBundleVersion)"
  if [ "$EXT_BUILD" != "$APP_BUILD" ]; then
    echo "✘ $(basename "$appex") is build $EXT_BUILD but the app is $APP_BUILD" >&2
    exit 1
  fi
done
echo "✓ app and extensions agree on build $APP_BUILD (version $(plist_get "$APP" CFBundleShortVersionString))"

cat > "$OPTS" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>method</key>
	<string>app-store-connect</string>
	<key>teamID</key>
	<string>$TEAM</string>
	<key>signingStyle</key>
	<string>automatic</string>
	<key>uploadSymbols</key>
	<true/>
	<!-- Keep our own build number: with this true, Xcode would helpfully
	     renumber the upload and defeat the commit-count scheme above. -->
	<key>manageAppVersionAndBuildNumber</key>
	<false/>
</dict>
</plist>
PLIST

echo "▶ Exporting for App Store Connect…"
xcodebuild -exportArchive -archivePath "$ARCHIVE" \
  -exportOptionsPlist "$OPTS" -exportPath "$EXPORT_DIR" \
  -allowProvisioningUpdates

echo "✓ $EXPORT_DIR/Moshpit.ipa  ($(du -h "$EXPORT_DIR/Moshpit.ipa" | cut -f1))"
echo
echo "  Upload with either:"
echo "    open $ARCHIVE          # Xcode Organizer ▸ Distribute App"
echo "    open -a Transporter $EXPORT_DIR/Moshpit.ipa   # drag-and-drop uploader"
echo "  (altool --upload-app is gone since Xcode 15 — don't reach for it.)"
