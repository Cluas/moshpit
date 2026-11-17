#!/bin/sh
# cargo-nextest, pinned and checked, into $CARGO_HOME/bin: the live tests
# are archived once and run in shards (`--partition`, filtersets), with a
# JUnit report per shard.
set -eu
version=0.9.150
case "$(uname -m)" in
    x86_64) triple=x86_64-unknown-linux-gnu; digest=b0959bc64232974224cf9fb22c915845398c0e8668fa7831eb5b43fb1eef781f ;;
    aarch64) triple=aarch64-unknown-linux-gnu; digest=9164568e9840c6509177086828ac7d3178c2d8feb0658b4d2bff62fa620e98c8 ;;
    *) echo "install-nextest: no pinned build for $(uname -m)" >&2; exit 2 ;;
esac
bin="${CARGO_HOME:-$HOME/.cargo}/bin"
mkdir -p "$bin"
tmp=$(mktemp)
curl -fsSL --retry 3 -o "$tmp" \
    "https://github.com/nextest-rs/nextest/releases/download/cargo-nextest-$version/cargo-nextest-$version-$triple.tar.gz"
echo "$digest  $tmp" | sha256sum -c -
tar -xzf "$tmp" -C "$bin"
rm -f "$tmp"
"$bin/cargo-nextest" nextest --version
