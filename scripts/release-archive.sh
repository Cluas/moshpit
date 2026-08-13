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

# Tester notes are part of shipping a build, not an afterthought: TestFlight
# asks for "What to Test" per build, and a build uploaded without them wastes
# testers on things we already know are broken. Warned rather than enforced —
# a local archive you never upload doesn't need them.
NOTES="docs/testflight/build-$BUILD.md"
if [ ! -f "$NOTES" ]; then
  echo "⚠ no tester notes at $NOTES — write them before uploading to TestFlight"
  echo "  (start from the previous build's: $(ls -1 docs/testflight/build-*.md 2>/dev/null | tail -1 || echo 'none yet'))"
fi

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

# Export authenticates with the App Store Connect API key, not with whatever
# Apple ID happens to be signed into Xcode.
#
# The distribution certificate is Cloud Managed, so exporting has to ASK Apple
# for it — and with no account in Xcode's settings that fails with a pair of
# errors that name neither cause nor cure:
#     error: exportArchive No Accounts
#     error: exportArchive No signing certificate "iOS Distribution" found
# (Hit on build 336, after two releases had exported fine: an Xcode account
# session simply expired.) The key we already use to upload can fetch the
# certificate too, which makes releasing independent of an interactive login.
#
# Credentials live outside the repo — the key id and issuer id identify the
# account, so they are not committed. Falls back to the account path if the
# config is absent, which is exactly the old behaviour.
AUTH=()
# shellcheck disable=SC1090
[ -f "$HOME/.appstoreconnect/asc.env" ] && . "$HOME/.appstoreconnect/asc.env"
KEY_FILE="$HOME/.appstoreconnect/private_keys/AuthKey_${ASC_KEY_ID:-}.p8"
if [ -n "${ASC_KEY_ID:-}" ] && [ -n "${ASC_ISSUER_ID:-}" ] && [ -f "$KEY_FILE" ]; then
  AUTH=(-authenticationKeyPath "$KEY_FILE"
        -authenticationKeyID "$ASC_KEY_ID"
        -authenticationKeyIssuerID "$ASC_ISSUER_ID")
else
  echo "⚠ no ASC API key config — exporting via Xcode's signed-in account instead."
  echo "  If this fails with 'No Accounts', write ~/.appstoreconnect/asc.env with"
  echo "  ASC_KEY_ID and ASC_ISSUER_ID, and put the .p8 in private_keys/."
fi

echo "▶ Exporting for App Store Connect…"
xcodebuild -exportArchive -archivePath "$ARCHIVE" \
  -exportOptionsPlist "$OPTS" -exportPath "$EXPORT_DIR" \
  -allowProvisioningUpdates "${AUTH[@]}"

echo "✓ $EXPORT_DIR/Moshpit.ipa  ($(du -h "$EXPORT_DIR/Moshpit.ipa" | cut -f1))"
echo
echo "  Upload with either:"
echo "    open $ARCHIVE          # Xcode Organizer ▸ Distribute App"
echo "    open -a Transporter $EXPORT_DIR/Moshpit.ipa   # drag-and-drop uploader"
echo "  (altool --upload-app is gone since Xcode 15 — don't reach for it.)"
if [ -f "$NOTES" ]; then
  echo
  echo "  Tester notes for this build (paste into TestFlight ▸ What to Test):"
  echo "    $NOTES"
fi
