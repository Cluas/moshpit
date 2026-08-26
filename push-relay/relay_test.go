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
	testSecret    = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
	testSendToken = "aaaabbbbccccddddeeeeffff00001111aaaabbbbccccddddeeeeffff00001111"
	testConn      = "3F2504E0-4F89-11D3-9A0C-0305E82C3301"
)

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
	reg, err := NewRegistry(filepath.Join(t.TempDir(), "devices.json"), 10)
	if err != nil {
		t.Fatal(err)
	}
	prod := &fakeSender{}
	r := &relay{
		reg:               reg,
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

func register(t *testing.T, srv *httptest.Server, env string) {
	t.Helper()
	body := map[string]string{
		"apnsToken":     strings.Repeat("ab", 32),
		"sendTokenHash": SendTokenHash(testSendToken),
		"env":           env,
	}
	b, _ := json.Marshal(body)
	resp, err := srv.Client().Post(srv.URL+"/v1/register", "application/json", strings.NewReader(string(b)))
	if err != nil {
		t.Fatal(err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != 200 {
		t.Fatalf("register: status %d", resp.StatusCode)
	}
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

func sealedBody(t *testing.T, cat, thread string, status sealbox.Status) map[string]any {
	t.Helper()
	pt, _ := json.Marshal(status)
	e, err := sealbox.Seal(testSecret, pt)
	if err != nil {
		t.Fatal(err)
	}
	return map[string]any{"env": e, "cat": cat, "thread": thread}
}

// TestNotifyBuildsExpectedPush is the contract with the phone: category,
// mutable-content, collapse id, and an opaque envelope the relay passed through
// untouched.
func TestNotifyBuildsExpectedPush(t *testing.T) {
	_, sender, srv := newTestRelay(t)
	register(t, srv, "sandbox")

	thread := "moshpit.attention." + testConn + ".%3"
	want := sealbox.Status{
		Conn: testConn, Host: "m1-pro", Pane: "%3", Agent: "claude",
		State: "attention", Title: "Bash: rm -rf build", TS: 1755900000,
	}
	resp := notify(t, srv, testSendToken, sealedBody(t, "attention", thread, want))
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
			register(t, srv, "sandbox")
			status := sealbox.Status{
				Conn: testConn, Host: "m1-pro", Pane: "%3", Agent: "claude",
				State: tc.cat, TS: 1755900000,
			}
			resp := notify(t, srv, testSendToken,
				sealedBody(t, tc.cat, "moshpit."+tc.cat+"."+testConn+".%3", status))
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
	register(t, srv, "sandbox")
	// Same shape as MOSHPIT_RELAY_INTERRUPTION_LEVEL=passive.
	r.interruptionLevel = "passive"
	status := sealbox.Status{Conn: testConn, Host: "m1-pro", Pane: "%3",
		State: "done", TS: 1755900000}
	resp := notify(t, srv, testSendToken, sealedBody(t, "done", "t", status))
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

func TestNotifyRejectsUnknownOrMissingToken(t *testing.T) {
	_, _, srv := newTestRelay(t)
	register(t, srv, "production")
	body := sealedBody(t, "done", "moshpit.done.x.%1", sealbox.Status{State: "done"})

	resp := notify(t, srv, "not-the-token", body)
	resp.Body.Close()
	if resp.StatusCode != http.StatusUnauthorized {
		t.Errorf("unknown token: status %d, want 401", resp.StatusCode)
	}

	req, _ := http.NewRequest("POST", srv.URL+"/v1/notify", strings.NewReader("{}"))
	bare, err := srv.Client().Do(req)
	if err != nil {
		t.Fatal(err)
	}
	bare.Body.Close()
	if bare.StatusCode != http.StatusUnauthorized {
		t.Errorf("no auth header: status %d, want 401", bare.StatusCode)
	}
}

// TestCategoryIsAnAllowlist matters because the category decides which BUTTONS
// the lock screen shows. A host that could name any category could dress a
// finished agent up as an approval prompt.
func TestCategoryIsAnAllowlist(t *testing.T) {
	_, sender, srv := newTestRelay(t)
	register(t, srv, "production")
	for _, cat := range []string{"working", "", "moshpit.category.attention", "../attention"} {
		resp := notify(t, srv, testSendToken,
			sealedBody(t, cat, "moshpit.done.x.%1", sealbox.Status{State: "x"}))
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
	register(t, srv, "production")
	for _, thread := range []string{
		strings.Repeat("x", 65), // APNs answers >64 bytes with BadCollapseId
		"has space",             // not header-safe
		"line\r\nbreak",         // header injection
	} {
		resp := notify(t, srv, testSendToken,
			sealedBody(t, "done", thread, sealbox.Status{State: "done"}))
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

func TestGoneDropsDevice(t *testing.T) {
	r, sender, srv := newTestRelay(t)
	register(t, srv, "production")
	sender.resp = apns.Response{StatusCode: 410, Reason: "Unregistered"}

	resp := notify(t, srv, testSendToken,
		sealedBody(t, "done", "moshpit.done.x.%1", sealbox.Status{State: "done"}))
	resp.Body.Close()
	if resp.StatusCode != http.StatusGone {
		t.Errorf("status %d, want 410", resp.StatusCode)
	}
	if r.reg.Len() != 0 {
		t.Error("a device APNs called Unregistered stayed in the registry")
	}
}

// TestBadDeviceTokenFallsBackToOtherEnvironment covers the one case the phone's
// own #if DEBUG guess gets wrong: a TestFlight build installed over an Xcode one.
func TestBadDeviceTokenFallsBackToOtherEnvironment(t *testing.T) {
	reg, _ := NewRegistry("", 10)
	prod := &fakeSender{resp: apns.Response{StatusCode: 400, Reason: "BadDeviceToken"}}
	sand := &fakeSender{}
	r := &relay{reg: reg, prod: prod, sandbox: sand, throttle: newThrottle(),
		interruptionLevel: "active", now: time.Now}
	srv := httptest.NewServer(r.routes())
	defer srv.Close()
	register(t, srv, "production")

	resp := notify(t, srv, testSendToken,
		sealedBody(t, "done", "moshpit.done.x.%1", sealbox.Status{State: "done"}))
	resp.Body.Close()
	if resp.StatusCode != 200 {
		t.Fatalf("status %d, want the sandbox retry to succeed", resp.StatusCode)
	}
	if sand.count() != 1 {
		t.Error("sandbox was never tried")
	}
	if d, ok := r.reg.Get(SendTokenHash(testSendToken)); !ok || d.Env != "sandbox" {
		t.Errorf("device env = %+v; the correction was not remembered", d)
	}
}

func TestRegistryRejectsGarbageAndCapsSize(t *testing.T) {
	reg, _ := NewRegistry("", 1)
	r := &relay{reg: reg, prod: &fakeSender{}, sandbox: &fakeSender{},
		throttle: newThrottle(), now: time.Now}
	srv := httptest.NewServer(r.routes())
	defer srv.Close()

	post := func(body string) int {
		resp, err := srv.Client().Post(srv.URL+"/v1/register", "application/json",
			strings.NewReader(body))
		if err != nil {
			t.Fatal(err)
		}
		defer resp.Body.Close()
		return resp.StatusCode
	}
	if got := post(`{"apnsToken":"zz","sendTokenHash":"` + strings.Repeat("a", 64) + `"}`); got != 400 {
		t.Errorf("non-hex token: status %d, want 400", got)
	}
	if got := post(`{"apnsToken":"` + strings.Repeat("a", 64) + `","sendTokenHash":"short"}`); got != 400 {
		t.Errorf("short hash: status %d, want 400", got)
	}
	if got := post(`{"apnsToken":"` + strings.Repeat("a", 64) + `","sendTokenHash":"` +
		strings.Repeat("b", 64) + `"}`); got != 200 {
		t.Errorf("valid registration: status %d", got)
	}
	if got := post(`{"apnsToken":"` + strings.Repeat("a", 64) + `","sendTokenHash":"` +
		strings.Repeat("c", 64) + `"}`); got != http.StatusInsufficientStorage {
		t.Errorf("over the cap: status %d, want 507", got)
	}
}

func TestRegistrySurvivesRestart(t *testing.T) {
	path := filepath.Join(t.TempDir(), "nested", "devices.json")
	reg, err := NewRegistry(path, 10)
	if err != nil {
		t.Fatal(err)
	}
	if err := reg.Put("hash", Device{APNsToken: "ff", Env: "sandbox", UpdatedAt: time.Now()}); err != nil {
		t.Fatal(err)
	}
	again, err := NewRegistry(path, 10)
	if err != nil {
		t.Fatal(err)
	}
	if d, ok := again.Get("hash"); !ok || d.APNsToken != "ff" {
		t.Errorf("device did not survive a restart: %+v", d)
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

	type device struct{ conn, secret, token string }
	devices := []device{
		{"AAAAAAAA-0000-0000-0000-000000000001", strings.Repeat("11", 32), strings.Repeat("a1", 32)},
		{"BBBBBBBB-0000-0000-0000-000000000002", strings.Repeat("22", 32), strings.Repeat("b2", 32)},
		{"CCCCCCCC-0000-0000-0000-000000000003", strings.Repeat("33", 32), strings.Repeat("c3", 32)},
	}
	for _, d := range devices {
		body, _ := json.Marshal(map[string]string{
			"apnsToken":     strings.Repeat("ab", 32),
			"sendTokenHash": SendTokenHash(d.token),
			"env":           "sandbox",
		})
		resp, err := srv.Client().Post(srv.URL+"/v1/register", "application/json",
			strings.NewReader(string(body)))
		if err != nil {
			t.Fatal(err)
		}
		resp.Body.Close()
	}

	home := t.TempDir()
	confFor := func(d device) string {
		return "RELAY_URL=" + srv.URL + "\nSEND_TOKEN=" + d.token +
			"\nSECRET=" + d.secret + "\nCONN=" + d.conn + "\n"
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

	cmd := exec.Command("sh", script, "attention", "claude", "may I?")
	cmd.Env = append(os.Environ(), "HOME="+home, "MOSHPIT_PUSH_HOST=m1-pro",
		"TMUX_PANE=%3", "TMUX=")
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("sender failed: %v\n%s", err, out)
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
	register(t, srv, "sandbox")

	conf := filepath.Join(t.TempDir(), "push.conf")
	err = os.WriteFile(conf, []byte(
		"RELAY_URL="+srv.URL+"\n"+
			"SEND_TOKEN="+testSendToken+"\n"+
			// Deliberately UPPERCASE: the shell must lowercase the secret before
			// deriving, or the phone rejects its own messages with a MAC error.
			"SECRET="+strings.ToUpper(testSecret)+"\n"+
			"CONN="+testConn+"\n"), 0o600)
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

func TestUnregisterIsAuthenticatedAndIdempotent(t *testing.T) {
	r, _, srv := newTestRelay(t)
	register(t, srv, "sandbox")

	del := func(token string) int {
		req, _ := http.NewRequest("DELETE", srv.URL+"/v1/register", nil)
		if token != "" {
			req.Header.Set("authorization", "Bearer "+token)
		}
		resp, err := srv.Client().Do(req)
		if err != nil {
			t.Fatal(err)
		}
		resp.Body.Close()
		return resp.StatusCode
	}

	if got := del(""); got != http.StatusUnauthorized {
		t.Errorf("no token: %d, want 401", got)
	}
	if got := del("not-the-token"); got != http.StatusOK {
		// An unknown token deletes nothing; answering OK keeps the endpoint from
		// telling a prober which tokens exist.
		t.Errorf("unknown token: %d, want 200", got)
	}
	if r.reg.Len() != 1 {
		t.Error("an unknown token deleted a real registration")
	}
	if got := del(testSendToken); got != http.StatusOK {
		t.Errorf("delete: %d", got)
	}
	if r.reg.Len() != 0 {
		t.Error("the device was not removed")
	}
	// Idempotent: the phone retrying a delete that worked must not see an error.
	if got := del(testSendToken); got != http.StatusOK {
		t.Errorf("repeat delete: %d, want 200", got)
	}
}

// The reply path: phone files a decision, host collects it. The relay routes and
// never opens — a compromised one can hand the host any bytes and the waiter
// drops them on the MAC, which is the whole safety of this direction.
func registerWithRespond(t *testing.T, srv *httptest.Server) string {
	t.Helper()
	respondToken := strings.Repeat("9e", 32)
	body := map[string]string{
		"apnsToken":        strings.Repeat("ab", 32),
		"sendTokenHash":    SendTokenHash(testSendToken),
		"respondTokenHash": SendTokenHash(respondToken),
		"env":              "sandbox",
	}
	b, _ := json.Marshal(body)
	resp, err := srv.Client().Post(srv.URL+"/v1/register", "application/json", strings.NewReader(string(b)))
	if err != nil {
		t.Fatal(err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != 200 {
		t.Fatalf("register: %d", resp.StatusCode)
	}
	return respondToken
}

func postRespond(t *testing.T, srv *httptest.Server, token, pane string) *http.Response {
	t.Helper()
	e, err := sealbox.Seal(testSecret, []byte(`{"decision":"allow"}`))
	if err != nil {
		t.Fatal(err)
	}
	b, _ := json.Marshal(map[string]any{"env": e, "pane": pane})
	req, _ := http.NewRequest("POST", srv.URL+"/v1/respond", strings.NewReader(string(b)))
	req.Header.Set("authorization", "Bearer "+token)
	req.Header.Set("content-type", "application/json")
	resp, err := srv.Client().Do(req)
	if err != nil {
		t.Fatal(err)
	}
	return resp
}

func awaitOnce(t *testing.T, srv *httptest.Server, token string) (int, []map[string]any) {
	t.Helper()
	req, _ := http.NewRequest("GET", srv.URL+"/v1/await", nil)
	req.Header.Set("authorization", "Bearer "+token)
	resp, err := srv.Client().Do(req)
	if err != nil {
		t.Fatal(err)
	}
	defer resp.Body.Close()
	var out struct {
		Decisions []map[string]any `json:"decisions"`
	}
	_ = json.NewDecoder(resp.Body).Decode(&out)
	return resp.StatusCode, out.Decisions
}
