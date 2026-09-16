#!/usr/bin/env bash
# Produce the open-source tree of Moshpit from this (private) development repo.
#
# The public repository (github.com/Cluas/moshpit) is not a mirror of this one:
# it carries the app, its extensions, the tests, the push relay and the
# developer tooling, and nothing else. What stays here, and why:
#
#   marketing/            app icon drafts (the website itself is its own repo, moshpit-site)
#   scripts/oss-denylist.txt      the literal strings the leak scan below refuses
#   scripts/publish-design-docs.sh, scripts/capture/capture-marketing-shots.sh
#                         bound to the maintainer's local servers and site checkout
#
# The App Store pipeline (release-upload/promote/listing/screenshots), the store
# listing and runbook, the TestFlight notes, the App Review demo host and the
# OSS readiness audit moved to their own private repository, moshpit-ops, on
# 2026-09-14. They were never exported from here either; release-archive.sh
# (public) only looks for the tester notes in that checkout when it is present.
#
# Usage:
#   scripts/export-oss.sh [DEST]                 copy the public tree into DEST
#   scripts/export-oss.sh [DEST] --commit        replay the development commits
#                                                since the last export as commits
#   scripts/export-oss.sh [DEST] --commit --since <sha>
#                                                ...starting after <sha> when DEST's
#                                                last commit carries no source mark
#
# DEST defaults to ../moshpit-oss next to this checkout. The first run creates
# a fresh repository (single root commit). Later runs with --commit walk the
# development history since the previous export, first-parent, and re-create
# each commit that touches a public path: the same author, date and message,
# plus a Source-Commit trailer naming the development commit it came from. The
# public log therefore reads like an ordinary history — one commit per change,
# in the author's words — with the private-only paths simply absent. The
# trailer is also how the next run finds where the previous export stopped
# (the older "Sync from private <sha>" subjects are recognised too).
#
# A development commit whose tree fails the leak scan is not published on its
# own: its changes ride along in the next commit whose tree passes, which says
# so in its body and names it in Folds-Source-Commit trailers. Every tree is
# judged by today's rules, so this is what happens when a word joins the
# denylist after the commit that first wrote it. If HEAD's own tree fails,
# nothing is published and the run stops — fix the tree first.
#
# Signing.xcconfig is exported with DEVELOPMENT_TEAM blanked (contributors fill
# in their own), and the result is scanned for anything that must not leave:
# the maintainer's Team ID, App Store Connect identifiers, tokens, private-key
# blocks with key material, and the credential once quoted in the audit doc.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
DEST="$ROOT/../moshpit-oss"
COMMIT=0
SINCE_ARG=""
while [ $# -gt 0 ]; do
  case "$1" in
    --commit) COMMIT=1 ;;
    --since) shift; SINCE_ARG=${1:-}; [ -n "$SINCE_ARG" ] || { echo "export-oss: --since needs a commit" >&2; exit 64; } ;;
    -*) echo "export-oss: unknown option $1" >&2; exit 64 ;;
    *) DEST=$1 ;;
  esac
  shift
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

LIST=$(mktemp); SCOPE=$(mktemp)
WT=""
cleanup() {
  rm -f "$LIST" "$LIST.dest" "$SCOPE"
  [ -n "$WT" ] && git -C "$ROOT" worktree remove --force "$WT" 2>/dev/null
  true
}
trap cleanup EXIT

# Copy the public tree of SRC (a checkout of this repository — the working
# tree, or a detached worktree at some commit) into DEST and scan it. The
# scan's Team ID and denylist always come from the current tree, so an old
# commit is judged by today's rules.
export_tree() {
  local SRC=$1
  git -C "$SRC" ls-files -z -- "${PUBLIC[@]}" 2>/dev/null | tr '\0' '\n' | LC_ALL=C sort -u > "$LIST"
  [ -s "$LIST" ] || { echo "export-oss: nothing matched in $SRC" >&2; exit 1; }

  # DEST is a working checkout too (people run xcodegen and build there), so
  # "stale" means files git would track that are no longer listed — never the
  # generated Moshpit.xcodeproj or build/, which the exported .gitignore covers.
  [ -d "$DEST/.git" ] || git -C "$DEST" init -q -b main
  rsync -a --files-from="$LIST" "$SRC/" "$DEST/"
  git -C "$DEST" ls-files -z --cached --others --exclude-standard | tr '\0' '\n' | LC_ALL=C sort -u > "$LIST.dest"
  LC_ALL=C comm -13 "$LIST" "$LIST.dest" | while IFS= read -r stale; do rm -f "$DEST/$stale"; done
  find "$DEST" -type d -empty -not -path "$DEST/.git*" -delete

  # Contributors sign with their own Team; the maintainer's stays here.
  [ -f "$DEST/Signing.xcconfig" ] && sed -i '' -E 's/^DEVELOPMENT_TEAM = .*/DEVELOPMENT_TEAM =/' "$DEST/Signing.xcconfig"

  # --- leak scan -----------------------------------------------------------
  local fail=0 TEAM
  TEAM=$(sed -nE 's/^DEVELOPMENT_TEAM = *([A-Z0-9]{10}).*/\1/p' "$ROOT/Signing.xcconfig" | head -1)
  # Scan what git would publish (tracked + untracked-but-not-ignored), never the
  # generated project or build/, which carry the signing Team of whoever built.
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
  if [ "$fail" -ne 0 ]; then return 2; fi
}

# Comments and docs may still name private-only paths; say so, once, for the
# tree that ends up published.
dangling_refs() {
  local refs
  refs=$(cd "$DEST" && tr '\n' '\0' < "$SCOPE" | xargs -0 grep -IEn -- 'marketing/|docs/appstore|docs/testflight|deploy/review-demo|app-review\.md|OSS_READINESS|release-(upload|promote|listing|screenshots)|deploy-site|shot-upload|capture-marketing-shots' 2>/dev/null || true)
  if [ -n "$refs" ]; then
    echo "note: $(echo "$refs" | wc -l | tr -d ' ') reference(s) to private-only paths (comments/docs, harmless):"
    echo "$refs" | cut -d: -f1 | sort | uniq -c | sort -rn | head -8
  fi
}

# The development commit the previous export stopped at: the Source-Commit
# trailer on DEST's last commit, or the sha in the older "Sync from private
# <sha>" / root-commit wording. Prints nothing when there is no mark.
previous_source() {
  local msg
  msg=$(git -C "$DEST" log -1 --format=%B 2>/dev/null) || return 0
  { sed -n 's/^Source-Commit: \([0-9a-f]*\).*/\1/p' <<<"$msg"
    sed -n 's/^Sync from private \([0-9a-f]*\).*/\1/p' <<<"$msg"
    sed -n 's/^Exported from the development tree at \([0-9a-f]*\).*/\1/p' <<<"$msg"
  } | grep . | head -1 || true
}

if [ "$COMMIT" -eq 0 ]; then
  export_tree "$ROOT" || { echo "export-oss: refusing to continue" >&2; exit 2; }
  dangling_refs
  echo "exported $(wc -l < "$LIST" | tr -d ' ') files to $DEST"
  exit 0
fi

# --- first export: one root commit ---------------------------------------
if ! git -C "$DEST" rev-parse -q --verify HEAD >/dev/null 2>&1; then
  export_tree "$ROOT" || { echo "export-oss: refusing to continue" >&2; exit 2; }
  dangling_refs
  HEAD_SHA=$(git -C "$ROOT" rev-parse HEAD)
  git -C "$DEST" add -A
  git -C "$DEST" commit -q -m "Moshpit goes open source: the app, its extensions and the push relay" \
    -m "Exported from the development tree at ${HEAD_SHA:0:7} by scripts/export-oss.sh." \
    -m "Source-Commit: $HEAD_SHA"
  git -C "$DEST" log --oneline -1
  exit 0
fi

# --- later exports: replay the development commits -------------------------
SINCE=${SINCE_ARG:-$(previous_source)}
if [ -z "$SINCE" ]; then
  echo "export-oss: $DEST's last commit carries no Source-Commit mark; pass --since <development sha>" >&2
  exit 65
fi
if ! git -C "$ROOT" merge-base --is-ancestor "$SINCE" HEAD 2>/dev/null; then
  echo "export-oss: $SINCE is not an ancestor of HEAD in $ROOT" >&2
  exit 65
fi
if ! git -C "$ROOT" diff --quiet HEAD -- "${PUBLIC[@]}" 2>/dev/null; then
  echo "note: uncommitted changes to public paths in $ROOT are not exported — commit them first"
fi

# The development commit's message, with a paragraph naming the commits folded
# into this one (before the trailer block, so the original trailers stay
# trailers) and the source trailers appended.
compose_message() { # sha folded...
  local sha=$1 msg note="" f
  local -a folds=()
  shift
  msg=$(git -C "$ROOT" log -1 --format=%B "$sha")
  for f in "$@"; do folds+=(--trailer "Folds-Source-Commit: $f"); done
  if [ $# -gt 0 ]; then
    note="Carries the changes of $# earlier development commit(s) whose trees the"
    note+=$'\n'"export's leak scan would not publish on their own:"
    for f in "$@"; do note+=$'\n'"  ${f:0:7} $(git -C "$ROOT" log -1 --format=%s "$f")"; done
  fi
  # (the note goes in through the environment: macOS awk rejects a -v value
  # with newlines in it)
  printf '%s\n' "$msg" \
    | NOTE="$note" awk -v has="$(printf '%s\n' "$msg" | git interpret-trailers --parse | grep -c . || true)" \
        'BEGIN { RS = ""; ORS = ""; note = ENVIRON["NOTE"] }
         { p[NR] = $0 }
         END { for (i = 1; i <= NR; i++) { if (note != "" && i == NR && has > 0) print note "\n\n"; print p[i] "\n\n" }
               if (note != "" && has == 0) print note "\n" }' \
    | git interpret-trailers ${folds[@]+"${folds[@]}"} --trailer "Source-Commit: $sha"
}

made=0
folded=()
for sha in $(git -C "$ROOT" rev-list --reverse --first-parent "$SINCE..HEAD"); do
  # Commits that touch no public path leave nothing to publish.
  if git -C "$ROOT" diff --quiet "$sha^" "$sha" -- "${PUBLIC[@]}" 2>/dev/null; then continue; fi
  WT=$(mktemp -d "${TMPDIR:-/tmp}/export-oss.XXXXXX")
  git -C "$ROOT" worktree add --detach -q "$WT" "$sha"
  if ! export_tree "$WT"; then
    # This tree must not be published as it stands. Put DEST back and carry
    # the commit into the next one whose tree passes (its changes are in
    # that tree anyway).
    echo "  folding ${sha:0:7} into the next clean commit: $(git -C "$ROOT" log -1 --format=%s "$sha")"
    git -C "$DEST" checkout -q -- . && git -C "$DEST" clean -qfd
    folded+=("$sha")
    git -C "$ROOT" worktree remove --force "$WT"; WT=""
    continue
  fi
  git -C "$DEST" add -A
  if git -C "$DEST" diff --cached --quiet; then
    folded=()
    git -C "$ROOT" worktree remove --force "$WT"; WT=""
    continue
  fi
  compose_message "$sha" ${folded[@]+"${folded[@]}"} \
    | git -C "$DEST" commit -q -F - \
        --author="$(git -C "$ROOT" log -1 --format='%an <%ae>' "$sha")" \
        --date="$(git -C "$ROOT" log -1 --format=%aD "$sha")"
  folded=()
  git -C "$DEST" log --oneline -1
  made=$((made + 1))
  git -C "$ROOT" worktree remove --force "$WT"; WT=""
done
if [ "${#folded[@]}" -gt 0 ]; then
  echo "export-oss: the tree at HEAD fails the leak scan; ${#folded[@]} commit(s) since the last export stay unpublished" >&2
  exit 2
fi
dangling_refs
if [ "$made" -eq 0 ]; then echo "nothing to commit: $DEST is already at $(git -C "$ROOT" rev-parse --short HEAD)"; fi
