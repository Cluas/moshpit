// Package apns is a minimal APNs provider client: HTTP/2, token-based
// authentication, no third-party dependencies.
//
// # Why token-based (JWT) and not certificates
//
// Both auth schemes speak the same HTTP/2 provider API. The difference is what
// you have to keep alive:
//
//	certificate auth — a per-APP .p12 presented as a TLS client certificate.
//	                   Expires every year. Separate cert per bundle id.
//	                   Renewal is a manual portal + Keychain dance, and it
//	                   fails CLOSED at 3am on a date nobody wrote down.
//	token auth       — one .p8 ECDSA P-256 key per TEAM, created once at
//	                   developer.apple.com → Keys → APNs. No expiry. Works for
//	                   every bundle id in the team, and for BOTH the sandbox and
//	                   production hosts with no separate material.
//
// For Moshpit that last property is what settles it: the same relay deployment
// can serve TestFlight/dev builds (sandbox) and App Store builds (production)
// off one secret, selected per request by which host we dial.
//
// # The JWT
//
//	header  {"alg":"ES256","kid":"<10-char Key ID>","typ":"JWT"}
//	claims  {"iss":"<10-char Team ID>","iat":<unix seconds>[,"sub":"<topic>"]}
//	         signed ES256 over base64url(header) + "." + base64url(claims)
//
// Three things that bite:
//
//   - ES256 wants the RAW r||s signature, 64 bytes, each half zero-padded to
//     32. Go's ecdsa.SignASN1 (and anything DER) produces a variable-length
//     ASN.1 sequence, which APNs rejects as InvalidProviderToken. `sign` below
//     does the fixed-width encoding by hand.
//   - APNs refuses a token whose `iat` is more than an hour old, AND throttles
//     providers that mint new ones too eagerly (reason
//     TooManyProviderTokenUpdates). So the token is CACHED and reused across
//     pushes, refreshed on a timer well inside the hour — never per request.
//   - `sub` is required only when the key is restricted to specific topics at
//     creation time. Sending it for an unrestricted key is allowed but pointless,
//     so it is opt-in via config rather than always-on.
//
// # Request shape
//
//	POST https://api.push.apple.com/3/device/<hex device token>
//	authorization: bearer <JWT>
//	apns-push-type: alert            (required; iOS 13+)
//	apns-topic:     com.cluas.moshpit
//	apns-priority:  10               (deliver now; 5 = power-conserving)
//	apns-expiration: <unix seconds>  (0 = try once, never store)
//	apns-collapse-id: <=64 bytes     (a newer push REPLACES an unread older one)
//	apns-id: <uuid>                  (ours, for correlating with the response)
//
// A 200 means APNs accepted it, not that a phone saw it. The response's
// `apns-unique-id` is the handle for Apple's Push Notification Console, which is
// the only way to see what actually happened to a push — worth logging, because
// it is the one thing that makes a "my phone stayed silent" report debuggable.
package apns

import (
	"bytes"
	"context"
	"crypto/ecdsa"
	"crypto/rand"
	"crypto/sha256"
	"crypto/x509"
	"encoding/base64"
	"encoding/json"
	"encoding/pem"
	"errors"
	"fmt"
	"io"
	"net/http"
	"strconv"
	"sync"
	"time"
)

const (
	// HostProduction serves App Store and TestFlight-from-App-Store builds.
	HostProduction = "https://api.push.apple.com"
	// HostSandbox serves anything signed with a development profile — i.e. every
	// build that comes out of Xcode, including the ones we test with.
	HostSandbox = "https://api.sandbox.push.apple.com"

	// tokenLifetime is how long a minted JWT is reused. Apple's hard ceiling is
	// one hour; staying well inside it costs nothing and leaves room for clock
	// skew on the relay host.
	tokenLifetime = 45 * time.Minute
)

// PushType values for the apns-push-type header. Only the two Moshpit needs are
// named; the header is mandatory, and guessing wrong is a 400.
const (
	PushTypeAlert = "alert"
	// PushTypeLiveActivity also requires apns-topic to be
	// "<bundle-id>.push-type.liveactivity". Unused by T1; here so the constant
	// exists at the same place when the Live Activity path lands.
	PushTypeLiveActivity = "liveactivity"
)

// Config is everything the relay needs from the operator. All of it comes from
// the environment; none of it is ever logged.
type Config struct {
	// KeyP8 is the PEM text of the .p8 downloaded from developer.apple.com.
	KeyP8 []byte
	// KeyID is the 10-character Key ID shown next to that key.
	KeyID string
	// TeamID is the 10-character Apple Developer team id.
	TeamID string
	// Topic is the app's bundle id.
	Topic string
	// Subject, when non-empty, is sent as the JWT's `sub` claim. Set it only if
	// the .p8 was created restricted to specific topics.
	Subject string
	// Host is HostProduction or HostSandbox.
	Host string
}

// Client sends pushes to one APNs environment.
type Client struct {
	cfg  Config
	key  *ecdsa.PrivateKey
	http *http.Client

	mu       sync.Mutex
	token    string
	tokenExp time.Time
	// now is overridable so tests can age a token out without sleeping.
	now func() time.Time
}

// Notification is one push.
type Notification struct {
	DeviceToken string
	// Payload is the full JSON body, `aps` and all. The relay builds it; this
	// package does not inspect it, so it stays honest about being a transport.
	Payload  []byte
	PushType string
	Priority int
	// CollapseID replaces an unread earlier push with the same id. For Moshpit
	// it is a per-pane value, so an agent that re-prompts twice does not stack
	// two lock-screen cards for one question.
	CollapseID string
	Expiration time.Time
	ID         string
}

// Response is what APNs said.
type Response struct {
	StatusCode int
	// APNsID echoes our apns-id.
	APNsID string
	// UniqueID is apns-unique-id — the handle for Apple's Push Notification
	// Console. Only present on newer APNs responses.
	UniqueID string
	// Reason is APNs' machine-readable failure name, e.g. "BadDeviceToken".
	Reason string
	// Timestamp is set with Unregistered/BadDeviceToken: the moment the token
	// stopped being valid.
	Timestamp int64
}

// OK reports whether APNs accepted the push. It says nothing about whether a
// phone ever displayed it.
func (r Response) OK() bool { return r.StatusCode == http.StatusOK }

// Gone reports that this device token is dead and must be dropped from the
// registry. Retrying it forever is how a provider earns a rate limit.
func (r Response) Gone() bool {
	return r.StatusCode == http.StatusGone ||
		r.Reason == "BadDeviceToken" || r.Reason == "Unregistered"
}

// New parses the .p8 and prepares a client. It does no network I/O.
func New(cfg Config) (*Client, error) {
	if cfg.KeyID == "" || cfg.TeamID == "" || cfg.Topic == "" {
		return nil, errors.New("apns: KeyID, TeamID and Topic are all required")
	}
	if cfg.Host == "" {
		cfg.Host = HostProduction
	}
	key, err := parseP8(cfg.KeyP8)
	if err != nil {
		return nil, err
	}
	return &Client{
		cfg: cfg,
		key: key,
		http: &http.Client{
			Timeout: 30 * time.Second,
			// Plain http.Transport negotiates HTTP/2 over ALPN by itself, which
			// is all APNs needs — and it POOLS the connection, so the expensive
			// TLS handshake happens once and every later push is one stream on
			// the same h2 connection. Setting a custom TLSClientConfig without
			// ForceAttemptHTTP2 is the classic way to silently lose that.
			Transport: &http.Transport{
				ForceAttemptHTTP2:   true,
				MaxIdleConns:        4,
				IdleConnTimeout:     10 * time.Minute,
				TLSHandshakeTimeout: 10 * time.Second,
			},
		},
		now: time.Now,
	}, nil
}

// NewWithHTTP is New with the transport injected, so tests can point a real
// client at a local server instead of Cupertino.
func NewWithHTTP(cfg Config, hc *http.Client) (*Client, error) {
	c, err := New(cfg)
	if err != nil {
		return nil, err
	}
	c.http = hc
	return c, nil
}

// Send delivers one notification.
func (c *Client) Send(ctx context.Context, n Notification) (Response, error) {
	tok, err := c.bearer()
	if err != nil {
		return Response{}, err
	}
	url := c.cfg.Host + "/3/device/" + n.DeviceToken
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, url, bytes.NewReader(n.Payload))
	if err != nil {
		return Response{}, err
	}
	req.Header.Set("authorization", "bearer "+tok)
	req.Header.Set("apns-topic", c.cfg.Topic)
	req.Header.Set("content-type", "application/json")
	if n.PushType != "" {
		req.Header.Set("apns-push-type", n.PushType)
	}
	if n.Priority != 0 {
		req.Header.Set("apns-priority", strconv.Itoa(n.Priority))
	}
	if n.CollapseID != "" {
		req.Header.Set("apns-collapse-id", n.CollapseID)
	}
	if !n.Expiration.IsZero() {
		req.Header.Set("apns-expiration", strconv.FormatInt(n.Expiration.Unix(), 10))
	}
	if n.ID != "" {
		req.Header.Set("apns-id", n.ID)
	}

	resp, err := c.http.Do(req)
	if err != nil {
		return Response{}, err
	}
	defer resp.Body.Close()
	body, _ := io.ReadAll(io.LimitReader(resp.Body, 1<<16))

	out := Response{
		StatusCode: resp.StatusCode,
		APNsID:     resp.Header.Get("apns-id"),
		UniqueID:   resp.Header.Get("apns-unique-id"),
	}
	if len(body) > 0 {
		var e struct {
			Reason    string `json:"reason"`
			Timestamp int64  `json:"timestamp"`
		}
		if json.Unmarshal(body, &e) == nil {
			out.Reason = e.Reason
			out.Timestamp = e.Timestamp
		}
	}
	// A 403 InvalidProviderToken / ExpiredProviderToken means the cached JWT is
	// no good regardless of our own clock arithmetic — drop it so the next push
	// mints a fresh one instead of retrying the same rejected token.
	if resp.StatusCode == http.StatusForbidden {
		c.mu.Lock()
		c.token, c.tokenExp = "", time.Time{}
		c.mu.Unlock()
	}
	return out, nil
}

// bearer returns the cached provider token, minting a new one when it is
// missing or close enough to Apple's one-hour ceiling to be worth replacing.
func (c *Client) bearer() (string, error) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.token != "" && c.now().Before(c.tokenExp) {
		return c.token, nil
	}
	iat := c.now()
	claims := map[string]any{"iss": c.cfg.TeamID, "iat": iat.Unix()}
	if c.cfg.Subject != "" {
		claims["sub"] = c.cfg.Subject
	}
	tok, err := c.sign(map[string]any{
		"alg": "ES256", "kid": c.cfg.KeyID, "typ": "JWT",
	}, claims)
	if err != nil {
		return "", err
	}
	c.token, c.tokenExp = tok, iat.Add(tokenLifetime)
	return tok, nil
}

func (c *Client) sign(header, claims map[string]any) (string, error) {
	hj, err := json.Marshal(header)
	if err != nil {
		return "", err
	}
	cj, err := json.Marshal(claims)
	if err != nil {
		return "", err
	}
	b64 := base64.RawURLEncoding.EncodeToString
	signing := b64(hj) + "." + b64(cj)

	digest := sha256.Sum256([]byte(signing))
	r, s, err := ecdsa.Sign(rand.Reader, c.key, digest[:])
	if err != nil {
		return "", err
	}
	// ES256 is fixed-width r||s, 32 bytes each. Anything DER-shaped here comes
	// back from APNs as InvalidProviderToken with no further explanation.
	sig := make([]byte, 64)
	r.FillBytes(sig[:32]) // FillBytes left-pads with zeros to exactly 32 bytes
	s.FillBytes(sig[32:])
	return signing + "." + b64(sig), nil
}

func parseP8(pemBytes []byte) (*ecdsa.PrivateKey, error) {
	if len(pemBytes) == 0 {
		return nil, errors.New("apns: empty .p8 key")
	}
	block, _ := pem.Decode(pemBytes)
	if block == nil {
		return nil, errors.New("apns: .p8 is not PEM — expected a -----BEGIN PRIVATE KEY----- block")
	}
	k, err := x509.ParsePKCS8PrivateKey(block.Bytes)
	if err != nil {
		return nil, fmt.Errorf("apns: parse .p8: %w", err)
	}
	ec, ok := k.(*ecdsa.PrivateKey)
	if !ok {
		return nil, fmt.Errorf("apns: .p8 holds %T, want an ECDSA P-256 key", k)
	}
	return ec, nil
}
