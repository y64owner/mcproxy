#!/bin/sh
set -eu

cd "$(dirname "$0")"

SUDO=""
[ "$(id -u)" -ne 0 ] && SUDO="sudo"

if ! command -v cargo >/dev/null 2>&1; then
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
fi
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"

cargo build --release

$SUDO install -Dm755 target/release/mcproxy /opt/mcproxy/mcproxy
$SUDO install -Dm644 mcproxy.service /etc/systemd/system/mcproxy.service
$SUDO systemctl daemon-reload
$SUDO systemctl enable mcproxy
$SUDO systemctl restart mcproxy
$SUDO systemctl --no-pager --full status mcproxy || true
