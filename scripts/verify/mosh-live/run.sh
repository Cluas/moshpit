#!/bin/bash
# Runs mosh-live (build.sh) against a real mosh-server, started through a
# private OpenSSH server: one this script starts as the current user on a
# free 127.0.0.1 port, with its own config, host key, client key,
# authorized_keys, pid file and log in a temp directory, and kills
# afterwards — with every mosh-server the checks started. The machine's sshd
# and ~/.ssh are never touched.
#
#   scripts/verify/mosh-live/run.sh [<check> ...]     (default: all)
#
# Checks:
#   reach       B171: SSH to `localhost` (sshd listens on 127.0.0.1 only, so
#               mosh-server -s binds 127.0.0.1) — mosh's UDP to the host name
#               (what the app did) and to the address SSH reached (the fix).
#   blackhole   B165: a session that has talked to its server goes silent
#               until the transport reports the return path dead; the rule
#               decides about the server; the path heals.
#   disconnect  B164: the user closes a session that has talked to its
#               server; the rule decides; are the server and its shell gone?
#   herdr       B208 (not in the default set): mosh+herdr's raw-attach
#               renderer on a private herdr (`--session moshpit-live-<pid>`,
#               its HOME in the temp directory, stopped afterwards) with two
#               panes that record what is typed into them; pane switches as
#               the app makes them, a probe line behind each, and where it
#               went (see herdr.swift). HERDR=<path> picks the herdr,
#               HERDR_TRIALS the tries per policy (12), HERDR_POLICIES the
#               policies ("landed poll pairrand gone stay left").
#   herdr-life  round 27 (not in the default set): the same private herdr;
#               what becomes of the renderer when another client takes its
#               pane over, its pane closes, herdr restarts, a switch names a
#               terminal that is gone — where a line typed then goes, and
#               whether the app's way back works — and what the connect-time
#               cleanup kills (see herdr-life.swift). HERDR_TRIALS the tries
#               per scenario (5), HERDR_SCENARIOS the scenarios
#               ("takeover close restart missing cleanup"; also "cover" and,
#               round 28, "follow": a second mosh session with a loop of its
#               own on the same private herdr — another device — holds a pane
#               this one follows the focus onto).
#
# The login shell's HOME is a fresh directory in the temp directory (sshd's
# SetEnv), so the user's rc files never run and a private herdr session
# resolves there.
#
# MOSH_LIVE=<binary> picks the build (default target/mosh-live/mosh-live);
# SSHD=<path> the sshd (default /usr/sbin/sshd). Prints what each check saw.
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../../.." && pwd)"
live="${MOSH_LIVE:-$repo/target/mosh-live/mosh-live}"
sshd="${SSHD:-/usr/sbin/sshd}"
checks=("$@")
[ ${#checks[@]} -eq 0 ] && checks=(reach blackhole disconnect)

[ -x "$live" ] || { echo "no $live: run scripts/verify/mosh-live/build.sh first" >&2; exit 2; }
command -v mosh-server > /dev/null || [ -x /opt/homebrew/bin/mosh-server ] \
    || { echo "no mosh-server on this machine" >&2; exit 2; }
dir="$(mktemp -d /tmp/mosh-live.XXXXXX)"
pid=""
herdr_pid=""
cleanup() {
    # Every mosh-server a check started (they print `PID <n>`), the private
    # herdr, then sshd.
    for server in $(cat "$dir"/*.out 2>/dev/null | sed -n 's/^PID \([0-9]*\)$/\1/p'); do
        kill "$server" 2>/dev/null
    done
    if [ -n "$herdr_pid" ]; then kill "$herdr_pid" 2>/dev/null; wait "$herdr_pid" 2>/dev/null; fi
    if [ -n "$pid" ]; then kill "$pid" 2>/dev/null; wait "$pid" 2>/dev/null; fi
    rm -rf "$dir"
}
trap cleanup EXIT

user="$(id -un)"
port="$(python3 -I -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')"
ssh-keygen -q -t ed25519 -N '' -C host -f "$dir/host_ed25519"
ssh-keygen -q -t ed25519 -N '' -C client -f "$dir/client"
cp "$dir/client.pub" "$dir/authorized_keys"
mkdir -p "$dir/home"
printf "PROMPT='%%%% '\nunsetopt BEEP\n" > "$dir/home/.zshrc"
# MOSH_SERVER_NETWORK_TMOUT: a server a check failed to stop still goes away
# on its own a minute after its client went quiet.
cat > "$dir/sshd_config" <<EOF
Port $port
ListenAddress 127.0.0.1
HostKey $dir/host_ed25519
PidFile $dir/sshd.pid
AuthorizedKeysFile $dir/authorized_keys
StrictModes no
UsePAM no
UseDNS no
PasswordAuthentication no
KbdInteractiveAuthentication no
SetEnv MOSH_SERVER_NETWORK_TMOUT=60 HOME=$dir/home ZDOTDIR=$dir/home
LogLevel VERBOSE
EOF
"$sshd" -D -f "$dir/sshd_config" -E "$dir/sshd.log" < /dev/null &
pid=$!
for _ in $(seq 100); do
    if nc -z 127.0.0.1 "$port" 2>/dev/null; then break; fi
    sleep 0.05
done
nc -z 127.0.0.1 "$port" 2>/dev/null || { echo "sshd didn't start:" >&2; cat "$dir/sshd.log" >&2; exit 2; }

run() {
    local name="$1"
    shift
    "$live" "$@" "$port" "$user" "$dir/client" "$dir/known_hosts" > "$dir/$name.out" 2>&1
}

for check in "${checks[@]}"; do
    case "$check" in
    reach)
        for target in name peer; do
            echo "== reach: mosh's UDP to the $target"
            "$live" reach localhost "$port" "$user" "$dir/client" "$dir/known_hosts" "$target" \
                > "$dir/reach-$target.out" 2>&1
            sed 's/^/   /' "$dir/reach-$target.out"
        done
        ;;
    blackhole)
        echo "== blackhole: a contacted session goes silent, then heals"
        run blackhole blackhole 127.0.0.1
        sed 's/^/   /' "$dir/blackhole.out"
        ;;
    disconnect)
        echo "== disconnect: the user closes a contacted session"
        run disconnect disconnect 127.0.0.1
        sed 's/^/   /' "$dir/disconnect.out"
        ;;
    herdr|herdr-life)
        echo "== herdr: pane switches under mosh's raw-attach renderer, and where a reply goes"
        herdr_bin="${HERDR:-$(command -v herdr || echo /opt/homebrew/bin/herdr)}"
        if [ ! -x "$herdr_bin" ]; then echo "   no herdr on this machine"; continue; fi
        session="moshpit-live-$$"
        # Every herdr command names the private session: a temporary HOME
        # alone still finds the user's default server.
        H() { env -i HOME="$dir/home" ZDOTDIR="$dir/home" SHELL=/bin/zsh TERM=xterm-256color \
                  LANG=en_US.UTF-8 PATH=/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin \
                  "$herdr_bin" --session "$session" "$@"; }
        case "$(H status 2>&1)" in
        *"$dir/home/"*) ;;
        *) echo "the herdr session isn't private" >&2; exit 2 ;;
        esac
        env -i HOME="$dir/home" ZDOTDIR="$dir/home" SHELL=/bin/zsh TERM=xterm-256color LANG=en_US.UTF-8 \
            PATH=/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin \
            "$herdr_bin" --session "$session" server > "$dir/herdr-server.log" 2>&1 < /dev/null &
        herdr_pid=$!
        for _ in $(seq 100); do H api snapshot > /dev/null 2>&1 && break; sleep 0.1; done
        H workspace create --focus --label live > /dev/null
        p1="$(H api snapshot | python3 -I -c 'import json,sys; print(json.load(sys.stdin)["result"]["snapshot"]["panes"][0]["pane_id"])')"
        H pane split "$p1" --direction right > /dev/null
        panes="$(H api snapshot | python3 -I -c 'import json,sys
for p in json.load(sys.stdin)["result"]["snapshot"]["panes"]: print(p["pane_id"], p["terminal_id"])')"
        p2="$(echo "$panes" | awk -v p1="$p1" '$1 != p1 { print $1; exit }')"
        t1="$(echo "$panes" | awk -v p="$p1" '$1 == p { print $2 }')"
        t2="$(echo "$panes" | awk -v p="$p2" '$1 == p { print $2 }')"
        H pane run "$p1" "python3 -I '$here/herdr-recorder.py' A '$dir/paneA.log'" > /dev/null
        H pane run "$p2" "python3 -I '$here/herdr-recorder.py' B '$dir/paneB.log'" > /dev/null
        sleep 1
        if [ "$check" = herdr-life ]; then
            # shellcheck disable=SC2086
            HERDR_EVICT="python3 -I '$here/herdr-evict.py'" MOSH_LIVE_HERE="$here" "$live" herdr-life 127.0.0.1 \
                "$port" "$user" "$dir/client" "$dir/known_hosts" "$herdr_bin --session $session" \
                "$p1" "$t1" "$p2" "$t2" "$dir/paneA.log" "$dir/paneB.log" "${HERDR_TRIALS:-5}" \
                ${HERDR_SCENARIOS:-takeover close restart missing cleanup} > "$dir/herdr.out" 2>&1
        else
        # shellcheck disable=SC2086
        HERDR_EVICT="python3 -I '$here/herdr-evict.py'" "$live" herdr-retarget 127.0.0.1 "$port" "$user" \
            "$dir/client" "$dir/known_hosts" "$herdr_bin --session $session" "$t1" "$t2" \
            "$dir/paneA.log" "$dir/paneB.log" "${HERDR_TRIALS:-12}" ${HERDR_POLICIES:-landed poll pairrand gone stay left} \
            > "$dir/herdr.out" 2>&1
        fi
        grep -v '^loop: ' "$dir/herdr.out" | sed 's/^/   /'
        H server stop > /dev/null 2>&1
        ;;
    *)
        echo "unknown check $check" >&2
        exit 2
        ;;
    esac
done
