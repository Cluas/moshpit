#!/bin/sh
# verify-app-language.sh — the island, the widget and the pushed cards write
# in the language Moshpit shows, not the system's (AppLanguage; UI-DESIGN.md
# §10), checked on this Mac over the real catalogs.
#
# The device the unit tests run on is in English, so it can only show one
# direction (Moshpit set to Chinese). This runs both, with this process's
# languages standing in for the system's: the owner's phone (system Chinese,
# Moshpit English) and the reverse. The catalogs are compiled with the tool
# Xcode builds them with, so the push extension's has no en.lproj here either.
#
# Usage: scripts/verify/verify-app-language.sh
set -eu
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

for pair in "app:Moshpit/Resources" "island:Extensions/MoshpitIsland" "push:Extensions/MoshpitPush"; do
  name=${pair%%:*}; dir=${pair#*:}
  mkdir -p "$TMP/$name.bundle"
  xcrun xcstringstool compile "$ROOT/$dir/Localizable.xcstrings" --output-directory "$TMP/$name.bundle"
  # As the real bundles say it: English is the development language, the
  # one a lookup falls back to.
  plutil -create xml1 "$TMP/$name.bundle/Info.plist"
  plutil -insert CFBundleDevelopmentRegion -string en "$TMP/$name.bundle/Info.plist"
  plutil -insert CFBundleIdentifier -string "verify.app-language.$name" "$TMP/$name.bundle/Info.plist"
done

printf 'public enum PushPrefs { public static let suite = "moshpit.verify.app-language.%s" }\n' "$$" > "$TMP/stub.swift"
xcrun swiftc -O -o "$TMP/check" \
  "$ROOT/Packages/MoshpitKit/Sources/MoshpitKit/AppLanguage.swift" "$TMP/stub.swift" \
  "$ROOT/scripts/verify/app-language/main.swift"

status=0
"$TMP/check" "$TMP" en -AppleLanguages '(zh-Hans)' || status=1
"$TMP/check" "$TMP" zh-Hans -AppleLanguages '(en)' || status=1
[ "$status" -eq 0 ] && echo "both directions: the extensions write in Moshpit's language"
exit "$status"
