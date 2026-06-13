#!/usr/bin/env bash
# Print the Team IDs available to Xcode after you've added your Apple ID
# (Xcode ▸ Settings ▸ Accounts). A free account shows one "(Personal Team)".
# Copy the 10-char ID into Signing.xcconfig → DEVELOPMENT_TEAM.
set -euo pipefail

found=0
# Provisioning profiles carry the TeamIdentifier once Xcode has created one.
for p in "$HOME/Library/Developer/Xcode/UserData/Provisioning Profiles/"*.mobileprovision \
         "$HOME/Library/MobileDevice/Provisioning Profiles/"*.mobileprovision; do
  [ -e "$p" ] || continue
  tid=$(security cms -D -i "$p" 2>/dev/null \
        | plutil -extract TeamIdentifier.0 raw - 2>/dev/null || true)
  name=$(security cms -D -i "$p" 2>/dev/null \
        | plutil -extract TeamName raw - 2>/dev/null || true)
  [ -n "${tid:-}" ] && { echo "$tid    $name"; found=1; }
done | sort -u

# Fallback: signing identities in the keychain.
if [ "$found" -eq 0 ]; then
  security find-identity -v -p codesigning 2>/dev/null | grep -o '([A-Z0-9]\{10\})' | tr -d '()' | sort -u
fi

if [ "$found" -eq 0 ] && ! security find-identity -v -p codesigning 2>/dev/null | grep -q '('; then
  echo "No team found yet. Add your Apple ID in Xcode ▸ Settings ▸ Accounts," >&2
  echo "then build once to the device so Xcode provisions a Personal Team." >&2
  exit 1
fi
