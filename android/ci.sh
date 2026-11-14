#!/usr/bin/env bash
# The Android release build as CI runs it: the host's Java unit tests (JUnit)
# and the Play bundle (app/build/outputs/bundle/release/app-release.aab), by
# Gradle, with libnsh.so from android/rust.sh (cargo profile `app`).
#
# One script for both places that run Gradle:
#   - CI (.github/workflows/ci.yml, job android): ubuntu-latest, JDK 17 from
#     setup-java, Gradle from setup-gradle, the runner's Android SDK;
#   - android/docker.sh: the same build in a Linux container on the
#     development Mac (no JDK 17+ starts on macOS 27), where only a JDK is
#     installed.
# What a place lacks, this fetches: the Android command-line tools (when
# $ANDROID_HOME has no sdkmanager), rustup (when there is none), Gradle (when
# none is on PATH). Then, everywhere: the SDK parts (platform, build-tools,
# the NDK android/gradle/libs.versions.toml pins), the toolchain
# rust-toolchain.toml pins with the Android target, and Gradle.
#
#   ANDROID_HOME=<sdk> android/ci.sh                 # unit tests + release bundle
#   ANDROID_HOME=<sdk> android/ci.sh :app:assembleDebug   # other Gradle tasks
#
#   TOOLS_CACHE   where fetched Gradle and command-line tools go
#                 (default ~/.cache/moshpit-android)
#   MOSHPIT_UPLOAD_KEYSTORE and co: the upload key (app/build.gradle); without
#                 it the bundle is unsigned.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
: "${ANDROID_HOME:?ANDROID_HOME: the Android SDK (created when it does not exist)}"
export ANDROID_HOME
TOOLS_CACHE=${TOOLS_CACHE:-$HOME/.cache/moshpit-android}

# Pinned downloads (used only where the tool is missing).
GRADLE_VERSION=8.11.1
GRADLE_SHA256=f397b287023acdba1e9f6fc5ea72d22dd63669d59ed4a289a29b1a76eee151c6
# cmdline-tools 23.0; the SHA-1 is the one Google's repository2-3.xml lists.
CMDLINE_TOOLS=commandlinetools-linux-16111833_latest.zip
CMDLINE_TOOLS_SHA1=e025545c62a8e64c7559119566a569fb1dec5f60

NDK=$(sed -n 's/^ndk = "\(.*\)"/\1/p' "$HERE/gradle/libs.versions.toml")
COMPILE_SDK=$(sed -n 's/^compileSdk = "\(.*\)"/\1/p' "$HERE/gradle/libs.versions.toml")
[ -n "$NDK" ] && [ -n "$COMPILE_SDK" ] || { echo "libs.versions.toml: no ndk / compileSdk" >&2; exit 1; }

step() { printf '\n== %s\n' "$*"; }
fetch() { # url file
    [ -s "$2" ] || { mkdir -p "$(dirname "$2")"; curl -fsSL --retry 3 -o "$2.part" "$1" && mv "$2.part" "$2"; }
}

# --- Android SDK -----------------------------------------------------------------
SDKMANAGER="$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager"
if [ ! -x "$SDKMANAGER" ]; then
    step "Android command-line tools ($CMDLINE_TOOLS)"
    fetch "https://dl.google.com/android/repository/$CMDLINE_TOOLS" "$TOOLS_CACHE/$CMDLINE_TOOLS"
    echo "$CMDLINE_TOOLS_SHA1  $TOOLS_CACHE/$CMDLINE_TOOLS" | sha1sum -c -
    rm -rf "$ANDROID_HOME/cmdline-tools/latest" "$TOOLS_CACHE/cmdline-tools"
    mkdir -p "$ANDROID_HOME/cmdline-tools"
    unzip -q -d "$TOOLS_CACHE" "$TOOLS_CACHE/$CMDLINE_TOOLS"
    mv "$TOOLS_CACHE/cmdline-tools" "$ANDROID_HOME/cmdline-tools/latest"
fi
step "Android SDK: platforms;android-$COMPILE_SDK, build-tools;35.0.0, ndk;$NDK"
yes | "$SDKMANAGER" --licenses >/dev/null || true
"$SDKMANAGER" "platforms;android-$COMPILE_SDK" "build-tools;35.0.0" "ndk;$NDK" | grep -v '^\[' || true
[ -d "$ANDROID_HOME/ndk/$NDK" ] || { echo "ndk;$NDK did not install" >&2; exit 1; }

# --- Rust ------------------------------------------------------------------------
if ! command -v rustup >/dev/null; then
    step "rustup"
    curl -fsSL --retry 3 https://sh.rustup.rs | sh -s -- -y --no-modify-path --profile minimal --default-toolchain none
    export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
fi
step "Rust (rust-toolchain.toml) and aarch64-linux-android"
(cd "$ROOT" && rustup toolchain install && rustup target add aarch64-linux-android && rustc --version)

# --- Gradle ----------------------------------------------------------------------
GRADLE=gradle
if ! command -v gradle >/dev/null; then
    step "Gradle $GRADLE_VERSION"
    zip="$TOOLS_CACHE/gradle-$GRADLE_VERSION-bin.zip"
    fetch "https://services.gradle.org/distributions/gradle-$GRADLE_VERSION-bin.zip" "$zip"
    echo "$GRADLE_SHA256  $zip" | sha256sum -c -
    [ -x "$TOOLS_CACHE/gradle-$GRADLE_VERSION/bin/gradle" ] || unzip -q -o -d "$TOOLS_CACHE" "$zip"
    GRADLE="$TOOLS_CACHE/gradle-$GRADLE_VERSION/bin/gradle"
fi

if [ $# -eq 0 ]; then set -- :app:testDebugUnitTest :app:bundleRelease; fi

# The release library before Gradle starts: rustc's link-time optimisation of
# the whole library and Gradle's daemon don't hold memory at the same time (a
# 4 GB container ran out with both). Gradle's own cargo task then finds it
# built (android/app/build.gradle runs the same android/rust.sh).
case " $* " in
*Release*)
    step "libnsh.so (cargo profile app), before Gradle"
    ANDROID_NDK_HOME="$ANDROID_HOME/ndk/$NDK" "$HERE/rust.sh" app \
        "$HERE/app/build/rust/release/jniLibs" "$HERE/app/build/rust/release/symbols"
    ;;
esac

step "Gradle: $*"
java -version 2>&1 | head -1
"$GRADLE" -p "$HERE" --no-daemon --stacktrace "$@"

for aab in "$HERE"/app/build/outputs/bundle/release/*.aab; do
    [ -e "$aab" ] && ls -l "$aab" | awk '{ print "bundle: " $NF " (" $5 " bytes)" }'
done
