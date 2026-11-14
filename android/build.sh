#!/usr/bin/env bash
# Builds Moshpit's APK without Gradle (android/build/moshpit-debug.apk), the way
# gate G3's spike did: this machine can't run Gradle (every JDK 17+ crashes on
# macOS 27, and there is no Rosetta), so Zulu 15 runs javac, D8 and apksigner
# as jars, and the SDK's aapt2 and zipalign do the rest. CI builds the release
# AAB with Gradle instead (android/app/build.gradle); both build from the same
# sources, manifest and resources.
#
#   android/build.sh                 # Rust (release profile) + Java + APK
#   SKIP_RUST=1 android/build.sh     # reuse the last libnsh.so
#   PROFILE=app android/build.sh     # the shipping Rust profile (slow: LTO)
#
# Steps:
#   1. libnsh.so: crates/android (android/rust.sh).
#   2. Resources: the app's and Termux's view's, compiled and linked with the
#      manifest by aapt2, which writes R.java for both packages.
#   3. Java: NSH's runtime from the very checkout Cargo.lock pins (its Java
#      and Rust halves match), Termux's terminal libraries, the host; javac
#      for Java 15, one classes.dex by D8.
#   4. The APK: libnsh.so stored (uncompressed), the dex, zipalign on 16 KB
#      pages, apksigner with the debug key.
#   5. The host's Java unit tests (android/app/src/test), on the JVM, with a
#      tiny runner (no JUnit here).
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
OUT="$HERE/build"
APP="$HERE/app/src/main"

PACKAGE=com.cluas.moshpit
ABI=arm64-v8a
MIN_SDK=26
TARGET_SDK=35
VERSION_NAME=${VERSION_NAME:-0.1.0}
VERSION_CODE=${VERSION_CODE:-1}
PROFILE=${PROFILE:-release}

: "${ANDROID_HOME:=$HOME/Library/Android/sdk}"
: "${JAVA_HOME:=/Library/Java/JavaVirtualMachines/zulu-15.jdk/Contents/Home}"
export ANDROID_HOME
BUILD_TOOLS="${ANDROID_BUILD_TOOLS:-$ANDROID_HOME/build-tools/35.0.0}"
ANDROID_JAR="$ANDROID_HOME/platforms/android-$TARGET_SDK/android.jar"
JAVA="$JAVA_HOME/bin/java"
JAVAC="$JAVA_HOME/bin/javac"
JAR="$JAVA_HOME/bin/jar"
NICE=(nice -n 10)

step() { printf '\n== %s\n' "$*"; }
need() { [ -e "$1" ] || { echo "missing: $1 ($2)" >&2; exit 1; }; }
# javac for Java 15; its notes (deprecations) are dropped, errors stop the build.
javac15() {
    local log status=0
    log=$(mktemp)
    "${NICE[@]}" "$JAVAC" --release 15 -Xlint:-options -encoding UTF-8 "$@" >"$log" 2>&1 || status=$?
    grep -v '^Note: ' "$log" || true
    rm -f "$log"
    return "$status"
}
# The JVM drops hs_err_pid*.log where it crashes; never in the repository.
trap 'rm -f "$ROOT"/hs_err_pid*.log "$HERE"/hs_err_pid*.log' EXIT

need "$BUILD_TOOLS/aapt2" "build-tools 35.0.0"
need "$BUILD_TOOLS/lib/d8.jar" "build-tools 35.0.0"
need "$ANDROID_JAR" "platforms;android-$TARGET_SDK"
need "$JAVAC" "a JDK (JAVA_HOME)"
mkdir -p "$OUT"

# --- 1. libnsh.so --------------------------------------------------------------
if [ "${SKIP_RUST:-0}" != 1 ] || [ ! -e "$OUT/apk/lib/$ABI/libnsh.so" ]; then
    step "libnsh.so (crates/android, $PROFILE)"
    ABIS=$ABI "$HERE/rust.sh" "$PROFILE" "$OUT/apk/lib" "$OUT/symbols"
fi

# --- 2. Resources ----------------------------------------------------------------
step "resources (aapt2)"
rm -rf "$OUT/res" "$OUT/gen" "$OUT/base.apk"
mkdir -p "$OUT/res" "$OUT/gen"
"${NICE[@]}" "$BUILD_TOOLS/aapt2" compile --dir "$APP/res" -o "$OUT/res/app.zip"
"${NICE[@]}" "$BUILD_TOOLS/aapt2" compile --dir "$HERE/termux/terminal-view/src/main/res" -o "$OUT/res/termux-view.zip"
# The manifest as Gradle merges it: the package (Gradle's namespace) and the
# stored native library (Gradle's useLegacyPackaging false).
sed -e 's|<manifest xmlns:android="http://schemas.android.com/apk/res/android"|& package="'"$PACKAGE"'"|' \
    -e 's|<application |<application android:extractNativeLibs="false" |' \
    "$APP/AndroidManifest.xml" >"$OUT/AndroidManifest.xml"
grep -q "package=\"$PACKAGE\"" "$OUT/AndroidManifest.xml" && grep -q 'extractNativeLibs="false"' "$OUT/AndroidManifest.xml" ||
    { echo "the manifest's copy lacks the package or extractNativeLibs" >&2; exit 1; }
"${NICE[@]}" "$BUILD_TOOLS/aapt2" link -o "$OUT/base.apk" \
    -I "$ANDROID_JAR" \
    --manifest "$OUT/AndroidManifest.xml" \
    --java "$OUT/gen" --extra-packages com.termux.view \
    --min-sdk-version "$MIN_SDK" --target-sdk-version "$TARGET_SDK" \
    --version-code "$VERSION_CODE" --version-name "$VERSION_NAME" \
    --debug-mode \
    "$OUT/res/termux-view.zip" "$OUT/res/app.zip"

# --- 3. Java -----------------------------------------------------------------------
step "NSH's Java runtime, from the checkout Cargo.lock pins"
NSH_JAVA=$(cd "$ROOT" && cargo metadata --locked --format-version 1 --filter-platform aarch64-linux-android 2>/dev/null |
    python3 -I -c '
import json, os, sys
meta = json.load(sys.stdin)
pkg = next(p for p in meta["packages"] if p["name"] == "nsh-platform-android")
print(os.path.join(os.path.dirname(pkg["manifest_path"]), "java"))
')
need "$NSH_JAVA/com/longbridge/nsh/NshView.java" "NSH's Java sources"
echo "$NSH_JAVA"
rm -rf "$OUT/classes" && mkdir -p "$OUT/classes/nsh" "$OUT/classes/app" "$OUT/dex"
find "$NSH_JAVA" -name '*.java' > "$OUT/classes/nsh-sources.txt"
javac15 -nowarn -classpath "$ANDROID_JAR" -d "$OUT/classes/nsh" @"$OUT/classes/nsh-sources.txt"

step "Termux's terminal libraries and the host (javac)"
{
    find "$HERE/termux" -name '*.java'
    find "$APP/java" -name '*.java'
    find "$OUT/gen" -name '*.java'
} > "$OUT/classes/app-sources.txt"
javac15 -classpath "$ANDROID_JAR:$OUT/classes/nsh" -d "$OUT/classes/app" @"$OUT/classes/app-sources.txt"

step "classes.dex (D8)"
"$JAR" cf "$OUT/classes/nsh.jar" -C "$OUT/classes/nsh" .
"$JAR" cf "$OUT/classes/app.jar" -C "$OUT/classes/app" .
"${NICE[@]}" "$JAVA" -cp "$BUILD_TOOLS/lib/d8.jar" com.android.tools.r8.D8 \
    --min-api "$MIN_SDK" --lib "$ANDROID_JAR" --output "$OUT/dex" \
    "$OUT/classes/nsh.jar" "$OUT/classes/app.jar"

# --- 4. The APK --------------------------------------------------------------------
step "APK"
rm -f "$OUT"/apk/classes*.dex
cp "$OUT"/dex/classes*.dex "$OUT/apk/"
cp "$OUT/base.apk" "$OUT/unaligned.apk"
(
    cd "$OUT/apk"
    # Native libraries stored: the system maps them from the APK (extractNativeLibs="false").
    zip -q -0 -X "$OUT/unaligned.apk" "lib/$ABI/libnsh.so"
    zip -q -9 -X "$OUT/unaligned.apk" classes*.dex
)
rm -f "$OUT/aligned.apk"
# -P 16: shared libraries on 16 KB pages.
"$BUILD_TOOLS/zipalign" -P 16 -f 4 "$OUT/unaligned.apk" "$OUT/aligned.apk"
KEYSTORE="$HOME/.android/debug.keystore"
if [ ! -e "$KEYSTORE" ]; then
    mkdir -p "$(dirname "$KEYSTORE")"
    "$JAVA_HOME/bin/keytool" -genkeypair -keystore "$KEYSTORE" -storepass android -alias androiddebugkey \
        -keypass android -keyalg RSA -keysize 2048 -validity 10000 -dname "CN=Android Debug,O=Android,C=US"
fi
"${NICE[@]}" "$JAVA" -jar "$BUILD_TOOLS/lib/apksigner.jar" sign --ks "$KEYSTORE" --ks-key-alias androiddebugkey \
    --ks-pass pass:android --key-pass pass:android --out "$OUT/moshpit-debug.apk" "$OUT/aligned.apk"
"$BUILD_TOOLS/zipalign" -c -P 16 4 "$OUT/moshpit-debug.apk"
"$JAVA" -jar "$BUILD_TOOLS/lib/apksigner.jar" verify "$OUT/moshpit-debug.apk"
rm -f "$OUT/unaligned.apk" "$OUT/aligned.apk"
ls -l "$OUT/moshpit-debug.apk" | awk '{ print "built " $NF " (" $5 " bytes)" }'

# --- 5. Java unit tests -------------------------------------------------------------
if [ "${SKIP_TESTS:-0}" != 1 ]; then
    step "Java unit tests (JVM)"
    "$HERE/unit.sh"
fi
