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
set -euo pipefail

REPO=ghcr.io/cluas/offhook-site
SITE="$(cd "$(dirname "$0")/.." && pwd)/marketing/site"
TAG="$(git -C "$(dirname "$SITE")/.." rev-parse --short HEAD)-$(date +%H%M)"
DRY=${1:-}

# The cluster runs both. A single-arch image schedules onto half the nodes and
# ImagePullBackOffs on the rest, which looks like a broken site to everyone
# whose request happens to land wrong.
PLATFORMS=linux/amd64,linux/arm64

say() { printf '\n\033[1m%s\033[0m\n' "$*"; }

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
docker build -q -t offhook-site:check "$SITE" >/dev/null
docker run --rm offhook-site:check nginx -t

# Gate 3: the files the site cannot work without.
for f in index.html assets/offhook.css assets/docs-index.json assets/docs-search.js; do
  docker run --rm offhook-site:check test -f "/usr/share/nginx/html/$f" \
    || { echo "missing: $f" >&2; exit 1; }
done
echo "  required files present"

if [[ "$DRY" == "--dry-run" ]]; then
  say "Dry run — built and verified, pushed nothing"
  exit 0
fi

say "Building and pushing $REPO:$TAG ($PLATFORMS)"
docker buildx build --builder multiarch --platform "$PLATFORMS" \
  -t "$REPO:$TAG" -t "$REPO:latest" --push "$SITE"

say "Rolling out"
# Apply the manifest first: it is what removes the old ConfigMap volume
# mounts, which sat on exactly the paths the image serves from. Setting the
# image without it leaves nginx reading the mounted ConfigMaps — the deploy
# reports success and ships nothing.
kubectl apply -f "$SITE/k8s.yaml"
kubectl set image deploy/offhook-site "nginx=$REPO:$TAG"
kubectl rollout status deploy/offhook-site --timeout=180s

say "Verifying from outside the cluster"
for u in / /docs /docs/herdr /pricing /zh/docs/herdr; do
  code=$(curl -s -o /dev/null -w '%{http_code}' "https://offhook.cluas.eu.org$u")
  printf '  %-18s %s\n' "$u" "$code"
  [[ "$code" == 200 ]] || { echo "  ^ FAILED" >&2; exit 1; }
done
say "Deployed $REPO:$TAG"
