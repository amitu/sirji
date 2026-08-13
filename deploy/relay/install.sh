#!/usr/bin/env bash
#
# Install a private iroh relay. Run as root on the server.
#
#   RELAY_HOSTNAME=relay.example.com CONTACT=you@example.com ./install.sh
#
# Idempotent: safe to re-run after editing the config.

set -euo pipefail

RELAY_HOSTNAME="${RELAY_HOSTNAME:?set RELAY_HOSTNAME to the DNS name pointing at this box}"
CONTACT="${CONTACT:?set CONTACT to an email for Let's Encrypt expiry notices}"
IROH_RELAY_VERSION="${IROH_RELAY_VERSION:-1.0.3}"

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo "==> checking DNS"
resolved="$(getent hosts "$RELAY_HOSTNAME" | awk '{print $1}' | head -1 || true)"
if [ -z "$resolved" ]; then
    echo "!! $RELAY_HOSTNAME does not resolve."
    echo "   Let's Encrypt validates over HTTP on port 80, so DNS must point here"
    echo "   BEFORE this runs, or certificate issuance fails and the relay will not"
    echo "   serve HTTPS. Add the A/AAAA record, wait for it, then re-run."
    exit 1
fi
echo "    $RELAY_HOSTNAME -> $resolved"

echo "==> installing build prerequisites"
apt-get update -qq
apt-get install -y -qq build-essential pkg-config libssl-dev curl

if ! command -v cargo >/dev/null 2>&1; then
    echo "==> installing rust"
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path
    # shellcheck disable=SC1091
    . "$HOME/.cargo/env"
fi

echo "==> building iroh-relay $IROH_RELAY_VERSION"
# The server lives behind a feature flag; without it the crate builds no binary.
cargo install iroh-relay \
    --version "$IROH_RELAY_VERSION" \
    --features server \
    --locked \
    --root /usr/local
/usr/local/bin/iroh-relay --help >/dev/null && echo "    installed"

echo "==> creating the service user"
id -u iroh-relay >/dev/null 2>&1 || useradd --system --no-create-home --shell /usr/sbin/nologin iroh-relay

echo "==> installing config"
install -d -m 0755 /etc/iroh-relay
if [ -f /etc/iroh-relay/relay.toml ]; then
    echo "    /etc/iroh-relay/relay.toml exists, leaving it alone"
else
    sed -e "s|relay.example.com|$RELAY_HOSTNAME|" \
        -e "s|you@example.com|$CONTACT|" \
        "$here/relay.toml" > /etc/iroh-relay/relay.toml
    chmod 0644 /etc/iroh-relay/relay.toml
    echo "    wrote /etc/iroh-relay/relay.toml"
fi

echo "==> preparing the state directory"
# systemd creates this too, but the relay may start before anyone notices it is
# missing, and a certificate that cannot be cached fails slowly rather than loudly.
install -d -o iroh-relay -g iroh-relay -m 0700 /var/lib/iroh-relay

echo "==> installing the unit"
install -m 0644 "$here/iroh-relay.service" /etc/systemd/system/iroh-relay.service
systemctl daemon-reload
systemctl enable iroh-relay

echo "==> opening the firewall"
if command -v ufw >/dev/null 2>&1 && ufw status | grep -q "Status: active"; then
    ufw allow 80/tcp   comment "iroh-relay: ACME + captive portal probe"
    ufw allow 443/tcp  comment "iroh-relay: relay websocket"
    ufw allow 7842/udp comment "iroh-relay: QUIC address discovery"
else
    echo "    ufw not active; ensure 80/tcp, 443/tcp and 7842/udp reach this host"
fi

echo "==> starting"
systemctl restart iroh-relay
sleep 3
systemctl is-active --quiet iroh-relay && echo "    running" || {
    echo "!! not running:"
    journalctl -u iroh-relay -n 40 --no-pager
    exit 1
}

cat <<EOF

Relay installed at https://$RELAY_HOSTNAME

Certificate issuance happens on first start and takes a few seconds. Check it
worked — this must print the relay's page, not a certificate error:

    curl -sS https://$RELAY_HOSTNAME/ | head -3

Then point a sirji at it:

    export SIRJI_RELAY=https://$RELAY_HOSTNAME
    sirji daemon

and confirm it says 'connected' rather than 'DOWN':

    sirji status

Before leaving this up: the config's access control defaults to 'everyone', so
anyone who finds this host can relay through it at your expense. See the 'access'
section of /etc/iroh-relay/relay.toml.
EOF
