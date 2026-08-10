#!/usr/bin/env bash
# Build, verify and ship the marketing site as a container image.
#
# Replaces the three-ConfigMap deploy. That worked until it didn't: a ConfigMap
# caps at 1MiB, the pages map reached 876KiB of it, and the docs still need
# dozens of screenshots and a set of short clips. Anything media-heavy was
# simply not deployable. An image has no such ceiling, is one immutable
# artifact instead of three mutable maps, and updates without the
# rollout-restart needed to make a mounted ConfigMap reach nginx.
#
#   scripts/deploy-site.sh            # build, check, push, roll out
#   scripts/deploy-site.sh --dry-run  # build and check locally, push nothing
#
# If the push cannot reach the keychain, export a GitHub token with
# write:packages first and this script will use it instead:
#   export GHCR_TOKEN=ghp_…    # or: GHCR_TOKEN=$(gh auth token)
#
set -euo pipefail

REPO=ghcr.io/cluas/moshpit-site
SITE="$(cd "$(dirname "$0")/.." && pwd)/marketing/site"
TAG="$(git -C "$(dirname "$SITE")/.." rev-parse --short HEAD)-$(date +%H%M)"
DRY=${1:-}

# The cluster runs both. A single-arch image schedules onto half the nodes and
# ImagePullBackOffs on the rest, which looks like a broken site to everyone
# whose request happens to land wrong.
PLATFORMS=linux/amd64,linux/arm64

say() { printf '\n\033[1m%s\033[0m\n' "$*"; }
die() { printf '\n\033[1;31m%s\033[0m\n' "$*" >&2; exit 1; }

# Preflight: Docker here is configured with credsStore=osxkeychain, and when
# the keychain will not unlock for a non-interactive shell the helper does not
# fail — it blocks forever. Every registry call then hangs, including the pull
# of a public base image, with no output at all. Ten minutes of silence looks
# exactly like a slow multi-arch build, so probe it up front and say so.
#
# GHCR_TOKEN skips the helper entirely: a throwaway DOCKER_CONFIG holding just
# that token, which is also what makes this script work on CI.
if [[ "${DRY:-}" != "--dry-run" ]]; then
  if [[ -n "${GHCR_TOKEN:-}" ]]; then
    # DOCKER_CONFIG is not just credentials — the CLI resolves plugins,
    # buildx builders and daemon contexts under it. Pointing it at an empty
    # directory to dodge the keychain hid all three in turn: first "no builder
    # multiarch", then "unable to parse docker host orbstack". So mirror the
    # real directory and replace only the file that names the keychain.
    export DOCKER_CONFIG="$(mktemp -d)"
    for entry in "$HOME"/.docker/*; do
      [[ "$(basename "$entry")" == "config.json" ]] && continue
      ln -sfn "$entry" "$DOCKER_CONFIG/$(basename "$entry")" 2>/dev/null || true
    done
    # Same settings as the user's config, minus credsStore, so the token below
    # is written into this throwaway file instead of the keychain.
    python3 - "$HOME/.docker/config.json" "$DOCKER_CONFIG/config.json" <<'PY'
import json, sys
try:
    cfg = json.load(open(sys.argv[1]))
except Exception:
    cfg = {}
cfg.pop("credsStore", None); cfg.pop("credHelpers", None); cfg.pop("auths", None)
json.dump(cfg, open(sys.argv[2], "w"))
PY
    printf '%s' "$GHCR_TOKEN" | docker login ghcr.io -u "${GHCR_USER:-cluas}" --password-stdin >/dev/null \
      || die "GHCR_TOKEN was rejected by ghcr.io. It needs the write:packages scope.

A plain \`gh auth token\` does not carry it — the default gh scopes are
repo/read:org/gist. Add it once:

    gh auth refresh -h github.com -s write:packages"
    say "Authenticated to ghcr.io with GHCR_TOKEN"
  elif ! echo ghcr.io | timeout 5 docker-credential-osxkeychain get >/dev/null 2>&1; then
    die "Docker's credential helper is not responding (docker-credential-osxkeychain hangs on ghcr.io).

The keychain will not unlock for this shell, so any push would hang silently.
Either unlock it by running a docker command yourself in a terminal where the
keychain prompt can appear, or hand this script a token instead:

    gh auth refresh -h github.com -s write:packages
    export GHCR_TOKEN=\$(gh auth token)
    scripts/deploy-site.sh"
  fi
fi

say "Rebuilding the docs shell"
python3 "$(dirname "$0")/build-docs.py"

# Gate 1: the pages must not reference an asset relatively. Served at pretty
# paths like /docs/herdr, a relative href resolves against /docs/ and 404s —
# this once shipped every docs page with no stylesheet at all.
say "Checking asset references"
if grep -rlE '(href|src)="[^"/#][^":]*\.(css|js|png|jpe?g|svg|json)' "$SITE"/*.html; then
  echo "  ^ these pages reference an asset relatively; must be root-absolute" >&2
  exit 1
fi
echo "  all root-absolute"

# Gate 2: nginx has to accept the config. Run natively — the image itself
# carries no RUN step so that the multi-arch build stays a pure file copy.
say "Validating nginx.conf"
docker build -q -t moshpit-site:check "$SITE" >/dev/null
docker run --rm moshpit-site:check nginx -t

# Gate 3: the files the site cannot work without.
for f in index.html assets/moshpit.css assets/docs-index.json assets/docs-search.js; do
  docker run --rm moshpit-site:check test -f "/usr/share/nginx/html/$f" \
    || { echo "missing: $f" >&2; exit 1; }
done
echo "  required files present"

if [[ "$DRY" == "--dry-run" ]]; then
  say "Dry run — built and verified, pushed nothing"
  exit 0
fi

say "Building and pushing $REPO:$TAG ($PLATFORMS)"
# The default docker driver cannot emit a multi-arch manifest; that needs a
# docker-container builder. Create it if this machine has not got one rather
# than failing after every gate has already passed.
if ! docker buildx inspect multiarch >/dev/null 2>&1; then
  say "Creating the multiarch builder"
  docker buildx create --name multiarch --driver docker-container --bootstrap >/dev/null
fi
docker buildx build --builder multiarch --platform "$PLATFORMS" \
  -t "$REPO:$TAG" -t "$REPO:latest" --push "$SITE"

say "Rolling out"
# Apply the manifest first: it is what removes the old ConfigMap volume
# mounts, which sat on exactly the paths the image serves from. Setting the
# image without it leaves nginx reading the mounted ConfigMaps — the deploy
# reports success and ships nothing.
kubectl apply -f "$SITE/k8s.yaml"
kubectl set image deploy/moshpit-site "nginx=$REPO:$TAG"
kubectl rollout status deploy/moshpit-site --timeout=180s

say "Verifying from outside the cluster"
for u in / /docs /docs/herdr /pricing /zh/docs/herdr; do
  code=$(curl -s -o /dev/null -w '%{http_code}' "https://moshpit.cluas.eu.org$u")
  printf '  %-18s %s\n' "$u" "$code"
  [[ "$code" == 200 ]] || { echo "  ^ FAILED" >&2; exit 1; }
done
say "Deployed $REPO:$TAG"
