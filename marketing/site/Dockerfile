# The site outgrew ConfigMaps.
#
# It shipped as three of them (pages, assets, nginx.conf) because a ConfigMap
# caps at 1MiB and the pages plus screenshots did not fit in one. At the point
# this file was written the pages map was at 876KiB of its 1024KiB — roughly
# two more screenshots of headroom — and the docs still need dozens of stills
# and a set of short clips before they explain anything. Baking the site into
# the image removes the ceiling, makes a deploy one immutable artifact instead
# of three mutable maps, and drops the rollout-restart dance needed to make a
# mounted ConfigMap update actually reach nginx.
#
# Build, check and push with scripts/deploy-site.sh — the cluster runs a mix of
# amd64 and arm64 nodes, so the pushed manifest has to cover both.
#
# Deliberately no RUN step. A multi-arch build would have to emulate the
# foreign architecture to execute one, which turned a file copy into a build
# that timed out after three minutes; with COPY alone each platform is just
# files and the push is seconds. The `nginx -t` gate still runs — natively, on
# one architecture, in deploy-site.sh before anything is pushed.
FROM nginx:1.27-alpine

# nginx.conf carries the pretty-URL map, gzip, and the cache policy.
COPY nginx.conf /etc/nginx/conf.d/default.conf

# Pages at the root, assets under /assets. The `try_files … /assets/$uri`
# fallback in nginx.conf means pages can keep referencing /moshpit.css and
# /island-working.jpg without knowing which directory they landed in — the
# same contract the ConfigMap split established, kept so no page has to change.
COPY *.html /usr/share/nginx/html/
COPY assets/ /usr/share/nginx/html/assets/
