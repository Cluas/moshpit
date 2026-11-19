#!/bin/bash
# Checks how the app's "Build Rust" phase picks its cargo profile, without
# building anything: scripts/rust/build-ios.sh runs as it is, against a
# stand-in cargo, rustc and rustup (a fake CARGO_HOME) that note what they
# were asked, and the project and workspace manifests are read for the
# settings the script relies on.
#
#   scripts/rust/test-build-ios.sh
#
# What it holds:
#   - Release builds `app`, Debug builds `app-test`, as project.yml sets
#     MOSHPIT_RUST_PROFILE per configuration, for every SDK and arch.
#   - A run by hand (no setting, no configuration) builds `app`.
#   - An Xcode build whose project predates the setting is refused before
#     cargo runs, and so is a profile that isn't one of the two.
#   - project.yml: each configuration names a profile Cargo.toml defines,
#     and the app links from that profile's directory. The sideload IPA (scripts/release/build-ipa.sh) builds
#     `app` whatever its configuration.
#   - Cargo.toml: `app-test` is what the test builds need (release
#     semantics, opt-level 2, no whole-graph LTO, 16 codegen units, line
#     tables); `app` keeps its whole-graph LTO; `app-canary` is `app` with
#     ThinLTO and nothing else, built when the command line asks for it.
#
# Needs bash, python3 (3.11+, for tomllib) and xcodegen.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
script="$root/scripts/rust/build-ios.sh"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
failures=0

fail() {
    echo "FAIL: $*" >&2
    failures=$((failures + 1))
}

# A CARGO_HOME whose cargo notes its arguments instead of building; rustc
# answers the sysroot query with one that has every iOS target installed.
fake="$work/cargo-home"
mkdir -p "$fake/bin" "$work/sysroot/lib/rustlib/"{aarch64-apple-ios,aarch64-apple-ios-sim,x86_64-apple-ios}
cat > "$fake/bin/cargo" <<EOF
#!/bin/bash
echo "\$* | CARGO_TARGET_DIR=\$CARGO_TARGET_DIR" >> "$work/calls"
EOF
cat > "$fake/bin/rustc" <<EOF
#!/bin/bash
[ "\$*" = "--print sysroot" ] && echo "$work/sysroot"
EOF
cat > "$fake/bin/rustup" <<EOF
#!/bin/bash
echo "rustup \$*" >> "$work/calls"
EOF
chmod +x "$fake/bin/"*

# Runs the script with the environment `$@` (name=value words, then `--`,
# then its arguments); its exit status, output and cargo's calls land in
# $status, $output and $calls.
run() {
    local vars=()
    while [ "$1" != -- ]; do vars+=("$1"); shift; done
    shift
    : > "$work/calls"
    set +e
    output="$(env -i HOME="$HOME" PATH=/usr/bin:/bin CARGO_HOME="$fake" ${vars[@]+"${vars[@]}"} \
        bash "$script" "$@" 2>&1)"
    status=$?
    set -e
    calls="$(cat "$work/calls")"
}

expect_build() {
    local name="$1" want="$2"
    if [ "$status" -ne 0 ]; then
        fail "$name: exit $status: $output"
    elif [ "$calls" != "$want" ]; then
        fail "$name: cargo was asked
$calls
expected
$want"
    fi
}

target_dir="CARGO_TARGET_DIR=$root/target"
device_app="build --locked -p moshpit-ffi --profile app --target aarch64-apple-ios | $target_dir"
device_test="build --locked -p moshpit-ffi --profile app-test --target aarch64-apple-ios | $target_dir"

run CONFIGURATION=Release MOSHPIT_RUST_PROFILE=app PLATFORM_NAME=iphoneos ARCHS=arm64 --
expect_build "Release, device" "$device_app"

run CONFIGURATION=Debug MOSHPIT_RUST_PROFILE=app-test PLATFORM_NAME=iphoneos ARCHS=arm64 --
expect_build "Debug, device" "$device_test"

run CONFIGURATION=Debug MOSHPIT_RUST_PROFILE=app-test PLATFORM_NAME=iphonesimulator ARCHS="arm64 x86_64" --
expect_build "Debug, simulator" "build --locked -p moshpit-ffi --profile app-test --target aarch64-apple-ios-sim | $target_dir
build --locked -p moshpit-ffi --profile app-test --target x86_64-apple-ios | $target_dir"

run --
expect_build "by hand" "$device_app"

run -- iphonesimulator arm64
expect_build "by hand, simulator" \
    "build --locked -p moshpit-ffi --profile app --target aarch64-apple-ios-sim | $target_dir"

run MOSHPIT_RUST_PROFILE=app-test --
expect_build "by hand, app-test" "$device_test"

# rustup's proxies on PATH and not in $CARGO_HOME/bin (a package manager's
# rustup: Homebrew's, the macOS CI image's): found there. That directory is
# the fixed environment's PATH, so `rustup target add` for a target its
# toolchain lacks resolves there too.
mkdir -p "$work/no-cargo-home" "$work/path-bin" "$work/bare-sysroot/lib/rustlib"
cp "$fake/bin/cargo" "$fake/bin/rustup" "$work/path-bin/"
cat > "$work/path-bin/rustc" <<EOF
#!/bin/bash
[ "\$*" = "--print sysroot" ] && echo "$work/bare-sysroot"
EOF
chmod +x "$work/path-bin/"*
run CARGO_HOME="$work/no-cargo-home" PATH="$work/path-bin:/usr/bin:/bin" \
    CONFIGURATION=Debug MOSHPIT_RUST_PROFILE=app-test PLATFORM_NAME=iphonesimulator ARCHS=arm64 --
expect_build "cargo on PATH, not in CARGO_HOME" "rustup target add aarch64-apple-ios-sim
build --locked -p moshpit-ffi --profile app-test --target aarch64-apple-ios-sim | $target_dir"

# Neither: refused before anything runs, naming both places looked.
run CARGO_HOME="$work/no-cargo-home" --
if [ "$status" -eq 0 ] || [ -n "$calls" ] || [[ "$output" != *"$work/no-cargo-home/bin"* ]] \
    || [[ "$output" != *"nor on PATH (/usr/bin:/bin)"* ]]; then
    fail "no cargo anywhere: exit $status, cargo asked [$calls]: $output"
fi

# Canary's command line asks for app-canary.
run CONFIGURATION=Release MOSHPIT_RUST_PROFILE=app-canary PLATFORM_NAME=iphoneos ARCHS=arm64 --
expect_build "Release, app-canary" \
    "build --locked -p moshpit-ffi --profile app-canary --target aarch64-apple-ios | $target_dir"

# A project generated before MOSHPIT_RUST_PROFILE existed links from
# target/<target>/app: building anything else would link a stale library.
for configuration in Debug Release; do
    run CONFIGURATION="$configuration" PLATFORM_NAME=iphoneos ARCHS=arm64 --
    if [ "$status" -eq 0 ] || [ -n "$calls" ] || [[ "$output" != *"xcodegen generate"* ]]; then
        fail "$configuration without MOSHPIT_RUST_PROFILE: exit $status, cargo asked [$calls]: $output"
    fi
done

run MOSHPIT_RUST_PROFILE=release --
if [ "$status" -eq 0 ] || [ -n "$calls" ]; then
    fail "an unknown profile was built: exit $status, cargo asked [$calls]"
fi

# Xcode does not count the library among the link's inputs (Canary 174 and
# 175 shipped the previous Rust): when cargo changes it, the app's
# executable goes, so the link runs; when cargo leaves it be, the
# executable stays (no relink for nothing); the background build, which
# removes it up front, leaves it to the link. A copy of the script in a
# repository of its own, whose cargo writes the library when asked to.
repo="$work/repo"
mkdir -p "$repo/scripts/rust" "$work/relink-home/bin" "$work/products/Moshpit.app"
cp "$script" "$repo/scripts/rust/build-ios.sh"
cp "$fake/bin/rustc" "$fake/bin/rustup" "$work/relink-home/bin/"
cat > "$work/relink-home/bin/cargo" <<CARGO
#!/bin/bash
echo "\$* | CARGO_TARGET_DIR=\$CARGO_TARGET_DIR" >> "$work/calls"
if [ -e "$work/cargo-writes" ]; then
    mkdir -p "\$CARGO_TARGET_DIR/aarch64-apple-ios/app-canary"
    echo "\$RANDOM" >> "\$CARGO_TARGET_DIR/aarch64-apple-ios/app-canary/libmoshpit_ffi.a"
fi
CARGO
chmod +x "$work/relink-home/bin/"*
exe="$work/products/Moshpit.app/Moshpit"
relink_run() {
    local saved="$script"
    script="$repo/scripts/rust/build-ios.sh"
    run CARGO_HOME="$work/relink-home" CONFIGURATION=Release MOSHPIT_RUST_PROFILE=app-canary \
        PLATFORM_NAME=iphoneos ARCHS=arm64 TARGET_BUILD_DIR="$work/products" \
        EXECUTABLE_PATH=Moshpit.app/Moshpit "$@" --
    script="$saved"
}
echo linked > "$exe"
touch "$work/cargo-writes"
relink_run
[ "$status" -eq 0 ] || fail "relink, library changed: exit $status: $output"
[ ! -e "$exe" ] || fail "the library changed and the app's executable stayed: Xcode would skip the link"
echo linked > "$exe"
rm "$work/cargo-writes"
relink_run
{ [ "$status" -eq 0 ] && [ -e "$exe" ]; } \
    || fail "the library unchanged and the app's executable went (exit $status): $output"
touch "$work/cargo-writes"
relink_run MOSHPIT_RUST_IN_BACKGROUND=1
{ [ "$status" -eq 0 ] && [ -e "$exe" ]; } \
    || fail "the background build removed the executable after the fact (exit $status): $output"
rm -f "$work/cargo-writes"
# shellcheck disable=SC2016 # the script's own words
if ! grep -qF 'rm -f "$TARGET_BUILD_DIR/$EXECUTABLE_PATH"' "$root/scripts/rust/build-ios-background.sh"; then
    fail "the background build no longer removes the app's executable before cargo runs"
fi

# The manifests the script and the link rely on.
spec="$work/spec.json"
(cd "$root" && xcodegen dump --type json --spec project.yml > "$spec")
python3 -I - "$root/Cargo.toml" "$spec" "$root/scripts/release/build-ipa.sh" <<'EOF' || failures=$((failures + 1))
import json, re, sys, tomllib

cargo = tomllib.load(open(sys.argv[1], "rb"))
spec = json.load(open(sys.argv[2]))
ipa = open(sys.argv[3]).read()
profiles = cargo["profile"]
problems = []

def check(what, ok):
    if not ok:
        problems.append(what)

test = profiles.get("app-test", {})
check("app-test inherits release", test.get("inherits") == "release")
check("app-test is opt-level 2", test.get("opt-level") == 2)
check("app-test has no whole-graph LTO", test.get("lto") is False)
check("app-test has 16 codegen units", test.get("codegen-units") == 16)
check("app-test keeps line tables", test.get("debug") == "line-tables-only")
check("app-test sets nothing that changes what the code does",
      not {"debug-assertions", "overflow-checks", "panic"} & set(test))
check("app keeps its whole-graph LTO", profiles["app"].get("lto") is True
      and profiles["app"].get("codegen-units") == 1)
canary = profiles.get("app-canary", {})
check("app-canary is app's code with ThinLTO and nothing else",
      canary == {"inherits": "app", "lto": "thin", "codegen-units": 16})

app = spec["targets"]["Moshpit"]["settings"]
configs = app.get("configs", {})
want = {"Debug": "app-test", "Release": "app"}
for config, profile in want.items():
    got = configs.get(config, {}).get("MOSHPIT_RUST_PROFILE")
    check(f"{config} builds {profile} (project.yml says {got})", got == profile)
    check(f"{config}'s profile is one Cargo.toml defines", got in profiles)
check("the app links from its profile's directory",
      app["base"]["LIBRARY_SEARCH_PATHS"].endswith("/$(MOSHPIT_RUST_TARGET)/$(MOSHPIT_RUST_PROFILE)"))
check("no base setting overrides the configurations'", "MOSHPIT_RUST_PROFILE" not in app["base"])

# Its xcodebuild command, continuation lines joined.
command = re.search(r"^xcodebuild (?:.*\\\n)*.*$", ipa, re.M)
check("the sideload IPA builds the shipping profile",
      command is not None and "MOSHPIT_RUST_PROFILE=app" in command.group(0).split())

for problem in problems:
    print("FAIL: " + problem, file=sys.stderr)
sys.exit(1 if problems else 0)
EOF

if [ "$failures" -ne 0 ]; then
    echo "test-build-ios: $failures failed" >&2
    exit 1
fi
echo "test-build-ios: ok"
