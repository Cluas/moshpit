#!/bin/sh
# verify-apns-auth.sh — prove the relay can authenticate to the REAL APNs.
#
# This is the one thing an APNs key unlocks without a phone, and it is worth
# doing on its own because it validates every part of the provider path that is
# easy to get silently wrong:
#
#   * the .p8 parses as a PKCS#8 ECDSA P-256 key
#   * the JWT is signed ES256 with a RAW r||s signature (a DER one earns a bare
#     403 with no explanation)
#   * the Key ID, Team ID and topic are the ones Apple expects
#   * HTTP/2 over TLS to api.sandbox.push.apple.com works from this network
#
# How to read the result. We deliberately send to a device token that cannot
# exist, so success is a SPECIFIC failure:
#
#   BadDeviceToken       PASS - Apple authenticated us, then rejected the token
#   InvalidProviderToken FAIL - the JWT is wrong (signature, key id, or encoding)
#   ExpiredProviderToken FAIL - clock skew, or an iat older than an hour
#   TopicDisallowed      FAIL - the key is not enabled for this bundle id
#   MissingTopic         FAIL - apns-topic never made it into the request
#
# Usage:
#   scripts/verify-apns-auth.sh <path to .p8> <key id> <team id> [topic]
set -eu

KEY=${1:?usage: verify-apns-auth.sh <AuthKey.p8> <keyId> <teamId> [topic]}
KEY_ID=${2:?missing key id}
TEAM_ID=${3:?missing team id}
TOPIC=${4:-com.cluas.moshpit}

ROOT=$(cd "$(dirname "$0")/.." && pwd)
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT INT TERM

[ -r "$KEY" ] || { echo "cannot read $KEY" >&2; exit 1; }

cat > "$TMP/probe.go" <<'GO'
package main

import (
	"context"
	"fmt"
	"os"

	"github.com/cluas/moshpit/push-relay/apns"
)

// A well-formed token that was never issued: 64 hex characters. Anything
// malformed would be rejected before Apple looks at our credentials, which
// would tell us nothing about them.
const unusedToken = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"

func main() {
	key, err := os.ReadFile(os.Args[1])
	if err != nil {
		fmt.Println("read key:", err)
		os.Exit(1)
	}
	client, err := apns.New(apns.Config{
		KeyP8:  key,
		KeyID:  os.Args[2],
		TeamID: os.Args[3],
		Topic:  os.Args[4],
		Host:   apns.HostSandbox,
	})
	if err != nil {
		fmt.Println("client:", err)
		os.Exit(1)
	}
	resp, err := client.Send(context.Background(), apns.Notification{
		DeviceToken: unusedToken,
		Payload:     []byte(`{"aps":{"alert":"probe"}}`),
		PushType:    apns.PushTypeAlert,
		Priority:    10,
	})
	if err != nil {
		fmt.Println("transport:", err)
		os.Exit(1)
	}
	fmt.Printf("status=%d reason=%s apns-id=%s unique=%s\n",
		resp.StatusCode, resp.Reason, resp.APNsID, resp.UniqueID)
	switch resp.Reason {
	case "BadDeviceToken", "Unregistered":
		fmt.Println("PASS: APNs authenticated the provider token and then rejected the fake device token.")
	case "":
		if resp.OK() {
			// Would mean Apple accepted a token we invented. Not expected.
			fmt.Println("UNEXPECTED: APNs accepted a push to a token that was never issued.")
			os.Exit(1)
		}
		fmt.Println("FAIL: no reason given.")
		os.Exit(1)
	default:
		fmt.Printf("FAIL: %s - see the table at the top of verify-apns-auth.sh\n", resp.Reason)
		os.Exit(1)
	}
}
GO

# Build inside the relay module so the apns package resolves without touching it.
mkdir -p "$ROOT/push-relay/cmd-probe-tmp"
cp "$TMP/probe.go" "$ROOT/push-relay/cmd-probe-tmp/main.go"
trap 'rm -rf "$TMP" "$ROOT/push-relay/cmd-probe-tmp"' EXIT INT TERM

echo "probing api.sandbox.push.apple.com  topic=$TOPIC key=$KEY_ID team=$TEAM_ID"
(cd "$ROOT/push-relay" && go run ./cmd-probe-tmp "$KEY" "$KEY_ID" "$TEAM_ID" "$TOPIC")
