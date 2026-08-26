package sealbox

import (
	"encoding/base64"
	"errors"
	"strings"
	"testing"
)

const secret = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"

func TestRoundTrip(t *testing.T) {
	for _, pt := range []string{
		"",
		"x",
		strings.Repeat("a", 16), // exactly one block: PKCS#7 must add a whole block
		`{"conn":"C","host":"m1-pro","pane":"%3","state":"attention","title":"rm -rf \"build\" 构建","ts":1}`,
	} {
		e, err := Seal(secret, []byte(pt))
		if err != nil {
			t.Fatal(err)
		}
		got, err := Open(secret, e)
		if err != nil {
			t.Fatalf("%q: %v", pt, err)
		}
		if string(got) != pt {
			t.Errorf("round trip = %q, want %q", got, pt)
		}
	}
}

func TestDerivationIsCaseInsensitiveOnTheSecret(t *testing.T) {
	lower, err := DeriveKeys(secret)
	if err != nil {
		t.Fatal(err)
	}
	upper, err := DeriveKeys(strings.ToUpper(secret))
	if err != nil {
		t.Fatal(err)
	}
	if lower != upper {
		t.Error("a hex secret differing only in case derived different keys; " +
			"the shell and the phone would disagree about their own key")
	}
	if lower.EncHex == lower.MacHex {
		t.Error("encryption and MAC subkeys are identical")
	}
	if len(lower.EncHex) != 64 || len(lower.MacHex) != 64 {
		t.Errorf("subkeys are %d/%d hex chars, want 64", len(lower.EncHex), len(lower.MacHex))
	}
}

// TestTamperingIsRejected is the point of encrypt-then-MAC. Each mutation is
// something a relay operator or a network attacker could do to a message in
// flight; none of them may produce plaintext.
func TestTamperingIsRejected(t *testing.T) {
	e, err := Seal(secret, []byte(`{"state":"done"}`))
	if err != nil {
		t.Fatal(err)
	}
	flip := func(b64 string) string {
		raw, err := base64.StdEncoding.DecodeString(b64)
		if err != nil {
			t.Fatal(err)
		}
		raw[0] ^= 0x01
		return base64.StdEncoding.EncodeToString(raw)
	}

	cases := map[string]Envelope{
		"flipped ciphertext bit": {V: 1, IV: e.IV, CT: flip(e.CT), MAC: e.MAC},
		"flipped MAC bit":        {V: 1, IV: e.IV, CT: e.CT, MAC: flip(e.MAC)},
		"swapped IV":             {V: 1, IV: strings.Repeat("00", 16), CT: e.CT, MAC: e.MAC},
		"stripped MAC":           {V: 1, IV: e.IV, CT: e.CT, MAC: ""},
		"wrong version":          {V: 2, IV: e.IV, CT: e.CT, MAC: e.MAC},
		"empty ciphertext":       {V: 1, IV: e.IV, CT: "", MAC: e.MAC},
		"short IV":               {V: 1, IV: "00", CT: e.CT, MAC: e.MAC},
		"non-block ciphertext":   {V: 1, IV: e.IV, CT: base64.StdEncoding.EncodeToString([]byte("abc")), MAC: e.MAC},
	}
	for name, bad := range cases {
		if _, err := Open(secret, bad); err == nil {
			t.Errorf("%s: Open succeeded", name)
		}
	}
}

func TestWrongSecretFailsAtTheMAC(t *testing.T) {
	e, _ := Seal(secret, []byte("hello"))
	other := strings.Repeat("ff", 32)
	_, err := Open(other, e)
	// It must fail at the MAC, NOT at the padding: a padding error would mean the
	// decryption ran on attacker-influenced input first.
	if !errors.Is(err, ErrBadMAC) {
		t.Errorf("err = %v, want ErrBadMAC", err)
	}
}

func TestBadSecretsRejected(t *testing.T) {
	for _, s := range []string{"", "abc", strings.Repeat("z", 64), strings.Repeat("a", 63)} {
		if _, err := DeriveKeys(s); err == nil {
			t.Errorf("DeriveKeys(%q) accepted a bad secret", s)
		}
	}
}

func TestSealWithIVIsDeterministic(t *testing.T) {
	iv := "000102030405060708090a0b0c0d0e0f"
	a, err := SealWithIV(secret, []byte("same"), iv)
	if err != nil {
		t.Fatal(err)
	}
	b, err := SealWithIV(secret, []byte("same"), iv)
	if err != nil {
		t.Fatal(err)
	}
	if a != b {
		t.Error("pinned-IV sealing is not deterministic; cross-language vectors " +
			"could not be compared byte for byte")
	}
}

func TestOpenStatusDecodes(t *testing.T) {
	e, _ := Seal(secret, []byte(`{"conn":"C1","host":"h","sess":"w","pane":"%3","agent":"claude","state":"attention","title":"t","ts":7}`))
	s, err := OpenStatus(secret, e)
	if err != nil {
		t.Fatal(err)
	}
	if s.Conn != "C1" || s.Pane != "%3" || s.State != "attention" || s.TS != 7 {
		t.Errorf("status = %+v", s)
	}
	// Non-JSON plaintext must be an error, not a zero-value Status that the app
	// would then act on.
	junk, _ := Seal(secret, []byte("not json"))
	if _, err := OpenStatus(secret, junk); err == nil {
		t.Error("OpenStatus accepted non-JSON plaintext")
	}
}

// TestOpenSSLVector is the frozen cross-language vector.
//
// It was produced by the exact openssl pipeline scripts/moshpit-push.sh runs,
// with the IV pinned so the bytes are reproducible. The same literal appears in
// the Swift test (MoshpitTests/Services/PushSealedBoxTests.swift). Three
// implementations, one vector: if any of them drifts, exactly one of these tests
// goes red instead of every notification silently becoming undecryptable in the
// field.
//
// Regenerate (all three copies together, never one) with:
//
//	scripts/push-vector.sh
func TestOpenSSLVector(t *testing.T) {
	const (
		iv  = "000102030405060708090a0b0c0d0e0f"
		ct  = "21HMutLOqDHXUpjykOXFikZCpepvsm5jV5f/9b8UHg7qOFw7ihvJXuhQMQ9iG/gbmStNwvoMQDCC6nukKAF2bmyrRIX4+lriKtR+xgIN1WWSeBXfoh2nk0KhJvMsvXqd9xJIt+ShRcbZH+NePfCXwHiOcRQ/Q3ZO89pgX/d3TO8yI0UEsVNyclSjlgqri/d/9l5q9KB/4xFJ/+mxZFLcm56XO8DAXn6nQt2mQ9Z7Dt6bBqw68BRhNdH05E7O3OqR"
		mac = "ZiSLy5cGr2XOMlJiX3ep8bVSAcmERPDSMpHnysktnBA="
	)
	// The derived subkeys are pinned too: a change in the derivation would
	// otherwise show up only as an opaque MAC failure.
	keys, err := DeriveKeys(secret)
	if err != nil {
		t.Fatal(err)
	}
	if keys.EncHex != "4ff81bf9801df5440677a30d2622a56d9ecfc556ea57c383cb057bde865e058d" {
		t.Errorf("enc subkey drifted: %s", keys.EncHex)
	}
	if keys.MacHex != "7fe30bcca8aa9e48de360f5d4cd48f1f83dfc68345ae6861f5cd449f5247ca31" {
		t.Errorf("mac subkey drifted: %s", keys.MacHex)
	}

	got, err := OpenStatus(secret, Envelope{V: 1, IV: iv, CT: ct, MAC: mac})
	if err != nil {
		t.Fatalf("could not open the openssl vector: %v", err)
	}
	want := Status{
		Conn: "3F2504E0-4F89-11D3-9A0C-0305E82C3301", Host: "m1-pro", Session: "work",
		Pane: "%3", Agent: "claude", State: "attention",
		Title: `Bash: rm -rf "build" 构建`, TS: 1755900000,
	}
	if got != want {
		t.Errorf("status = %+v\nwant  = %+v", got, want)
	}

	// Sealing with the same pinned IV must reproduce the vector byte for byte.
	again, err := SealWithIV(secret, mustJSON(t, want), iv)
	if err != nil {
		t.Fatal(err)
	}
	if again.CT != ct || again.MAC != mac {
		t.Error("Go does not reproduce the openssl bytes for the same input")
	}
}

func mustJSON(t *testing.T, s Status) []byte {
	t.Helper()
	// Field order must match what the shell's printf emits, or the ciphertext
	// differs for identical data.
	return []byte(`{"conn":"` + s.Conn + `","host":"` + s.Host + `","sess":"` + s.Session +
		`","pane":"` + s.Pane + `","agent":"` + s.Agent + `","state":"` + s.State +
		`","title":"Bash: rm -rf \"build\" 构建","ts":1755900000}`)
}
