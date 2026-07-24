#!/bin/bash
#
# build-mosh-ios.sh — Cross-compile mosh client core for iOS arm64
#
# Prerequisites: brew install automake autoconf libtool pkg-config protobuf
#
# Usage: ./scripts/build-mosh-ios.sh
#

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
BUILD_DIR="$PROJECT_DIR/build"
MOSH_VERSION="1.4.0"

IOS_SDK=$(xcrun --sdk iphoneos --show-sdk-path)
SIM_SDK=$(xcrun --sdk iphonesimulator --show-sdk-path)
IOS_MIN="17.0"

CC_IOS="$(xcrun --sdk iphoneos -f clang)"
CXX_IOS="$(xcrun --sdk iphoneos -f clang++)"
CC_SIM="$(xcrun --sdk iphonesimulator -f clang)"
CXX_SIM="$(xcrun --sdk iphonesimulator -f clang++)"

PROTOBUF_PREFIX="$BUILD_DIR/protobuf"
MOSH_SRC="$BUILD_DIR/mosh-$MOSH_VERSION"
OUTPUT="$BUILD_DIR/mosh-ios"

NCPU=$(sysctl -n hw.ncpu)

echo "=== Beacon: Building mosh for iOS ==="
echo "iOS SDK: $IOS_SDK"
echo "Sim SDK: $SIM_SDK"
echo ""

mkdir -p "$BUILD_DIR"

# ============================================================
# Step 1: Download mosh source
# ============================================================
if [ ! -d "$MOSH_SRC" ]; then
    echo ">>> [1/4] Downloading mosh $MOSH_VERSION..."
    cd "$BUILD_DIR"
    curl -sL "https://github.com/mobile-shell/mosh/releases/download/mosh-$MOSH_VERSION/mosh-$MOSH_VERSION.tar.gz" -o mosh.tar.gz
    tar xzf mosh.tar.gz && rm mosh.tar.gz
    echo "    Done."
else
    echo ">>> [1/4] mosh source already exists, skipping download."
fi

# ============================================================
# Step 2: Build protobuf for iOS (device + simulator)
# ============================================================
build_protobuf() {
    local ARCH=$1
    local SDK=$2
    local CC=$3
    local CXX=$4
    local PREFIX="$PROTOBUF_PREFIX/$ARCH"

    if [ -f "$PREFIX/lib/libprotobuf-lite.a" ]; then
        echo "    protobuf-$ARCH already built, skipping."
        return
    fi

    echo "    Building protobuf for $ARCH..."
    local PROTOBUF_VER="25.3"
    local PB_SRC="$BUILD_DIR/protobuf-$PROTOBUF_VER"

    if [ ! -d "$PB_SRC" ]; then
        cd "$BUILD_DIR"
        curl -sL "https://github.com/protocolbuffers/protobuf/releases/download/v$PROTOBUF_VER/protobuf-$PROTOBUF_VER.tar.gz" -o protobuf.tar.gz
        tar xzf protobuf.tar.gz && rm protobuf.tar.gz
    fi

    mkdir -p "$PB_SRC/build-$ARCH" && cd "$PB_SRC/build-$ARCH"

    cmake .. \
        -DCMAKE_SYSTEM_NAME=iOS \
        -DCMAKE_OSX_ARCHITECTURES="$ARCH" \
        -DCMAKE_OSX_SYSROOT="$SDK" \
        -DCMAKE_OSX_DEPLOYMENT_TARGET="$IOS_MIN" \
        -DCMAKE_INSTALL_PREFIX="$PREFIX" \
        -DCMAKE_BUILD_TYPE=Release \
        -Dprotobuf_BUILD_TESTS=OFF \
        -Dprotobuf_BUILD_EXAMPLES=OFF \
        -Dprotobuf_BUILD_PROTOC_BINARIES=OFF \
        -Dprotobuf_BUILD_LIBPROTOC=OFF \
        -Dprotobuf_BUILD_SHARED_LIBS=OFF \
        -DBUILD_SHARED_LIBS=OFF \
        > /dev/null 2>&1

    cmake --build . --config Release -j$NCPU > /dev/null 2>&1
    cmake --install . > /dev/null 2>&1
    echo "    protobuf-$ARCH done."
}

echo ">>> [2/4] Building protobuf..."
build_protobuf "arm64" "$IOS_SDK" "$CC_IOS" "$CXX_IOS"
build_protobuf "arm64" "$SIM_SDK" "$CC_SIM" "$CXX_SIM"

# ============================================================
# Step 3: Build mosh core objects for iOS arm64
# ============================================================
echo ">>> [3/4] Compiling mosh core for iOS arm64..."

MOSH_OBJ_DIR="$BUILD_DIR/mosh-obj-arm64"
mkdir -p "$MOSH_OBJ_DIR"

PB_PREFIX="$PROTOBUF_PREFIX/arm64"
PROTOC=$(which protoc)

# Generate protobuf .pb.cc / .pb.h from .proto files
echo "    Generating protobuf code..."
cd "$MOSH_SRC"
for proto in src/protobufs/*.proto; do
    $PROTOC --proto_path=src/protobufs --cpp_out="$MOSH_OBJ_DIR" "$proto"
done

# Compile flags
CFLAGS_IOS="-arch arm64 -isysroot $IOS_SDK -miphoneos-version-min=$IOS_MIN -O2 -DHAVE_CONFIG_H"
CXXFLAGS_IOS="$CFLAGS_IOS -std=c++17 -I$PB_PREFIX/include -I$MOSH_SRC/src -I$MOSH_SRC/src/crypto -I$MOSH_SRC/src/network -I$MOSH_SRC/src/statesync -I$MOSH_SRC/src/terminal -I$MOSH_SRC/src/util -I$MOSH_OBJ_DIR"

# Generate a minimal config.h for iOS
cat > "$MOSH_OBJ_DIR/config.h" << 'CONFEOF'
#define PACKAGE "mosh"
#define PACKAGE_VERSION "1.4.0"
#define HAVE_MEMORY 1
#define HAVE_UNISTD_H 1
#define HAVE_SYS_RESOURCE_H 1
#define HAVE_SYS_SOCKET_H 1
#define HAVE_NETINET_IN_H 1
#define HAVE_ARPA_INET_H 1
#define HAVE_LANGINFO_H 1
#define HAVE_WCHAR_H 1
#define HAVE_WCWIDTH 1
#define HAVE_MBRTOWC 1
#define HAVE_MBTOWC 1
#define HAVE_ISWPRINT 1
#define RETSIGTYPE void
CONFEOF

CXXFLAGS_IOS="$CXXFLAGS_IOS -I$MOSH_OBJ_DIR"

# Compile core source files
compile_file() {
    local src=$1
    local name=$(basename "$src" .cc)
    name=$(echo "$name" | sed 's/\.c$//') # handle .c files too
    local ext="${src##*.}"

    if [ "$ext" = "cc" ] || [ "$ext" = "cpp" ]; then
        $CXX_IOS $CXXFLAGS_IOS -c "$src" -o "$MOSH_OBJ_DIR/$name.o" 2>/dev/null && echo "    ✓ $name" || echo "    ✗ $name (skipped)"
    else
        $CC_IOS $CFLAGS_IOS -c "$src" -o "$MOSH_OBJ_DIR/$name.o" 2>/dev/null && echo "    ✓ $name" || echo "    ✗ $name (skipped)"
    fi
}

echo "    Compiling crypto..."
for f in "$MOSH_SRC"/src/crypto/*.cc; do compile_file "$f"; done

echo "    Compiling network..."
for f in "$MOSH_SRC"/src/network/*.cc; do compile_file "$f"; done

echo "    Compiling statesync..."
for f in "$MOSH_SRC"/src/statesync/*.cc; do compile_file "$f"; done

echo "    Compiling terminal..."
for f in "$MOSH_SRC"/src/terminal/*.cc; do compile_file "$f"; done

echo "    Compiling util..."
for f in "$MOSH_SRC"/src/util/*.cc; do compile_file "$f"; done

echo "    Compiling generated protobuf..."
for f in "$MOSH_OBJ_DIR"/*.pb.cc; do compile_file "$f"; done

# ============================================================
# Step 4: Create static library
# ============================================================
echo ">>> [4/4] Creating static library..."

mkdir -p "$OUTPUT/lib" "$OUTPUT/include/mosh"

# Collect all .o files
OBJ_FILES=$(find "$MOSH_OBJ_DIR" -name "*.o" 2>/dev/null)
OBJ_COUNT=$(echo "$OBJ_FILES" | wc -w | tr -d ' ')

if [ "$OBJ_COUNT" -eq 0 ]; then
    echo "    ERROR: No object files produced."
    exit 1
fi

ar rcs "$OUTPUT/lib/libmosh.a" $OBJ_FILES
ranlib "$OUTPUT/lib/libmosh.a"

# Copy headers
cp "$MOSH_SRC"/src/crypto/*.h "$OUTPUT/include/mosh/" 2>/dev/null || true
cp "$MOSH_SRC"/src/network/*.h "$OUTPUT/include/mosh/" 2>/dev/null || true
cp "$MOSH_SRC"/src/statesync/*.h "$OUTPUT/include/mosh/" 2>/dev/null || true
cp "$MOSH_SRC"/src/terminal/*.h "$OUTPUT/include/mosh/" 2>/dev/null || true
cp "$MOSH_SRC"/src/util/*.h "$OUTPUT/include/mosh/" 2>/dev/null || true
cp "$MOSH_OBJ_DIR"/*.pb.h "$OUTPUT/include/mosh/" 2>/dev/null || true
cp "$MOSH_OBJ_DIR/config.h" "$OUTPUT/include/mosh/" 2>/dev/null || true

# Copy protobuf-lite
cp "$PB_PREFIX/lib/libprotobuf-lite.a" "$OUTPUT/lib/" 2>/dev/null || \
cp "$PB_PREFIX/lib/libprotobuf.a" "$OUTPUT/lib/libprotobuf-lite.a" 2>/dev/null || true

echo ""
echo "=== Build complete ==="
echo ""
echo "Static library: $OUTPUT/lib/libmosh.a ($(du -h "$OUTPUT/lib/libmosh.a" | cut -f1))"
echo "Protobuf:       $OUTPUT/lib/libprotobuf-lite.a"
echo "Headers:        $OUTPUT/include/mosh/"
echo "Objects:        $OBJ_COUNT files compiled"
echo ""
echo "To integrate:"
echo "  1. Add libmosh.a + libprotobuf-lite.a to Xcode target"
echo "  2. Add \$OUTPUT/include to Header Search Paths"
echo "  3. Create Swift/ObjC bridge to call mosh client functions"
