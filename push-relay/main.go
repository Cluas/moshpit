// moshpit push relay — the smallest thing that can turn an agent's hook into a
// lock-screen notification on a phone that isn't running the app.
//
// It exists because APNs will only accept a push signed with the app's own
// team-level .p8 key, and that key cannot be handed out to every user's dev
// machine. So exactly one job lives on a server: hold the key, and forward
// sealed envelopes to device tokens.
//
//	dev host                        relay                      Apple
//	  agent hook                      |                          |
//	  seal(status) ── POST /v1/notify ─┤                          |
//	     + device token, in the body  │ verify HMAC, forward      |
//	                 Bearer sendToken ├── POST /3/device/… ──────►│ ──► phone
//
// The relay is STATELESS. It holds no registry, no database, no file of paired
// devices — the routing facts (the APNs device token, an env hint) ride inside
// each request, and the bearer credential is an HMAC the relay itself minted
// over those facts (see token.go). Restarting the relay loses nothing because
// there is nothing to lose.
//
// What the relay can see: that some device got a push, roughly when, whether it
// was an "attention" or a "done", and an opaque per-pane thread id. What it
// cannot see: the host, the session, the pane, the agent, or one character of
// what the agent asked — those live inside a sealbox envelope whose key exists
// only on the phone and on the user's own server. What it KEEPS: nothing —
// every request is served and forgotten, which is what lets the app's privacy
// label say "Data Not Collected" and mean it by Apple's definition (data is
// "collected" when it remains accessible longer than servicing the request
// takes). That boundary is the product promise; docs/PUSH.md states it in full
// and this file must not quietly widen it.
package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"log"
	"net/http"
	"os"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/cluas/moshpit/push-relay/apns"
	"github.com/cluas/moshpit/push-relay/sealbox"
)

// Sender is the APNs surface the relay uses, narrowed to one method so tests
// can substitute a local server (or nothing at all) for Cupertino.
type Sender interface {
	Send(ctx context.Context, n apns.Notification) (apns.Response, error)
}

// categories the host may ask for, mapped to the notification categories the
// app registers in AgentNotifications.Category. An allowlist, not a
// passthrough: the category decides which ACTION BUTTONS the lock screen
// offers, so letting the wire name one freely would let a compromised host
// dress a "done" up as an approval prompt.
var categories = map[string]string{
	"attention": "moshpit.category.attention",
	"done":      "moshpit.category.done",
}

// The fallback lines, as localisation keys. Kept in lockstep with the entries
// scripts/gen_xcstrings.py adds to the app string catalog — a key with no entry
// still renders (as this English), but untranslated.
const (
	fallbackAttentionTitle = "An agent needs you"
	fallbackAttentionBody  = "Open Moshpit to see what it is asking."
	fallbackDoneTitle      = "An agent finished"
	fallbackDoneBody       = "Open Moshpit to send the next instruction."
)

// expiries bound how long APNs may keep trying to deliver each kind.
//
// This is a SAFETY property, not tidiness. Allow/Deny on a lock screen is a
// blind keystroke — Enter or Esc into whatever prompt the pane holds now. The
// app refuses one when it knows the prompt has moved on, but a phone that was
// off for an hour has no such record and would let it through. An attention
// push that outlives its usefulness must therefore never arrive at all.
var expiries = map[string]time.Duration{
	"attention": 10 * time.Minute,
	"done":      1 * time.Hour,
}

type relay struct {
	// master signs and verifies send tokens (token.go). The one secret the
	// relay holds — and it is the RELAY's secret, not any user's data.
	master   []byte
	tokenTTL time.Duration
	prod     Sender
	sandbox  Sender
	throttle *throttle

	// interruptionLevel for an ATTENTION payload. A `done` is stepped down to
	// "active" in buildPayload — see the note there.
	//
	// Default "time-sensitive", matching what the app already asks for on its
	// LOCAL attention notifications (AgentActivityMonitor.postAttention): an
	// agent blocking on a question is precisely the case Focus should not
	// swallow. Where the app is not signed with the
	// com.apple.developer.usernotifications.time-sensitive entitlement — a free
	// Personal Team cannot — iOS silently downgrades it to a normal alert, so
	// the two paths stay consistent either way. Since 2026-08-25 the shipping
	// build carries the entitlement, so this is now load-bearing rather than a
	// request into the void.
	interruptionLevel string
	now               func() time.Time
}

func main() {
	log.SetFlags(log.LstdFlags | log.LUTC)

	addr := env("MOSHPIT_RELAY_ADDR", ":8080")

	master := []byte(os.Getenv("MOSHPIT_RELAY_HMAC_SECRET"))
	if path := os.Getenv("MOSHPIT_RELAY_HMAC_SECRET_FILE"); path != "" {
		b, err := os.ReadFile(path)
		if err != nil {
			log.Fatalf("hmac secret: %v", err)
		}
		master = []byte(strings.TrimSpace(string(b)))
	}

	r := &relay{
		master:            master,
		tokenTTL:          time.Duration(envInt("MOSHPIT_RELAY_TOKEN_TTL_DAYS", 45)) * 24 * time.Hour,
		throttle:          newThrottle(),
		interruptionLevel: env("MOSHPIT_RELAY_INTERRUPTION_LEVEL", "time-sensitive"),
		now:               time.Now,
	}

	if env("MOSHPIT_RELAY_DRY_RUN", "") != "" {
		// Dry run exists so the whole chain — hook → seal → HTTP → payload —
		// can be exercised end to end on a laptop with no Apple credentials at
		// all. It is the only mode the automated e2e test uses.
		log.Printf("DRY RUN: pushes are logged, not sent")
		if len(r.master) == 0 {
			// A harness should not need to invent a secret to test plumbing,
			// but production must never fall back to a known value.
			r.master = []byte("moshpit-dry-run-master-not-for-production")
			log.Printf("DRY RUN: using the built-in HMAC master")
		}
		s := &dryRunSender{}
		r.prod, r.sandbox = s, s
	} else {
		cfg, err := apnsConfigFromEnv()
		if err != nil {
			log.Fatalf("apns config: %v", err)
		}
		cfg.Host = apns.HostProduction
		prod, err := apns.New(cfg)
		if err != nil {
			log.Fatalf("apns production: %v", err)
		}
		cfg.Host = apns.HostSandbox
		sand, err := apns.New(cfg)
		if err != nil {
			log.Fatalf("apns sandbox: %v", err)
		}
		r.prod, r.sandbox = prod, sand
		log.Printf("APNs token auth ready: team=%s key=%s topic=%s",
			cfg.TeamID, cfg.KeyID, cfg.Topic)
	}
	if len(r.master) < 32 {
		log.Fatalf("set MOSHPIT_RELAY_HMAC_SECRET (or _FILE) to at least 32 bytes — send tokens are HMACs of it")
	}

	srv := &http.Server{
		Addr:              addr,
		Handler:           r.routes(),
		ReadHeaderTimeout: 10 * time.Second,
		ReadTimeout:       20 * time.Second,
		WriteTimeout:      20 * time.Second,
	}
	log.Printf("listening on %s (stateless; token ttl %s)", addr, r.tokenTTL)
	if err := srv.ListenAndServe(); err != nil && !errors.Is(err, http.ErrServerClosed) {
		log.Fatal(err)
	}
}

func (r *relay) routes() http.Handler {
	mux := http.NewServeMux()
	mux.HandleFunc("POST /v1/mint", r.handleMint)
	mux.HandleFunc("POST /v1/notify", r.handleNotify)
	// The v1 registry endpoints. 410, not 404: an old build's registration
	// attempt should read as "this flow is gone, update the app", not as a
	// relay that lost its routes.
	gone := func(w http.ResponseWriter, _ *http.Request) {
		httpError(w, http.StatusGone, "this relay is stateless — update Moshpit and re-pair")
	}
	mux.HandleFunc("POST /v1/register", gone)
	mux.HandleFunc("DELETE /v1/register", gone)
	mux.HandleFunc("GET /healthz", func(w http.ResponseWriter, _ *http.Request) {
		w.Header().Set("content-type", "application/json")
		fmt.Fprint(w, `{"ok":true,"stateless":true}`)
	})
	return mux
}

// MARK: - mint

type mintRequest struct {
	// APNsToken is the hex device token from
	// application(_:didRegisterForRemoteNotificationsWithDeviceToken:).
	APNsToken string `json:"apnsToken"`
	// Conn is the phone's own random id for the connection this token will be
	// handed to — an opaque string to the relay, bound into the HMAC so each
	// host carries its own credential.
	Conn string `json:"conn"`
}

// handleMint issues a send token and stores nothing.
//
// Deliberately unauthenticated, exactly as /v1/register was: every input is
// minted by the phone, so there is no prior secret to authenticate WITH, and a
// caller who invents inputs gains only the ability to push (rate-limited,
// undecryptable) to a device whose token they already hold — a power that
// possession of a device token always implied. Unlike register, there is
// nothing here to fill up: no storage exists.
func (r *relay) handleMint(w http.ResponseWriter, req *http.Request) {
	var body mintRequest
	if !readJSON(w, req, &body) {
		return
	}
	if !isHex(body.APNsToken, 64, 200) {
		httpError(w, http.StatusBadRequest, "apnsToken must be 64-200 hex chars")
		return
	}
	if !isCollapseID(body.Conn) || body.Conn == "" {
		httpError(w, http.StatusBadRequest,
			"conn must be 1-64 printable ASCII bytes with no whitespace")
		return
	}
	iat := r.now().Unix()
	token := mintSendToken(r.master, body.APNsToken, body.Conn, iat)
	// The fingerprint of the HASH, matching what /v1/notify logs for the same
	// credential, so a mint and its later pushes correlate in the journal.
	log.Printf("mint device=%s ttl=%s", Fingerprint(SendTokenHash(token)), r.tokenTTL)
	writeJSON(w, http.StatusOK, map[string]any{"ok": true, "sendToken": token, "iat": iat})
}

// MARK: - decisions travelling back

// The longest one round of a host's long poll may last.
//
// Bounded so the relay never holds an unbounded number of open requests, and so
// an idle host reconnects often enough that a dead one is noticed. The host
// loops; this is one round, not the whole wait.
// MARK: - notify

type notifyRequest struct {
	// Envelope is the sealbox the phone will open. The relay treats it as
	// opaque bytes and never calls sealbox.Open.
	Envelope sealbox.Envelope `json:"env"`
	// Category is "attention" or "done" — see the `categories` allowlist for
	// why this one field is in the clear.
	Category string `json:"cat"`
	// Thread is an opaque per-pane id, HMAC'd on the host so the relay learns
	// no pane names. Used for apns-collapse-id and aps.thread-id: a re-prompt
	// REPLACES its predecessor instead of stacking a second card for one
	// question.
	Thread string `json:"thread"`
	// Tok is the APNs device token to deliver to. In v1 the relay looked this
	// up in its registry; now the host's conf carries it and every request
	// brings its own routing. The bearer token is an HMAC over it, so a host
	// cannot point its credential at a different phone.
	Tok string `json:"tok"`
	// TokEnv is a HINT — "production" or "sandbox" — for which APNs host the
	// token lives on. A wrong hint costs one extra APNs round trip (see send),
	// never a lost push.
	TokEnv string `json:"tokEnv"`
	// Conn is the phone's opaque per-connection id, echoed from the conf. Bound
	// into the bearer HMAC; also visible in Thread, so it tells the relay
	// nothing Thread did not.
	Conn string `json:"conn"`
	// IAT is the mint time of the bearer token, seconds. Carried because the
	// relay stores nothing: verification recomputes the HMAC over exactly what
	// was minted, and expiry is judged against this value.
	IAT int64 `json:"iat"`
}

func (r *relay) handleNotify(w http.ResponseWriter, req *http.Request) {
	sendToken, ok := bearer(req)
	if !ok {
		httpError(w, http.StatusUnauthorized, "missing bearer token")
		return
	}

	var body notifyRequest
	if !readJSON(w, req, &body) {
		return
	}
	// One status and one wording for every credential failure — forged,
	// expired, retargeted, or a body missing its routing facts entirely — so a
	// prober learns nothing about which part failed. No separate shape check
	// on `tok`: a value the mint endpoint would have refused can never have a
	// matching HMAC, so verification already rejects it.
	if !verifySendToken(r.master, sendToken, body.Tok, body.Conn,
		body.IAT, r.now(), r.tokenTTL) {
		httpError(w, http.StatusUnauthorized, "unknown or expired send token")
		return
	}
	hash := SendTokenHash(sendToken)
	device := Device{APNsToken: lowerHex(body.Tok), Env: "production"}
	if body.TokEnv == "sandbox" {
		device.Env = "sandbox"
	}
	category, ok := categories[body.Category]
	if !ok {
		httpError(w, http.StatusBadRequest, "cat must be attention or done")
		return
	}
	if body.Envelope.V != sealbox.Version || body.Envelope.CT == "" ||
		body.Envelope.IV == "" || body.Envelope.MAC == "" {
		httpError(w, http.StatusBadRequest, "malformed envelope")
		return
	}
	if !isCollapseID(body.Thread) {
		httpError(w, http.StatusBadRequest,
			"thread must be empty, or <=64 printable ASCII bytes with no whitespace")
		return
	}

	// Floor per PANE, cap per device. Keying the floor on the device alone meant
	// a `done` could swallow the `attention` that followed it two seconds later
	// — "you replied from the lock screen, the agent immediately asked for
	// permission" is exactly that sequence, and it dropped the one notification
	// in the pair that mattered. Two agents asking at once had the same problem.
	if wait, ok := r.throttle.allow(hash, body.Thread, r.now()); !ok {
		w.Header().Set("retry-after", strconv.Itoa(int(wait.Seconds())+1))
		httpError(w, http.StatusTooManyRequests, "slow down")
		return
	}

	payload, err := r.buildPayload(category, body)
	if err != nil {
		httpError(w, http.StatusRequestEntityTooLarge, err.Error())
		return
	}

	n := apns.Notification{
		DeviceToken: device.APNsToken,
		Payload:     payload,
		PushType:    apns.PushTypeAlert,
		Priority:    10,
		CollapseID:  body.Thread,
		Expiration:  r.now().Add(expiries[body.Category]),
	}

	resp, err := r.send(req.Context(), hash, device, n)
	if err != nil {
		log.Printf("notify device=%s cat=%s transport error: %v",
			Fingerprint(hash), body.Category, err)
		httpError(w, http.StatusBadGateway, "apns unreachable")
		return
	}
	// apns-unique-id is the ONLY handle for looking a push up in Apple's Push
	// Notification Console afterwards, which is the only way to find out what
	// happened to it. Log it or lose the ability to debug "my phone stayed
	// silent" reports.
	log.Printf("notify device=%s cat=%s status=%d reason=%q unique=%s",
		Fingerprint(hash), body.Category, resp.StatusCode, resp.Reason, resp.UniqueID)

	if resp.Gone() {
		// Nothing to delete any more — the token expired with the device. 410
		// still travels back so a --test run prints "pair again from the app".
		httpError(w, http.StatusGone, "device unregistered — pair again")
		return
	}
	if !resp.OK() {
		httpError(w, http.StatusBadGateway, "apns rejected: "+resp.Reason)
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"ok": true, "id": resp.UniqueID})
}

// Device is the routing half of one push: where to deliver and on which APNs
// host. It exists per REQUEST — nothing retains one.
type Device struct {
	APNsToken string
	Env       string
}

// send picks the APNs host the request hints at, and — on BadDeviceToken —
// retries the other one once.
//
// That retry is not paranoia: a token minted by an Xcode build is only valid on
// sandbox, an App Store build's only on production, and the phone's own guess
// (#if DEBUG) is wrong for exactly one common case — a TestFlight build
// installed over a development one. v1 remembered the corrected answer in its
// registry; a stateless relay cannot, so a device with a wrong hint pays one
// extra APNs round trip per push until the app rewrites the host's conf. Logged
// each time, because a journal full of "answered on <other env>" is how an
// operator notices the hint is systematically wrong.
func (r *relay) send(ctx context.Context, hash string, d Device, n apns.Notification) (apns.Response, error) {
	first, second, otherEnv := r.prod, r.sandbox, "sandbox"
	if d.Env == "sandbox" {
		first, second, otherEnv = r.sandbox, r.prod, "production"
	}
	resp, err := first.Send(ctx, n)
	if err != nil {
		return resp, err
	}
	if resp.Reason == "BadDeviceToken" {
		resp2, err2 := second.Send(ctx, n)
		if err2 == nil && resp2.OK() {
			log.Printf("device=%s answered on %s — the conf's tokEnv hint is wrong", Fingerprint(hash), otherEnv)
			return resp2, nil
		}
	}
	return resp, nil
}

// buildPayload assembles the APNs JSON.
//
// It sets NO notification category, and that is deliberate. Allow and Deny need
// the connection and pane ids that live inside the sealed envelope, so a
// notification the extension has not opened has nowhere to send a keystroke —
// and a tap that silently does nothing while the user believes they approved is
// the worst outcome this feature has. The extension ADDS the actionable category
// after it decrypts (PushRemoteNotification.apply). The consequence is that both
// cases where it cannot — no matching key, or the extension never running
// because it timed out — show the translated fallback with no buttons rather
// than buttons that lie.
//
// An earlier version put the category in the payload and argued the fallback
// should stay actionable. That argument was wrong, and a peer review caught it:
// the fallback cannot BE actionable, because what you would need to act on it is
// exactly what failed to arrive.
//
// `category` therefore only picks which translated fallback line to send.
//
// The visible strings are LOCALISATION KEYS, not text: the relay does not know
// what the notification says (that is the whole point) and could not translate
// it if it did. iOS resolves title-loc-key/loc-key against the app's own string
// catalog, so the fallback a user sees is in their language.
//
// The keys are the ENGLISH SENTENCES themselves, matching the app's convention
// that a string key IS its English value (scripts/gen_xcstrings.py). That is not
// cosmetic: when a lookup misses, iOS displays the key verbatim, so the worst
// case is readable English rather than "push.fallback.attention.title" on
// someone lock screen.
//
// This text is only ever seen when the notification service extension fails to
// decrypt in time — otherwise it replaces both lines with the real ones.
func (r *relay) buildPayload(category string, body notifyRequest) ([]byte, error) {
	type alert struct {
		TitleLocKey string `json:"title-loc-key"`
		LocKey      string `json:"loc-key"`
	}
	type aps struct {
		Alert             alert  `json:"alert"`
		Sound             string `json:"sound,omitempty"`
		ThreadID          string `json:"thread-id,omitempty"`
		MutableContent    int    `json:"mutable-content"`
		InterruptionLevel string `json:"interruption-level,omitempty"`
	}
	titleKey, bodyKey := fallbackAttentionTitle, fallbackAttentionBody
	// Only a QUESTION breaks through Focus. "Finished" is worth a notification
	// and not worth piercing Do Not Disturb, which is the same line the app
	// draws locally: AgentActivityMonitor.postAttention sets .timeSensitive and
	// postDone leaves the default .active.
	//
	// The distinction had no effect until the entitlement shipped — iOS was
	// downgrading everything to .active anyway, so sending one level for both
	// looked harmless. It stopped being harmless the moment the entitlement was
	// real, which is the sort of thing an entitlement change turns up.
	level := r.interruptionLevel
	if category == categories["done"] {
		titleKey, bodyKey = fallbackDoneTitle, fallbackDoneBody
		if level == "time-sensitive" {
			level = "active"
		}
	}
	payload := struct {
		APS aps              `json:"aps"`
		MP  sealbox.Envelope `json:"mp"`
	}{
		APS: aps{
			Alert:    alert{TitleLocKey: titleKey, LocKey: bodyKey},
			Sound:    "default",
			ThreadID: body.Thread,
			// Without mutable-content the extension is never invoked and every
			// push shows the generic fallback forever.
			MutableContent:    1,
			InterruptionLevel: level,
		},
		MP: body.Envelope,
	}
	b, err := json.Marshal(payload)
	if err != nil {
		return nil, err
	}
	// APNs caps an alert notification at 4 KB. The host script bounds the title
	// long before this, so hitting it means something upstream stopped
	// truncating — fail loudly rather than let APNs reject it opaquely.
	if len(b) > 4096 {
		return nil, fmt.Errorf("payload %d bytes exceeds the 4096-byte APNs limit", len(b))
	}
	return b, nil
}

// MARK: - throttle

// throttle bounds pushes per device.
//
// Agent state flips fast — a hook can fire several times a second while an
// agent works through tool calls — and a phone that buzzes on each one is worse
// than no notifications at all. The floor between pushes is the real product
// decision here; the hourly cap only stops a runaway loop from burning a user's
// attention (and our APNs standing) overnight.
type throttle struct {
	mu   sync.Mutex
	seen map[string]*bucket
}

type bucket struct {
	last  time.Time
	hour  time.Time
	count int
}

const (
	minInterval = 3 * time.Second
	hourlyCap   = 120
)

func newThrottle() *throttle { return &throttle{seen: map[string]*bucket{}} }

// allow applies the per-pane floor and the per-device hourly cap.
//
// `device` bounds how much a runaway host can spend of a user's attention;
// `thread` (the collapse id, one per pane) is what the floor is measured
// against, so one pane going quiet cannot silence another.
//
// A host CAN evade the floor by varying its thread, and that is accepted: the
// per-device hourly cap is the real ceiling and is unaffected, so the worst case
// is unchanged. Written down because it looks like a hole until you check the
// cap.
func (t *throttle) allow(device, thread string, now time.Time) (time.Duration, bool) {
	t.mu.Lock()
	defer t.mu.Unlock()
	// Buckets that stopped being used are dead weight in a process meant to run
	// for months. Pruned opportunistically rather than on a timer: the map is
	// only ever touched from here.
	for k, v := range t.seen {
		if now.Sub(v.last) > 2*time.Hour {
			delete(t.seen, k)
		}
	}
	hourly := t.bucket(device, now)
	if now.Sub(hourly.hour) >= time.Hour {
		hourly.hour, hourly.count = now, 0
	}
	if hourly.count >= hourlyCap {
		return hourly.hour.Add(time.Hour).Sub(now), false
	}
	floor := t.bucket(device+"\x00"+thread, now)
	if !floor.last.IsZero() && now.Sub(floor.last) < minInterval {
		return minInterval - now.Sub(floor.last), false
	}
	floor.last = now
	hourly.last = now
	hourly.count++
	return 0, true
}

func (t *throttle) bucket(key string, now time.Time) *bucket {
	b, ok := t.seen[key]
	if !ok {
		b = &bucket{hour: now}
		t.seen[key] = b
	}
	return b
}

// MARK: - plumbing

// dryRunSender logs instead of sending. It keeps NOTHING: an earlier version
// accumulated every notification in a slice, which in a long dry-run deployment
// is an unbounded leak of exactly the payloads this service is supposed to
// forward and forget.
type dryRunSender struct{}

func (d *dryRunSender) Send(_ context.Context, n apns.Notification) (apns.Response, error) {
	log.Printf("DRY RUN push to %s… collapse=%s payload=%s",
		Fingerprint(n.DeviceToken), n.CollapseID, string(n.Payload))
	return apns.Response{StatusCode: http.StatusOK, UniqueID: "dry-run"}, nil
}

func apnsConfigFromEnv() (apns.Config, error) {
	key := []byte(os.Getenv("MOSHPIT_APNS_KEY"))
	if path := os.Getenv("MOSHPIT_APNS_KEY_FILE"); path != "" {
		b, err := os.ReadFile(path)
		if err != nil {
			return apns.Config{}, err
		}
		key = b
	}
	cfg := apns.Config{
		KeyP8:   key,
		KeyID:   os.Getenv("MOSHPIT_APNS_KEY_ID"),
		TeamID:  os.Getenv("MOSHPIT_APNS_TEAM_ID"),
		Topic:   env("MOSHPIT_APNS_TOPIC", "com.cluas.moshpit"),
		Subject: os.Getenv("MOSHPIT_APNS_SUBJECT"),
	}
	if len(cfg.KeyP8) == 0 {
		return cfg, errors.New("set MOSHPIT_APNS_KEY_FILE (or MOSHPIT_APNS_KEY), or MOSHPIT_RELAY_DRY_RUN=1")
	}
	return cfg, nil
}

func bearer(req *http.Request) (string, bool) {
	h := req.Header.Get("authorization")
	// RFC 7235 makes the scheme case-insensitive, and the two-variant check this
	// replaces rejected "BEARER" from any client that capitalised it.
	const scheme = "bearer "
	if len(h) < len(scheme) || !strings.EqualFold(h[:len(scheme)], scheme) {
		return "", false
	}
	tok := strings.TrimSpace(h[len(scheme):])
	return tok, tok != ""
}

func readJSON(w http.ResponseWriter, req *http.Request, dst any) bool {
	dec := json.NewDecoder(http.MaxBytesReader(w, req.Body, 8<<10))
	dec.DisallowUnknownFields()
	if err := dec.Decode(dst); err != nil {
		httpError(w, http.StatusBadRequest, "bad JSON: "+err.Error())
		return false
	}
	return true
}

func writeJSON(w http.ResponseWriter, status int, v any) {
	w.Header().Set("content-type", "application/json")
	w.WriteHeader(status)
	_ = json.NewEncoder(w).Encode(v)
}

func httpError(w http.ResponseWriter, status int, msg string) {
	writeJSON(w, status, map[string]any{"ok": false, "error": msg})
}

// isCollapseID accepts empty — that is a caller asking NOT to collapse, and the
// sender omits the header for it. Everything else guards the one field that
// becomes an HTTP HEADER (apns-collapse-id)
// AND the delivered notification's identifier.
//
// Two separate reasons to be strict. A CR or LF here would be header injection
// into the APNs request, and APNs itself rejects anything over 64 bytes with
// BadCollapseId — which would surface to the user as "notifications just don't
// work" with nothing in the log to explain it. The value is expected to be
// "moshpit.<state>.<connection uuid>.<pane>": 57 bytes for a typical tmux pane,
// so the ceiling is real but not tight.
func isCollapseID(s string) bool {
	if len(s) > 64 {
		return false
	}
	for _, c := range []byte(s) {
		if c < 0x21 || c > 0x7e {
			return false
		}
	}
	return true
}

func isHex(s string, min, max int) bool {
	if len(s) < min || len(s) > max {
		return false
	}
	for _, c := range s {
		if !(c >= '0' && c <= '9' || c >= 'a' && c <= 'f' || c >= 'A' && c <= 'F') {
			return false
		}
	}
	return true
}

func env(key, def string) string {
	if v := os.Getenv(key); v != "" {
		return v
	}
	return def
}

func envInt(key string, def int) int {
	if v := os.Getenv(key); v != "" {
		if n, err := strconv.Atoi(v); err == nil {
			return n
		}
	}
	return def
}
