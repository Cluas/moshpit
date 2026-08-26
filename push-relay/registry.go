package main

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"sync"
	"time"
)

// Device is one phone, as the relay knows it. Note what is NOT here: no host
// names, no session or pane ids, no agent names, no message text. The relay
// routes ciphertext by token and stores nothing it could be asked to hand over.
type Device struct {
	// APNsToken is the hex device token from the phone.
	APNsToken string `json:"apnsToken"`
	// Env is "production" or "sandbox" — which APNs host this token lives on.
	Env string `json:"env"`
	// SendHash is this row's own key, carried inside the value so a lookup by
	// respond token can name the mailbox the host polls.
	SendHash string `json:"sendHash,omitempty"`
	// UpdatedAt is the last registration. Device tokens rotate (reinstall, OS
	// restore, some updates), so the phone re-registers on every launch and the
	// newest one under a given send-token wins.
	UpdatedAt time.Time `json:"updatedAt"`
}

// Registry maps a send-token HASH to the device it may push to.
//
// The relay never holds the send token itself, only sha256 of it. So a stolen
// registry file lets an attacker push to a device (it holds the APNs token) but
// NOT forge a request as the dev host, and never read a single message: the
// pairing secret that opens the envelopes exists only on the phone and on the
// user's own server.
type Registry struct {
	mu   sync.RWMutex
	path string
	byID map[string]Device
	// max caps the registry so an unauthenticated /v1/register cannot be used to
	// grow the file without bound.
	max int
}

var ErrRegistryFull = errors.New("registry full")

func NewRegistry(path string, max int) (*Registry, error) {
	r := &Registry{path: path, byID: map[string]Device{}, max: max}
	if path == "" {
		return r, nil
	}
	b, err := os.ReadFile(path)
	if errors.Is(err, os.ErrNotExist) {
		return r, nil
	}
	if err != nil {
		return nil, err
	}
	if err := json.Unmarshal(b, &r.byID); err != nil {
		return nil, err
	}
	return r, nil
}

// SendTokenHash is the registry's key: sha256 of the bearer credential the dev
// host sends. Hex, lowercase.
func SendTokenHash(sendToken string) string {
	sum := sha256.Sum256([]byte(sendToken))
	return hex.EncodeToString(sum[:])
}

func (r *Registry) Put(hash string, d Device) error {
	r.mu.Lock()
	defer r.mu.Unlock()
	existing, exists := r.byID[hash]
	if !exists && r.max > 0 && len(r.byID) >= r.max {
		return ErrRegistryFull
	}
	// Every phone re-announces itself on every launch, and /v1/register is
	// unauthenticated, so rewriting the whole file each time turns a cheap
	// request into a full-registry write. Only the timestamp differs on a
	// no-op re-registration, and nothing reads it.
	if exists && existing.APNsToken == d.APNsToken && existing.Env == d.Env {
		return nil
	}
	r.byID[hash] = d
	return r.persistLocked()
}

func (r *Registry) Get(hash string) (Device, bool) {
	r.mu.RLock()
	defer r.mu.RUnlock()
	d, ok := r.byID[hash]
	return d, ok
}

// Delete drops a device. Called when APNs reports the token is Gone —
// retrying a dead token indefinitely is how a provider earns a rate limit.
func (r *Registry) Delete(hash string) {
	r.mu.Lock()
	defer r.mu.Unlock()
	delete(r.byID, hash)
	_ = r.persistLocked()
}

// SetEnv records that this token actually lives on the other APNs host, after a
// BadDeviceToken taught us the phone's own guess was wrong.
func (r *Registry) SetEnv(hash, env string) {
	r.mu.Lock()
	defer r.mu.Unlock()
	if d, ok := r.byID[hash]; ok {
		d.Env = env
		r.byID[hash] = d
		_ = r.persistLocked()
	}
}

func (r *Registry) Len() int {
	r.mu.RLock()
	defer r.mu.RUnlock()
	return len(r.byID)
}

// persistLocked writes via a temp file + rename, so a crash mid-write cannot
// leave a truncated registry that loses every paired device at once.
func (r *Registry) persistLocked() error {
	if r.path == "" {
		return nil
	}
	if err := os.MkdirAll(filepath.Dir(r.path), 0o700); err != nil {
		return err
	}
	b, err := json.Marshal(r.byID)
	if err != nil {
		return err
	}
	tmp := r.path + ".tmp"
	if err := os.WriteFile(tmp, b, 0o600); err != nil {
		return err
	}
	return os.Rename(tmp, r.path)
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
