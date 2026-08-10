# The built site, as an immutable image.
#
# Deliberately no RUN step, and deliberately no `npm run build` in here either.
# The cluster runs a mix of amd64 and arm64 nodes, so the manifest has to cover
# both; a multi-arch build would emulate the foreign architecture to execute any
# RUN, which is what once turned a file copy into a build that timed out after
# three minutes. With COPY alone each platform is just files and the push takes
# seconds. `astro build` and `nginx -t` both run natively, on one architecture,
# in scripts/deploy-site.sh before anything is pushed.
#
# This means dist/ must already exist. deploy-site.sh guarantees that; building
# this Dockerfile by hand does not — but a missing dist/ makes the COPY itself
# fail, so the failure mode is a build error rather than an empty site.
FROM nginx:1.27-alpine

COPY nginx.conf /etc/nginx/conf.d/default.conf

# The Astro output, verbatim: pages at the paths their URLs imply, hashed CSS
# and JS under /_astro/, the Pagefind index under /pagefind/, and everything
# from public/ at the root.
COPY dist/ /usr/share/nginx/html/
