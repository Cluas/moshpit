#!/bin/sh
# Builds a tmux release from its source tarball into PREFIX (bin/tmux),
# the tarball checked against the digest pinned here. The live tests run on
# the tmux versions the app meets in the field: 3.6a (what most hosts still
# run) and 3.8 (where a phone's own group is safe, B812, and a plain
# client's layouts are JSON, B776).
#
#   scripts/e2e/build-tmux.sh 3.8 /opt/e2e/tmux-3.8
set -eu
version=$1
prefix=$2
case "$version" in
    3.6a) digest=b6d8d9c76585db8ef5fa00d4931902fa4b8cbe8166f528f44fc403961a3f3759 ;;
    3.8) digest=e79c699c7e949dccd0a4a125e17b8d1261e311b16979dbc8eb34542f3966d82e ;;
    *) echo "build-tmux: no digest pinned for tmux $version" >&2; exit 2 ;;
esac
if [ -x "$prefix/bin/tmux" ] && [ "$("$prefix/bin/tmux" -V)" = "tmux $version" ]; then
    echo "tmux $version already in $prefix"
    exit 0
fi
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
url="https://github.com/tmux/tmux/releases/download/$version/tmux-$version.tar.gz"
curl -fsSL --retry 3 -o "$work/tmux.tar.gz" "$url"
echo "$digest  $work/tmux.tar.gz" | sha256sum -c -
tar -xzf "$work/tmux.tar.gz" -C "$work"
cd "$work/tmux-$version"
./configure --prefix="$prefix" >/dev/null
make -j"$(nproc)" >/dev/null
mkdir -p "$prefix"
make install >/dev/null
"$prefix/bin/tmux" -V
