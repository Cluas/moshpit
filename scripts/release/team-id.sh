#!/usr/bin/env bash
# Print the Team IDs available to Xcode after you've added your Apple ID
# (Xcode ▸ Settings ▸ Accounts). A free account shows one "(Personal Team)".
# Copy the 10-char ID into Signing.xcconfig → DEVELOPMENT_TEAM.
set -euo pipefail

found=0

# Source 1: Xcode's own account cache — populated the moment you sign in,
# BEFORE any certificate or provisioning profile exists. (The sources below
# only appear after a first signed build, which is exactly the chicken-and-egg
# a brand-new paid account is stuck in.)
teams=$(defaults read com.apple.dt.Xcode IDEProvisioningTeamByIdentifier 2>/dev/null \
  | awk -F'"' '
      /teamID/   { line=$0; gsub(/[ ;]/, "", line); sub(/.*teamID=/, "", line); id=line }
      /teamName/ { name=$2 }
      /teamType/ { line=$0; gsub(/[ ;]/, "", line); sub(/.*teamType=/, "", line); type=line }
      id != "" && name != "" && type != "" { printf "%s    %s (%s)\n", id, name, type; id=""; name=""; type="" }
    ' || true)
if [ -n "$teams" ]; then
  printf '%s\n' "$teams" | sort -u
  found=1
fi

# Source 2: provisioning profiles carry the TeamIdentifier once Xcode has
# created one. (A `for … done | sort` pipeline runs in a subshell where
# `found=1` would be lost — collect the output instead.)
if [ "$found" -eq 0 ]; then
  profiles=$(
    for p in "$HOME/Library/Developer/Xcode/UserData/Provisioning Profiles/"*.mobileprovision \
             "$HOME/Library/MobileDevice/Provisioning Profiles/"*.mobileprovision; do
      [ -e "$p" ] || continue
      tid=$(security cms -D -i "$p" 2>/dev/null \
            | plutil -extract TeamIdentifier.0 raw - 2>/dev/null || true)
      name=$(security cms -D -i "$p" 2>/dev/null \
            | plutil -extract TeamName raw - 2>/dev/null || true)
      [ -n "${tid:-}" ] && echo "$tid    $name"
    done | sort -u
  )
  if [ -n "$profiles" ]; then
    printf '%s\n' "$profiles"
    found=1
  fi
fi

# Source 3: signing identities in the keychain. `|| true` keeps a no-match
# grep from killing the script under `set -o pipefail` before the guidance
# below ever prints (the failure mode this script used to die of, silently).
if [ "$found" -eq 0 ]; then
  ids=$(security find-identity -v -p codesigning 2>/dev/null \
        | grep -o '([A-Z0-9]\{10\})' | tr -d '()' | sort -u || true)
  if [ -n "$ids" ]; then
    printf '%s\n' "$ids"
    found=1
  fi
fi

if [ "$found" -eq 0 ]; then
  echo "No team found yet. Add your Apple ID in Xcode ▸ Settings ▸ Accounts —" >&2
  echo "the account cache alone is enough, no build needed. If it still shows" >&2
  echo "nothing, your Team ID is on https://developer.apple.com/account under" >&2
  echo "Membership details." >&2
  exit 1
fi
