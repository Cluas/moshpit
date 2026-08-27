package main

import (
	"crypto/hmac"
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"time"
)

// Send tokens, v2 — the stateless replacement for the registry.
//
// v1 kept a table: sha256(sendToken) → device token. The table was small and
// honest, but it was still a server-side record of every paired phone, kept for
// as long as the pairing lived — which is precisely the shape of thing the
// product promises not to hold ("Data Not Collected" is a claim about storage,
// and a registry is storage). v2 removes the table by making the credential
// carry its own proof: the relay MINTS a token by HMAC-ing the routing facts
// with a key only the relay holds, and VERIFIES a push by recomputing that HMAC
// from the same facts, which now travel inside the request. Nothing is looked
// up, so nothing needs to be stored.
//
//	sendToken = hex(HMAC-SHA256(master, "moshpit-send-v2|" + apnsToken + "|" + conn + "|" + iat))
//
// What each bound field buys:
//
//   - apnsToken — the token authorizes pushing to ONE device. A host cannot
//     redirect its credential at someone else's phone.
//   - conn — the phone's random per-connection id. Keeps tokens per host, so
//     the throttle's per-device ceiling cannot be reset by a host swapping
//     credentials with another, and log fingerprints distinguish hosts.
//   - iat — mint time, seconds. Tokens EXPIRE (default 45 days; the app
//     re-mints and rewrites the host's conf long before that). Expiry is the
//     revocation story a stateless design has: v1 could delete a row the
//     moment a user unpaired, v2 cannot un-issue an HMAC — so every credential
//     carries a horizon instead, and a conf left behind on a decommissioned
//     machine dies of old age. Instant revocation remains what it always
//     really was: deleting the pairing SECRET from the phone, without which a
//     push renders only the generic fallback line.
//
// The apnsToken is lowercased before signing — the same normalisation the
// notify path applies — so the hex casing a client happens to use cannot split
// one credential into two.
func mintSendToken(master []byte, apnsToken, conn string, iat int64) string {
	mac := hmac.New(sha256.New, master)
	fmt.Fprintf(mac, "moshpit-send-v2|%s|%s|%d", lowerHex(apnsToken), conn, iat)
	return hex.EncodeToString(mac.Sum(nil))
}

// verifySendToken recomputes the mint and compares in constant time.
//
// The iat window is checked on BOTH sides: a token from the future (beyond
// small clock skew) is as invalid as an expired one, because accepting it
// would let a leaked master key mint tokens that outlive its rotation.
func verifySendToken(master []byte, presented, apnsToken, conn string,
	iat int64, now time.Time, ttl time.Duration) bool {
	if iat <= 0 {
		return false
	}
	issued := time.Unix(iat, 0)
	if issued.After(now.Add(5 * time.Minute)) {
		return false
	}
	if now.Sub(issued) > ttl {
		return false
	}
	p, err := hex.DecodeString(presented)
	if err != nil {
		return false
	}
	w, err := hex.DecodeString(mintSendToken(master, apnsToken, conn, iat))
	if err != nil {
		return false
	}
	return hmac.Equal(p, w)
}

// SendTokenHash survives from v1 as the throttle's key and the log handle: the
// journal correlates a host's pushes without ever containing the credential.
func SendTokenHash(sendToken string) string {
	sum := sha256.Sum256([]byte(sendToken))
	return hex.EncodeToString(sum[:])
}

// Fingerprint is a short, non-reversible handle for logs. Tokens and hashes
// never appear in the log whole — an operator needs to correlate lines, not to
// be handed push credentials by the journal.
func Fingerprint(s string) string {
	if len(s) >= 8 {
		return s[:8]
	}
	return "short"
}

func lowerHex(s string) string {
	b := []byte(s)
	for i, c := range b {
		if c >= 'A' && c <= 'F' {
			b[i] = c + ('a' - 'A')
		}
	}
	return string(b)
}
