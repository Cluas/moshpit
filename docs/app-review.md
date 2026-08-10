# App Review — demo host and reviewer notes

Moshpit opens on an empty connection list. A reviewer has no server of their
own, so without a host to point it at they cannot evaluate the app at all —
which is a Guideline 2.1 rejection waiting to happen. This file holds the demo
host we hand them, and the text that goes in App Store Connect.

## The demo host

Deployed from [`deploy/review-demo/demo.yaml`](../deploy/review-demo/demo.yaml)
onto the Phoenix (`us-phoenix-1`) k3s node, behind a DNS name so the pod can move without reissuing the review notes — reviewers are in California and
every other node in that cluster is in Asia.

| | |
|---|---|
| Host | `demo.moshpit.cluas.eu.org` (A → `129.146.175.82`, unproxied, TTL 300) |
| Port | `2222` |
| User | `review` |
| Password | in `kubectl -n moshpit-demo get secret review-demo-creds -o jsonpath='{.data.password}' \| base64 -d` |
| Host key (ed25519) | `SHA256:o4EzO5AH+xU9G7ORoVtJbK/ME7iDJtRxRe5WR30gnxk` |

The host key lives in a Secret and is mounted into the pod, so it survives
restarts. That matters: the app warns loudly when a host key changes, and a pod
that minted a new key on every boot greeted the reviewer with a possible-
interception banner. Publishing the fingerprint also turns the check into a
feature demo — they can verify it matches.

### Staged state

The pod boots with two multiplexers running, both showing the same work:

- **herdr** — `payments-api` (an agent blocked on a permission prompt) and
  `dashboard` (an agent working). This is what lights up the AGENTS section.
- **tmux** — `payments-api` (3 windows: claude / server / tests) and
  `dashboard` (2 windows: build / shell).

The pane holding the "agent" replays a real Claude Code turn stopped on a
permission prompt (`deploy/review-demo/demo.yaml` → `claude-turn.sh`). A live
`claude` would need a real API key and would render a real account's state, so
the pane content is staged; everything outside the pane — the tree, the agent
rows, the state dots, the breadcrumb — is the app reading the live server.

### Safety

The password is handed to a third party and should be assumed public. The pod
is built so that whoever has it gets a sandbox and nothing else:

- sshd refuses TCP, stream-local, agent and X11 forwarding, and tunnels — so
  the box cannot be used to reach the cluster network or the tailnet. Verified
  by pushing traffic through an `-L` tunnel: refused.
- A NetworkPolicy drops egress to every private range. Verified from inside
  the pod: the cluster network and the Kubernetes API are both unreachable.
- No service-account token, no host mounts, no privileged bits, no persistence.
  `kubectl -n moshpit-demo rollout restart deploy/review-demo` returns it to a
  known state.

Rotate the password (and re-run the rollout) after the review is done:

```sh
kubectl -n moshpit-demo delete secret review-demo-creds
# ...recreate with a new password, then:
kubectl -n moshpit-demo rollout restart deploy/review-demo
```

## Text for App Store Connect → App Review Information

> **Demo account is not an account in the app.** Moshpit has no accounts, no
> server of ours, and no sign-in. It is an SSH/Mosh terminal: it connects to
> *your* machines. So that you can evaluate it, we run a throwaway Linux host
> for this review.
>
> **Add the connection** (tap **+** on the home screen):
>
> - Name: `Demo`
> - Host: `demo.moshpit.cluas.eu.org`
> - Port: `2222`
> - Username: `review`
> - Authentication: **Password** — `<password>`
> - Under **ADVANCED → Multiplexer**, choose **herdr**
>
> Tap **Save**, then tap the card to connect. The first connection shows the
> host's fingerprint for you to check; it should read
> `SHA256:o4EzO5AH+xU9G7ORoVtJbK/ME7iDJtRxRe5WR30gnxk`. Tap **Trust**.
>
> **What you should see**
>
> 1. The card expands into two sections. **AGENTS** lists two coding agents on
>    that host — one marked `NEEDS YOU` (amber), one working (teal).
> 2. Tap the amber agent's **OPEN** button. You land in its terminal pane, on a
>    permission prompt from Claude Code.
> 3. Type `1` and press return to answer it — this is the app's purpose: the
>    agent was waiting on a human, and you answered from a phone.
> 4. Back on the home screen, the **WORKSPACES** tree lists the host's sessions
>    and windows. Tapping any row opens that pane full screen.
>
> **Live Activity / Dynamic Island.** When an agent needs a human, Moshpit
> raises a Live Activity with Allow / Deny / Reply buttons. On the demo host the
> agent is already in that state when you connect, so the activity appears
> shortly after connecting; lock the device to see it on the Lock Screen.
>
> Everything is a direct connection from the device to that host. There is no
> Moshpit server, no relay, and no account to create.

## Release plan

First submission targets **every storefront except mainland China**. China
requires an ICP filing, which requires a registered domain and a filing that
takes weeks; the other 175 storefronts do not, so there is no reason to hold
the launch for it. China gets added later as an additional territory once
`moshpit.cn` is registered and filed.

### Testing it yourself

Test from plain cellular, not from a machine running a proxy. Clash-style TUN
proxies hijack `*.cluas.eu.org` into their fake-IP range (198.18.x) and answer
the TCP connect themselves, so SSH to the hostname fails locally while working
fine for everyone else. The IP path is unaffected.
