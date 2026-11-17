#!/bin/sh
# The host the live tests run against, on an Ubuntu runner (or a throwaway
# container that plays one): OpenSSH's sshd (the tests start private ones
# on the loopback, as the current user), mosh-server, zsh and python3 for
# the herdr and pty rigs, and herdr at the path the rig knows it by. Run
# with sudo available (root in a container).
set -eu
sudo=""
[ "$(id -u)" -eq 0 ] || sudo=sudo
export DEBIAN_FRONTEND=noninteractive
$sudo apt-get update -qq
$sudo apt-get install -y -qq --no-install-recommends \
    openssh-server openssh-client mosh zsh python3 \
    build-essential pkg-config libevent-dev libncurses-dev bison \
    curl ca-certificates git iptables iproute2 procps psmisc locales lsof sudo >/dev/null
# The tmux under test is placed by use-tmux.sh; none other may answer.
$sudo apt-get remove -y -qq tmux >/dev/null 2>&1 || true
# Remove distro golang-go if present; push-relay requires Go 1.24+.
$sudo apt-get remove -y -qq golang-go >/dev/null 2>&1 || true
# Go 1.24, pinned (push-relay in live_hostcare_borrowed requires go 1.24).
go_version=1.24.4
case "$(uname -m)" in
    x86_64) go_arch=amd64; go_digest=77e5da33bb72aeaef1ba4418b6fe511bc4d041873cbf82e5aa6318740df98717 ;;
    aarch64) go_arch=arm64; go_digest=d5501ee5aca0f258d5fe9bfaed401958445014495dc115f202d43d5210b45241 ;;
    *) go_arch="" ;;
esac
if [ -n "$go_arch" ] && { [ ! -x /usr/local/go/bin/go ] || ! /usr/local/go/bin/go version | grep -q "go$go_version "; }; then
    tmp=$(mktemp)
    curl -fsSL --retry 3 -o "$tmp" "https://go.dev/dl/go${go_version}.linux-${go_arch}.tar.gz"
    echo "$go_digest  $tmp" | sha256sum -c -
    $sudo rm -rf /usr/local/go
    $sudo tar -C /usr/local -xzf "$tmp"
    rm -f "$tmp"
fi
if [ -x /usr/local/go/bin/go ]; then
    $sudo ln -sf /usr/local/go/bin/go /usr/local/bin/go
    $sudo ln -sf /usr/local/go/bin/gofmt /usr/local/bin/gofmt
    go version
fi
$sudo mkdir -p /opt/e2e
$sudo chown "$(id -u):$(id -g)" /opt/e2e
# mosh-server wants a UTF-8 locale.
$sudo sed -i 's/^# *en_US.UTF-8/en_US.UTF-8/' /etc/locale.gen
$sudo locale-gen >/dev/null
# sshd checks its privilege separation directory even when it runs as the
# user it serves.
$sudo mkdir -p /run/sshd
$sudo chmod 0755 /run/sshd
# herdr, pinned (Apache-2.0; github.com/herdrdev/herdr), where the rig and
# the app's probed PATH look for it, as on a Mac with Homebrew.
herdr_version=v0.9.5
case "$(uname -m)" in
    x86_64) asset=herdr-linux-x86_64; digest=14d9e8bc8b4a9e0118d77c32266f76d461f09b40342eb23752960bd89f59929d ;;
    aarch64) asset=herdr-linux-aarch64; digest=17d7536f2da50859523680ef279dffcac8e4a235f4e46fe8da6c2ec44141897c ;;
    *) asset="" ;;
esac
if [ -n "$asset" ]; then
    tmp=$(mktemp)
    curl -fsSL --retry 3 -o "$tmp" "https://github.com/herdrdev/herdr/releases/download/$herdr_version/$asset"
    echo "$digest  $tmp" | sha256sum -c -
    $sudo install -D -m 0755 "$tmp" /opt/homebrew/bin/herdr
    rm -f "$tmp"
    /opt/homebrew/bin/herdr --version
else
    echo "::notice::herdr: no pinned release for $(uname -m); its live tests skip"
fi
