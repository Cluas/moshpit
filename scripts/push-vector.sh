#!/bin/sh
# push-vector.sh — regenerate the frozen cross-language test vector for the push
# envelope (format v1).
#
# Three implementations must agree byte for byte: the openssl pipeline in
# scripts/moshpit-push.sh (which runs on users' servers), the Go one in
# push-relay/sealbox, and the Swift one in Moshpit/Services/Push. The vector is
# how that agreement is TESTED rather than assumed, and it is pinned in two
# places:
#
#   push-relay/sealbox/sealbox_test.go       TestOpenSSLVector
#   MoshpitTests/Services/PushSealedBoxTests.swift
#
# Run this, then paste the output into BOTH. Never update one alone — a vector
# that only one side agrees with tests nothing.
set -eu

SECRET=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
IV=000102030405060708090a0b0c0d0e0f
PLAIN='{"conn":"3F2504E0-4F89-11D3-9A0C-0305E82C3301","host":"m1-pro","sess":"work","pane":"%3","agent":"claude","state":"attention","title":"Bash: rm -rf \"build\" 构建","ts":1755900000}'

KE=$(printf '%s' 'moshpit-push-enc-v1' | openssl dgst -sha256 -hmac "$SECRET" | awk '{print $NF}')
KM=$(printf '%s' 'moshpit-push-mac-v1' | openssl dgst -sha256 -hmac "$SECRET" | awk '{print $NF}')
CT=$(printf '%s' "$PLAIN" | openssl enc -aes-256-cbc -K "$KE" -iv "$IV" -a -A)
MAC=$(printf 'v1|%s|%s' "$IV" "$CT" | openssl dgst -sha256 -hmac "$KM" -binary | openssl base64 -A)

cat <<OUT
secret  $SECRET
encHex  $KE
macHex  $KM
iv      $IV
ct      $CT
mac     $MAC
plain   $PLAIN
openssl $(openssl version)
OUT
