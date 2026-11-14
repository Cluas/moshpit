#!/usr/bin/env bash
# The host's Java unit tests (android/app/src/test/java), on this machine's
# JVM: JUnit 4 is not here, so a stand-in of its annotation and assertions
# (android/unit-runner) and a small runner take its place. The Gradle build
# runs the same tests with the real JUnit (testDebugUnitTest).
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
: "${ANDROID_HOME:=$HOME/Library/Android/sdk}"
: "${JAVA_HOME:=/Library/Java/JavaVirtualMachines/zulu-15.jdk/Contents/Home}"
ANDROID_JAR="$ANDROID_HOME/platforms/android-35/android.jar"
OUT="$HERE/build/unit"
trap 'rm -f "$ROOT"/hs_err_pid*.log "$HERE"/hs_err_pid*.log' EXIT
rm -rf "$OUT" && mkdir -p "$OUT"
# The classes under test are plain Java; android.jar only lets their
# neighbours' signatures resolve.
SOURCES=(
    "$HERE/app/src/main/java/com/cluas/moshpit/SecretBox.java"
    "$HERE/app/src/main/java/com/cluas/moshpit/ui/Symbols.java"
    "$HERE/app/src/main/java/com/cluas/moshpit/ui/Colors.java"
    "$HERE/app/src/main/java/com/cluas/moshpit/term/ByteChannel.java"
)
TESTS=()
while IFS= read -r t; do TESTS+=("$t"); done < <(cd "$HERE/app/src/test/java" && find . -name '*Test.java' | sed 's#^\./##' | sort)
"$JAVA_HOME/bin/javac" --release 15 -encoding UTF-8 -d "$OUT" -classpath "$ANDROID_JAR" \
    "$HERE"/unit-runner/org/junit/*.java "$HERE/unit-runner/MiniRunner.java" "${SOURCES[@]}" \
    "${TESTS[@]/#/$HERE/app/src/test/java/}"
CLASSES=()
for t in "${TESTS[@]}"; do CLASSES+=("$(echo "${t%.java}" | tr / .)"); done
"$JAVA_HOME/bin/java" -cp "$OUT" MiniRunner "${CLASSES[@]}"
