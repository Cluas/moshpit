// Package sealbox implements Moshpit's push envelope: the format a coding
// agent's hook seals its status into on the dev host, and only the phone can
// open.
//
// The relay does NOT decrypt — it holds no key and this package's Open is never
// called in the serving path. sealbox lives here so the relay's tests, and the
// verify helper, can prove the three implementations of one format agree:
//
//	shell (openssl, on the dev host)  →  Go (here)  →  Swift (PushSealedBox)
//
// # Format v1 — AES-256-CBC + HMAC-SHA256, encrypt-then-MAC
//
//	KeHex = hex(HMAC-SHA256(key: ascii(secretHex), msg: "moshpit-push-enc-v1"))
//	KmHex = hex(HMAC-SHA256(key: ascii(secretHex), msg: "moshpit-push-mac-v1"))
//	ivHex = 16 random bytes, lowercase hex
//	ct    = base64(AES-256-CBC(key: KeHex, iv: ivHex, pkcs7(plaintext)))
//	mac   = base64(HMAC-SHA256(key: ascii(KmHex), msg: "v1|" + ivHex + "|" + ct))
//	wire  = {"v":1,"iv":ivHex,"ct":ct,"mac":mac}
//
// Every design choice here is forced by ONE constraint: the sending end is a
// POSIX `sh` script running on someone else's server, with nothing but openssl
// and curl. Three consequences worth stating, because each looks like a
// mistake until you try to write the shell:
//
//   - CBC + HMAC, not AES-GCM. `openssl enc` refuses AEAD ciphers outright
//     ("AEAD ciphers not supported by enc"), so a GCM format would be
//     unsendable from the host without shipping a binary there.
//
//   - Subkeys are used in their HEX-STRING form (as openssl's `-K` hex argument
//     and as an ASCII `-hmac` key), never as raw bytes. `openssl dgst -mac HMAC
//     -macopt hexkey:…` would allow raw keys but does not exist in the LibreSSL
//     that macOS ships as /usr/bin/openssl; plain `-hmac <string>` exists in
//     both. Raw-byte keys are also unpassable through a shell argument at all
//     once they contain NUL.
//
//   - The MAC covers the ASCII wire text ("v1|" + ivHex + "|" + ct), not the
//     raw iv||ct bytes. Hex-to-binary conversion in portable `sh` needs xxd (not
//     always installed) or octal printf gymnastics; MACing the encoding instead
//     is equivalent — base64/hex are injective, and decoding happens only after
//     the MAC verifies, so a non-canonical encoding changes the MAC input and is
//     rejected rather than silently normalised.
//
// The whole point of the format is that the relay is a dumb pipe: it routes by
// token and sees ciphertext. What it unavoidably learns is documented in
// docs/PUSH.md — do not let this file's privacy claims drift from that list.
package sealbox

import (
	"bytes"
	"crypto/aes"
	"crypto/cipher"
	"crypto/hmac"
	"crypto/rand"
	"crypto/sha256"
	"crypto/subtle"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"strings"
)

const (
	// Version is the only envelope version this build speaks. It is covered by
	// the MAC (as the "v1" prefix), so nothing can strip it to force a weaker
	// interpretation.
	Version = 1

	encInfo = "moshpit-push-enc-v1"
	macInfo = "moshpit-push-mac-v1"
	macTag  = "v1"

	// SecretHexLen is the pairing secret's length as it appears in
	// ~/.moshpit/push.conf — 32 bytes, hex.
	SecretHexLen = 64
)

var (
	ErrBadSecret  = errors.New("sealbox: secret must be 64 hex chars")
	ErrBadVersion = errors.New("sealbox: unsupported envelope version")
	ErrBadMAC     = errors.New("sealbox: MAC mismatch")
	ErrBadPadding = errors.New("sealbox: bad PKCS#7 padding")
	ErrMalformed  = errors.New("sealbox: malformed envelope")
)

// Envelope is the wire form. Field names are short because the whole APNs
// notification — envelope included — is capped at 4 KB.
type Envelope struct {
	V   int    `json:"v"`
	IV  string `json:"iv"`
	CT  string `json:"ct"`
	MAC string `json:"mac"`
}

// Keys are the two subkeys derived from a pairing secret, in the hex-string
// form both openssl and every other implementation consume.
type Keys struct {
	// EncHex is passed to `openssl enc -K`; its decoded bytes key AES-256.
	EncHex string
	// MacHex is used as an ASCII HMAC key — its bytes are the 64 hex
	// characters, not the 32 they encode. See the package comment.
	MacHex string
}

// DeriveKeys splits a pairing secret into its encryption and MAC subkeys.
func DeriveKeys(secretHex string) (Keys, error) {
	if len(secretHex) != SecretHexLen {
		return Keys{}, ErrBadSecret
	}
	if _, err := hex.DecodeString(secretHex); err != nil {
		return Keys{}, ErrBadSecret
	}
	// Lowercase-normalised: the shell derives from whatever push.conf holds,
	// and a hex secret that differs only in case must not yield a different key.
	secretHex = strings.ToLower(secretHex)
	mk := func(info string) string {
		m := hmac.New(sha256.New, []byte(secretHex))
		m.Write([]byte(info))
		return hex.EncodeToString(m.Sum(nil))
	}
	return Keys{EncHex: mk(encInfo), MacHex: mk(macInfo)}, nil
}

// Seal encrypts plaintext under the pairing secret. Production sealing is done
// by the dev host's shell script; this exists for tests and the verify helper.
func Seal(secretHex string, plaintext []byte) (Envelope, error) {
	iv := make([]byte, aes.BlockSize)
	if _, err := rand.Read(iv); err != nil {
		return Envelope{}, err
	}
	return SealWithIV(secretHex, plaintext, hex.EncodeToString(iv))
}

// SealWithIV is Seal with the IV pinned, so tests can assert byte-for-byte
// agreement with openssl instead of merely "it round-trips".
func SealWithIV(secretHex string, plaintext []byte, ivHex string) (Envelope, error) {
	keys, err := DeriveKeys(secretHex)
	if err != nil {
		return Envelope{}, err
	}
	iv, err := hex.DecodeString(ivHex)
	if err != nil || len(iv) != aes.BlockSize {
		return Envelope{}, ErrMalformed
	}
	encKey, err := hex.DecodeString(keys.EncHex)
	if err != nil {
		return Envelope{}, err
	}
	block, err := aes.NewCipher(encKey)
	if err != nil {
		return Envelope{}, err
	}
	padded := pkcs7Pad(plaintext, aes.BlockSize)
	raw := make([]byte, len(padded))
	cipher.NewCBCEncrypter(block, iv).CryptBlocks(raw, padded)
	ct := base64.StdEncoding.EncodeToString(raw)
	return Envelope{
		V:   Version,
		IV:  strings.ToLower(ivHex),
		CT:  ct,
		MAC: tag(keys.MacHex, strings.ToLower(ivHex), ct),
	}, nil
}

// Open authenticates and decrypts an envelope.
//
// The MAC is verified BEFORE any padding is inspected. That ordering is the
// whole reason for encrypt-then-MAC: a CBC padding oracle needs an
// attacker-chosen ciphertext to reach the unpadding step, and this denies it
// that reach.
func Open(secretHex string, e Envelope) ([]byte, error) {
	if e.V != Version {
		return nil, ErrBadVersion
	}
	keys, err := DeriveKeys(secretHex)
	if err != nil {
		return nil, err
	}
	iv, err := hex.DecodeString(e.IV)
	if err != nil || len(iv) != aes.BlockSize {
		return nil, ErrMalformed
	}
	want, err := base64.StdEncoding.DecodeString(tag(keys.MacHex, strings.ToLower(e.IV), e.CT))
	if err != nil {
		return nil, ErrMalformed
	}
	got, err := base64.StdEncoding.DecodeString(e.MAC)
	if err != nil {
		return nil, ErrMalformed
	}
	if subtle.ConstantTimeCompare(got, want) != 1 {
		return nil, ErrBadMAC
	}
	ct, err := base64.StdEncoding.DecodeString(e.CT)
	if err != nil {
		return nil, ErrMalformed
	}
	if len(ct) == 0 || len(ct)%aes.BlockSize != 0 {
		return nil, ErrMalformed
	}
	encKey, err := hex.DecodeString(keys.EncHex)
	if err != nil {
		return nil, err
	}
	block, err := aes.NewCipher(encKey)
	if err != nil {
		return nil, err
	}
	pt := make([]byte, len(ct))
	cipher.NewCBCDecrypter(block, iv).CryptBlocks(pt, ct)
	return pkcs7Unpad(pt, aes.BlockSize)
}

// Status is the plaintext an agent hook seals: what the agent is doing, and
// where.
//
// Conn is the phone's own connection UUID, handed to the host at pairing and
// echoed back here. Carrying it means the notification arrives already knowing
// which saved connection it belongs to, so the extension can drop it straight
// into userInfo and the app's existing notification-action path — the one that
// already routes lock-screen Allow/Deny to a live pane — needs no new
// host-name-to-connection resolver at all.
type Status struct {
	Conn    string `json:"conn"`
	Host    string `json:"host"`
	Session string `json:"sess,omitempty"`
	Pane    string `json:"pane"`
	Agent   string `json:"agent,omitempty"`
	State   string `json:"state"`
	Title   string `json:"title,omitempty"`
	TS      int64  `json:"ts"`
	// Dur is how long the episode this status CLOSES ran, in seconds — for a
	// `done`, the length of the turn that just finished. The phone uses it to
	// decide whether a finished turn is worth a sound (a 3-minute build ending
	// is; a 20-second answer is not). Optional: absent from older senders and
	// from states where it means nothing, and both sides must tolerate that.
	Dur int64 `json:"dur,omitempty"`
}

// OpenStatus is Open plus the JSON decode every consumer actually wants.
func OpenStatus(secretHex string, e Envelope) (Status, error) {
	pt, err := Open(secretHex, e)
	if err != nil {
		return Status{}, err
	}
	var s Status
	if err := json.Unmarshal(pt, &s); err != nil {
		return Status{}, fmt.Errorf("sealbox: plaintext is not a Status: %w", err)
	}
	return s, nil
}

func tag(macKeyHex, ivHex, ctB64 string) string {
	m := hmac.New(sha256.New, []byte(macKeyHex))
	m.Write([]byte(macTag + "|" + ivHex + "|" + ctB64))
	return base64.StdEncoding.EncodeToString(m.Sum(nil))
}

func pkcs7Pad(b []byte, size int) []byte {
	n := size - len(b)%size
	return append(b, bytes.Repeat([]byte{byte(n)}, n)...)
}

func pkcs7Unpad(b []byte, size int) ([]byte, error) {
	if len(b) == 0 || len(b)%size != 0 {
		return nil, ErrBadPadding
	}
	n := int(b[len(b)-1])
	if n == 0 || n > size || n > len(b) {
		return nil, ErrBadPadding
	}
	for _, c := range b[len(b)-n:] {
		if int(c) != n {
			return nil, ErrBadPadding
		}
	}
	return b[:len(b)-n], nil
}
