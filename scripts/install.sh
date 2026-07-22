#!/usr/bin/env bash
#
# rod one-line installer for Raspberry Pi OS (64-bit).
#
#   curl -sSL https://raw.githubusercontent.com/rezreal/rod/main/scripts/install.sh | sudo bash
#
# Downloads the latest release binary, installs it as a systemd service that
# starts on boot, and drops a default config. Safe to re-run to update the
# binary (your config is left untouched). Set ROD_REPO to override the
# GitHub repo it pulls from.
set -euo pipefail

REPO="${ROD_REPO:-rezreal/rod}"
BIN_DIR=/usr/local/bin
BIN="$BIN_DIR/rod"
CFG_DIR=/etc/rod
CFG="$CFG_DIR/config.toml"
UNIT=/etc/systemd/system/rod.service
# Set ROD_HOSTNAME="" to keep the current hostname.
HOSTNAME_WANT="${ROD_HOSTNAME:-rod}"

say()  { printf '\033[36m▶ %s\033[0m\n' "$*"; }
ok()   { printf '\033[32m✓ %s\033[0m\n' "$*"; }
die()  { printf '\033[31m✗ %s\033[0m\n' "$*" >&2; exit 1; }

[ "$(id -u)" -eq 0 ] || die "Please run with sudo:  curl -sSL …/install.sh | sudo bash"

# ── 1. Pick the right binary for this Pi ──────────────────────────────────────
case "$(uname -m)" in
  aarch64|arm64) asset="rod-aarch64-unknown-linux-gnu" ;;
  *) die "Unsupported architecture '$(uname -m)'. rod needs 64-bit Raspberry Pi OS (Pi 3/4/5)." ;;
esac

# ── 2. Which user runs the service ────────────────────────────────────────────
# Prefer the human installing it; fall back to the classic 'pi' user, then root.
run_user="${SUDO_USER:-}"
if [ -z "$run_user" ] || [ "$run_user" = root ]; then
  if id pi >/dev/null 2>&1; then run_user=pi; else run_user=root; fi
fi
say "Service will run as: $run_user"

# ── 3. Download + verify the latest release ───────────────────────────────────
say "Looking up the latest rod release…"
api="https://api.github.com/repos/$REPO/releases/latest"
dl=$(curl -fsSL "$api" | grep -o "https://[^\"]*$asset" | head -1) \
  || die "Couldn't reach GitHub. Is the Pi online?"
[ -n "$dl" ] || die "No '$asset' in the latest release of $REPO."

tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
say "Downloading $asset…"
curl -fSL# "$dl" -o "$tmp/rod" || die "Download failed."
if curl -fsSL "$dl.sha256" -o "$tmp/sha" 2>/dev/null; then
  ( cd "$tmp" && echo "$(awk '{print $1}' sha)  rod" | sha256sum -c - ) \
    || die "Checksum mismatch — refusing to install a corrupt binary."
  ok "Checksum verified"
else
  printf '\033[33m⚠ no .sha256 published; skipping checksum\033[0m\n'
fi
install -m755 "$tmp/rod" "$BIN"
ok "Installed $BIN"

# ── 4. Permissions: Bluetooth (D-Bus) + serial (dialout) ──────────────────────
if [ "$run_user" != root ]; then
  usermod -aG bluetooth,dialout "$run_user" 2>/dev/null || true
fi

# ── 5. Default config (never overwrite an existing one) ───────────────────────
install -d -m755 "$CFG_DIR"
if [ ! -f "$CFG" ]; then
  curl -fsSL "https://raw.githubusercontent.com/$REPO/main/config.toml" -o "$CFG" \
    && ok "Wrote default config to $CFG" \
    || printf '\033[33m⚠ could not fetch default config; create %s yourself\033[0m\n' "$CFG"
else
  ok "Kept existing config $CFG"
fi

# ── 6. mDNS so the Pi is reachable as <hostname>.local ────────────────────────
if ! command -v avahi-daemon >/dev/null 2>&1; then
  say "Installing avahi (for rod.local)…"
  apt-get update -qq && apt-get install -y -qq avahi-daemon >/dev/null 2>&1 || true
fi
if [ -n "$HOSTNAME_WANT" ] && [ "$(hostname)" != "$HOSTNAME_WANT" ]; then
  hostnamectl set-hostname "$HOSTNAME_WANT" 2>/dev/null \
    && ok "Hostname set to $HOSTNAME_WANT (reachable as $HOSTNAME_WANT.local)" || true
fi

# ── 7. systemd service ────────────────────────────────────────────────────────
say "Installing the systemd service…"
cat > "$UNIT" <<UNIT
[Unit]
Description=rod: IAI actuator -> Handy FW4 BLE bridge
Documentation=https://github.com/$REPO
After=bluetooth.service
Wants=bluetooth.service

[Service]
Type=simple
User=$run_user
SupplementaryGroups=dialout bluetooth
# Clear any rfkill soft-block on the radio (runs as root via the leading +).
ExecStartPre=+/usr/sbin/rfkill unblock bluetooth
ExecStart=$BIN $CFG
Restart=on-failure
RestartSec=3
Environment=RUST_LOG=rod=info
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=multi-user.target
UNIT

systemctl daemon-reload
systemctl enable --now rod
ok "Service enabled and started"

cat <<DONE

$(ok "rod is installed and running.")

  Status:   sudo systemctl status rod
  Logs:     journalctl -u rod -f
  Restart:  sudo systemctl restart rod

Open the Rod web app and connect over Bluetooth — the device shows up
as "Rod-…". To update later, just re-run this installer.
DONE
