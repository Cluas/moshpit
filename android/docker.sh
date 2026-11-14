#!/usr/bin/env bash
# CI's Android build (android/ci.sh) in a Linux arm64 container, for the
# development Mac where no JDK 17+ starts (macOS 27) and Gradle can't run.
# The container starts from a plain Debian with a JDK 17; ci.sh fetches the
# rest as on a bare machine. The Android SDK's Linux tools (the NDK's clang,
# AGP's aapt2) exist for x86_64 only: the container gets x86_64's C runtime
# and the Docker VM runs them emulated (OrbStack does; rustc, the JDK and
# Gradle run natively).
#
#   android/docker.sh <cache dir> [gradle tasks…]
#
# The OrbStack VM this was verified in has 4 GB, about 2 GB of it free:
# the app profile's fat LTO of the whole library (one rustc, one codegen
# unit) was killed for memory there, with or without Gradle beside it. The
# container builds that profile with thin LTO unless CARGO_PROFILE_APP_LTO
# says otherwise (CARGO_PROFILE_APP_LTO=fat on a VM with 6 GB or more): the
# same sources, Gradle tasks and packaging; CI's bundle is fat LTO, a few
# percent smaller.
#
# <cache dir> holds everything the build fetches and makes (Debian packages,
# the SDK, rustup, cargo's registry, Gradle's caches, a copy of the worktree
# and its target/): a second run reuses it; delete it when done. The worktree
# itself is mounted read-only, never written. The bundle lands in
# android/build/gradle/. The image is debian:bookworm-slim; remove it when
# done if it was pulled for this (`docker rmi debian:bookworm-slim`).
set -euo pipefail

IMAGE=debian:bookworm-slim

if [ "${1:-}" = --inside ]; then
    shift
    export DEBIAN_FRONTEND=noninteractive
    # Keep downloaded packages in the cache (the image deletes them by default).
    rm -f /etc/apt/apt.conf.d/docker-clean
    mkdir -p /cache/apt/partial
    echo 'Dir::Cache::archives "/cache/apt";' >/etc/apt/apt.conf.d/90cache
    dpkg --add-architecture amd64
    apt-get update -qq
    apt-get install -y -qq --no-install-recommends \
        openjdk-17-jdk-headless ca-certificates curl unzip git python3 gcc libc6-dev rsync \
        libc6:amd64 libstdc++6:amd64 zlib1g:amd64 >/dev/null
    rsync -a --delete \
        --exclude /target/ --exclude /android/build/ --exclude /android/app/build/ \
        --exclude /android/.gradle/ --exclude '/android/termux/*/build/' --exclude /.git \
        /src/ /cache/work/
    export HOME=/cache/home ANDROID_HOME=/cache/sdk CARGO_HOME=/cache/cargo RUSTUP_HOME=/cache/rustup \
        GRADLE_USER_HOME=/cache/gradle TOOLS_CACHE=/cache/tools
    export PATH="$CARGO_HOME/bin:$PATH"
    mkdir -p "$HOME"
    status=0
    /cache/work/android/ci.sh "$@" || status=$?
    rm -rf /cache/out && mkdir -p /cache/out
    cp -a /cache/work/android/app/build/outputs/bundle/release/. /cache/out/ 2>/dev/null || true
    cp -a /cache/work/android/app/build/reports /cache/out/reports 2>/dev/null || true
    exit "$status"
fi

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
CACHE=${1:?usage: android/docker.sh <cache dir> [gradle tasks…]}
shift
mkdir -p "$CACHE"
CACHE="$(cd "$CACHE" && pwd -P)"
status=0
docker run --rm --platform linux/arm64 --name "moshpit-android-$$" \
    -v "$ROOT:/src:ro" -v "$CACHE:/cache" \
    -e CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}" \
    -e CARGO_PROFILE_APP_LTO="${CARGO_PROFILE_APP_LTO:-thin}" \
    "$IMAGE" bash /src/android/docker.sh --inside "$@" || status=$?
mkdir -p "$HERE/build/gradle"
cp -a "$CACHE/out/." "$HERE/build/gradle/" 2>/dev/null || true
ls -l "$HERE"/build/gradle/*.aab 2>/dev/null || true
exit "$status"
