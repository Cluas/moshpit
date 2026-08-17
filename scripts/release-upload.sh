#!/usr/bin/env bash
# Upload build/AppStore/Moshpit.ipa and finish the two steps App Store Connect
# will otherwise make you do by hand, every single time:
#
#   1. Export compliance. A fresh build lands with `usesNonExemptEncryption`
#      unset, which TestFlight shows as "Missing Compliance" and which stops
#      testers installing it. Moshpit's answer is the same on every build —
#      standard encryption algorithms alongside the OS's, not distributed in
#      France, no documentation required — and App Store Connect encodes that
#      outcome as `usesNonExemptEncryption = false` on the build. Writing the
#      field is therefore exactly what clicking through the questionnaire does.
#
#      It cannot be answered in the binary instead: the plist key's `true`
#      needs an ITSEncryptionExportComplianceCode, and the not-in-France path
#      issues no code, so `true` fails the upload outright with ITMS-90592.
#
#   2. Distribution. Builds do NOT reach beta groups on their own — verified on
#      336, which sat uploaded and processed while both groups still listed 334
#      as their newest. Testers simply never see it.
#
# Usage:  scripts/release-upload.sh [--no-groups]
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

IPA="build/AppStore/Moshpit.ipa"
[ -f "$IPA" ] || { echo "✘ no $IPA — run scripts/release-archive.sh first" >&2; exit 1; }

# shellcheck disable=SC1090
[ -f "$HOME/.appstoreconnect/asc.env" ] && . "$HOME/.appstoreconnect/asc.env"
: "${ASC_KEY_ID:?set ASC_KEY_ID in ~/.appstoreconnect/asc.env}"
: "${ASC_ISSUER_ID:?set ASC_ISSUER_ID in ~/.appstoreconnect/asc.env}"
KEY="$HOME/.appstoreconnect/private_keys/AuthKey_${ASC_KEY_ID}.p8"
[ -f "$KEY" ] || { echo "✘ no key at $KEY" >&2; exit 1; }

APP_ID=6799896801
VERSION="$(/usr/libexec/PlistBuddy -c 'Print :ApplicationProperties:CFBundleVersion' \
           build/Moshpit.xcarchive/Info.plist 2>/dev/null || true)"

altool() { xcrun altool "$@" --api-key "$ASC_KEY_ID" --api-issuer "$ASC_ISSUER_ID" --p8-file-path "$KEY"; }

# Bearer token for the REST calls. altool prints it on stderr among its own
# chatter, hence the grep for the JWT shape rather than a plain capture.
mint_jwt() {
  xcrun altool --generate-jwt --apiKey "$ASC_KEY_ID" --apiIssuer "$ASC_ISSUER_ID" \
    --p8-file-path "$KEY" 2>&1 \
    | grep -oE 'ey[A-Za-z0-9_-]+\.ey[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+'
}

echo "▶ Validating…"
altool --validate-app -f "$IPA" --type ios | tail -3

echo "▶ Uploading…"
altool --upload-app -f "$IPA" --type ios | tail -3

JWT="$(mint_jwt)"
# -g so curl leaves the brackets in filter[app] alone instead of globbing them.
api() { curl -sg -X "${1}" -H "Authorization: Bearer $JWT" \
        -H "Content-Type: application/json" \
        ${3:+-d "$3"} "https://api.appstoreconnect.apple.com${2}"; }

echo "▶ Waiting for build $VERSION to finish processing…"
BUILD_ID=""
for _ in $(seq 1 40); do
  # Version goes through the environment, not through string-substitution into
  # the program text.
  BUILD_ID="$(api GET "/v1/builds?filter[app]=$APP_ID&limit=10" | WANT="$VERSION" python3 -c '
import json, os, sys
want = os.environ["WANT"]
for b in json.load(sys.stdin).get("data", []):
    a = b["attributes"]
    if a.get("version") == want and a.get("processingState") == "VALID":
        print(b["id"]); break
' || true)"
  [ -n "$BUILD_ID" ] && break
  sleep 30
  JWT="$(mint_jwt)"    # tokens expire in 20 minutes; processing can outlast that
done
[ -n "$BUILD_ID" ] || { echo "✘ build $VERSION never reached VALID" >&2; exit 1; }
echo "✓ build $VERSION is $BUILD_ID"

echo "▶ Answering export compliance…"
api PATCH "/v1/builds/$BUILD_ID" \
  "{\"data\":{\"type\":\"builds\",\"id\":\"$BUILD_ID\",\"attributes\":{\"usesNonExemptEncryption\":false}}}" \
  > /dev/null
echo "✓ usesNonExemptEncryption = false"

[ "${1:-}" = "--no-groups" ] && { echo "▶ Stopping before test info, groups and review."; exit 0; }

# The rest mirrors, in order, what the App Store Connect UI makes you do:
# pick the build for the group, fill in what testers are told, then press
# Submit for Review. All three have API equivalents, and the last one is the
# step that was missing when this script first ran — a build sitting in an
# external group with no review submission behind it is the "status problem"
# that had to be undone by hand on 336.

NOTES="docs/testflight/build-$VERSION.md"
if [ -f "$NOTES" ]; then
  echo "▶ Test information (What to Test)…"
  # TestFlight renders What to Test as PLAIN TEXT — markdown reaches testers
  # as literal ## and ** (user report, build 352). Refuse rather than ship
  # symbols: the tester-facing half is everything after the first bare ---.
  if sed '1,/^---$/d' "$NOTES" | grep -qE '^#|\*\*|^```'; then
    echo "✘ $NOTES contains markdown (##/**/\`\`\`) in the What to Test section." >&2
    echo "  TestFlight shows it verbatim — rewrite as plain text (see the file header)." >&2
    exit 1
  fi
  # Everything after the first bare `---`: the file's header explains where the
  # text goes and is not part of what testers read.
  NOTES_FILE="$NOTES" BUILD_ID="$BUILD_ID" JWT="$JWT" python3 - <<'PY'
import json, os, urllib.request, urllib.error
BASE = "https://api.appstoreconnect.apple.com"
H = {"Authorization": "Bearer " + os.environ["JWT"], "Content-Type": "application/json"}
bid = os.environ["BUILD_ID"]
text = open(os.environ["NOTES_FILE"]).read().split("\n---\n", 1)[1].strip()

def call(method, path, body=None):
    req = urllib.request.Request(BASE + path, method=method, headers=H)
    try:
        with urllib.request.urlopen(req, json.dumps(body).encode() if body else None) as r:
            raw = r.read()
            return True, (json.loads(raw) if raw else {})
    except urllib.error.HTTPError as e:
        try: return False, json.loads(e.read())
        except Exception: return False, {"errors": [{"title": "HTTP %d" % e.code}]}

ok, existing = call("GET", "/v1/builds/%s/betaBuildLocalizations" % bid)
target = next((l["id"] for l in (existing.get("data", []) if ok else [])
               if l["attributes"].get("locale") == "en-US"), None)
if target:
    ok, res = call("PATCH", "/v1/betaBuildLocalizations/" + target,
                   {"data": {"type": "betaBuildLocalizations", "id": target,
                             "attributes": {"whatsNew": text}}})
else:
    ok, res = call("POST", "/v1/betaBuildLocalizations",
                   {"data": {"type": "betaBuildLocalizations",
                             "attributes": {"locale": "en-US", "whatsNew": text},
                             "relationships": {"build": {"data": {"type": "builds", "id": bid}}}}})
print("  %s %d chars" % ("✓" if ok else "✘", len(text)))
if not ok:
    for e in res.get("errors", []): print("     ", e.get("title"), e.get("detail", ""))
PY
else
  echo "⚠ no $NOTES — testers will see an empty What to Test."
fi

echo "▶ Adding to beta groups…"
api GET "/v1/apps/$APP_ID/betaGroups?limit=20" | python3 -c "
import json, sys
print('\n'.join('%s %s' % (g['id'], g['attributes'].get('name', '?'))
                for g in json.load(sys.stdin).get('data', [])))
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
echo "Build $VERSION: uploaded, compliant, described, distributed, submitted."
[ "$STATE" = "APPROVED" ] || echo "Review is still $STATE — testers get it once that turns APPROVED."
