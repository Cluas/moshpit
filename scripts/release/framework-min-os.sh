#!/bin/bash
# An embedded framework's Info.plist must not claim an older iOS than its
# binary was built for. App Store Connect refuses such an upload while it
# processes it (ITMS-90208, "does not support the minimum OS Version
# specified in the Info.plist") after the upload itself has succeeded, and
# neither local validation nor a device install checks it.
#
# onnxruntime.framework (onnxruntime-libs 1.28.2, through sherpa-onnx) ships
# MinimumOSVersion 13.0 around a binary whose LC_BUILD_VERSION says 18.0.
# Run after the frameworks are embedded: a framework whose plist is below
# its binary's minos is raised to it, then signed again with the identity
# this build signs with (the export re-signs it once more for distribution).
set -euo pipefail

dir="${TARGET_BUILD_DIR:?}/${FRAMEWORKS_FOLDER_PATH:?}"
[ -d "$dir" ] || exit 0

# 0 when version $1 is lower than version $2 (dotted numbers, missing parts 0).
older() {
    awk -v a="$1" -v b="$2" 'BEGIN {
        n = split(a, x, "."); m = split(b, y, ".")
        for (i = 1; i <= (n > m ? n : m); i++) {
            p = (i <= n ? x[i] + 0 : 0); q = (i <= m ? y[i] + 0 : 0)
            if (p < q) exit 0
            if (p > q) exit 1
        }
        exit 1
    }'
}

for fw in "$dir"/*.framework; do
    [ -d "$fw" ] || continue
    name=$(basename "$fw" .framework)
    plist="$fw/Info.plist"
    bin="$fw/$name"
    [ -f "$plist" ] && [ -f "$bin" ] || continue
    need=$(xcrun vtool -show-build "$bin" 2>/dev/null | awk '$1 == "minos" { print $2; exit }')
    [ -n "$need" ] || continue
    have=$(/usr/libexec/PlistBuddy -c 'Print :MinimumOSVersion' "$plist" 2>/dev/null || echo 0)
    older "$have" "$need" || continue
    /usr/libexec/PlistBuddy -c "Set :MinimumOSVersion $need" "$plist" 2>/dev/null ||
        /usr/libexec/PlistBuddy -c "Add :MinimumOSVersion string $need" "$plist"
    echo "note: $name.framework: MinimumOSVersion $have -> $need, its binary's minos"
    if [ "${CODE_SIGNING_ALLOWED:-YES}" = YES ] && [ -n "${EXPANDED_CODE_SIGN_IDENTITY:-}" ]; then
        codesign --force --sign "$EXPANDED_CODE_SIGN_IDENTITY" --timestamp=none \
            --preserve-metadata=identifier,flags "$fw"
    fi
done
