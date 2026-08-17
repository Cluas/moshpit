#!/usr/bin/env bash
# Promote an already-uploaded build from Canary to the EXTERNAL beta groups.
#
# The release flow is two-stage since build 353:
#
#   1. scripts/release-upload.sh — validate, upload, compliance, What to Test.
#      The internal "Canary" group (hasAccessToAllBuilds) sees every build the
#      moment processing finishes, with no beta review: that's where we test
#      on real devices.
#   2. THIS script — after the build survives Canary, hand it to the external
#      groups and submit Beta App Review. External testers never see the
#      builds that didn't survive.
#
# Usage:  scripts/release-promote.sh [build-number]
#         (defaults to the number in BUILD_NUMBER — the build just shipped)
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

# shellcheck disable=SC1090
[ -f "$HOME/.appstoreconnect/asc.env" ] && . "$HOME/.appstoreconnect/asc.env"
: "${ASC_KEY_ID:?set ASC_KEY_ID in ~/.appstoreconnect/asc.env}"
: "${ASC_ISSUER_ID:?set ASC_ISSUER_ID in ~/.appstoreconnect/asc.env}"
KEY="$HOME/.appstoreconnect/private_keys/AuthKey_${ASC_KEY_ID}.p8"
[ -f "$KEY" ] || { echo "✘ no key at $KEY" >&2; exit 1; }

APP_ID=6799896801
VERSION="${1:-$(tr -cd '0-9' < BUILD_NUMBER 2>/dev/null || true)}"
[[ "$VERSION" =~ ^[1-9][0-9]*$ ]] || { echo "✘ which build? pass a number or fix BUILD_NUMBER" >&2; exit 1; }

mint_jwt() {
  xcrun altool --generate-jwt --apiKey "$ASC_KEY_ID" --apiIssuer "$ASC_ISSUER_ID" \
    --p8-file-path "$KEY" 2>&1 \
    | grep -oE 'ey[A-Za-z0-9_-]+\.ey[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+'
}
JWT="$(mint_jwt)"
api() { curl -sg -X "${1}" -H "Authorization: Bearer $JWT" \
        -H "Content-Type: application/json" \
        ${3:+-d "$3"} "https://api.appstoreconnect.apple.com${2}"; }

echo "▶ Locating build ${VERSION}…"
BUILD_ID="$(api GET "/v1/builds?filter[app]=$APP_ID&limit=10" | WANT="$VERSION" python3 -c '
import json, os, sys
want = os.environ["WANT"]
for b in json.load(sys.stdin).get("data", []):
    if b["attributes"].get("version") == want and b["attributes"].get("processingState") == "VALID":
        print(b["id"]); break
')"
[ -n "$BUILD_ID" ] || { echo "✘ no processed build $VERSION on App Store Connect" >&2; exit 1; }
echo "  ✓ $BUILD_ID"

# External groups only: Canary (internal, hasAccessToAllBuilds) already has
# every build, and re-adding it here would just be noise.
echo "▶ Adding to EXTERNAL beta groups…"
api GET "/v1/apps/$APP_ID/betaGroups?limit=20" | python3 -c "
import json, sys
for g in json.load(sys.stdin).get('data', []):
    if not g['attributes'].get('isInternalGroup'):
        print(g['id'], g['attributes'].get('name', '?'))
" | while read -r gid gname; do
  [ -n "$gid" ] || continue
  api POST "/v1/betaGroups/$gid/relationships/builds" \
    "{\"data\":[{\"type\":\"builds\",\"id\":\"$BUILD_ID\"}]}" > /dev/null
  echo "  ✓ $gname"
done

# The Submit for Review button, bottom right. External groups need it, and it
# usually comes back approved within moments.
echo "▶ Submitting for Beta App Review…"
api POST "/v1/betaAppReviewSubmissions" \
  "{\"data\":{\"type\":\"betaAppReviewSubmissions\",\"relationships\":{\"build\":{\"data\":{\"type\":\"builds\",\"id\":\"$BUILD_ID\"}}}}}" \
  | python3 -c "
import json, sys
d = json.load(sys.stdin)
if 'errors' in d:
    # Re-running over an already-submitted build is expected, not a failure.
    print('  ·', '; '.join(e.get('detail') or e.get('title', '') for e in d['errors']))
else:
    print('  ✓', d['data']['attributes'].get('betaReviewState'))
"

STATE=""
for _ in $(seq 1 10); do
  STATE="$(api GET "/v1/builds/$BUILD_ID/betaAppReviewSubmission" | python3 -c "
import json, sys
print((json.load(sys.stdin).get('data') or {}).get('attributes', {}).get('betaReviewState', ''))
" || true)"
  [ "$STATE" = "APPROVED" ] && { echo "✓ beta review approved"; break; }
  [ "$STATE" = "REJECTED" ] && { echo "✘ beta review rejected — see App Store Connect" >&2; exit 1; }
  sleep 20
done

echo
echo "Build $VERSION promoted: external groups added, review $STATE."
