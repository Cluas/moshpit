#!/usr/bin/env bash
# Produce the open-source tree of Moshpit from this (private) development repo.
#
# The public repository (github.com/Cluas/moshpit) is not a mirror of this one:
# it carries the app, its extensions, the tests, the push relay and the
# developer tooling, and nothing else. What stays here, and why:
#
#   marketing/            app icon drafts (the website itself is its own repo, moshpit-site)
#   docs/appstore/        store listing, pricing strategy, release runbook
#   docs/testflight*      per-build tester notes and the machine-state guide
#   docs/app-review.md    the App Review demo host and how to rotate it
#   deploy/review-demo/   that demo host's manifests
#   docs/OSS_READINESS_AUDIT.md   an internal self-audit
#   scripts/release-{upload,promote,listing,screenshots}.*, shot-upload.py,
#   publish-design-docs.sh, capture-marketing-shots.sh
#                         bound to the maintainer's App Store Connect account,
#                         k3s cluster and mainland mirror host
#
# Usage:
#   scripts/export-oss.sh [DEST]            copy the public tree into DEST
#   scripts/export-oss.sh [DEST] --commit   ...and commit it there
#
# DEST defaults to ../moshpit-oss next to this checkout. The first run creates
# a fresh repository (single root commit); later runs replace the tree and, with
# --commit, record one "Sync from private <sha>" commit — the public history is
# a sequence of release snapshots, not every working commit.
#
# Signing.xcconfig is exported with DEVELOPMENT_TEAM blanked (contributors fill
# in their own), and the result is scanned for anything that must not leave:
# the maintainer's Team ID, App Store Connect identifiers, tokens, private-key
# blocks with key material, and the credential once quoted in the audit doc.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
DEST="$ROOT/../moshpit-oss"
COMMIT=0
for arg in "$@"; do
  case "$arg" in
    --commit) COMMIT=1 ;;
    -*) echo "export-oss: unknown option $arg" >&2; exit 64 ;;
    *) DEST=$arg ;;
  esac
done
DEST=$(mkdir -p "$DEST" && cd "$DEST" && pwd)

PUBLIC=(
  Moshpit Extensions Tests Packages
  Moshpit.xcodeproj/project.xcworkspace/xcshareddata/swiftpm/Package.resolved
  project.yml Signing.xcconfig BUILD_NUMBER Brewfile .swiftlint.yml .swiftformat .gitignore
  .github
  LICENSE README.md README-zh.md CONTRIBUTING.md CODE_OF_CONDUCT.md SECURITY.md NOTICES.md CHANGELOG.md
  
  docs/ARCHITECTURE.md docs/PATCHES.md docs/PUSH.md docs/install-free-account.md docs/design docs/assets
  push-relay
  scripts/export-oss.sh scripts/host scripts/gen scripts/verify scripts/spikes
  scripts/release/build-ipa.sh scripts/release/release-archive.sh scripts/release/team-id.sh
  scripts/capture/capture-flow-shots.sh scripts/capture/capture-mosh-switch-bytes.sh
  scripts/capture/capture_mosh_switch_bytes.swift scripts/capture/stage
)

LIST=$(mktemp)
trap 'rm -f "$LIST" "$LIST.dest"' EXIT
git -C "$ROOT" ls-files -z -- "${PUBLIC[@]}" | tr '\0' '\n' | LC_ALL=C sort -u > "$LIST"
[ -s "$LIST" ] || { echo "export-oss: nothing matched" >&2; exit 1; }

# DEST is a working checkout too (people run xcodegen and build there), so
# "stale" means files git would track that are no longer listed — never the
# generated Moshpit.xcodeproj or build/, which the exported .gitignore covers.
[ -d "$DEST/.git" ] || git -C "$DEST" init -q -b main
rsync -a --files-from="$LIST" "$ROOT/" "$DEST/"
git -C "$DEST" ls-files -z --cached --others --exclude-standard | tr '\0' '\n' | LC_ALL=C sort -u > "$LIST.dest"
LC_ALL=C comm -13 "$LIST" "$LIST.dest" | while IFS= read -r stale; do rm -f "$DEST/$stale"; done
find "$DEST" -type d -empty -not -path "$DEST/.git*" -delete

# Contributors sign with their own Team; the maintainer's stays here.
sed -i '' -E 's/^DEVELOPMENT_TEAM = .*/DEVELOPMENT_TEAM =/' "$DEST/Signing.xcconfig"

# --- leak scan -------------------------------------------------------------
fail=0
TEAM=$(sed -nE 's/^DEVELOPMENT_TEAM = *([A-Z0-9]{10}).*/\1/p' "$ROOT/Signing.xcconfig" | head -1)
# Scan what git would publish (tracked + untracked-but-not-ignored), never the
# generated project or build/, which carry the signing Team of whoever built.
SCOPE=$(mktemp); trap 'rm -f "$LIST" "$LIST.dest" "$SCOPE"' EXIT
git -C "$DEST" ls-files -z --cached --others --exclude-standard | tr '\0' '\n' | grep -v '^scripts/export-oss.sh$' > "$SCOPE" || true
scan() { # label, grep flags, pattern
  local hits
  hits=$(cd "$DEST" && tr '\n' '\0' < "$SCOPE" | xargs -0 grep -In $2 -- "$3" 2>/dev/null | head -5 || true)
  if [ -n "$hits" ]; then echo "LEAK ($1):"; echo "$hits" | cut -c1-160; fail=1; fi
}
[ -n "$TEAM" ] && scan "team id" -F "$TEAM"
scan "asc identifiers" -E 'ASC_(KEY|ISSUER)_ID=[A-Za-z0-9]|AuthKey_[A-Z0-9]*[A-WYZ0-9][A-Z0-9]*\.p8'
scan "tokens" -E 'ghp_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}|AKID[A-Za-z0-9]{20,}|tskey-[a-z]+-[A-Za-z0-9]{10,}|xox[bap]-[A-Za-z0-9-]{10,}'
scan "key material" -E '^[A-Za-z0-9+/]{64}$'
# Literal strings that must never appear (old credentials, personal mailboxes).
# The list itself is private: it is not in PUBLIC above.
if [ -f "$ROOT/scripts/oss-denylist.txt" ]; then
  while IFS= read -r word; do
    [ -n "$word" ] && [ "${word#\#}" = "$word" ] && scan "denylist" -F "$word"
  done < "$ROOT/scripts/oss-denylist.txt"
fi
if [ "$fail" -ne 0 ]; then echo "export-oss: refusing to continue" >&2; exit 2; fi

# --- dangling references (warnings only) ----------------------------------
refs=$(cd "$DEST" && tr '\n' '\0' < "$SCOPE" | xargs -0 grep -IEn -- 'marketing/|docs/appstore|docs/testflight|deploy/review-demo|app-review\.md|OSS_READINESS|release-(upload|promote|listing|screenshots)|deploy-site|shot-upload|capture-marketing-shots' 2>/dev/null || true)
if [ -n "$refs" ]; then
  echo "note: $(echo "$refs" | wc -l | tr -d ' ') reference(s) to private-only paths (comments/docs, harmless):"
  echo "$refs" | cut -d: -f1 | sort | uniq -c | sort -rn | head -8
fi

echo "exported $(wc -l < "$LIST" | tr -d ' ') files to $DEST"

# --- commit ----------------------------------------------------------------
if [ "$COMMIT" -eq 1 ]; then
  SHA=$(git -C "$ROOT" rev-parse --short HEAD)
  if ! git -C "$DEST" rev-parse -q --verify HEAD >/dev/null 2>&1; then
    git -C "$DEST" add -A
    git -C "$DEST" commit -q -m "Moshpit goes open source: the app, its extensions and the push relay" \
      -m "Exported from the development tree at $SHA by scripts/export-oss.sh."
  else
    git -C "$DEST" add -A
    if git -C "$DEST" diff --cached --quiet; then echo "nothing to commit"; else
      git -C "$DEST" commit -q -m "Sync from private $SHA"
    fi
  fi
  git -C "$DEST" log --oneline -1
fi
