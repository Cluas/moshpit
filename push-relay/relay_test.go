package main

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/cluas/moshpit/push-relay/apns"
	"github.com/cluas/moshpit/push-relay/sealbox"
)

const (
	testSecret = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
	testConn   = "3F2504E0-4F89-11D3-9A0C-0305E82C3301"
	testMaster = "test-master-secret-longer-than-32-bytes!"
)

var testAPNsToken = strings.Repeat("ab", 32)

type fakeSender struct {
	mu    sync.Mutex
	calls []apns.Notification
	resp  apns.Response
	err   error
}

func (f *fakeSender) Send(_ context.Context, n apns.Notification) (apns.Response, error) {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.calls = append(f.calls, n)
	if f.resp.StatusCode == 0 && f.err == nil {
		return apns.Response{StatusCode: 200, UniqueID: "u1"}, nil
	}
	return f.resp, f.err
}

func (f *fakeSender) all() []apns.Notification {
	f.mu.Lock()
	defer f.mu.Unlock()
	out := make([]apns.Notification, len(f.calls))
	copy(out, f.calls)
	return out
}

func (f *fakeSender) last() apns.Notification {
	f.mu.Lock()
	defer f.mu.Unlock()
	if len(f.calls) == 0 {
		return apns.Notification{}
	}
	return f.calls[len(f.calls)-1]
}

func (f *fakeSender) count() int {
	f.mu.Lock()
	defer f.mu.Unlock()
	return len(f.calls)
}

func newTestRelay(t *testing.T) (*relay, *fakeSender, *httptest.Server) {
	t.Helper()
	prod := &fakeSender{}
	r := &relay{
		master:            []byte(testMaster),
		tokenTTL:          45 * 24 * time.Hour,
		prod:              prod,
		sandbox:           prod,
		throttle:          newThrottle(),
		interruptionLevel: "time-sensitive",
		now:               time.Now,
	}
	srv := httptest.NewServer(r.routes())
	t.Cleanup(srv.Close)
	return r, prod, srv
}

// cred is one minted send token plus the facts it was minted over — everything
// a host's conf holds that the relay verifies against.
type cred struct {
	token  string
	iat    int64
	tok    string
	tokEnv string
	conn   string
}

// mint asks the relay for a send token the way the phone does.
func mint(t *testing.T, srv *httptest.Server, apnsToken, conn, env string) cred {
	t.Helper()
	b, _ := json.Marshal(map[string]string{"apnsToken": apnsToken, "conn": conn})
	resp, err := srv.Client().Post(srv.URL+"/v1/mint", "application/json", strings.NewReader(string(b)))
	if err != nil {
		t.Fatal(err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != 200 {
		t.Fatalf("mint: status %d", resp.StatusCode)
	}
	var out struct {
		SendToken string `json:"sendToken"`
		IAT       int64  `json:"iat"`
	}
	if err := json.NewDecoder(resp.Body).Decode(&out); err != nil {
		t.Fatal(err)
	}
	return cred{token: out.SendToken, iat: out.IAT, tok: apnsToken, tokEnv: env, conn: conn}
}

func testCred(t *testing.T, srv *httptest.Server, env string) cred {
	return mint(t, srv, testAPNsToken, testConn, env)
}

func notify(t *testing.T, srv *httptest.Server, token string, body any) *http.Response {
	t.Helper()
	b, _ := json.Marshal(body)
	req, _ := http.NewRequest("POST", srv.URL+"/v1/notify", strings.NewReader(string(b)))
	req.Header.Set("authorization", "Bearer "+token)
	req.Header.Set("content-type", "application/json")
	resp, err := srv.Client().Do(req)
	if err != nil {
		t.Fatal(err)
	}
	return resp
}

// sealedBody builds the notify request the shell sender would: the envelope,
// the clear routing facts, and the credential's own mint facts.
func sealedBody(t *testing.T, c cred, cat, thread string, status sealbox.Status) map[string]any {
	t.Helper()
	pt, _ := json.Marshal(status)
	e, err := sealbox.Seal(testSecret, pt)
	if err != nil {
		t.Fatal(err)
	}
	return map[string]any{"env": e, "cat": cat, "thread": thread,
		"tok": c.tok, "tokEnv": c.tokEnv, "conn": c.conn, "iat": c.iat}
}

// TestNotifyBuildsExpectedPush is the contract with the phone: category,
// mutable-content, collapse id, and an opaque envelope the relay passed through
// untouched.
func TestNotifyBuildsExpectedPush(t *testing.T) {
	_, sender, srv := newTestRelay(t)
	c := testCred(t, srv, "sandbox")

	thread := "moshpit.attention." + testConn + ".%3"
	want := sealbox.Status{
		Conn: testConn, Host: "m1-pro", Pane: "%3", Agent: "claude",
		State: "attention", Title: "Bash: rm -rf build", TS: 1755900000,
	}
	resp := notify(t, srv, c.token, sealedBody(t, c, "attention", thread, want))
	defer resp.Body.Close()
	if resp.StatusCode != 200 {
		t.Fatalf("status %d", resp.StatusCode)
	}

	got := sender.last()
	if got.PushType != apns.PushTypeAlert {
		t.Errorf("push type = %q", got.PushType)
	}
	if got.CollapseID != thread {
		t.Errorf("collapse id = %q, want the app's own local notification identifier %q",
			got.CollapseID, thread)
	}
	if got.Expiration.IsZero() {
		t.Error("no expiration: a stale approval prompt must not be deliverable forever")
	}

	var payload struct {
		APS struct {
			Category          string `json:"category"`
			MutableContent    int    `json:"mutable-content"`
			ThreadID          string `json:"thread-id"`
			InterruptionLevel string `json:"interruption-level"`
			Alert             struct {
				TitleLocKey string `json:"title-loc-key"`
				LocKey      string `json:"loc-key"`
				Title       string `json:"title"`
				Body        string `json:"body"`
			} `json:"alert"`
		} `json:"aps"`
		MP sealbox.Envelope `json:"mp"`
	}
	if err := json.Unmarshal(got.Payload, &payload); err != nil {
		t.Fatal(err)
	}
	// No category on the wire: the extension adds the actionable one only after
	// it has the pane ids that make Allow/Deny mean anything.
	if payload.APS.Category != "" {
		t.Errorf("category = %q, want none — an unopened push must not offer buttons",
			payload.APS.Category)
	}
	if payload.APS.MutableContent != 1 {
		t.Error("mutable-content must be 1 or the decrypting extension never runs")
	}
	if payload.APS.InterruptionLevel != "time-sensitive" {
		t.Errorf("interruption-level = %q", payload.APS.InterruptionLevel)
	}
	// The visible strings must be KEYS, never text: the relay cannot read the
	// message and must not appear to.
	if payload.APS.Alert.Title != "" || payload.APS.Alert.Body != "" {
		t.Errorf("relay put literal text in the alert: %+v", payload.APS.Alert)
	}
	if payload.APS.Alert.TitleLocKey != fallbackAttentionTitle {
		t.Errorf("title-loc-key = %q", payload.APS.Alert.TitleLocKey)
	}
	// A missed lookup renders the key itself, so the key must read as a
	// sentence rather than an identifier.
	if strings.Contains(payload.APS.Alert.TitleLocKey, ".title") {
		t.Error("fallback key looks like an identifier; it would be shown verbatim on a miss")
	}

	// And the envelope must arrive byte-identical, still sealed.
	back, err := sealbox.OpenStatus(testSecret, payload.MP)
	if err != nil {
		t.Fatalf("envelope did not survive the relay: %v", err)
	}
	if back != want {
		t.Errorf("status round-trip = %+v, want %+v", back, want)
	}
}

// TestOnlyAQuestionBreaksThroughFocus pins which pushes may pierce Do Not
// Disturb. The line is the same one the app draws locally — postAttention sets
// .timeSensitive, postDone leaves the default — and it had no observable effect
// until the app shipped the time-sensitive entitlement, because iOS was
// downgrading every level to "active" anyway. A relay that sent one level for
// both was harmless right up until it wasn't.
func TestOnlyAQuestionBreaksThroughFocus(t *testing.T) {
	for _, tc := range []struct {
		cat, want string
	}{
		{"attention", "time-sensitive"},
		{"done", "active"},
	} {
		t.Run(tc.cat, func(t *testing.T) {
			_, sender, srv := newTestRelay(t)
			c := testCred(t, srv, "sandbox")
			status := sealbox.Status{
				Conn: testConn, Host: "m1-pro", Pane: "%3", Agent: "claude",
				State: tc.cat, TS: 1755900000,
			}
			resp := notify(t, srv, c.token,
				sealedBody(t, c, tc.cat, "moshpit."+tc.cat+"."+testConn+".%3", status))
			defer resp.Body.Close()
			if resp.StatusCode != 200 {
				t.Fatalf("status %d", resp.StatusCode)
			}
			var payload struct {
				APS struct {
					InterruptionLevel string `json:"interruption-level"`
				} `json:"aps"`
			}
			if err := json.Unmarshal(sender.last().Payload, &payload); err != nil {
				t.Fatal(err)
			}
			if payload.APS.InterruptionLevel != tc.want {
				t.Errorf("%s interruption-level = %q, want %q",
					tc.cat, payload.APS.InterruptionLevel, tc.want)
			}
		})
	}
}

// An operator who turns the level down must turn it down for BOTH, never have
// the done path silently promoted back up by the step-down logic.
func TestSteppingDownDoneNeverStepsItUp(t *testing.T) {
	r, sender, srv := newTestRelay(t)
	c := testCred(t, srv, "sandbox")
	// Same shape as MOSHPIT_RELAY_INTERRUPTION_LEVEL=passive.
	r.interruptionLevel = "passive"
	status := sealbox.Status{Conn: testConn, Host: "m1-pro", Pane: "%3",
		State: "done", TS: 1755900000}
	resp := notify(t, srv, c.token, sealedBody(t, c, "done", "t", status))
	defer resp.Body.Close()
	var payload struct {
		APS struct {
			InterruptionLevel string `json:"interruption-level"`
		} `json:"aps"`
	}
	if err := json.Unmarshal(sender.last().Payload, &payload); err != nil {
		t.Fatal(err)
	}
	if payload.APS.InterruptionLevel != "passive" {
		t.Errorf("done level = %q, want the configured %q — the step-down must only ever lower",
			payload.APS.InterruptionLevel, "passive")
	}
}

// TestNotifyRejectsForgedOrRetargetedTokens is the auth contract of a stateless
// relay: the bearer must be the HMAC over EXACTLY the routing facts in the
// body, so a credential cannot be forged, aged past its TTL, or pointed at a
// different phone than it was minted for.
func TestNotifyRejectsForgedOrRetargetedTokens(t *testing.T) {
	_, sender, srv := newTestRelay(t)
	c := testCred(t, srv, "production")
	status := sealbox.Status{State: "done"}

	// A made-up bearer.
	resp := notify(t, srv, "not-the-token", sealedBody(t, c, "done", "moshpit.done.x.%1", status))
	resp.Body.Close()
	if resp.StatusCode != http.StatusUnauthorized {
		t.Errorf("forged token: status %d, want 401", resp.StatusCode)
	}

	// A real bearer aimed at a DIFFERENT device than it was minted for.
	retargeted := c
	retargeted.tok = strings.Repeat("cd", 32)
	resp = notify(t, srv, c.token, sealedBody(t, retargeted, "done", "moshpit.done.x.%2", status))
	resp.Body.Close()
	if resp.StatusCode != http.StatusUnauthorized {
		t.Errorf("retargeted token: status %d, want 401", resp.StatusCode)
	}

	// A real bearer whose claimed mint time was altered.
	aged := c
	aged.iat = c.iat - 60
	resp = notify(t, srv, c.token, sealedBody(t, aged, "done", "moshpit.done.x.%3", status))
	resp.Body.Close()
	if resp.StatusCode != http.StatusUnauthorized {
		t.Errorf("altered iat: status %d, want 401", resp.StatusCode)
	}

	// No auth header at all.
	req, _ := http.NewRequest("POST", srv.URL+"/v1/notify", strings.NewReader("{}"))
	bare, err := srv.Client().Do(req)
	if err != nil {
		t.Fatal(err)
	}
	bare.Body.Close()
	if bare.StatusCode != http.StatusUnauthorized {
		t.Errorf("no auth header: status %d, want 401", bare.StatusCode)
	}

	if sender.count() != 0 {
		t.Error("a rejected credential still reached APNs")
	}
}

// TestSendTokenExpiry pins the revocation story a stateless relay has: a token
// past its TTL stops working, and one claiming to be from the future (beyond
// clock skew) never starts.
func TestSendTokenExpiry(t *testing.T) {
	r, _, srv := newTestRelay(t)
	c := testCred(t, srv, "production")
	status := sealbox.Status{State: "done"}

	// Age the relay's clock past the TTL: yesterday's mint must still work,
	// but one minted TTL+ ago must not.
	r.now = func() time.Time { return time.Unix(c.iat, 0).Add(r.tokenTTL + time.Minute) }
	resp := notify(t, srv, c.token, sealedBody(t, c, "done", "moshpit.done.x.%1", status))
	resp.Body.Close()
	if resp.StatusCode != http.StatusUnauthorized {
		t.Errorf("expired token: status %d, want 401", resp.StatusCode)
	}

	// A token from the future: the relay's clock is BEHIND the claimed iat by
	// more than the skew allowance.
	r.now = func() time.Time { return time.Unix(c.iat, 0).Add(-10 * time.Minute) }
	resp = notify(t, srv, c.token, sealedBody(t, c, "done", "moshpit.done.x.%2", status))
	resp.Body.Close()
	if resp.StatusCode != http.StatusUnauthorized {
		t.Errorf("future token: status %d, want 401", resp.StatusCode)
	}

	// And within its window it works — same credential, same body shape.
	r.now = func() time.Time { return time.Unix(c.iat, 0).Add(24 * time.Hour) }
	resp = notify(t, srv, c.token, sealedBody(t, c, "done", "moshpit.done.x.%3", status))
	resp.Body.Close()
	if resp.StatusCode != 200 {
		t.Errorf("token inside its window: status %d, want 200", resp.StatusCode)
	}
}

// TestCategoryIsAnAllowlist matters because the category decides which BUTTONS
// the lock screen shows. A host that could name any category could dress a
// finished agent up as an approval prompt.
func TestCategoryIsAnAllowlist(t *testing.T) {
	_, sender, srv := newTestRelay(t)
	c := testCred(t, srv, "production")
	for _, cat := range []string{"working", "", "moshpit.category.attention", "../attention"} {
		resp := notify(t, srv, c.token,
			sealedBody(t, c, cat, "moshpit.done.x.%1", sealbox.Status{State: "x"}))
		resp.Body.Close()
		if resp.StatusCode != http.StatusBadRequest {
			t.Errorf("cat %q: status %d, want 400", cat, resp.StatusCode)
		}
	}
	if sender.count() != 0 {
		t.Error("a rejected category still reached APNs")
	}
}

func TestCollapseIDIsBounded(t *testing.T) {
	_, _, srv := newTestRelay(t)
	c := testCred(t, srv, "production")
	for _, thread := range []string{
		strings.Repeat("x", 65), // APNs answers >64 bytes with BadCollapseId
		"has space",             // not header-safe
		"line\r\nbreak",         // header injection
	} {
		resp := notify(t, srv, c.token,
			sealedBody(t, c, "done", thread, sealbox.Status{State: "done"}))
		resp.Body.Close()
		if resp.StatusCode != http.StatusBadRequest {
			t.Errorf("thread %q: status %d, want 400", thread, resp.StatusCode)
		}
	}
}

func TestThrottleFloorIsPerPane(t *testing.T) {
	base := time.Now()
	th := newThrottle()
	if _, ok := th.allow("dev", "pane-a", base); !ok {
		t.Fatal("first push must pass")
	}
	if _, ok := th.allow("dev", "pane-a", base.Add(time.Second)); ok {
		t.Error("a second push for the SAME pane one second later must be dropped")
	}
	// The case that used to lose the important half: a done, then the attention
	// it provoked, on a different pane, inside the floor.
	if _, ok := th.allow("dev", "pane-b", base.Add(time.Second)); !ok {
		t.Error("a different pane must not be silenced by the first pane's floor")
	}
	if _, ok := th.allow("dev", "pane-a", base.Add(minInterval)); !ok {
		t.Error("a push after the floor must pass")
	}
	if _, ok := th.allow("other-dev", "pane-a", base.Add(time.Second)); !ok {
		t.Error("throttle leaked across devices")
	}
}

func TestThrottleHourlyCapIsPerDevice(t *testing.T) {
	base := time.Now()
	th := newThrottle()
	at := base
	// Spend the hour across MANY panes, so the floor never intervenes.
	for i := 0; i < hourlyCap; i++ {
		at = at.Add(time.Second)
		if _, ok := th.allow("dev", fmt.Sprintf("pane-%d", i), at); !ok {
			t.Fatalf("push %d was refused before the cap", i)
		}
	}
	if _, ok := th.allow("dev", "pane-new", at.Add(time.Second)); ok {
		t.Error("hourly cap was not enforced")
	}
	if _, ok := th.allow("dev", "pane-new", base.Add(2*time.Hour)); !ok {
		t.Error("cap did not reset in the next hour")
	}
}

// TestGoneTravelsBack: APNs saying the device is gone must surface as a 410 so
// a --test run prints "pair again from the app". There is no registry row to
// drop any more — the token dies of its TTL instead.
func TestGoneTravelsBack(t *testing.T) {
	_, sender, srv := newTestRelay(t)
	c := testCred(t, srv, "production")
	sender.resp = apns.Response{StatusCode: 410, Reason: "Unregistered"}

	resp := notify(t, srv, c.token,
		sealedBody(t, c, "done", "moshpit.done.x.%1", sealbox.Status{State: "done"}))
	resp.Body.Close()
	if resp.StatusCode != http.StatusGone {
		t.Errorf("status %d, want 410", resp.StatusCode)
	}
}

// TestBadDeviceTokenFallsBackToOtherEnvironment covers the one case the phone's
// own #if DEBUG guess gets wrong: a TestFlight build installed over an Xcode
// one. The hint rides in each request now, so there is nothing to remember —
// only the retry itself matters.
func TestBadDeviceTokenFallsBackToOtherEnvironment(t *testing.T) {
	prod := &fakeSender{resp: apns.Response{StatusCode: 400, Reason: "BadDeviceToken"}}
	sand := &fakeSender{}
	r := &relay{master: []byte(testMaster), tokenTTL: 45 * 24 * time.Hour,
		prod: prod, sandbox: sand, throttle: newThrottle(),
		interruptionLevel: "active", now: time.Now}
	srv := httptest.NewServer(r.routes())
	defer srv.Close()
	c := testCred(t, srv, "production")

	resp := notify(t, srv, c.token,
		sealedBody(t, c, "done", "moshpit.done.x.%1", sealbox.Status{State: "done"}))
	resp.Body.Close()
	if resp.StatusCode != 200 {
		t.Fatalf("status %d, want the sandbox retry to succeed", resp.StatusCode)
	}
	if prod.count() != 1 || sand.count() != 1 {
		t.Errorf("prod tried %d times, sandbox %d — want one attempt each",
			prod.count(), sand.count())
	}
}

// TestMintValidatesShape: the mint endpoint stores nothing, so shape checks are
// all the resistance it needs — plus proof that what it returns actually
// authenticates a push.
func TestMintValidatesShape(t *testing.T) {
	_, sender, srv := newTestRelay(t)

	post := func(body string) int {
		resp, err := srv.Client().Post(srv.URL+"/v1/mint", "application/json",
			strings.NewReader(body))
		if err != nil {
			t.Fatal(err)
		}
		defer resp.Body.Close()
		return resp.StatusCode
	}
	if got := post(`{"apnsToken":"zz","conn":"c1"}`); got != 400 {
		t.Errorf("non-hex token: status %d, want 400", got)
	}
	if got := post(`{"apnsToken":"` + testAPNsToken + `","conn":""}`); got != 400 {
		t.Errorf("empty conn: status %d, want 400", got)
	}
	if got := post(`{"apnsToken":"` + testAPNsToken + `","conn":"has space"}`); got != 400 {
		t.Errorf("conn with whitespace: status %d, want 400", got)
	}

	c := testCred(t, srv, "production")
	if len(c.token) != 64 {
		t.Errorf("minted token is %d chars, want 64 hex", len(c.token))
	}
	resp := notify(t, srv, c.token,
		sealedBody(t, c, "done", "moshpit.done.x.%1", sealbox.Status{State: "done"}))
	resp.Body.Close()
	if resp.StatusCode != 200 {
		t.Errorf("a freshly minted token failed to authenticate: %d", resp.StatusCode)
	}
	if sender.count() != 1 {
		t.Errorf("pushes sent = %d, want 1", sender.count())
	}
}

// TestTokenCasingCannotSplitACredential: the phone sends the device token in
// whatever hex casing iOS handed it; the conf and the mint must agree even if
// one side lowercases.
func TestTokenCasingCannotSplitACredential(t *testing.T) {
	_, _, srv := newTestRelay(t)
	c := mint(t, srv, strings.ToUpper(testAPNsToken), testConn, "production")
	// The conf carries the same token lowercased.
	lowered := c
	lowered.tok = strings.ToLower(c.tok)
	resp := notify(t, srv, c.token,
		sealedBody(t, lowered, "done", "moshpit.done.x.%1", sealbox.Status{State: "done"}))
	resp.Body.Close()
	if resp.StatusCode != 200 {
		t.Errorf("hex casing split the credential: status %d", resp.StatusCode)
	}
}

// TestShellSenderInterop is the load-bearing test of this whole spike: it runs
// the REAL scripts/moshpit-push.sh — the openssl-only sealing that will run on
// users' servers — against the real relay, and proves the Go side can open what
// the shell sealed.
//
// If this ever fails after a change to either side, the two implementations of
// format v1 have diverged and every notification silently becomes undecryptable.
// TestShellSenderFansOutToEveryDevice pins the multi-device contract at the
// only layer that can break it end to end: the REAL shell sender, a push.d/
// directory with two device pairings plus a legacy push.conf, and the real
// relay. One `push.conf` used to be the whole story, which quietly meant one
// phone per host — the second device's pairing overwrote the first's secret and
// its notifications just stopped. Every device must now receive its own
// envelope, sealed with ITS secret, routed by ITS token, carrying ITS
// connection id.
func TestShellSenderFansOutToEveryDevice(t *testing.T) {
	script, err := filepath.Abs("../scripts/moshpit-push.sh")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(script); err != nil {
		t.Skipf("sender script not found: %v", err)
	}
	if _, err := exec.LookPath("openssl"); err != nil {
		t.Skip("openssl not installed")
	}

	_, sender, srv := newTestRelay(t)

	type device struct {
		conn, secret, apns string
		c                  cred
	}
	devices := []device{
		{conn: "AAAAAAAA-0000-0000-0000-000000000001", secret: strings.Repeat("11", 32), apns: strings.Repeat("a1", 32)},
		{conn: "BBBBBBBB-0000-0000-0000-000000000002", secret: strings.Repeat("22", 32), apns: strings.Repeat("b2", 32)},
		{conn: "CCCCCCCC-0000-0000-0000-000000000003", secret: strings.Repeat("33", 32), apns: strings.Repeat("c3", 32)},
	}
	for i := range devices {
		devices[i].c = mint(t, srv, devices[i].apns, devices[i].conn, "sandbox")
	}

	home := t.TempDir()
	confFor := func(d device) string {
		return "RELAY_URL=" + srv.URL + "\nSEND_TOKEN=" + d.c.token +
			"\nSECRET=" + d.secret + "\nCONN=" + d.conn +
			"\nAPNS_TOKEN=" + d.apns + "\nAPNS_ENV=sandbox" +
			"\nSEND_IAT=" + strconv.FormatInt(d.c.iat, 10) + "\n"
	}
	// Two modern per-device files, one legacy single file — all three must fly.
	if err := os.MkdirAll(filepath.Join(home, ".moshpit", "push.d"), 0o700); err != nil {
		t.Fatal(err)
	}
	for _, d := range devices[:2] {
		if err := os.WriteFile(filepath.Join(home, ".moshpit", "push.d", d.conn+".conf"),
			[]byte(confFor(d)), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	if err := os.WriteFile(filepath.Join(home, ".moshpit", "push.conf"),
		[]byte(confFor(devices[2])), 0o600); err != nil {
		t.Fatal(err)
	}
	// And one conf from the registry era — no APNS_TOKEN, no SEND_IAT. The
	// sender must skip it in silence rather than earn a 401: the app rewrites
	// these on its next connect.
	if err := os.WriteFile(filepath.Join(home, ".moshpit", "push.d", "DDDDDDDD-legacy.conf"),
		[]byte("RELAY_URL="+srv.URL+"\nSEND_TOKEN="+strings.Repeat("d4", 32)+
			"\nSECRET="+strings.Repeat("44", 32)+"\nCONN=DDDDDDDD-0000-0000-0000-000000000004\n"), 0o600); err != nil {
		t.Fatal(err)
	}

	cmd := exec.Command("sh", script, "attention", "claude", "may I?")
	cmd.Env = append(os.Environ(), "HOME="+home, "MOSHPIT_PUSH_HOST=m1-pro",
		"TMUX_PANE=%3", "TMUX=")
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("sender failed: %v\n%s", err, out)
	} else if len(out) > 0 {
		t.Fatalf("sender wrote output in hook mode: %q", out)
	}

	pushes := sender.all()
	if len(pushes) != 3 {
		t.Fatalf("got %d pushes, want 3 (one per paired device)", len(pushes))
	}
	seen := map[string]bool{}
	for _, p := range pushes {
		var payload struct {
			MP sealbox.Envelope `json:"mp"`
		}
		if err := json.Unmarshal(p.Payload, &payload); err != nil {
			t.Fatal(err)
		}
		// The envelope must open with exactly ONE device's secret — its own —
		// and the plaintext must echo that device's connection id.
		opened := 0
		for _, d := range devices {
			st, err := sealbox.OpenStatus(d.secret, payload.MP)
			if err != nil {
				continue
			}
			opened++
			if st.Conn != d.conn {
				t.Errorf("envelope sealed with %s's secret carries conn %q", d.conn, st.Conn)
			}
			seen[d.conn] = true
		}
		if opened != 1 {
			t.Errorf("an envelope opened with %d device secrets, want exactly 1", opened)
		}
	}
	for _, d := range devices {
		if !seen[d.conn] {
			t.Errorf("device %s never got its push", d.conn)
		}
	}
}

func TestShellSenderInterop(t *testing.T) {
	script, err := filepath.Abs("../scripts/moshpit-push.sh")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(script); err != nil {
		t.Skipf("sender script not found: %v", err)
	}
	if _, err := exec.LookPath("openssl"); err != nil {
		t.Skip("openssl not installed")
	}

	_, sender, srv := newTestRelay(t)
	c := testCred(t, srv, "sandbox")

	conf := filepath.Join(t.TempDir(), "push.conf")
	err = os.WriteFile(conf, []byte(
		"RELAY_URL="+srv.URL+"\n"+
			"SEND_TOKEN="+c.token+"\n"+
			// Deliberately UPPERCASE: the shell must lowercase the secret before
			// deriving, or the phone rejects its own messages with a MAC error.
			"SECRET="+strings.ToUpper(testSecret)+"\n"+
			"CONN="+testConn+"\n"+
			"APNS_TOKEN="+testAPNsToken+"\n"+
			"APNS_ENV=sandbox\n"+
			"SEND_IAT="+strconv.FormatInt(c.iat, 10)+"\n"), 0o600)
	if err != nil {
		t.Fatal(err)
	}

	run := func(args ...string) {
		t.Helper()
		cmd := exec.Command("sh", append([]string{script}, args...)...)
		cmd.Env = append(os.Environ(),
			"MOSHPIT_PUSH_CONF="+conf,
			"MOSHPIT_PUSH_HOST=m1-pro",
			"TMUX_PANE=%3",
			"TMUX=",
		)
		out, err := cmd.CombinedOutput()
		if err != nil {
			t.Fatalf("sender failed: %v\n%s", err, out)
		}
		if len(out) > 0 {
			t.Errorf("sender wrote to stdout/stderr inside an agent hook: %q", out)
		}
	}

	// A title with a quote, a backslash and a multi-byte character: all three
	// have broken hand-rolled shell JSON before.
	run("attention", "claude", `Bash: rm -rf "build" \ 构建`)

	if sender.count() != 1 {
		t.Fatalf("relay saw %d pushes, want 1", sender.count())
	}
	var payload struct {
		MP sealbox.Envelope `json:"mp"`
	}
	if err := json.Unmarshal(sender.last().Payload, &payload); err != nil {
		t.Fatal(err)
	}
	got, err := sealbox.OpenStatus(testSecret, payload.MP)
	if err != nil {
		t.Fatalf("Go could not open what the shell sealed: %v", err)
	}
	if got.Conn != testConn || got.Host != "m1-pro" || got.Pane != "%3" ||
		got.Agent != "claude" || got.State != "attention" {
		t.Errorf("status = %+v", got)
	}
	if got.Title != `Bash: rm -rf "build" \ 构建` {
		t.Errorf("title = %q — shell JSON escaping is wrong", got.Title)
	}
	if got.TS < 1_700_000_000 {
		t.Errorf("ts = %d, want a real unix timestamp", got.TS)
	}
	// Per CONNECTION, not per pane: every waiting prompt on a host collapses
	// into one summary card, and the id must still equal the app's local
	// notification identifier so a push replaces the local copy instead of
	// stacking a second card.
	if want := "moshpit.attention." + testConn; sender.last().CollapseID != want {
		t.Errorf("collapse id = %q, want %q (must equal the app's local identifier)",
			sender.last().CollapseID, want)
	}

	// `working` fires on every tool call. Pushing it would spend a phone buzz
	// on nothing, so the sender must refuse it outright.
	run("working", "claude", "reading a file")
	if sender.count() != 1 {
		t.Errorf("sender pushed a 'working' state; count now %d", sender.count())
	}

	// A missing config file is the normal state on an unpaired host: silent no-op.
	cmd := exec.Command("sh", script, "attention", "claude", "x")
	cmd.Env = append(os.Environ(), "MOSHPIT_PUSH_CONF="+conf+".absent")
	if out, err := cmd.CombinedOutput(); err != nil || len(out) > 0 {
		t.Errorf("unpaired host: err=%v out=%q — must be a silent exit 0", err, out)
	}
}

// TestPayloadStaysUnderAPNsLimit guards the 4 KB ceiling with a title at the
// length the sender caps to.
func TestPayloadStaysUnderAPNsLimit(t *testing.T) {
	r, _, _ := newTestRelay(t)
	status := sealbox.Status{
		Conn: testConn, Host: strings.Repeat("h", 64), Session: strings.Repeat("s", 64),
		Pane: "%999", Agent: "claude", State: "attention",
		Title: strings.Repeat("字", 120), TS: 1755900000,
	}
	pt, _ := json.Marshal(status)
	e, _ := sealbox.Seal(testSecret, pt)
	b, err := r.buildPayload("moshpit.category.attention",
		notifyRequest{Envelope: e, Category: "attention", Thread: "moshpit.attention.x.%1"})
	if err != nil {
		t.Fatal(err)
	}
	if len(b) > 4096 {
		t.Errorf("payload is %d bytes", len(b))
	}
	t.Logf("worst-case payload: %s bytes", strconv.Itoa(len(b)))
}

// TestLegacyRegisterEndpointsAreGone: an old build must read "update the app",
// never "route not found".
func TestLegacyRegisterEndpointsAreGone(t *testing.T) {
	_, _, srv := newTestRelay(t)

	resp, err := srv.Client().Post(srv.URL+"/v1/register", "application/json",
		strings.NewReader(`{"apnsToken":"`+testAPNsToken+`","sendTokenHash":"`+strings.Repeat("a", 64)+`"}`))
	if err != nil {
		t.Fatal(err)
	}
	resp.Body.Close()
	if resp.StatusCode != http.StatusGone {
		t.Errorf("POST /v1/register: %d, want 410", resp.StatusCode)
	}

	req, _ := http.NewRequest("DELETE", srv.URL+"/v1/register", nil)
	req.Header.Set("authorization", "Bearer x")
	del, err := srv.Client().Do(req)
	if err != nil {
		t.Fatal(err)
	}
	del.Body.Close()
	if del.StatusCode != http.StatusGone {
		t.Errorf("DELETE /v1/register: %d, want 410", del.StatusCode)
	}
}
