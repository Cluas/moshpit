package apns

import (
	"context"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/sha256"
	"crypto/x509"
	"encoding/base64"
	"encoding/json"
	"encoding/pem"
	"math/big"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"
)

// newTestKey returns a P-256 key in the same PKCS#8 PEM shape as an
// Apple-issued .p8.
func newTestKey(t *testing.T) ([]byte, *ecdsa.PublicKey) {
	t.Helper()
	k, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	der, err := x509.MarshalPKCS8PrivateKey(k)
	if err != nil {
		t.Fatal(err)
	}
	return pem.EncodeToMemory(&pem.Block{Type: "PRIVATE KEY", Bytes: der}), &k.PublicKey
}

func testConfig(p8 []byte) Config {
	return Config{
		KeyP8:  p8,
		KeyID:  "ABCD123456",
		TeamID: "TEAM123456",
		Topic:  "com.cluas.moshpit",
	}
}

// TestProviderTokenIsVerifiableES256 is the test that matters most, because the
// failure it guards against is invisible: APNs answers a wrongly-encoded
// signature with a bare 403 InvalidProviderToken and no hint that the problem is
// DER-vs-raw rather than a bad key or team id.
func TestProviderTokenIsVerifiableES256(t *testing.T) {
	p8, pub := newTestKey(t)
	c, err := New(testConfig(p8))
	if err != nil {
		t.Fatal(err)
	}
	tok, err := c.bearer()
	if err != nil {
		t.Fatal(err)
	}
	parts := strings.Split(tok, ".")
	if len(parts) != 3 {
		t.Fatalf("want 3 JWT segments, got %d", len(parts))
	}

	var hdr struct{ Alg, Kid, Typ string }
	mustDecodeJSON(t, parts[0], &hdr)
	if hdr.Alg != "ES256" || hdr.Kid != "ABCD123456" || hdr.Typ != "JWT" {
		t.Errorf("header = %+v", hdr)
	}

	var claims map[string]any
	mustDecodeJSON(t, parts[1], &claims)
	if claims["iss"] != "TEAM123456" {
		t.Errorf("iss = %v", claims["iss"])
	}
	if _, ok := claims["iat"]; !ok {
		t.Error("iat missing — APNs rejects a token without it")
	}
	if _, ok := claims["sub"]; ok {
		t.Error("sub present but no Subject configured; it is only for topic-restricted keys")
	}

	sig, err := base64.RawURLEncoding.DecodeString(parts[2])
	if err != nil {
		t.Fatal(err)
	}
	if len(sig) != 64 {
		t.Fatalf("signature is %d bytes; ES256 requires raw r||s of exactly 64 "+
			"(a DER/ASN.1 signature is the classic InvalidProviderToken cause)", len(sig))
	}
	digest := sha256.Sum256([]byte(parts[0] + "." + parts[1]))
	r := new(big.Int).SetBytes(sig[:32])
	s := new(big.Int).SetBytes(sig[32:])
	if !ecdsa.Verify(pub, digest[:], r, s) {
		t.Error("signature does not verify against the key that signed it")
	}
}

func TestSubjectClaimOnlyWhenConfigured(t *testing.T) {
	p8, _ := newTestKey(t)
	cfg := testConfig(p8)
	cfg.Subject = "com.cluas.moshpit"
	c, err := New(cfg)
	if err != nil {
		t.Fatal(err)
	}
	tok, err := c.bearer()
	if err != nil {
		t.Fatal(err)
	}
	var claims map[string]any
	mustDecodeJSON(t, strings.Split(tok, ".")[1], &claims)
	if claims["sub"] != "com.cluas.moshpit" {
		t.Errorf("sub = %v, want the topic", claims["sub"])
	}
}

// TestTokenIsReusedThenRefreshed pins the caching behaviour. Minting a token per
// push is not merely wasteful — APNs answers it with TooManyProviderTokenUpdates.
func TestTokenIsReusedThenRefreshed(t *testing.T) {
	p8, _ := newTestKey(t)
	c, err := New(testConfig(p8))
	if err != nil {
		t.Fatal(err)
	}
	base := time.Now()
	c.now = func() time.Time { return base }

	first, _ := c.bearer()
	again, _ := c.bearer()
	if first != again {
		t.Error("token was re-minted within its lifetime")
	}

	c.now = func() time.Time { return base.Add(tokenLifetime + time.Second) }
	later, _ := c.bearer()
	if later == first {
		t.Error("token was not refreshed after its lifetime elapsed")
	}
	if tokenLifetime >= time.Hour {
		t.Errorf("tokenLifetime %v must stay inside Apple's one-hour ceiling", tokenLifetime)
	}
}

func TestSendSetsRequiredHeaders(t *testing.T) {
	p8, _ := newTestKey(t)
	var got *http.Request
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		got = r
		w.Header().Set("apns-id", "11111111-2222-3333-4444-555555555555")
		w.Header().Set("apns-unique-id", "UNIQUE-1")
		w.WriteHeader(http.StatusOK)
	}))
	defer srv.Close()

	cfg := testConfig(p8)
	cfg.Host = srv.URL
	c, err := NewWithHTTP(cfg, srv.Client())
	if err != nil {
		t.Fatal(err)
	}
	exp := time.Now().Add(10 * time.Minute)
	resp, err := c.Send(context.Background(), Notification{
		DeviceToken: "deadbeef",
		Payload:     []byte(`{"aps":{}}`),
		PushType:    PushTypeAlert,
		Priority:    10,
		CollapseID:  "moshpit.attention.x.%3",
		Expiration:  exp,
		ID:          "abc",
	})
	if err != nil {
		t.Fatal(err)
	}
	if !resp.OK() || resp.UniqueID != "UNIQUE-1" {
		t.Errorf("resp = %+v", resp)
	}
	if got.URL.Path != "/3/device/deadbeef" {
		t.Errorf("path = %s", got.URL.Path)
	}
	for h, want := range map[string]string{
		"apns-push-type":   "alert",
		"apns-topic":       "com.cluas.moshpit",
		"apns-priority":    "10",
		"apns-collapse-id": "moshpit.attention.x.%3",
		"apns-id":          "abc",
	} {
		if got.Header.Get(h) != want {
			t.Errorf("%s = %q, want %q", h, got.Header.Get(h), want)
		}
	}
	if !strings.HasPrefix(got.Header.Get("authorization"), "bearer ") {
		t.Errorf("authorization = %q; APNs wants a lowercase 'bearer ' prefix",
			got.Header.Get("authorization"))
	}
	if got.Header.Get("apns-expiration") == "" {
		t.Error("apns-expiration missing — the push would be stored and retried indefinitely")
	}
}

func TestForbiddenClearsCachedToken(t *testing.T) {
	p8, _ := newTestKey(t)
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusForbidden)
		_, _ = w.Write([]byte(`{"reason":"ExpiredProviderToken"}`))
	}))
	defer srv.Close()
	cfg := testConfig(p8)
	cfg.Host = srv.URL
	c, _ := NewWithHTTP(cfg, srv.Client())

	resp, err := c.Send(context.Background(), Notification{DeviceToken: "t", Payload: []byte("{}")})
	if err != nil {
		t.Fatal(err)
	}
	if resp.Reason != "ExpiredProviderToken" {
		t.Errorf("reason = %q", resp.Reason)
	}
	if c.token != "" {
		t.Error("a rejected provider token stayed cached; every later push would reuse it")
	}
}

func TestGoneClassification(t *testing.T) {
	cases := []struct {
		resp Response
		gone bool
	}{
		{Response{StatusCode: 410}, true},
		{Response{StatusCode: 400, Reason: "BadDeviceToken"}, true},
		{Response{StatusCode: 410, Reason: "Unregistered"}, true},
		{Response{StatusCode: 429, Reason: "TooManyRequests"}, false},
		{Response{StatusCode: 200}, false},
	}
	for _, c := range cases {
		if got := c.resp.Gone(); got != c.gone {
			t.Errorf("%+v Gone() = %v, want %v", c.resp, got, c.gone)
		}
	}
}

func TestRejectsNonECKey(t *testing.T) {
	if _, err := New(testConfig([]byte("not a pem"))); err == nil {
		t.Error("expected a parse failure for garbage key material")
	}
	if _, err := New(Config{KeyP8: []byte("x")}); err == nil {
		t.Error("expected a config failure when KeyID/TeamID/Topic are unset")
	}
}

func mustDecodeJSON(t *testing.T, seg string, dst any) {
	t.Helper()
	raw, err := base64.RawURLEncoding.DecodeString(seg)
	if err != nil {
		t.Fatalf("segment is not base64url: %v", err)
	}
	if err := json.Unmarshal(raw, dst); err != nil {
		t.Fatalf("segment is not JSON: %v", err)
	}
}
