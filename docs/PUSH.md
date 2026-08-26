# Remote agent notifications (APNs)

How an agent on your own server wakes a phone that is not running Moshpit.

## Why this exists

The Vibe Island already knows what every agent is doing: hooks stamp
`@moshpit_*` options onto tmux panes, `AgentActivityMonitor` reads them over the
live tmux `-CC` channel, and it posts local notifications and drives the Dynamic
Island. That path has one hard limit, and it is not a bug we can fix:

> iOS suspends the app seconds after it leaves the screen and kills its sockets
> shortly after. Nothing in Moshpit is executing to notice the agent asking.

So "Claude is blocked on a permission prompt" reaches you only while you are
already looking at your phone — which is the moment you least need telling. A
push notification is the only mechanism iOS offers for waking a suspended or
terminated app, and Apple delivers one only if it is signed with **this app's
team key**. That key cannot be handed out to every user's dev machine, so
exactly one job has to live on a server.

Nothing else moves off the device. The relay is not a proxy for your terminal,
does not hold your SSH credentials, and never sees your session.

## Shape

```
dev host                          relay (ours)                 Apple        phone
--------                          ------------                 -----        -----
agent hook fires
  moshpit-stamp.sh
    |- tmux set @moshpit_state ...  (the existing local path, unchanged)
    `- moshpit-push.sh  -- POST /v1/notify --+
         seal(status, SECRET)                |  Bearer SEND_TOKEN
                                             |- sha256 -> device token
                                             |- sign JWT (ES256, .p8)
                                             `-- POST /3/device/... -> APNs -> push
                                                                               |
                                             MoshpitPush (NSE) <----------------'
                                               open(envelope, SECRET)
                                               rewrite title + body
                                               inject connectionId + paneId
                                                      |
                                             lock screen: Allow / Deny / Reply
                                                      |
                                             AgentNotificationHandler
                                               -> AgentControlBridge -> live pane
```

The last two steps are existing code, and the sentence that used to be here —
that a pushed notification is equivalent to a local one, "which is why this
change is as small as it is" — was wrong in a way worth keeping on the page,
because it is the load-bearing claim the whole design rested on.

**The two are not equivalent. On the property the buttons depend on, they are
opposites.** A local notification implies a live session: the app posted it
BECAUSE it was watching one. A push implies the app is gone, which is the entire
reason it was sent. And Allow/Deny reach the agent through
`SessionHub.deliverAgentInput`, whose first line is

    guard !bytes.isEmpty, let active = sessions[connectionId] else { return false }

against an in-memory dictionary written only when a person opens a session in the
UI. Nothing restores sessions at launch. So when iOS relaunches a terminated app
to handle a notification action, that dictionary is empty and the tap returns
false without one attempt at delivery. Two further walls sit behind it: the
actions are declared with no `.authenticationRequired`, and every
`KeychainService` item is `WhenUnlockedThisDeviceOnly`, so a locked phone cannot
read the SSH credentials a reconnect would need.

`AgentControlBridge` had said as much all along — "a fully torn-down session
can't be revived from a background action" — while this document claimed
equivalence and the notification kept offering the button. The honest reading of
"why this change is as small as it is" is that it was small because it had not
done the part that needed doing.

**So a pushed notification now carries no action buttons at all**, only the wake
and the tap that opens to the pane; local notifications keep theirs, where the
session is live by construction. A button that cannot succeed is the thing this
feature has spent its whole life removing, and this one could not succeed in
precisely the case it was built for. The real answer — the host, which is awake
and has the tmux pane, collecting the decision instead of the phone trying to
deliver it — is designed but not built; see Known gaps.

## The privacy boundary

The relay is a dumb pipe by construction, not by promise.

**What it necessarily learns**

| | why it is unavoidable |
|---|---|
| a device token | it is the address APNs delivers to |
| roughly when a push happened | it forwarded it |
| `attention` vs `done` | it picks which translated fallback line to send, and how long the push may stay deliverable (10 minutes vs an hour) |
| a collapse id: `moshpit.<state>.<connection uuid>.<pane>` | it becomes an HTTP header, and matching the app's own notification identifier is what stops a push from buzzing on top of a local one |

The connection uuid in that collapse id is generated on the phone and means
nothing anywhere else; the pane id is a small integer like `%3`.

**What it cannot learn**

The host name, the session name, the agent, and every character of what the
agent asked. Those travel inside a sealed envelope whose key is stored in
exactly two places: this phone, and your own `~/.moshpit/push.conf` at mode
0600. The relay is never told the key, and a stolen relay database does not
contain one.

**A caveat that belongs next to that claim, not below it.** "Stored in two
places" is about storage. While the sender runs, openssl takes its key as a
command-line argument — it offers no file or fd alternative for `enc` — so the
encryption subkey and the pairing secret it derives from are briefly visible in
that process's argv. On Linux `/proc/<pid>/cmdline` is world-readable by
default, so another local user on a SHARED host can capture them by polling
`ps`, and hooks fire often. The send token is kept out of argv (curl reads it
from a `mktemp` 0600 config file), because that one can be protected; the
encryption key cannot be, portably. On a machine you share with people you would
not hand your agent transcripts to, either do not pair it or run with
`hidepid=2`.

Since the reply path landed, that paragraph is worth reading a second time with
a harder question in mind. It used to describe a confidentiality loss: someone
polling `ps` learns what your agents are asking. The pairing secret now also
authenticates decisions travelling the other way, so the same capture is a
capability — with the secret and a delivery path to the relay, a local user can
seal an `allow` and have your host press Enter on a permission prompt. The
waiter narrows this (only Enter, Esc and Ctrl-C, only into a pane whose agent is
still alive and still on the same question, only for ten minutes), which is why
its whitelist is a whitelist. It does not close it. On a shared host, `hidepid=2`
stopped being hardening and became the precondition.

It does not even hold the credential that authorises sending: it stores
`sha256(SEND_TOKEN)`, so its database can route pushes but cannot originate
them.

## APNs: token-based authentication

Both APNs auth schemes speak the same HTTP/2 provider API. The difference is
what you have to keep alive.

| | certificate auth | **token auth (what we use)** |
|---|---|---|
| material | a per-app `.p12`, presented as a TLS client cert | one per-**team** `.p8` ECDSA P-256 key |
| expiry | every year, fails closed at 3am on a date nobody wrote down | none |
| scope | one bundle id | every app in the team |
| environments | separate cert per environment in practice | same key for sandbox **and** production |
| renewal | portal + Keychain dance | — |

That last row is what settles it here: one relay deployment serves development
builds (sandbox) and App Store builds (production) off one secret, choosing per
request which host to dial.

The provider token is a JWT:

```
header  {"alg":"ES256","kid":"<10-char Key ID>","typ":"JWT"}
claims  {"iss":"<10-char Team ID>","iat":<unix seconds>}
```

Three things bite, all of them handled in `push-relay/apns/apns.go` and pinned
by its tests:

- **ES256 wants a raw `r||s` signature, 64 bytes, each half left-padded to 32.**
  Anything DER/ASN.1-shaped — which is what most signing APIs hand you, Go's
  `ecdsa.SignASN1` included — comes back as a bare `403 InvalidProviderToken`
  with no hint that the encoding is the problem.
- **Cache the token.** APNs rejects an `iat` older than an hour *and* throttles
  providers that mint new ones too eagerly (`TooManyProviderTokenUpdates`). We
  reuse one for 45 minutes, and drop it immediately on any 403.
- **`sub` is only for topic-restricted keys.** Opt-in via `MOSHPIT_APNS_SUBJECT`
  rather than always-on.

Request headers that matter: `apns-push-type: alert` (mandatory since iOS 13),
`apns-topic: com.cluas.moshpit`, `apns-priority: 10`, `apns-expiration`,
`apns-collapse-id`. The response's **`apns-unique-id`** is the only handle for
looking a push up afterwards in Apple's Push Notification Console — the relay
logs it, because without it "my phone stayed silent" is undebuggable.

A `200` means APNs accepted the push. It does not mean a phone showed it.

## The envelope: format v1

Specified once in `push-relay/sealbox/sealbox.go`, implemented three times, and
pinned to one frozen vector (`scripts/push-vector.sh`).

```
KeHex = hex(HMAC-SHA256(key: ascii(SECRET), msg: "moshpit-push-enc-v1"))
KmHex = hex(HMAC-SHA256(key: ascii(SECRET), msg: "moshpit-push-mac-v1"))
ivHex = 16 random bytes, lowercase hex
ct    = base64(AES-256-CBC(key: KeHex, iv: ivHex, pkcs7(plaintext)))
mac   = base64(HMAC-SHA256(key: ascii(KmHex), msg: "v1|" + ivHex + "|" + ct))
```

Every choice here is forced by the sender being a POSIX `sh` script on someone
else's server with nothing but `openssl` and `curl`:

- **CBC + HMAC, not AES-GCM** (which the rest of the app uses). `openssl enc`
  refuses AEAD ciphers outright — "AEAD ciphers not supported by enc" — so a GCM
  format would be unsendable without shipping a binary to every host.
- **Subkeys used in hex-string form.** `openssl dgst -mac HMAC -macopt hexkey:...`
  would take raw keys but does not exist in the LibreSSL macOS ships as
  `/usr/bin/openssl`; plain `-hmac <string>` exists in both. Raw bytes also
  cannot pass through a shell argument once they contain NUL.
- **The MAC covers the ASCII wire text**, not raw `iv||ct`. Hex-to-binary in
  portable `sh` needs `xxd` (not always installed) or octal-printf gymnastics.
  MACing the encoding is equivalent: base64 and hex are injective, and decoding
  happens only after the MAC verifies, so a non-canonical encoding changes the
  MAC input and is rejected rather than normalised.

Encrypt-then-MAC, with the MAC verified before any padding is inspected. That
ordering is what makes CBC safe here: a padding oracle needs attacker-chosen
ciphertext to reach the unpadding step.

The plaintext is one JSON object:

```json
{"conn":"<the phone's connection uuid>","host":"m1-pro","sess":"work",
 "pane":"%3","agent":"claude","state":"attention",
 "title":"Bash: rm -rf build","ts":1755900000}
```

`conn` is why no host-name-to-connection lookup exists anywhere: the phone told
the host its own id at pairing, so a notification arrives already knowing which
saved connection it belongs to.

## Pairing, and everything else installed on a host

Per **connection**, not per device. Each host gets its own secret and its own
send token, which buys three things: unpairing one server cannot break the
others, a compromised host cannot forge notifications appearing to come from a
different one, and rate limiting is per host.

Nothing is pasted into a shell. `HostInstaller` (`Moshpit/Services/Install/`)
runs everything over the same SSH exec channel image attachment uses —
`SessionHub.ActiveSession.acquireFileTransferSSH()`: the in-band session when
there is one, the mosh sidecar otherwise, an on-demand dial for pure mosh from
the in-memory secret cache so it never re-prompts Face ID. That means installing
works identically on SSH, mosh and herdr, needs no tmux, touches no pane, and
cannot be blocked by an agent holding the one that happens to be active.

  * **Files go up as base64.** The base64 alphabet cannot contain a quote, a
    dollar or a newline, so there is no escaping to get wrong and nothing in a
    config file or a secret can survive into the command line.
  * **`push.conf` is written with `umask 077`, not chmod'd after** — a chmod
    leaves a window where a file holding two secrets is world-readable.
  * **The sender lands before the secrets it reads.** A host must never hold a
    pairing secret with no program that could use it.
  * **A write is confirmed by digest**, and a mismatch is reported rather than
    assumed away.
  * **`~/.moshpit/manifest.json`** records what is installed, content-addressed
    by SHA-256. That is how a stale copy is detectable at all: the day the stamp
    script gained a push hand-off, every installed copy silently stopped being
    current and nothing could tell. A digest cannot be forgotten the way a
    version number can.
  * **Agent configs are merged in Swift**, not on the host. The flow this
    replaces carried a jq program plus a python3 fallback plus a heredoc to hold
    them; the host now needs only `cat` and `base64 -d`. A config that will not
    parse is reported, never overwritten, and the copy from before Moshpit ever
    touched it is kept once as `<config>.moshpit.orig`.

Where the secrets live on the phone: a file in the App Group container with
`.completeFileProtectionUntilFirstUserAuthentication`, excluded from backup. Not
the app's `KeychainService`, because every item it writes is
`WhenUnlockedThisDeviceOnly` — unreadable by an extension running on the lock
screen, which is exactly when an "agent needs you" matters. A keychain item in
an App-Group access group is the obvious hardening step and is deliberately not
the first implementation: that path cannot be verified on the simulator, and a
store that silently fails on device would present as "push just doesn't work".

### Proving it, rather than claiming it

Two checks, both automatic, neither asking the user to go and demonstrate
anything:

**Landing.** Files exist, digests match, the config parses, the tools are there.

**The runtime path, fired for real.** For pushes, the host is asked to send one
`--test` notification carrying a nonce, and the phone waits for that exact nonce
to arrive — a signal that travels host → relay → Apple → phone and cannot be
satisfied by any local lie. For hooks, the stamp script is invoked directly and
the pane read back, which proves it exists, is executable and can reach tmux
without waiting for an agent turn. Both use the reserved agent label
`moshpit-selftest`, which `AgentActivityMonitor` skips so proving an install
leaves no phantom agent on the island, and which the notification delegate
recognises so the proof is not shown to the user as a notification about
plumbing.

## Two safety properties worth naming

**Attention pushes expire in 10 minutes.** Allow/Deny from a lock screen is a
blind keystroke — Enter or Esc into whatever prompt the pane holds *now*. The
app refuses one when it knows the prompt has moved on
(`AgentActivityMonitor.attentionState`), but a phone that was off for an hour
has no such record and would let it through. So a stale attention push must
never arrive at all. `done` gets an hour; it is only ever informational.

**Only a decrypted push is actionable.** The payload carries NO notification
category; the extension adds one after it opens the envelope. So Allow and Deny
appear exactly when the pane ids needed to act on them are present, and never
otherwise — not when no key matches, and not when the extension never runs
because it timed out. Those cases show the translated fallback with no buttons,
and a body tap that opens the app.

That is the second version of this design. The first put the category in the
payload and argued the fallback should stay actionable; a peer review pointed out
the fallback *cannot* be actionable, because what you would need in order to act
on it is exactly what failed to arrive — and that a dead Allow is this feature's
worst outcome, since the user walks away believing they approved while the agent
is still waiting. The relay is still told `attention` or `done`, but only to pick
the fallback wording and the expiry.

**Attention expires on the device, too.** `apns-expiration` is set by the relay,
which is the party this design does not trust — a compromised one could hold an
envelope and replay it at will. `ts` is inside the sealed envelope, so the
extension checks it (`PushRemoteNotification.attentionLifetime`, 15 minutes with
slack for clock skew) and drops the buttons on anything older, putting that
guarantee back on the phone.

## Why there are no buttons

A notification from Moshpit wakes you, names the agent, says what it is asking,
and opens that pane when tapped. It offers nothing else, and the nothing is the
design.

For a while it offered Allow, Deny, Reply and Stop, and there was a whole
mechanism behind them: a `/v1/respond` endpoint, a mailbox in the relay, a
`/v1/await` long poll, and `moshpit-await.sh` on the host verifying a sealed
decision and pressing the key. It worked — proven end to end on a real phone
against the deployed relay. It has been removed anyway, and the reason is not
that it was hard.

**Allow was a blind keystroke.** It sent Enter, which accepts whatever option the
agent had highlighted. Tapping it meant approving an action you had not read, on
the strength of a one-line title, from an app whose entire value proposition is
that you *can* read it. Moshpit is a terminal. The correct answer to "an agent
needs you" is to look at the pane — which is exactly what tapping the
notification does. A button that lets you skip the looking is not a convenience
in this product; it is an invitation to rubber-stamp an agent's permission
requests from a lock screen.

Three delivery problems pointed the same way, and are worth recording because
each was discovered as a bug before the premise was questioned:

1. **The local path could not work in the case it existed for.**
   `SessionHub.deliverAgentInput` begins `guard let active = sessions[connectionId]
   else { return false }`, and nothing restores sessions at launch. A local
   notification implies a live session — it is how the app knew to post it. A push
   implies the app is gone — it is why it was sent.
2. **The relay path could only press keys, never type.** The host waiter's
   whitelist was Enter, Esc and Ctrl-C on purpose, so `Reply` and the quick-reply
   chips silently did nothing at the far end — a tap that takes a typed sentence
   and drops it.
3. **The buttons were wired too late to fire at all.**
   `AgentControlBridge.shared.handler` was assigned in a `.task` on the root view,
   and iOS launches the app to the BACKGROUND to run a `LiveActivityIntent`, where
   a `WindowGroup`'s content may never mount. `await handler?(...)` then did
   nothing, silently — no keystroke, no error, no log. This is what a user
   reported as "I tap it and nothing happens", and they were right.

That last one is fixed regardless, because it was never only about buttons: the
notification DELEGATE was registered in the same late `.task`, so a cold launch
from a tapped notification could reach iOS's callback before anything existed to
receive it. Everything the system can call into before a view exists now happens
in `MoshpitApp.init()` — the only point before every reader, which is the same
conclusion `-MOSHPIT_RESET` reached for the same reason.

What the removal took with it: `/v1/respond`, `/v1/await`, the relay's mailbox,
`scripts/moshpit-await.sh` and its install component, `PushDecisionSender`, the
pairing's respond token, the notification categories, and the Live Activity's
control row. The relay still knows nothing it did not know before, and now has
two fewer endpoints to be wrong about.

## Pairing is machinery, not a ceremony

Pairing mints a per-device secret (the end-to-end encryption) and a send token
(who may make this phone buzz). Neither can be hardcoded — a shared secret would
be everyone's secret — but nothing in the exchange ever needed a human's
judgment, and the Pair / Re-pair / Update buttons existed only because the
machinery had no other trigger. Now it has one: `HostAutoCare` runs once per
connection per app run, whenever a session's control plane comes up, and
silently puts right whatever drifted — stale scripts reinstalled (the sender is
checked EXPLICITLY; `hooksStatus` folds in the stamp but not the sender, and a
stale sender under a fresh pairing fails silently), a missing or stale pairing
written, a never-paired device minted and registered.

The one thing it never does silently is the FIRST hook install: that edits the
user's agent config, and it is the single genuine consent moment the feature
has. A host with no hooks gets one question ("Enable agent notifications on
this host?") with Enable / Not Now / Don't Ask Again; the Host Setup sheet
remains the diagnostic surface and the door for changing a "no".

**One host, many devices.** A single `push.conf` used to mean one phone per
host: the second device's pairing overwrote the first's secret and its
notifications just stopped, with nothing anywhere saying why. Pairings now live
one file per device in `~/.moshpit/push.d/<connection-id>.conf`; the sender
fans out — every paired device gets its own envelope, sealed with ITS secret,
routed by ITS token, carrying ITS connection id — and the legacy single file is
still honored, so nothing already installed breaks. `--test` takes an optional
connection id so a self-test proves one pairing instead of pinging every phone
in the house. The manifest records each device under `pairing.<conn>`; one
device unpairing removes only its own entry (pinned by a test, because that IS
the old bug in miniature).

Harnesses launch with `-MOSHPIT_AUTOCARE_OFF`: every seeded test connection
points at a real login's real `$HOME`, and auto-care would otherwise pair a
throwaway connection onto the developer's actual host every run.

## Quiet by design

The first shipped version notified on everything: every prompt, every finished
turn, per pane, with sound, at `.timeSensitive` — and the entitlement work made
that pierce Focus. The user's verdict was direct: "我们不能一直通知然后打扰用户",
and, more precisely: agents deliberately PARKED at their prompt are not asking
for anything — for those, every NEEDS YOU surface is noise.

Four rules replaced it. The principle behind all four: a notification's job is
to end a wait the user doesn't know about, not to broadcast events.

**Parked is not asking.** Claude Code fires its Notification hook both for real
blocking questions (a permission prompt, mid-turn) and for idle reminders
("Claude is waiting for your input") on a pane left at the prompt on purpose.
The stamp script now tells them apart by the TRANSITION — a real question can
only interrupt a turn, so it arrives on a `working` (or `attention`) pane; an
idle reminder arrives on a `done` one — with the message text as a
Claude-specific second belt. An idle reminder stamps nothing and pushes
nothing: the pane stays `done` everywhere, island included. Killed at the
source, so every surface quiets at once.

**The grace window.** A question must STAND for 30 seconds before any phone
hears about it. Most questions are answered at the desk within that; those now
end silently. Host side, the stamp forks `(sleep 30; same state AND same
episode; push)`; phone side, the monitor schedules the announcement and
re-checks before firing. `MOSHPIT_NOTIFY_GRACE` / `-MOSHPIT_ANNOUNCE_GRACE`
shorten it for tests.

**One card, one edge.** Attention collapses to a single summary per connection
("claude +2"), not a card per pane — the sender's collapse id and the app's
notification identifier are both `moshpit.attention.<conn>`. Sound and the
Focus breach belong to the 0→1 EDGE alone: the moment "nobody is waiting"
becomes "someone is". Agents joining or leaving the wait update the card
silently at `.passive`. The edge is read from `PushStanding`, an App Group set
both the app and the extension consult — whichever of the local announcement
and the push lands first takes the edge; the other sees "already standing" and
stays silent. Entries expire after 15 minutes, because nothing tells a dead app
that a prompt was answered at the desk; expiry keeps that blindness from
muting tomorrow's real prompt.

**A finish is information, not an interruption.** `done` renders `.passive`
and silent unless the closing episode ran ≥3 minutes (`Status.dur`, measured on
the host since the last human interaction — the walked-away case). Older
senders send no duration; absent reads as short, because quieter is the
recoverable direction.

Verified host-side against a real tmux pane by `scripts/verify-stamp-quiet.sh`:
a parked pane's idle nag stays `done` and pushes nothing; a question answered
inside the window never reaches the sender; one that stands the window out
pushes exactly once, with the question; a `done` carries its duration.

## Throttling and coalescing

Only `attention` and `done` are ever pushed. `working` fires on every tool call
and stays on the tmux bridge, where it costs nothing — that alone removes most
of the volume.

Beyond that: the relay enforces a 3-second floor between pushes per host and a
120/hour cap, and sets `apns-collapse-id` so a re-prompt **replaces** its
predecessor rather than stacking a second card for one question. The collapse id
is deliberately the same string the app uses as its local notification
identifier, which buys two more behaviours for free — a push replaces the local
notification for that pane instead of buzzing twice, and the app's existing
"revoke a prompt that has moved on" sweep removes the pushed copy too.

## Running the relay

**The relay address is not a user setting.** The pairing sheet shows which
relay will be used and, in Release builds, offers no way to change it — every
audience that could legitimately want a different one has a better door. An
App Store install can only use the hosted relay: its entire power is the APNs
signing key for this bundle id, and any other address just breaks push with no
error anywhere. Someone self-hosting must rebuild the app under their own team
and key regardless, so their door is the one-line default in
`AppSettings.pushRelayURL`. Test harnesses use the DEBUG-only field. A visible
text box served nobody and had already shipped one confusion.


Zero third-party dependencies: Go's standard library negotiates HTTP/2 over
ALPN, and `crypto/ecdsa` signs the JWT.

```
MOSHPIT_APNS_KEY_FILE       path to the .p8 from developer.apple.com
MOSHPIT_APNS_KEY_ID         10-char Key ID
MOSHPIT_APNS_TEAM_ID        10-char Team ID
MOSHPIT_APNS_TOPIC          com.cluas.moshpit          (default)
MOSHPIT_APNS_SUBJECT        only for topic-restricted keys
MOSHPIT_RELAY_ADDR          :8080                      (default)
MOSHPIT_RELAY_STATE         /var/lib/moshpit-relay/devices.json
MOSHPIT_RELAY_MAX_DEVICES   10000    (caps an unauthenticated /v1/register)
MOSHPIT_RELAY_DRY_RUN       any value: log pushes instead of sending
```

`MOSHPIT_RELAY_DRY_RUN=1` needs no Apple credentials at all and is what the
automated end-to-end test uses.

`/v1/register` is unauthenticated on purpose: both values in it are minted by
the phone, so there is no prior secret to authenticate *with*, and a caller who
invents a pair can only push to a device they already control. What it has to
resist is being used as free storage, hence the strict shape checks and the
device cap.

## What has to be done by hand at developer.apple.com

One visit, four things. Measured on 2026-08-24 by decoding the profiles this Mac
had cached (`security cms -D`) and by attempting a real device build — so this is
what IS, not what should be:

    com.cluas.moshpit          aps-environment: absent   App Group: present
    com.cluas.moshpit.island   aps-environment: absent   App Group: present
    com.cluas.moshpit.share    aps-environment: absent   App Group: present
    com.cluas.moshpit.push     App ID does not exist

**1. The APNs key.** Keys -> `+` -> Apple Push Notifications service (APNs) ->
name, Continue, Register. Download the `.p8` **once**; Apple will not offer it
again. Note the Key ID. Nothing in this document works without it — though one
already exists and has authenticated against real APNs, so this step is done.

**2. Push Notifications on the `com.cluas.moshpit` App ID.** Not done: the cached
profile carries no `aps-environment`, which is exactly what a device build says
("doesn't include the Push Notifications capability"). Simulator builds do not
check entitlements, which is why every test in this document passed without it.

**3. The `com.cluas.moshpit.push` App ID, with App Groups.** It does not exist,
so a device build cannot sign the notification service extension at all — and
this is NOT a signing formality to tidy up at release time. **The App Group IS
the decryption capability.** The extension reads the pairing secret out of the
App Group container (`PushPairingStore`); with the entitlement unauthorised,
`containerURL(forSecurityApplicationGroupIdentifier:)` returns nil, `read()`
returns an empty list, and the extension logs "no key opened this envelope (0
available)" and shows the translated fallback — for every push, forever. A phone
in that state looks like it is receiving notifications and never shows one word
the agent said. So: create the App ID, enable **App Groups**, and add
`group.com.cluas.moshpit`, the same one the other three carry.

**4. Time Sensitive, if that decision goes yes.** It is a capability on the App ID
like the two above, so it belongs in this same visit rather than a second one —
see the entitlement note in Known gaps for what it costs.

Then hand the `.p8` and the two ids to the relay via the environment above.

### One error message that misleads

A device build with `-allowProvisioningUpdates` can fail with **"No Accounts: Add
a new account in Accounts settings."** That does not mean there is no developer
account: `security find-identity -v -p codesigning` shows an
`Apple Development: Wenlong Hu` certificate on this Mac. It means the CLI has no
signed-in Xcode account SESSION with which to register the two new App IDs and
mint their profiles. Doing steps 2 and 3 in the portal (or once in Xcode with the
account signed in) is what clears it — not adding an account you already have.

## Known gaps

- ~~The setup sheet has not been driven visually.~~ **Captured 2026-08-24** by
  `MoshpitUITests/HostSetupScreenshotUITest`, which launches straight into each
  state through a DEBUG-only seam (`-MOSHPIT_HOSTSETUP_DEMO <scenario>`,
  `HostSetupDemo`). The sheet cannot be reached by tapping in a simulator at
  all — Settings gates its row on a live SSH session, so the row is disabled and
  the sheet's content is empty; that is the screen's own precondition, not an
  automation limit. Driving it from a canned channel is also better than reaching
  a real host: a host is in one state at a time, and the two most worth reviewing
  (out-of-date, and a relay that does not know this phone) are the hardest to
  arrange on purpose.

  Looking at the result immediately found three things no test had: the hooks row
  reported "Current" directly above a card saying the script was out of date (it
  reported only the config registration, not the script the agent actually
  invokes — now `InstallState.hooksStatus` takes the worse of the two), and the
  relay field sat empty while the row above it displayed the paired address, so
  "Re-pair" failed with "enter the address" while the address was on screen. Both
  fixed, both pinned by tests. A peer looking at the same six shots found a
  third: the "Pairing" row's detail fell back to whatever was typed in the field
  below it, so a "Not installed" chip sat above an address rendered exactly like
  a fact. That line now carries only the relay the host is actually paired with.

  Worth recording about the harness itself: getting each shot to stand on its
  own took three attempts, because the relay setting outlives a reinstall and my
  first two resets ran after the code that reads it — `-MOSHPIT_RESET` is
  handled in `RootView`'s task, which the demo branch never mounts, and a `.task`
  on the demo view races `HostSetupView`'s own. `MoshpitApp.init()` is the only
  point before every reader.
- **The installer's commands now really run; the button and the app's own channel
  do not.** `scripts/verify-host-install.sh` sends the actual `HostCommands`
  strings over ssh to a real host with `HOME` pointed at a scratch directory, and
  they hold: the base64 hop is byte-exact (the digest of what lands equals the
  SHA-256 the app computed over the content, which is the only thing that proves
  it, and both scripts contain single quotes — that is why the hop exists);
  `umask 077` means `push.conf` is `-rw-------` from birth rather than after a
  chmod; a `settings.json` carrying single quotes, backticks, `$HOME` and CJK
  survives byte-identical and still parses on the host; `backupOnce` run twice
  still leaves one `.orig`; and the hook self-test fires the stamp script at a
  real tmux pane, which reports `working|moshpit-selftest|` and clears to
  nothing — the sheet's claim that it "fires the stamp script and reads the pane
  back" is now evidence rather than a description.

  Two things it deliberately does not cover, and no tooling is planned for them:
  the SwiftUI tap, and the app's OWN exec channel — the script uses `ssh(1)`,
  while the app uses Citadel over its in-band session or the mosh sidecar. The
  tap is worth a human pressing Install once, not a harness. The channel carries
  one specific residual risk worth watching on that first real install: a
  `writeFile` for the sender is a single command line of several kilobytes of
  base64, which OpenSSH handled and Citadel's exec channel has never been asked
  to carry.
- ~~Whether the extension runs while the app is FOREGROUND~~ — **verified
  2026-08-24**, and it does. With the app frontmost, a real `--test` push through
  APNs produced both halves of the chain in the log:

      MoshpitPush[39629] [push] opened a done push from m1-pro
      Moshpit[39098]     [push] self-test push arrived: selftest-fgtest1169

  So the extension decrypts, the delegate recognises the reserved label,
  suppresses the banner, and the nonce reaches the sheet that is waiting on it.
  The technique came from a peer review and needs no UI driving at all: the
  extension's own log line only exists if the extension ran, which separates
  "not invoked in the foreground" from "invoked but could not decrypt".
- **Live Activity push (T2) is not built.** The Dynamic Island still updates
  only in-process. Push-to-start (`Activity.pushToStartTokenUpdates`) would let
  the island appear with the app not running; the payload for it cannot be
  end-to-end encrypted, because ActivityKit decodes `content-state` itself with
  no extension in the path. That asymmetry needs a decision, not just code.
- ~~No reverse control path (T3).~~ **Built, then deleted — and the deletion is
  the answer, not a retreat.** See "Why there are no buttons". Answering an agent
  from a lock screen is not a missing feature of this product; it is the one thing
  a terminal app should not make easy, because it means approving what you have
  not read. The notification's job is to get you to the pane.

  If it is ever revisited, the thing to revisit is not the transport — that part
  worked. It is whether there is a control worth offering blind at all. `Stop`
  (Ctrl-C) is the only candidate with the right shape: safe to send without
  reading, recoverable if wrong. `Allow` is the opposite on both counts.
- ~~Time Sensitive is asked for but not entitled.~~ **Shipped 2026-08-25.**
  `com.apple.developer.usernotifications.time-sensitive` is in `project.yml`, so
  an "agent needs you" notification is delivered at `.timeSensitive` instead of
  being silently downgraded to `.active`.

  **Verified on an iPhone 16 Pro, 2026-08-25**, and the device log is worth
  quoting because it separates the two things this could mean:

      SpringBoard: Adding notification … interruption-level: 2
      … timeSensitiveSetting: Enabled …
      DoNotDisturb: … urgency: Time-Sensitive …
      donotdisturbd: Breakthrough is NOT allowed with reason:
                     mode configuration type

  Level 2 is `.timeSensitive`, `timeSensitiveSetting: Enabled` is iOS agreeing
  the app is entitled, and the DND subsystem classified the urgency correctly.
  The entitlement works. It still did not break through, because the Focus that
  happened to be on was configured `applicationConfigurationType: Exclusive` with
  an empty allow list and `minimumBreakthroughUrgency: essential`.

  So the honest claim is **eligible**, not **breaks through**. Every earlier
  draft of this line said the latter. Each Focus mode decides for itself, and a
  user who has told a Focus to allow nothing has said no — which the app must not
  and cannot override. Allowing it is one toggle in Settings → Focus → that mode
  → Time Sensitive Notifications.

  The decision was never about capability. An earlier version of this entry said
  a free Personal Team cannot sign that entitlement and stopped there, which read
  as "we can't" — true in general, wrong here, since this project is on a paid
  Developer Program account and the proof is in this document (an APNs Auth Key
  can only be created by a paid member). What it actually cost was one more
  commented-out line in `docs/install-free-account.md`, exactly the shape
  `aps-environment` already had. That page now covers both.

  **Turning it on immediately surfaced something the old state was hiding.** The
  relay sent `time-sensitive` in every payload, `done` included, and that looked
  harmless for as long as iOS was downgrading everything anyway. With the
  entitlement real it would have meant "agent finished" piercing Do Not Disturb.
  The relay now steps a `done` down to `active`, matching the line the app
  already drew locally — `postAttention` sets `.timeSensitive`, `postDone` leaves
  the default — and `TestOnlyAQuestionBreaksThroughFocus` pins it. Worth naming
  as its own lesson: an entitlement that is not provisioned makes every payload
  that requests it look identical, so the request itself goes unreviewed.

  One provisioning surprise, in the useful direction: the App ID capability did
  not have to be ticked by hand. `xcodebuild -allowProvisioningUpdates` added it
  and regenerated the profile, and both the signed binary and the embedded
  profile carry the key:

      codesign -d --entitlements - Moshpit.app
        [Key] com.apple.developer.usernotifications.time-sensitive
        [Value] [Bool] true
- **`aps-environment` breaks free-account device builds.** The Push
  Notifications capability is unavailable to a Personal Team; comment that one
  line out of `project.yml` and re-run `xcodegen generate`. Simulator builds are
  unaffected, which is why the tests do not notice.
- **`MoshpitPush` cannot be signed for ANY device build yet**, and the reason is
  mechanical rather than administrative — see step 3 above. An earlier version of
  this line said it "will need a provisioning profile for release, the same
  hand-made-once path MoshpitShare documents", which was wrong twice: it is not a
  release-time concern (every device build fails today), and calling it a
  provisioning chore hid the fact that the App Group on that App ID is what makes
  decryption possible at all.

  That is the third time in this feature that a mechanism got demoted to polish
  by its own documentation — after "a free Personal Team cannot sign
  time-sensitive" (which read as "we can't" when the account is paid and it is
  merely a decision) and `MOSAIC_SSH_KEY` (a mitigation that already existed and
  was never signposted). All three read as tidy-up work and all three were load
  bearing. Worth suspecting the pattern rather than the individual line.


- ~~Codex's `[[hooks.X]]` TOML shape~~ — **verified 2026-08-24 against codex-cli
  0.143.0, and the shape is right.** Asked Codex what it sees (`hooks/list` over
  `codex app-server`, with `CODEX_HOME` pointed at a scratch directory so the
  user's live config was never touched): all four of our events come back
  recognised, as `preToolUse` / `permissionRequest` / `userPromptSubmit` / `stop`,
  each `handlerType: "command"` with the command intact.

  **But an installed hook does nothing until the user trusts it.** Every entry
  arrives `trustStatus: "untrusted"`, and Codex will not run an untrusted hook —
  proved both ways: nothing fires normally, and with
  `--dangerously-bypass-hook-trust` the same config fires immediately. Trust is
  recorded against a hash of the hook entry as written, so updating the stamp
  script does not un-trust it; changing the command line would.

  That is this feature's signature failure in a new place — installed, reported
  Current, and nothing will ever fire — so it is surfaced rather than left
  implicit: `HookAgent.trustStep` carries it, the install report returns it, and
  the sheet shows "One more step, on the host" naming `/hooks`. The hook
  self-test's failure message names trust as the likely cause too, since an
  untrusted hook and a broken one look identical from the phone.

## Tests

| what | where | runs |
|---|---|---|
| envelope format, tampering, the frozen vector | `push-relay/sealbox/sealbox_test.go` | `go test ./...` |
| JWT is verifiable ES256, token caching, headers, 410 handling | `push-relay/apns/apns_test.go` | `go test ./...` |
| payload shape, allowlists, throttle, env fallback | `push-relay/relay_test.go` | `go test ./...` |
| **the real shell sender against the real relay** | `TestShellSenderInterop` | `go test ./...` |
| the same vector, opened by CryptoKit | `MoshpitTests/Services/PushSealedBoxTests.swift` | `xcodebuild -scheme MoshpitUnitTests test` |
| pairing material (the one-liner's tests went with it) | `MoshpitTests/Services/PushPairingTests.swift` | same |
| the shell↔Swift drift guard for both host scripts | `MoshpitTests/Services/HostScriptsTests.swift` | same |
| install engine: commands, digests, staleness, uninstall | `MoshpitTests/Services/HostInstallerTests.swift` | same |
| agent config merge and removal, JSON and TOML | `MoshpitTests/Services/AgentHookConfigTests.swift` | same |
| the setup screen's state machine | `MoshpitTests/Services/HostSetupModelTests.swift` | same |
| `awaitSelfTest` returns on timeout instead of hanging | `MoshpitTests/Services/PushServiceTests.swift` | same |
| extension rendering and userInfo routing | `MoshpitTests/Services/PushRemoteNotificationTests.swift` | same |
| the extension's own `didReceive`, against the real App Group store | `MoshpitTests/Services/PushNotificationServiceTests.swift` | same |
| hook -> seal -> relay -> payload, end to end | `scripts/verify-push-e2e.sh` | manual, no credentials needed |
| the installer's real commands against a real host | `scripts/verify-host-install.sh` | manual, needs passwordless ssh + tmux |
| **the lock-screen card's HEIGHT** | `MoshpitTests/Island/LockScreenHeightTests.swift` | `xcodebuild -scheme MoshpitUnitTests test` |

iOS clips a Lock Screen Live Activity at 160pt and reports nothing — no warning,
no log, no callback. The only signal is a photograph of a phone with the card
sliced off at both ends, which is how the overflow was found, after the layout had
been "budgeted" twice in comments by eye and shipped over-height anyway.

`LockScreenHeightTests` renders the real view with `ImageRenderer` at the real
width and reports the height it wanted, so the budget is an assertion instead of a
belief. What it found: **138pt for a single agent** — the commonest case, the one
the whole feature exists for, clipped since the day it shipped — and 367pt for
three. Most of that was the Allow/Deny row and the quick-reply chips, a third of
the card spent on buttons that could not work, while the line saying what the
agent was asking fell off the bottom. It is 104–152pt now, bounded, with the row
count capped by a constant the test reads rather than restates.

One fixture bug from writing it, worth keeping: the helper built each agent's `id`
from its NAME, so a test with two agents both called "claude" — the realistic case,
two Claude panes — handed `ForEach` two identical ids and measured 220pt for a
shape that actually renders 152. Real ids are `"<connectionUUID>:<paneId>"` and
cannot collide. A fixture that can makes every number in the file suspect.

**`xcrun simctl push` does NOT exercise the extension.** Verified on 2026-08-24
against an iPhone 17 Pro simulator: SpringBoard accepted the notification and
filed it under `com.cluas.moshpit`, while the `MoshpitPush` process never ran at
all — no log line, no process. It injects a payload into the notification system
and skips the APNs pipeline the extension hangs off. So it is a fine way to check
the FALLBACK rendering, and worthless as a check that decryption works. Do not
read a notification appearing after a `simctl push` as evidence of anything more
than that.

**The simulator can hold a real APNs token.** On an Apple Silicon Mac with iOS
16+, a simulator registers with Apple like a device does. Confirmed on the same
run: a seeded pairing produced the permission prompt, a granted prompt produced a
real 192-hex-character sandbox token, and the app registered it with the relay.
That is what makes a genuine end-to-end possible with no physical phone — relay →
APNs sandbox → simulator → extension → notification — and it is the test to run
the moment the `.p8` is in place.

**The whole chain, re-verified on device against the deployed relay —
2026-08-25.** After the reply path and the entitlement landed, every link was
driven again on an iPhone 16 Pro with the relay running the shipping image:

- **Forward.** A real hook push produced, in the device's own log,
  `hasMutableContent: 1` → iOS launching `com.cluas.moshpit.push` →
  `MoshpitPush: opened a attention push from mac-mini.lan`. The extension is
  what proves decryption: that line only exists if it ran and the key opened.
- **Reply — and then removed the same day.** For the record, because it says
  something about what a passing end-to-end does and does not settle: the real
  `moshpit-await.sh` long-polled `/v1/await` through the production ingress, a
  sealed `allow` went to `/v1/respond`, and `0d` landed in a real tmux pane,
  posted deliberately *after* a full 25-second poll round had expired to prove no
  proxy cuts the long poll. Relay log: `register` → `respond … queued` →
  `unregister`. It all worked, and then the user looked at the buttons it served
  and said they did not belong in the product — correctly, see "Why there are no
  buttons". A green chain is evidence the mechanism works, never evidence anyone
  wanted it.

**It works on a real device, in real use — 2026-08-25.** Relay deployed to k3s
with the shipping `.p8`, a device build installed on an iPhone 16 Pro, and a real
agent finishing a turn on `mac-mini.lan` produced this on the lock screen:

    ✓ claude
    mac-mini.lan · 0

That is decrypted content (`apply()`'s done branch), not the fallback — so the
extension was invoked on device, read the pairing secret out of the App Group,
and opened the envelope. It was not a self-test: the agent label was `claude`,
which means the feature was working in ordinary use rather than under a probe.

Two things fell out of that one screenshot that no test could have produced:

  * `postDeliveryFailure()` fired in the wild — "Not delivered / Your tap didn't
    reach the agent — open Moshpit and answer there." That confirms the
    lock-screen action buttons exist and are tappable on a PUSHED notification
    (previously unverified), and that the T3 gap below behaves as designed: the
    tap could not reach the pane, and the user was told instead of walking away
    believing they had approved.
  * `· 0` is tmux's default session name rendering as though it were a location.
    Technically correct, which is why nothing caught it; a numeric-only session
    name is now treated as no name.

**A self-test that times out on a lock screen is expected, not a defect.** The
proof returns through `willPresent`, which iOS calls only while Moshpit is
foreground. Lock the phone while a test is in flight and the notification arrives
perfectly and the sheet still reports failure. Its message now leads with that
rather than with "check your permissions", which would send someone to dismantle
a pairing that works.

**Provider auth, against the real APNs:** `scripts/verify-apns-auth.sh <key> <keyId>
<teamId>`. It pushes to a device token that cannot exist, so the PASS signal is a
specific failure — `BadDeviceToken` means Apple authenticated the provider token
first. Run with a key Apple never issued it answers `403 InvalidProviderToken`,
which is how you tell "our credentials are wrong" from "the network cannot reach
Cupertino". Both outcomes were observed on 2026-08-24.

**The last hop, verified 2026-08-24.** Not a simulation of it — the real thing,
end to end, on the iOS 26.2 simulator with the shipping `.p8`:

    scripts/moshpit-push.sh attention claude "Bash: rm -rf build/Release"
      → relay: notify device=b8e43bf9 cat=attention status=200
                unique=179abfb2-ce4a-7d2d-4c02-dc913f431210
      → APNs sandbox → simulator
      → MoshpitPush[68177] [com.cluas.moshpit:push] opened a attention push from m1-pro
      → lock screen:  claude
                      Bash: rm -rf build/Release — m1-pro

The app had NOT been launched since the simulator booted, which is the whole
point: the extension was started by the system, read the pairing secret out of
the App Group container, and opened an envelope the relay could not. A `done`
push rendered as `✓ claude / m1-pro` beside it, and repeated attention pushes for
the same pane collapsed into one card carrying a "2" badge rather than buzzing
twice — the collapse-id design working as described above.

**What is still unverified:** the lock-screen ACTION BUTTONS on a pushed
notification (Allow / Deny / Reply). The categories are registered at launch and
their identifiers are pinned by tests, and the buttons work on local
notifications, but `idb ui tap --duration` does not produce the force-press that
reveals them, so the pushed variant has not been driven end to end. Also
unverified: Time Sensitive delivery, which is still waiting on the entitlement
decision (the simulator reports `timeSensitiveSetting: NotSupported`, as expected
while it is absent). The physical-device gap is closed — see the real-device run
above.
