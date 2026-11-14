#!/bin/bash
# Runs the app's sign-in and connect code (ssh-live, see build.sh) against a
# private OpenSSH server: one this script starts as the current user on a
# free 127.0.0.1 port, with its own config, host key, authorized_keys, pid
# file and log in a temp directory, and kills afterwards. The machine's sshd
# and ~/.ssh are never touched.
#
#   scripts/verify/ssh-live/run.sh [<check> ...]     (default: all)
#
# Checks:
#   rsa    B129: RSA keys — ssh-keygen RSA 2048 and 3072 and the app's own
#          RSA-4096 — against OpenSSH's defaults (SHA-1 refused), a server
#          that takes only rsa-sha2-256, and one that takes only ssh-rsa.
#   socks  B130: the SOCKS dial against scripted fake proxies — each failure
#          ends the dial, a silent proxy at the 15 s budget (no sshd needed).
#   proxy  B131: signing in through a real SOCKS5 proxy (socks5.py) — an
#          authorized key signs in, an unauthorized one is refused as it is
#          without the proxy.
#   keys   B125/B133: keys stored exactly as pasted — ssh-keygen -m PEM RSA,
#          -m PKCS8 ECDSA, an OpenSSH key with Windows line endings, one with
#          every line indented, one behind a passphrase — sign in through the
#          app's connect, and the keychain then holds the converted key.
#   exec   B136: commands killed by a signal — before, their partial
#          output came back as success; after, a failure — and the
#          in-process server's channel closed with no exit status at all.
#   locked B211: the saved key behind a keychain the locked device keeps
#          shut — the connect fails as the lock (retried on the unlock),
#          never as a lost credential, before the server sees a connection;
#          the same key signs in once it is open.
#   side   W14b's baseline: what a feature beside a Rust session did before
#          it borrowed the session's connection — a sign-in of its own, then
#          a probe, a preview, a stream's first MiB, a folder, an upload —
#          against an sshd whose sessions and SFTP live in a scratch home;
#          one sign-in per round. (The borrowed side: crates/session's
#          tests/live_w14b.rs, on the same kind of sshd.) Not in the default
#          set: it is a measurement.
#   prompt B132: the new-host card answered after 12 s — Citadel's own
#          sign-in gives up at 10 s and the server then signs in a connection
#          nobody holds (before); the app's connect asks with nothing open
#          and signs in (after). Plus the same against the in-process server.
#
# SSHD=<path> picks the sshd (default /usr/sbin/sshd); SSH_LIVE=<binary> the
# harness (default target/ssh-live/ssh-live, built by build.sh). Prints a
# PASS or FAIL line per check, and exits non-zero on any FAIL.
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../../.." && pwd)"
live="${SSH_LIVE:-$repo/target/ssh-live/ssh-live}"
sshd="${SSHD:-/usr/sbin/sshd}"
checks=("$@")
[ ${#checks[@]} -eq 0 ] && checks=(rsa keys socks proxy prompt exec locked)

[ -x "$live" ] || { echo "no $live: run scripts/verify/ssh-live/build.sh first" >&2; exit 2; }
dir="$(mktemp -d /tmp/ssh-live.XXXXXX)"
pid=""
failures=0
cleanup() {
    if [ -n "$pid" ]; then kill "$pid" 2>/dev/null; wait "$pid" 2>/dev/null; fi
    rm -rf "$dir"
}
trap cleanup EXIT

user="$(id -un)"
port="$(python3 -I -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')"
ssh-keygen -q -t ed25519 -N '' -C host -f "$dir/host_ed25519"

pass() { echo "PASS $*"; }
fail() { echo "FAIL $*"; failures=$((failures + 1)); }

# Starts the private sshd with the extra config lines given.
start_sshd() {
    stop_sshd
    : > "$dir/sshd.log"
    cat > "$dir/sshd_config" <<EOF
Port $port
ListenAddress 127.0.0.1
HostKey $dir/host_ed25519
PidFile $dir/sshd.pid
AuthorizedKeysFile $dir/authorized_keys
StrictModes no
UsePAM no
PasswordAuthentication no
KbdInteractiveAuthentication no
LogLevel VERBOSE
$1
EOF
    "$sshd" -D -f "$dir/sshd_config" -E "$dir/sshd.log" < /dev/null &
    pid=$!
    for _ in $(seq 100); do
        if nc -z 127.0.0.1 "$port" 2>/dev/null; then return; fi
        sleep 0.05
    done
    echo "sshd didn't start:" >&2
    cat "$dir/sshd.log" >&2
    exit 2
}

stop_sshd() {
    if [ -n "$pid" ]; then kill "$pid" 2>/dev/null; wait "$pid" 2>/dev/null; pid=""; fi
}

fingerprint() { ssh-keygen -l -E sha256 -f "$1" | awk '{print $2}'; }

check_rsa() {
    ssh-keygen -q -t rsa -b 2048 -N '' -C rsa2048 -f "$dir/rsa2048"
    ssh-keygen -q -t rsa -b 3072 -N '' -C rsa3072 -f "$dir/rsa3072"
    "$live" genkey "$dir/app4096" > "$dir/app4096.info" || { fail "rsa: the app could not generate a key"; return; }
    cat "$dir/rsa2048.pub" "$dir/rsa3072.pub" "$dir/app4096.pub" > "$dir/authorized_keys"
    keys=(rsa2048 rsa3072 app4096)

    # OpenSSH's defaults: no SHA-1 signatures.
    start_sshd ""
    for key in "${keys[@]}"; do
        out="$("$live" signin 127.0.0.1 "$port" "$user" "$dir/$key" citadel 2>&1)"
        if [[ "$out" == refused* ]]; then
            pass "rsa: $key, Citadel's SHA-1 sign-in is refused (before): $out"
        else
            fail "rsa: $key, Citadel's SHA-1 sign-in: $out"
        fi
        out="$("$live" signin 127.0.0.1 "$port" "$user" "$dir/$key" app 2>&1)"
        if [[ "$out" == "signed-in rsa-sha2-512 signed-in" ]]; then
            pass "rsa: $key signs in with rsa-sha2-512 (after)"
        else
            fail "rsa: $key, the app's sign-in: $out"
        fi
        fp="$(fingerprint "$dir/$key.pub")"
        if grep -q "Accepted publickey for $user .* RSA $fp" "$dir/sshd.log"; then
            pass "rsa: sshd accepted $key ($fp)"
        else
            fail "rsa: sshd never accepted $key ($fp)"
        fi
    done
    if grep -q "signature algorithm ssh-rsa not in PubkeyAcceptedAlgorithms" "$dir/sshd.log"; then
        pass "rsa: sshd says why SHA-1 was refused: $(grep -m1 -o 'signature algorithm ssh-rsa not in PubkeyAcceptedAlgorithms' "$dir/sshd.log")"
    else
        fail "rsa: sshd never refused ssh-rsa"
    fi

    # A server that takes only rsa-sha2-256, and one that takes only SHA-1.
    for only in rsa-sha2-256 ssh-rsa; do
        start_sshd "PubkeyAcceptedAlgorithms $only"
        out="$("$live" signin 127.0.0.1 "$port" "$user" "$dir/app4096" app 2>&1)"
        if [[ "$out" == "signed-in $only signed-in" ]]; then
            pass "rsa: a server that takes only $only: the app falls back to it"
        else
            fail "rsa: a server that takes only $only: $out"
        fi
    done
    stop_sshd
}

check_socks() {
    for script in refused wants-password no-acceptable-methods not-socks hang-up unreachable; do
        out="$("$live" socks-fake "$script" 2>&1)"
        seconds="$(echo "$out" | sed -n 's/.*(\([0-9.]*\) s)$/\1/p')"
        if [[ "$out" == failed* ]] && awk -v s="$seconds" 'BEGIN { exit !(s < 3) }'; then
            pass "socks: $script ends the dial at once: $out"
        else
            fail "socks: $script: $out"
        fi
    done
    out="$("$live" socks-fake silent 2>&1)"
    seconds="$(echo "$out" | sed -n 's/.*(\([0-9.]*\) s)$/\1/p')"
    if [[ "$out" == failed* ]] && awk -v s="$seconds" 'BEGIN { exit !(s >= 14.5 && s < 17) }'; then
        pass "socks: a silent proxy ends the dial at the 15 s budget: $out"
    else
        fail "socks: silent: $out"
    fi
    out="$("$live" socks-fake success 2>&1)"
    if [[ "$out" == connected* ]]; then pass "socks: a proxy that connects hands the channel back: $out"; else fail "socks: success: $out"; fi
}

check_proxy() {
    ssh-keygen -q -t ed25519 -N '' -C allowed -f "$dir/allowed"
    ssh-keygen -q -t ed25519 -N '' -C stranger -f "$dir/stranger"
    cat "$dir/allowed.pub" > "$dir/authorized_keys"
    start_sshd ""
    python3 -I "$here/socks5.py" "$dir/socks.port" &
    socks_pid=$!
    for _ in $(seq 100); do [ -s "$dir/socks.port" ] && break; sleep 0.05; done
    proxy="$(cat "$dir/socks.port")"
    for key in allowed stranger; do
        for how in direct old app; do
            out="$("$live" socks-signin "$proxy" 127.0.0.1 "$port" "$user" "$dir/$key" "$how" 2>&1)"
            status=$?
            echo "  $key/$how (exit $status): $(echo "$out" | tail -1 | cut -c1-160)"
            eval "result_${key}_${how}=\"\$status \$out\""
        done
    done
    kill "$socks_pid" 2>/dev/null; wait "$socks_pid" 2>/dev/null
    stop_sshd
    if [[ "$result_allowed_app" == "0 signed-in signed-in" ]]; then
        pass "proxy: an authorized key signs in through the proxy"
    else
        fail "proxy: authorized key through the proxy: $result_allowed_app"
    fi
    if [[ "$result_stranger_app" == *allAuthenticationOptionsFailed* && "$result_stranger_direct" == *allAuthenticationOptionsFailed* ]]; then
        pass "proxy: an unauthorized key is refused through the proxy as it is directly"
    else
        fail "proxy: unauthorized key: through the proxy '$result_stranger_app', directly '$result_stranger_direct'"
    fi
}

check_prompt() {
    ssh-keygen -q -t ed25519 -N '' -C allowed -f "$dir/allowed"
    cat "$dir/allowed.pub" > "$dir/authorized_keys"
    start_sshd ""
    out="$("$live" citadel-slow-trust "$port" "$user" "$dir/allowed" 12 2>&1)"
    echo "$out" | sed 's/^/  before: /'
    accepted="$(grep -c "Accepted publickey for $user" "$dir/sshd.log")"
    if [[ "$out" == *"error at 10."*"imeout"* ]] && [ "$accepted" -eq 1 ]; then
        pass "prompt: before, the sign-in gave up at 10 s and the server then signed in a connection nobody held"
    else
        fail "prompt: before: $out (sshd accepted $accepted)"
    fi
    : > "$dir/sshd.log"
    out="$("$live" sshd-prompt "$port" "$user" "$dir/allowed" 12 2>&1)"
    echo "$out" | sed 's/^/  after: /'
    preauth="$(grep -c "Connection closed by 127.0.0.1 port [0-9]* \[preauth\]\|Connection closed by authenticating user\|Disconnected from" "$dir/sshd.log")"
    if [[ "$out" == *"signed-in at 1"*": signed-in" ]]; then
        pass "prompt: after, the card up for 12 s and the connect still signs in ($preauth closed before the question)"
    else
        fail "prompt: after: $out"
    fi
    stop_sshd
    out="$("$live" prompts 2>&1)"
    echo "$out" | sed 's/^/  in-process: /'
    if [[ "$out" == *"connections open while asking: 0; trusted: true"* && "$out" == *"closed afterwards: true"* \
          && "$out" == *"trusted: [:]; accepted 1; closed: true"* && "$out" == *"changed key: asked true; now trusted: true"* ]]; then
        pass "prompt: the in-process server's cases"
    else
        fail "prompt: in-process cases"
    fi
}

check_keys() {
    ssh-keygen -q -t rsa -b 2048 -m PEM -N '' -C pem -f "$dir/pem"
    ssh-keygen -q -t ecdsa -b 256 -m PKCS8 -N '' -C pkcs8 -f "$dir/pkcs8"
    ssh-keygen -q -t ed25519 -N '' -C crlf -f "$dir/crlf"
    ssh-keygen -q -t ed25519 -N '' -C indented -f "$dir/indented"
    ssh-keygen -q -t ed25519 -N 'correct horse' -C locked -f "$dir/locked"
    cat "$dir"/pem.pub "$dir"/pkcs8.pub "$dir"/crlf.pub "$dir"/indented.pub "$dir"/locked.pub > "$dir/authorized_keys"
    perl -pi -e 's/\n/\r\n/' "$dir/crlf"
    perl -pi -e 's/^/    /' "$dir/indented"
    start_sshd ""
    for key in pem pkcs8 crlf indented locked; do
        if [ "$key" = locked ]; then
            out="$("$live" stored-key "$port" "$user" "$dir/$key" 'correct horse' 2>&1)"
        else
            out="$("$live" stored-key "$port" "$user" "$dir/$key" 2>&1)"
        fi
        echo "$out" | sed "s/^/  $key: /"
        asked=1
        [ "$key" = locked ] || asked=0
        if [[ "$out" == *"reads it: false"* && "$out" == *"signed-in; passphrase asked ${asked}×; keychain now converted, read as stored: true, same key: true"* ]]; then
            pass "keys: $key, which the connect path couldn't read as stored, signs in and is kept converted"
        else
            fail "keys: $key"
        fi
    done
    stop_sshd
}

check_locked() {
    ssh-keygen -q -t ed25519 -N '' -C client -f "$dir/client"
    cat "$dir/client.pub" > "$dir/authorized_keys"
    start_sshd ""
    sleep 0.3   # start_sshd's own probe is a connection of its own
    before="$(grep -c "Connection from" "$dir/sshd.log")"
    out="$("$live" locked-keychain "$port" "$user" "$dir/client" locked 2>&1)"
    echo "  locked: $out"
    seen=$(($(grep -c "Connection from" "$dir/sshd.log") - before))
    if [[ "$out" == "failed: credentialLocked"* && "$seen" -eq 0 ]]; then
        pass "locked: the lock, retried on the unlock, and nothing reached the server"
    else
        fail "locked: $out ($seen connections reached the server)"
    fi
    out="$("$live" locked-keychain "$port" "$user" "$dir/client" unlocked 2>&1)"
    echo "  unlocked: $out"
    if [[ "$out" == "connected: signed-in" ]]; then
        pass "locked: unlocked, the same key signs in"
    else
        fail "locked: unlocked: $out"
    fi
    stop_sshd
}

check_exec() {
    ssh-keygen -q -t ed25519 -N '' -C allowed -f "$dir/allowed"
    cat "$dir/allowed.pub" > "$dir/authorized_keys"
    start_sshd ""
    out="$("$live" exec-ends "$port" "$user" "$dir/allowed" 2>&1)"
    stop_sshd
    echo "$out" | sed 's/^/  /'
    if [[ "$out" == *'before printf partial; kill -KILL $$: ok "partial"'* ]]; then
        pass "exec: before, a command killed by SIGKILL answered with its partial output"
    else
        fail "exec: before: the killed command"
    fi
    if [[ "$out" == *'after echo ok: ok "ok\n"'* && "$out" == *"after printf partial; exit 3: error command exited with status 3"* \
          && "$out" == *"after printf partial; kill -KILL \$\$: error command killed by SIGKILL"* \
          && "$out" == *"after printf partial; kill -TERM \$\$: error command killed by SIGTERM"* ]]; then
        pass "exec: after, only exit status 0 answers; a killed command fails, saying how"
    else
        fail "exec: after"
    fi
    out="$("$live" loopback 2>&1)"
    echo "$out" | grep -E "right password:|the app's session:" | sed 's/^/  /' | cut -c1-260
    if [[ "$out" == *'drops -> ok "partial"'* ]]; then
        pass "exec: before, Citadel answered a channel closed with no exit status with its partial output (in-process server)"
    else
        fail "exec: in-process before: $out"
    fi
    if [[ "$out" == *"drops -> error command's channel closed without an exit status; killed -> error command killed by SIGKILL"* ]]; then
        pass "exec: after, the app's session fails both (in-process server)"
    else
        fail "exec: in-process after"
    fi
}

check_side() {
    ssh-keygen -q -t ed25519 -N '' -C side -f "$dir/side"
    cat "$dir/side.pub" > "$dir/authorized_keys"
    home="$dir/home"
    mkdir -p "$home/notes"
    head -c 1048576 /dev/urandom > "$home/chart.png"
    head -c 4194304 /dev/urandom > "$home/clip.mp4"
    printf '# plan\n' > "$home/notes/plan.md"
    start_sshd "SetEnv HOME=$home ZDOTDIR=$home
Subsystem sftp internal-sftp -d $home"
    rounds=12
    out="$("$live" side-features "$port" "$user" "$dir/side" "$home" "$rounds" 2>&1)"
    status=$?
    stop_sshd
    echo "$out" | sed 's/^/  /'
    accepted="$(grep -c "Accepted publickey for $user" "$dir/sshd.log")"
    if [ "$status" -eq 0 ] && [ "$accepted" -eq "$rounds" ]; then
        pass "side: $rounds rounds, a sign-in each ($accepted in sshd's log)"
    else
        fail "side: exit $status, $accepted sign-ins"
    fi
}

for check in "${checks[@]}"; do
    case "$check" in
        exec) check_exec ;;
        keys) check_keys ;;
        locked) check_locked ;;
        prompt) check_prompt ;;
        rsa) check_rsa ;;
        side) check_side ;;
        socks) check_socks ;;
        proxy) check_proxy ;;
        *) echo "unknown check $check" >&2; exit 2 ;;
    esac
done
echo "$failures failed"
[ "$failures" -eq 0 ]
